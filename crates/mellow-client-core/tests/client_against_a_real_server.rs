//! The client core against a real in-process `mellow-server`.

// Test helpers outside `#[test]` functions: failing loudly is the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use mellow_client_core::state::ClientState;
use mellow_client_core::{ClientError, GatewayUpdate, InstanceAddress, RestClient, run_gateway};
use mellow_protocol::{
    ApiErrorCode, CreateInviteRequest, GatewayEvent, LoginRequest, MessageHistoryQuery,
    RegisterRequest, SendMessageRequest,
};
use mellow_server::commands::{InitializeOptions, initialize_instance, serve_instance};
use mellow_server::domain::account_rules::RegistrationMode;
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;

const PASSWORD: &str = "correct horse battery staple";
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(10);

struct RunningInstance {
    _directory: tempfile::TempDir,
    address: InstanceAddress,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl RunningInstance {
    async fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        // shortcut: the port is free when probed but could be taken before the server binds it.
        let listen_address = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let config = initialize_instance(&InitializeOptions {
            config_path: directory.path().join("mellow.toml"),
            data_directory: directory.path().join("data"),
            master_key_file: directory.path().join("master.key"),
            listen_address,
            registration_mode: RegistrationMode::Open,
            tls: None,
        })
        .await
        .unwrap();
        let (stop, stopped) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            serve_instance(&config, async {
                let _ = stopped.await;
            })
            .await
        });
        let address = InstanceAddress::parse(&format!("http://{listen_address}")).unwrap();
        for _ in 0..100 {
            if tokio::net::TcpStream::connect(listen_address).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Self {
            _directory: directory,
            address,
            stop,
            server,
        }
    }

    /// Waits for the database to close: SQLCipher crashes if it is still closing at exit.
    async fn stop(self) {
        let _ = self.stop.send(());
        self.server.await.unwrap().unwrap();
    }

    async fn registered(&self, username: &str) -> RestClient {
        let mut client = RestClient::new(self.address.clone()).unwrap();
        client
            .register(&RegisterRequest {
                username: username.into(),
                display_name: None,
                email: format!("{username}@example.org"),
                password: PASSWORD.into(),
                device_name: "test".into(),
            })
            .await
            .unwrap();
        client
    }
}

async fn next_update(updates: &mut mpsc::Receiver<GatewayUpdate>) -> GatewayUpdate {
    tokio::time::timeout(RECEIVE_TIMEOUT, updates.recv())
        .await
        .unwrap()
        .unwrap()
}

/// Skips presence and typing noise.
async fn next_message_event(updates: &mut mpsc::Receiver<GatewayUpdate>) -> GatewayEvent {
    loop {
        if let GatewayUpdate::Event(
            event @ (GatewayEvent::MessageCreate(_) | GatewayEvent::MessageDelete { .. }),
        ) = next_update(updates).await
        {
            return event;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn rest_and_gateway_keep_the_state_in_sync_until_logout() {
    let instance = RunningInstance::start().await;
    let owner = instance.registered("owner").await;
    let guild = owner.create_guild("Club").await.unwrap();
    let general = guild
        .channels
        .iter()
        .find(|channel| channel.name == "general")
        .unwrap()
        .id;
    let invite = owner
        .create_invite(guild.id, &CreateInviteRequest::default())
        .await
        .unwrap();

    let mut member = RestClient::new(instance.address.clone()).unwrap();
    let wrong_password = member
        .login(&LoginRequest {
            login: "nobody".into(),
            password: PASSWORD.into(),
            device_name: "test".into(),
        })
        .await;
    assert!(matches!(
        wrong_password,
        Err(ClientError::Api {
            code: ApiErrorCode::InvalidCredentials,
            ..
        })
    ));
    let mut member = instance.registered("member").await;
    member.join_with_invite(&invite.code).await.unwrap();
    assert!(matches!(
        member.join_with_invite("../users/@me").await,
        Err(ClientError::Api {
            code: ApiErrorCode::InvalidInvite,
            ..
        })
    ));

    let (sender, mut updates) = mpsc::channel(64);
    let gateway = tokio::spawn(run_gateway(
        instance.address.clone(),
        Zeroizing::new(member.token().unwrap().to_owned()),
        sender,
    ));
    let mut state = ClientState::default();
    let GatewayUpdate::Ready(ready) = next_update(&mut updates).await else {
        panic!("the first update is Ready");
    };
    state.apply_ready(*ready);
    assert_eq!(state.guilds().len(), 1);

    let sent = owner
        .send_message(
            general,
            &SendMessageRequest {
                content: "hello **member**".into(),
                reply_to_id: None,
                nonce: None,
            },
        )
        .await
        .unwrap();
    let event = next_message_event(&mut updates).await;
    assert_eq!(event, GatewayEvent::MessageCreate(sent.clone()));
    state.apply_event(event, 0);
    assert_eq!(state.messages(general), std::slice::from_ref(&sent));
    assert!(state.has_unread(general));

    let history = member
        .list_messages(general, &MessageHistoryQuery::default())
        .await
        .unwrap();
    assert_eq!(history, vec![sent.clone()]);
    member.acknowledge(general, sent.id).await.unwrap();
    member.start_typing(general).await.unwrap();
    assert_eq!(member.list_members(guild.id).await.unwrap().len(), 2);

    // Logging out revokes the token: the live session ends and identifying again is refused.
    member.logout().await.unwrap();
    assert!(matches!(
        member.current_user().await,
        Err(ClientError::NotSignedIn)
    ));
    loop {
        match next_update(&mut updates).await {
            GatewayUpdate::SignedOut => break,
            GatewayUpdate::Event(_) | GatewayUpdate::Disconnected => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    tokio::time::timeout(RECEIVE_TIMEOUT, gateway)
        .await
        .unwrap()
        .unwrap();
    instance.stop().await;
}
