//! The desktop's voice against a real in-process `mellow-server`: two signed-in sessions join a
//! voice channel through their `Voice` entities, with synthetic devices (no real microphone).

// The server refuses Windows until its master key file can be protected there (ADR-0006).
#![cfg(not(windows))]

use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext};
use mellow_client_core::{InstanceAddress, RestClient};
use mellow_media::{StreamFormat, SyntheticAudioDevices, SyntheticSound};
use mellow_protocol::{
    ChannelKind, CreateChannelRequest, CreateInviteRequest, RegisterRequest, Snowflake,
};
use mellow_server::commands::{InitializeOptions, initialize_instance, serve_instance_on_listener};
use mellow_server::config::VoiceConfig;
use mellow_server::domain::account_rules::{GuildCreationPolicy, RegistrationMode};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

use super::*;
use crate::session::ConnectionStatus;
use crate::strings_pending_i18n::{ActiveStrings, ENGLISH};
use crate::tokio_bridge::{build_network_runtime, install_network_runtime};

const PASSWORD: &str = "correct horse battery staple";
/// Generous: the engines run in real time, and a loaded CI machine is slow to connect. Never a
/// measure.
const STEP_DEADLINE: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

struct RunningInstance {
    _directory: tempfile::TempDir,
    address: InstanceAddress,
    stop: oneshot::Sender<()>,
    server: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl RunningInstance {
    async fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_address = listener.local_addr().unwrap();
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
            serve_instance_on_listener(&config, listener, async {
                let _ = stopped.await;
            })
            .await
        });
        Self {
            _directory: directory,
            address: InstanceAddress::parse(&format!("http://{listen_address}")).unwrap(),
            stop,
            server,
        }
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

    /// Waits for the database to close: SQLCipher crashes if it is still closing at exit.
    async fn stop(self) {
        let _ = self.stop.send(());
        self.server.await.unwrap().unwrap();
    }
}

