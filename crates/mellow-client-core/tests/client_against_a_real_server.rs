//! The client core against a real in-process `mellow-server`.

// The server refuses Windows until its master key file can be protected there (ADR-0006).
#![cfg(not(windows))]
// Test helpers outside `#[test]` functions: failing loudly is the point.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use mellow_client_core::state::ClientState;
use mellow_client_core::{
    ClientError, GatewayUpdate, InstanceAddress, RestClient, generate_message_nonce, run_gateway,
};
use mellow_protocol::{
    ApiErrorCode, AuditAction, AuditLogQuery, ChangePasswordRequest, ChannelKind, CreateBanRequest,
    CreateChannelRequest, CreateInviteRequest, CreateRoleRequest, EditMessageRequest, GatewayEvent,
    LoginRequest, Message, MessageHistoryQuery, OverwriteKind, Permissions, RegisterRequest,
    ReorderRolesRequest, SendMessageRequest, SetPermissionOverwriteRequest, Snowflake,
    UpdateChannelRequest, UpdateCurrentUserRequest, UpdateRoleRequest,
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
            event @ (GatewayEvent::MessageCreate(_)
            | GatewayEvent::MessageUpdate(_)
            | GatewayEvent::MessageDelete { .. }),
        ) = next_update(updates).await
        {
            return event;
        }
    }
}

