//! What the real-server tests share: an in-process instance, gateway connections and voice
//! participants with synthetic devices.

// Each test binary uses its own part of these helpers.
#![allow(dead_code)]

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt as _, StreamExt as _};
use mellow_client_core::{InstanceAddress, RestClient};
use mellow_media::{
    InMemoryPersonVoiceSettings, NoiseSuppressionMode, PlaybackProbe, ProcessingSettings,
    StreamFormat, SyntheticAudioDevices, SyntheticSound, VoiceSession, VoiceSessionEvent,
    VoiceSessionOptions,
};
use mellow_protocol::{
    ChannelKind, ClientFrame, CreateChannelRequest, CreateInviteRequest, GatewayEvent,
    RegisterRequest, ServerFrame, Snowflake,
};
use mellow_server::commands::{InitializeOptions, initialize_instance, serve_instance};
use mellow_server::config::VoiceConfig;
use mellow_server::domain::account_rules::{GuildCreationPolicy, RegistrationMode};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

pub const PASSWORD: &str = "correct horse battery staple";
/// Generous: the engines run in real time, and a loaded CI machine is slow to connect.
pub const HEARING_TIMEOUT: Duration = Duration::from_secs(30);
pub const LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;

pub struct RunningInstance {
    _directory: tempfile::TempDir,
    pub listen_address: SocketAddr,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl RunningInstance {
    pub async fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        // shortcut: the port is free when probed but could be taken before the server binds it.
        let listen_address = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let mut config = initialize_instance(&InitializeOptions {
            config_path: directory.path().join("mellow.toml"),
            data_directory: directory.path().join("data"),
            master_key_file: directory.path().join("master.key"),
            listen_address,
            registration_mode: RegistrationMode::Open,
            tls: None,
        })
        .await
        .unwrap();
        config.guild_creation = GuildCreationPolicy::Everyone;
        config.voice = VoiceConfig {
            udp_port: 0,
            public_address: Some("127.0.0.1".into()),
            ..VoiceConfig::default()
        };
        let (stop, stopped) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            serve_instance(&config, async {
                let _ = stopped.await;
            })
            .await
        });
        for _ in 0..100 {
            if tokio::net::TcpStream::connect(listen_address).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Self {
            _directory: directory,
            listen_address,
            stop,
            server,
        }
    }

    /// Waits for the database to close: SQLCipher crashes if it is still closing at exit.
    pub async fn stop(self) {
        let _ = self.stop.send(());
        self.server.await.unwrap().unwrap();
    }

    pub async fn registered(&self, username: &str) -> (RestClient, Snowflake) {
        let address = InstanceAddress::parse(&format!("http://{}", self.listen_address)).unwrap();
        let mut client = RestClient::new(address).unwrap();
        let session = client
            .register(&RegisterRequest {
                username: username.into(),
                display_name: None,
                email: format!("{username}@example.org"),
                password: PASSWORD.into(),
                device_name: "test".into(),
            })
            .await
            .unwrap();
        (client, session.user.id)
    }
}

/// A gateway connection run by a task: frames sent on `outgoing` go out, dispatched events come
/// out of `events`.
pub struct Gateway {
    pub outgoing: mpsc::UnboundedSender<ClientFrame>,
    pub events: mpsc::UnboundedReceiver<GatewayEvent>,
}

pub async fn identified_gateway(address: SocketAddr, token: &str) -> Gateway {
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/gateway?v=1"))
        .await
        .unwrap();
    let heartbeat_interval = match next_frame(&mut socket).await {
        ServerFrame::Hello { d } => Duration::from_millis(d.heartbeat_interval_milliseconds),
        other => panic!("expected hello, got {other:?}"),
    };
    let identify = ClientFrame::Identify {
        token: token.into(),
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&identify).unwrap().into(),
        ))
        .await
        .unwrap();
    assert!(matches!(
        next_frame(&mut socket).await,
        ServerFrame::Ready { .. }
    ));
    let (outgoing, mut outgoing_frames) = mpsc::unbounded_channel::<ClientFrame>();
    let (event_sender, events) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut heartbeat = tokio::time::interval(heartbeat_interval / 2);
        loop {
            let frame = tokio::select! {
                frame = outgoing_frames.recv() => match frame {
                    Some(frame) => frame,
                    None => return,
                },
                _ = heartbeat.tick() => ClientFrame::Heartbeat,
                message = socket.next() => {
                    let Some(Ok(message)) = message else { return };
                    if let Message::Text(text) = message
                        && let ServerFrame::Dispatch { event, .. } =
                            serde_json::from_str(&text).unwrap()
                        && event_sender.send(*event).is_err()
                    {
                        return;
                    }
                    continue;
                }
            };
            let text = serde_json::to_string(&frame).unwrap();
            if socket.send(Message::Text(text.into())).await.is_err() {
                return;
            }
        }
    });
    Gateway { outgoing, events }
}