/// A voice channel both share.
async fn shared_voice_channel(owner: &RestClient, member: &RestClient) -> Snowflake {
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

fn synthetic_devices(sound: SyntheticSound) -> AudioDevicesFactory {
    Arc::new(move || {
        Box::new(SyntheticAudioDevices::new(
            sound,
            StreamFormat {
                sample_rate: 48_000,
                channels: 1,
            },
            StreamFormat {
                sample_rate: 44_100,
                channels: 2,
            },
        ))
    })
}

fn wait_until(
    cx: &mut TestAppContext,
    what: &str,
    mut condition: impl FnMut(&mut TestAppContext) -> bool,
) {
    let deadline = Instant::now() + STEP_DEADLINE;
    loop {
        cx.run_until_parked();
        if condition(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "{what} did not happen in time");
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// A signed-in session kept in memory only, and its voice with synthetic devices.
fn signed_in(
    rest: RestClient,
    sound: SyntheticSound,
    cx: &mut TestAppContext,
) -> (Entity<Session>, Entity<Voice>) {
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, cx| session.begin_signed_in_session(rest, cx));
    let voice = cx.new(|cx| Voice::new(session.clone(), synthetic_devices(sound), cx));
    wait_until(cx, "the gateway's Ready", |cx| {
        session.read_with(cx, |session, _| {
            session.status() == ConnectionStatus::Connected
        })
    });
    (session, voice)
}

fn own_voice_state(
    session: &Entity<Session>,
    cx: &mut TestAppContext,
) -> Option<mellow_protocol::VoiceState> {
    session.read_with(cx, |session, _| {
        let state = session.state();
        state
            .current_user()
            .and_then(|user| state.voice_state_of(user.id))
            .cloned()
    })
}

#[gpui::test]
fn two_people_join_hear_of_each_other_mute_and_leave(cx: &mut TestAppContext) {
    let runtime: Runtime = build_network_runtime().unwrap();
    let instance = runtime.block_on(RunningInstance::start());
    // Answers come from Tokio and the engines' threads, outside GPUI's deterministic scheduler.
    cx.executor().allow_parking();
    let handle = runtime.handle().clone();
    cx.update(|cx| {
        install_network_runtime(handle, cx);
        cx.set_global(ActiveStrings(&ENGLISH));
    });
    let (owner, friend, channel_id) = runtime.block_on(async {
        let owner = instance.registered("owner").await;
        let friend = instance.registered("friend").await;
        let channel_id = shared_voice_channel(&owner, &friend).await;
        (owner, friend, channel_id)
    });
    let (owner_session, owner_voice) = signed_in(owner, SyntheticSound::Silence, cx);
    let (friend_session, friend_voice) = signed_in(
        friend,
        SyntheticSound::Sine {
            frequency: 440.0,
            amplitude: 0.3,
        },
        cx,
    );
    wait_until(cx, "the voice channel in the owner's state", |cx| {
        owner_session.read_with(cx, |session, _| {
            session.state().guild_channel(channel_id).is_some()
        })
    });
    // The shell asks for the open server's members; here the test does, for the names.
    let guild_id = owner_session.read_with(cx, |session, _| {
        session
            .state()
            .guild_of_channel(channel_id)
            .map(|guild| guild.id)
            .unwrap()
    });
    owner_session.update(cx, |session, cx| session.load_members(guild_id, false, cx));
    wait_until(cx, "the owner's member list", |cx| {
        owner_session.read_with(cx, |session, _| {
            session.state().members(guild_id).len() == 2
        })
    });

    owner_voice.update(cx, |voice, cx| voice.join(channel_id, cx));
    assert_eq!(
        owner_voice.read_with(cx, |voice, _| voice.phase()),
        Some(CallPhase::Connecting)
    );
    wait_until(cx, "the owner's connection", |cx| {
        owner_voice.read_with(cx, |voice, _| voice.phase() == Some(CallPhase::Connected))
            && own_voice_state(&owner_session, cx)
                .is_some_and(|state| state.channel_id == channel_id)
    });

    friend_voice.update(cx, |voice, cx| voice.join(channel_id, cx));
    wait_until(cx, "the friend's arrival, announced to the owner", |cx| {
        owner_voice.read_with(cx, |voice, _| {
            voice.announcement() == "friend joined Lounge."
        })
    });
    wait_until(
        cx,
        "the friend's voice, heard by the owner's instance",
        |cx| {
            let friend_id = friend_session.read_with(cx, |session, _| {
                session.state().current_user().map(|user| user.id)
            });
            owner_session.read_with(cx, |session, _| {
                friend_id.is_some_and(|id| session.state().is_speaking(id))
            })
        },
    );

    owner_voice.update(cx, Voice::toggle_microphone);
    wait_until(cx, "the owner's mute, seen by the instance", |cx| {
        own_voice_state(&owner_session, cx).is_some_and(|state| state.self_mute && !state.self_deaf)
    });
    owner_voice.update(cx, Voice::toggle_deafened);
    wait_until(cx, "the owner's deafen", |cx| {
        own_voice_state(&owner_session, cx).is_some_and(|state| state.self_mute && state.self_deaf)
    });

    friend_voice.update(cx, Voice::leave);
    assert_eq!(friend_voice.read_with(cx, |voice, _| voice.phase()), None);
    wait_until(cx, "the friend's leave, announced to the owner", |cx| {
        owner_voice.read_with(cx, |voice, _| voice.announcement() == "friend left Lounge.")
    });
    owner_voice.update(cx, Voice::leave);
    wait_until(cx, "the owner's leave, seen by the instance", |cx| {
        own_voice_state(&owner_session, cx).is_none()
    });

    drop((owner_voice, friend_voice, owner_session, friend_session));
    cx.run_until_parked();
    runtime.block_on(instance.stop());
}

#[gpui::test]
fn a_channel_the_state_does_not_know_joins_nothing(cx: &mut TestAppContext) {
    let runtime: Runtime = build_network_runtime().unwrap();
    let instance = runtime.block_on(RunningInstance::start());
    cx.executor().allow_parking();
    let handle = runtime.handle().clone();
    cx.update(|cx| {
        install_network_runtime(handle, cx);
        cx.set_global(ActiveStrings(&ENGLISH));
    });
    let owner = runtime.block_on(instance.registered("owner"));
    let (session, voice) = signed_in(owner, SyntheticSound::Silence, cx);
    // An unknown channel joins nothing.
    voice.update(cx, |voice, cx| voice.join(Snowflake::from_raw(1), cx));
    assert_eq!(voice.read_with(cx, |voice, _| voice.phase()), None);
    drop((voice, session));
    cx.run_until_parked();
    runtime.block_on(instance.stop());
}

async fn create_voice_channel(owner: &RestClient, guild_id: Snowflake, name: &str) -> Snowflake {
    owner
        .create_channel(
            guild_id,
            &CreateChannelRequest {
                kind: ChannelKind::Voice,
                name: name.into(),
                topic: String::new(),
                parent_id: None,
                user_limit: None,
            },
        )
        .await
        .unwrap()
        .id
}

/// One signed-in owner and two voice channels of their server.
fn owner_with_two_voice_channels(
    runtime: &Runtime,
    instance: &RunningInstance,
    cx: &mut TestAppContext,
) -> (Entity<Session>, Entity<Voice>, Snowflake, Snowflake) {
    cx.executor().allow_parking();
    let handle = runtime.handle().clone();
    cx.update(|cx| {
        install_network_runtime(handle, cx);
        cx.set_global(ActiveStrings(&ENGLISH));
    });
    let (owner, first, second) = runtime.block_on(async {
        let owner = instance.registered("owner").await;
        let guild = owner.create_guild("Club").await.unwrap();
        let first = create_voice_channel(&owner, guild.id, "Lounge").await;
        let second = create_voice_channel(&owner, guild.id, "Studio").await;
        (owner, first, second)
    });
    let (session, voice) = signed_in(owner, SyntheticSound::Silence, cx);
    wait_until(cx, "both voice channels in the state", |cx| {
        session.read_with(cx, |session, _| {
            let state = session.state();
            state.guild_channel(first).is_some() && state.guild_channel(second).is_some()
        })
    });
    (session, voice, first, second)
}

#[gpui::test]
fn a_mute_and_push_to_talk_chosen_while_connecting_reach_the_engine(cx: &mut TestAppContext) {
    let runtime: Runtime = build_network_runtime().unwrap();
    let instance = runtime.block_on(RunningInstance::start());
    let (session, voice, channel_id, _) = owner_with_two_voice_channels(&runtime, &instance, cx);

    voice.update(cx, |voice, cx| voice.join(channel_id, cx));
    assert!(voice.read_with(cx, |voice, _| voice.engine().is_none()));
    voice.update(cx, Voice::toggle_microphone);
    cx.update(|cx| {
        change_voice_settings(cx, |settings| {
            settings.input_mode = VoiceInputMode::PushToTalk;
            settings.push_to_talk_key = Some(
                mellow_client_core::shared_logic::voice_settings::PushToTalkKey {
                    code: "Space".into(),
                    label: "Space".into(),
                },
            );
        });
    });
    wait_until(cx, "the call, muted as the interface says", |cx| {
        voice.read_with(cx, |voice, _| voice.phase() == Some(CallPhase::Connected))
            && own_voice_state(&session, cx)
                .is_some_and(|state| state.channel_id == channel_id && state.self_mute)
    });
    // Space held for push-to-talk does not also type a space.
    let should_swallow = voice.update(cx, |voice, cx| {
        voice.push_to_talk_key_down("space", Some(" "), Modifiers::default(), false, cx)
    });
    assert!(should_swallow);
    voice.update(cx, |voice, cx| voice.push_to_talk_key_up("space", cx));
    assert!(!voice.read_with(cx, |voice, _| voice.is_push_to_talk_pressed));

    // Heard by the global listener and the window at once, then the window goes to the back:
    // the key stays held until the listener hears it come up.
    voice.update(cx, |voice, cx| {
        voice.global_push_to_talk_key_changed(true, cx);
        voice.push_to_talk_key_down("space", Some(" "), Modifiers::default(), false, cx);
        voice.window_deactivated(cx);
    });
    assert!(voice.read_with(cx, |voice, _| voice.is_push_to_talk_pressed));
    voice.update(cx, |voice, cx| {
        voice.global_push_to_talk_key_changed(false, cx);
    });
    assert!(!voice.read_with(cx, |voice, _| voice.is_push_to_talk_pressed));

    voice.update(cx, Voice::leave);
    drop((voice, session));
    cx.run_until_parked();
    runtime.block_on(instance.stop());
}

#[gpui::test]
fn another_channel_chosen_while_connecting_retargets_the_start(cx: &mut TestAppContext) {
    let runtime: Runtime = build_network_runtime().unwrap();
    let instance = runtime.block_on(RunningInstance::start());
    let (session, voice, first, second) = owner_with_two_voice_channels(&runtime, &instance, cx);

    voice.update(cx, |voice, cx| voice.join(first, cx));
    voice.update(cx, |voice, cx| voice.join(second, cx));
    wait_until(cx, "the call in the second channel", |cx| {
        voice.read_with(cx, |voice, _| {
            voice.phase() == Some(CallPhase::Connected) && voice.channel_id() == Some(second)
        }) && own_voice_state(&session, cx).is_some_and(|state| state.channel_id == second)
    });

    // Left and joined again while starting: the same start serves, no stale leave follows.
    voice.update(cx, Voice::leave);
    wait_until(cx, "the leave, seen by the instance", |cx| {
        own_voice_state(&session, cx).is_none()
    });
    voice.update(cx, |voice, cx| voice.join(first, cx));
    voice.update(cx, Voice::leave);
    assert_eq!(voice.read_with(cx, |voice, _| voice.phase()), None);
    voice.update(cx, |voice, cx| voice.join(first, cx));
    wait_until(cx, "the call in the first channel", |cx| {
        voice.read_with(cx, |voice, _| voice.phase() == Some(CallPhase::Connected))
            && own_voice_state(&session, cx).is_some_and(|state| state.channel_id == first)
    });
    assert!(voice.read_with(cx, |voice, _| voice.abandoned_start.is_none()));

    voice.update(cx, Voice::leave);
    drop((voice, session));
    cx.run_until_parked();
    runtime.block_on(instance.stop());
}