/// The member replies, the owner edits (after the member is refused), the member deletes its reply.
async fn replies_edits_and_deletes_reach_the_gateway(
    owner: &RestClient,
    member: &RestClient,
    general: Snowflake,
    sent: &Message,
    updates: &mut mpsc::Receiver<GatewayUpdate>,
) {
    let reply = member
        .send_message(
            general,
            &SendMessageRequest {
                content: "hi owner".into(),
                reply_to_id: Some(sent.id),
                nonce: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(reply.reply_to_id, Some(sent.id));
    assert_eq!(
        next_message_event(updates).await,
        GatewayEvent::MessageCreate(reply.clone())
    );
    let edit = EditMessageRequest {
        content: "hello again".into(),
    };
    assert!(matches!(
        member.edit_message(general, sent.id, &edit).await,
        Err(ClientError::Api {
            code: ApiErrorCode::MissingPermissions,
            ..
        })
    ));
    let edited = owner.edit_message(general, sent.id, &edit).await.unwrap();
    assert_eq!(edited.content, "hello again");
    assert!(edited.edited_at_unix_milliseconds.is_some());
    assert_eq!(
        next_message_event(updates).await,
        GatewayEvent::MessageUpdate(edited)
    );
    member.delete_message(general, reply.id).await.unwrap();
    assert!(matches!(
        next_message_event(updates).await,
        GatewayEvent::MessageDelete { id, .. } if id == reply.id
    ));
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

    replies_edits_and_deletes_reach_the_gateway(&owner, &member, general, &sent, &mut updates)
        .await;
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

#[tokio::test(flavor = "multi_thread")]
async fn retrying_a_send_whose_answer_was_lost_keeps_one_message() {
    let instance = RunningInstance::start().await;
    let owner = instance.registered("owner").await;
    let guild = owner.create_guild("Club").await.unwrap();
    let general = guild
        .channels
        .iter()
        .find(|channel| channel.name == "general")
        .unwrap()
        .id;
    let mut state = ClientState::default();

    let nonce = generate_message_nonce().unwrap();
    let request = state.queue_message(general, "hello".into(), None, nonce.clone());
    let first = owner.send_message(general, &request).await.unwrap();
    // The answer never reached the app: the user retries.
    state.mark_send_failed(&nonce);
    let (_, retried) = state.retry_pending_message(&nonce).unwrap();
    let second = owner.send_message(general, &retried).await.unwrap();
    assert_eq!(second.id, first.id);
    state.confirm_sent_message(&nonce, second);

    let history = owner
        .list_messages(general, &MessageHistoryQuery::default())
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(state.messages(general).len(), 1);
    assert_eq!(state.pending_messages(general).count(), 0);
    instance.stop().await;
}

fn is_refused_with(
    result: &Result<impl std::fmt::Debug, ClientError>,
    expected: ApiErrorCode,
) -> bool {
    matches!(result, Err(ClientError::Api { code, .. }) if *code == expected)
}

/// Every server-settings call of the client core, from the owner, with the member refused.
#[tokio::test(flavor = "multi_thread")]
async fn guild_administration_round_trips_through_the_rest_client() {
    let instance = RunningInstance::start().await;
    let owner = instance.registered("owner").await;
    let member = instance.registered("member").await;
    let guild = owner.create_guild("Moderated").await.unwrap();
    let invite = owner
        .create_invite(guild.id, &CreateInviteRequest::default())
        .await
        .unwrap();
    member.join_with_invite(&invite.code).await.unwrap();
    let general = guild
        .channels
        .iter()
        .find(|channel| channel.parent_id.is_some())
        .unwrap()
        .id;
    roles_and_overwrites_round_trip(&owner, &member, guild.id, general).await;
    moderation_round_trips(&owner, &member, guild.id, &invite.code).await;
    let temporary = owner
        .create_channel(
            guild.id,
            &CreateChannelRequest {
                kind: ChannelKind::Text,
                name: "temporary".into(),
                topic: String::new(),
                parent_id: None,
            },
        )
        .await
        .unwrap();
    owner.delete_channel(temporary.id).await.unwrap();
    instance.stop().await;
}

async fn roles_and_overwrites_round_trip(
    owner: &RestClient,
    member: &RestClient,
    guild_id: Snowflake,
    general: Snowflake,
) {
    let member_id = member.current_user().await.unwrap().id;
    let moderator = owner
        .create_role(
            guild_id,
            &CreateRoleRequest {
                name: "Moderator".into(),
                color: 0x0000_aaff,
                permissions: Permissions::KICK_MEMBERS,
            },
        )
        .await
        .unwrap();
    let helper_role = owner
        .create_role(
            guild_id,
            &CreateRoleRequest {
                name: "Helper".into(),
                color: 0,
                permissions: Permissions::empty(),
            },
        )
        .await
        .unwrap();
    let renamed = owner
        .update_role(
            guild_id,
            moderator.id,
            &UpdateRoleRequest {
                name: Some("Mods".into()),
                ..UpdateRoleRequest::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(renamed.name, "Mods");
    let reordered = owner
        .reorder_roles(
            guild_id,
            &ReorderRolesRequest {
                role_ids: vec![moderator.id, helper_role.id],
            },
        )
        .await
        .unwrap();
    let names: Vec<_> = reordered.iter().map(|role| role.name.as_str()).collect();
    assert_eq!(names, ["@everyone", "Mods", "Helper"]);
    owner
        .add_member_role(guild_id, member_id, helper_role.id)
        .await
        .unwrap();
    assert_eq!(
        member.guild(guild_id).await.unwrap().my_role_ids,
        [helper_role.id]
    );
    owner
        .remove_member_role(guild_id, member_id, helper_role.id)
        .await
        .unwrap();
    owner.delete_role(guild_id, helper_role.id).await.unwrap();

    let read_only = owner
        .set_permission_overwrite(
            general,
            guild_id,
            &SetPermissionOverwriteRequest {
                kind: OverwriteKind::Role,
                allow: Permissions::empty(),
                deny: Permissions::SEND_MESSAGES,
            },
        )
        .await
        .unwrap();
    assert_eq!(read_only.permission_overwrites.len(), 1);
    let cleared = owner
        .delete_permission_overwrite(general, guild_id)
        .await
        .unwrap();
    assert_eq!(cleared.permission_overwrites, []);
    let with_topic = owner
        .update_channel(
            general,
            &UpdateChannelRequest {
                name: None,
                topic: Some("Rules first".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(with_topic.topic, "Rules first");
}

/// Kick, ban and unban, then the audit log pages they wrote.
async fn moderation_round_trips(
    owner: &RestClient,
    member: &RestClient,
    guild_id: Snowflake,
    invite_code: &str,
) {
    let member_id = member.current_user().await.unwrap().id;
    let owner_id = owner.current_user().await.unwrap().id;
    assert!(is_refused_with(
        &member.kick_member(guild_id, owner_id).await,
        ApiErrorCode::MissingPermissions
    ));
    assert!(is_refused_with(
        &member.list_bans(guild_id).await,
        ApiErrorCode::MissingPermissions
    ));
    assert!(is_refused_with(
        &owner.leave_guild(guild_id).await,
        ApiErrorCode::OwnerCannotLeave
    ));
    owner.kick_member(guild_id, member_id).await.unwrap();
    owner
        .ban_member(
            guild_id,
            member_id,
            &CreateBanRequest {
                reason: "spam".into(),
            },
        )
        .await
        .unwrap();
    let bans = owner.list_bans(guild_id).await.unwrap();
    assert_eq!(bans.len(), 1);
    assert_eq!(
        (bans[0].user.id, bans[0].reason.as_str()),
        (member_id, "spam")
    );
    assert!(is_refused_with(
        &member.join_with_invite(invite_code).await,
        ApiErrorCode::Banned
    ));
    owner.unban_member(guild_id, member_id).await.unwrap();
    member.join_with_invite(invite_code).await.unwrap();
    member.leave_guild(guild_id).await.unwrap();

    let newest = owner
        .list_audit_log(
            guild_id,
            &AuditLogQuery {
                before: None,
                limit: Some(3),
            },
        )
        .await
        .unwrap();
    let actions: Vec<_> = newest.iter().map(|entry| entry.action).collect();
    assert_eq!(
        actions,
        [
            AuditAction::MemberBanRemove,
            AuditAction::MemberBanAdd,
            AuditAction::MemberKick
        ]
    );
    let older = owner
        .list_audit_log(
            guild_id,
            &AuditLogQuery {
                before: newest.last().map(|entry| entry.id),
                limit: Some(1),
            },
        )
        .await
        .unwrap();
    assert_eq!(older[0].action, AuditAction::ChannelUpdate);
}

/// A rename reaches the other members through the gateway; a password change signs out the
/// other devices only.
#[tokio::test(flavor = "multi_thread")]
async fn account_settings_round_trip_through_the_rest_client() {
    let instance = RunningInstance::start().await;
    let owner = instance.registered("owner").await;
    let member = instance.registered("member").await;
    let guild = owner.create_guild("Friends").await.unwrap();
    let invite = owner
        .create_invite(guild.id, &CreateInviteRequest::default())
        .await
        .unwrap();
    member.join_with_invite(&invite.code).await.unwrap();
    let (sender, mut updates) = mpsc::channel(64);
    let gateway = tokio::spawn(run_gateway(
        instance.address.clone(),
        Zeroizing::new(member.token().unwrap().to_owned()),
        sender,
    ));
    let GatewayUpdate::Ready(_) = next_update(&mut updates).await else {
        panic!("the first update is Ready");
    };

    let taken = owner
        .update_current_user(&UpdateCurrentUserRequest {
            username: Some("member".into()),
            display_name: None,
        })
        .await;
    assert!(is_refused_with(&taken, ApiErrorCode::UsernameTaken));
    let renamed = owner
        .update_current_user(&UpdateCurrentUserRequest {
            username: Some("the.owner".into()),
            display_name: Some("The Owner".into()),
        })
        .await
        .unwrap();
    assert_eq!(owner.current_user().await.unwrap(), renamed);
    let user_update = loop {
        if let GatewayUpdate::Event(GatewayEvent::UserUpdate(user)) =
            next_update(&mut updates).await
        {
            break user;
        }
    };
    assert_eq!(
        (user_update.id, user_update.display_name.as_str()),
        (renamed.id, "The Owner")
    );
    gateway.abort();

    let mut other_device = RestClient::new(instance.address.clone()).unwrap();
    let login = |password: &str| LoginRequest {
        login: "the.owner".into(),
        password: password.into(),
        device_name: "other".into(),
    };
    other_device.login(&login(PASSWORD)).await.unwrap();
    let wrong_current = owner
        .change_password(&ChangePasswordRequest {
            current_password: "not the password".into(),
            new_password: "a brand new password".into(),
        })
        .await;
    assert!(is_refused_with(
        &wrong_current,
        ApiErrorCode::InvalidCredentials
    ));
    owner
        .change_password(&ChangePasswordRequest {
            current_password: PASSWORD.into(),
            new_password: "a brand new password".into(),
        })
        .await
        .unwrap();
    assert!(owner.current_user().await.is_ok());
    assert!(
        other_device
            .current_user()
            .await
            .is_err_and(|error| error.needs_new_login())
    );
    let mut new_device = RestClient::new(instance.address.clone()).unwrap();
    new_device
        .login(&login("a brand new password"))
        .await
        .unwrap();
    instance.stop().await;
}