pub async fn next_frame(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> ServerFrame {
    loop {
        let message = tokio::time::timeout(HEARING_TIMEOUT, socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

/// A participant: its gateway, its voice session, what it played and the events it got.
pub struct Participant {
    pub gateway: Gateway,
    pub session: VoiceSession,
    pub session_events: std::sync::mpsc::Receiver<VoiceSessionEvent>,
    pub probe: Arc<PlaybackProbe>,
    pub seen_session_events: Vec<VoiceSessionEvent>,
}

impl Participant {
    pub fn join(
        gateway: Gateway,
        channel_id: Snowflake,
        own_user_id: Snowflake,
        sound: SyntheticSound,
    ) -> Self {
        let devices = SyntheticAudioDevices::new(
            sound,
            StreamFormat {
                sample_rate: 48_000,
                channels: 1,
            },
            // Another rate and channel count than the engine's, as most sound cards.
            StreamFormat {
                sample_rate: 44_100,
                channels: 2,
            },
        );
        let probe = devices.probe();
        let mut options = VoiceSessionOptions::new(channel_id, own_user_id, LOOPBACK);
        // A steady tone is what noise suppression removes, and gain control would reshape it.
        options.processing = ProcessingSettings {
            noise_suppression: NoiseSuppressionMode::Off,
            is_automatic_gain_on: false,
            ..ProcessingSettings::default()
        };
        let outgoing = gateway.outgoing.clone();
        let (session, session_events) = VoiceSession::join(
            options,
            Box::new(devices),
            Box::new(move |frame| outgoing.send(frame).is_ok()),
            Arc::new(InMemoryPersonVoiceSettings::default()),
        )
        .unwrap();
        Self {
            gateway,
            session,
            session_events,
            probe,
            seen_session_events: Vec::new(),
        }
    }

    /// Feeds the gateway's events to the session; returns them for the test to look at.
    pub fn pump(&mut self) -> Vec<GatewayEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.gateway.events.try_recv() {
            self.session.handle_gateway_event(&event);
            events.push(event);
        }
        self.seen_session_events
            .extend(self.session_events.try_iter());
        events
    }

    /// Pumps the gateway until an event satisfies `is_awaited`; fails after `HEARING_TIMEOUT`.
    pub async fn wait_for_event(&mut self, is_awaited: impl Fn(&GatewayEvent) -> bool) {
        let deadline = Instant::now() + HEARING_TIMEOUT;
        while !self.pump().iter().any(&is_awaited) {
            assert!(
                Instant::now() < deadline,
                "session events: {:?}",
                self.seen_session_events
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Pumps the gateway until the media connection is up.
    pub async fn wait_until_connected(&mut self) {
        let deadline = Instant::now() + HEARING_TIMEOUT;
        while !self
            .seen_session_events
            .contains(&VoiceSessionEvent::Connected)
        {
            assert!(
                Instant::now() < deadline,
                "session events: {:?}",
                self.seen_session_events
            );
            self.pump();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// A guild of `owner` with a voice channel, which `member` joined; the channel's id.
pub async fn shared_voice_channel(owner: &RestClient, member: &RestClient) -> Snowflake {
    let guild = owner.create_guild("Club").await.unwrap();
    let lounge = owner
        .create_channel(
            guild.id,
            &CreateChannelRequest {
                kind: ChannelKind::Voice,
                name: "Lounge".into(),
                topic: String::new(),
                parent_id: None,
                user_limit: None,
            },
        )
        .await
        .unwrap();
    let invite = owner
        .create_invite(guild.id, &CreateInviteRequest::default())
        .await
        .unwrap();
    member.join_with_invite(&invite.code).await.unwrap();
    lounge.id
}
