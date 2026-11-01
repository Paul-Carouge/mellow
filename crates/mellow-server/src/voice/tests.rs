//! Voice through the real gateway and UDP port, with headless `str0m` clients (as in the ADR-0030
//! spike): media really flows, so forwarding, suppression and the events are checked end to end.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mellow_protocol::{
    ChannelKind, ClientFrame, CreateChannelRequest, CreateGuildRequest, CreateInviteRequest,
    GatewayEvent, OverwriteKind, Permissions, SendMessageRequest, SetPermissionOverwriteRequest,
    Snowflake, VoiceDisconnectReason, VoiceJoinRefusal, VoiceState,
};
use str0m::change::{SdpAnswer, SdpPendingOffer};
use str0m::format::Codec;
use str0m::media::{Direction, Frequency, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};

use super::*;
use crate::config::VoiceConfig;
use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::{migrated_database, register_request};
use crate::domain::clock::now_unix_milliseconds;
use crate::domain::session_token::session_token_sha256;
use crate::gateway::tests::{ClientSocket, identified, receive_dispatch, send};
use crate::http::AppState;
use crate::http::tests::test_state;

const FRAME_INTERVAL: Duration = Duration::from_millis(20);
const OPUS_SAMPLES_PER_FRAME: u64 = 960;
const FRAME_BYTES: usize = 40;
/// RFC 6464 level of the frames the bots send: a voice.
const VOICE_LEVEL_DBOV: i8 = -20;
const HEARING_TIMEOUT: Duration = Duration::from_secs(10);
const RECEIVE_SLOTS: usize = 4;

struct Fixture {
    _directory: tempfile::TempDir,
    state: AppState,
    address: SocketAddr,
    tokens: HashMap<&'static str, String>,
    user_ids: HashMap<&'static str, Snowflake>,
    general: Snowflake,
    lounge: Snowflake,
    booth: Snowflake,
}

impl Fixture {
    fn token(&self, name: &str) -> &str {
        &self.tokens[name]
    }

    fn user_id(&self, name: &str) -> Snowflake {
        self.user_ids[name]
    }

    /// SQLCipher must not be closing connections when the test process exits (ADR-0006).
    async fn close(self) {
        self.state.database.close().await;
    }

    async fn deny_in_lounge(&self, name: &str, denied: Permissions) {
        self.state
            .guilds
            .set_permission_overwrite(
                self.user_id("owner"),
                self.lounge,
                self.user_id(name),
                SetPermissionOverwriteRequest {
                    kind: OverwriteKind::Member,
                    allow: Permissions::empty(),
                    deny: denied,
                },
            )
            .await
            .unwrap();
    }
}

/// Real time: the gateway validates sessions against the clock.
fn now() -> u64 {
    now_unix_milliseconds()
}

/// The owner, three members (`muted` lacks SPEAK in the lounge, `outsider` cannot view it), a
/// text channel, the lounge (no limit) and a booth for one person.
async fn fixture() -> Fixture {
    let (directory, database) = migrated_database().await;
    let mut state = test_state(database, RegistrationMode::Open);
    state.voice = VoiceService::start(
        &VoiceConfig {
            enabled: true,
            udp_port: 0,
            public_address: Some("127.0.0.1".into()),
            max_participants: 50,
        },
        false,
        state.guilds.clone(),
        state.events.clone(),
    );
    let mut tokens = HashMap::new();
    let mut user_ids = HashMap::new();
    for name in ["owner", "member", "muted", "outsider", "newcomer"] {
        let registered = state
            .accounts
            .register(
                register_request(name, &format!("{name}@example.org")),
                now(),
            )
            .await
            .unwrap();
        tokens.insert(name, registered.token);
        user_ids.insert(name, registered.user.id);
    }
    let owner = user_ids["owner"];
    let guild = state
        .guilds
        .create_guild(
            owner,
            false,
            CreateGuildRequest {
                name: "Club".into(),
                initial_categories: None,
            },
            now(),
        )
        .await
        .unwrap();
    let invite = state
        .guilds
        .create_invite(owner, guild.id, CreateInviteRequest::default(), now())
        .await
        .unwrap();
    for name in ["member", "muted", "outsider"] {
        state
            .guilds
            .join_with_invite(user_ids[name], &invite.code, now())
            .await
            .unwrap();
    }
    let voice_channel = |name: &str, user_limit| {
        let state = state.clone();
        let name = name.to_owned();
        async move {
            let request = CreateChannelRequest {
                kind: ChannelKind::Voice,
                name,
                topic: String::new(),
                parent_id: None,
                user_limit,
            };
            state
                .guilds
                .create_channel(owner, guild.id, request, now())
                .await
                .unwrap()
                .id
        }
    };
    let lounge = voice_channel("Lounge", None).await;
    let booth = voice_channel("Booth", Some(1)).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(crate::commands::serve_until_shutdown(
        listener,
        state.clone(),
        std::future::pending(),
    ));
    let fixture = Fixture {
        _directory: directory,
        state,
        address,
        tokens,
        user_ids,
        general: guild.channels[1].id,
        lounge,
        booth,
    };
    fixture.deny_in_lounge("muted", Permissions::SPEAK).await;
    fixture
        .deny_in_lounge("outsider", Permissions::VIEW_CHANNEL)
        .await;
    fixture
}

/// A headless participant's side of the WebRTC session, before the answer.
struct PendingBot {
    rtc: Rtc,
    pending: SdpPendingOffer,
    voice_mid: Mid,
    socket: UdpSocket,
    offer: String,
}

fn pending_bot() -> PendingBot {
    str0m::crypto::from_feature_flags().install_process_default();
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut rtc = Rtc::builder()
        .clear_codecs()
        .enable_opus(true, false)
        .build(Instant::now());
    rtc.add_local_candidate(Candidate::host(socket.local_addr().unwrap(), "udp").unwrap());
    let mut change = rtc.sdp_api();
    let voice_mid = change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
    for _ in 0..RECEIVE_SLOTS {
        change.add_media(MediaKind::Audio, Direction::RecvOnly, None, None, None);
    }
    let (offer, pending) = change.apply().unwrap();
    PendingBot {
        rtc,
        pending,
        voice_mid,
        socket,
        offer: offer.to_sdp_string(),
    }
}

/// A running headless participant: sends a frame tagged with `marker` every 20 ms while
/// `is_sending`, and counts the frames it hears by their marker.
struct Bot {
    heard: Arc<Mutex<HashMap<u8, u32>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Bot {
    fn heard_from(&self, marker: u8) -> u32 {
        self.heard
            .lock()
            .unwrap()
            .get(&marker)
            .copied()
            .unwrap_or(0)
    }

    async fn wait_until_hearing(&self, marker: u8) {
        let deadline = Instant::now() + HEARING_TIMEOUT;
        while self.heard_from(marker) == 0 {
            assert!(Instant::now() < deadline, "never heard {marker}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

impl Drop for Bot {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn start_bot(bot: PendingBot, answer: &str, marker: u8) -> Bot {
    let PendingBot {
        mut rtc,
        pending,
        voice_mid,
        socket,
        ..
    } = bot;
    rtc.sdp_api()
        .accept_answer(pending, SdpAnswer::from_sdp_string(answer).unwrap())
        .unwrap();
    let heard = Arc::new(Mutex::new(HashMap::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let heard = heard.clone();
        let stop = stop.clone();
        std::thread::spawn(move || run_bot(rtc, &socket, voice_mid, marker, &heard, &stop))
    };
    Bot {
        heard,
        stop,
        thread: Some(thread),
    }
}

fn run_bot(
    mut rtc: Rtc,
    socket: &UdpSocket,
    voice_mid: Mid,
    marker: u8,
    heard: &Mutex<HashMap<u8, u32>>,
    stop: &AtomicBool,
) {
    let local_address = socket.local_addr().unwrap();
    let mut buffer = vec![0_u8; 2_000];
    let mut is_connected = false;
    let mut next_frame_at = Instant::now();
    let mut rtp_time: u64 = 0;
    while !stop.load(Ordering::Relaxed) {
        let timeout = loop {
            match rtc.poll_output().unwrap() {
                Output::Timeout(at) => break at,
                Output::Transmit(transmit) => {
                    socket
                        .send_to(&transmit.contents, transmit.destination)
                        .unwrap();
                }
                // `Connected`, not the ICE state: DTLS is done and media can flow.
                Output::Event(Event::Connected) => is_connected = true,
                Output::Event(Event::MediaData(frame)) => {
                    if let Some(sender) = frame.data.first() {
                        *heard.lock().unwrap().entry(*sender).or_default() += 1;
                    }
                }
                Output::Event(_) => {}
            }
        };
        let now = Instant::now();
        if is_connected && now >= next_frame_at {
            if let Some(writer) = rtc.writer(voice_mid) {
                let payload_type = writer
                    .payload_params()
                    .find(|params| params.spec().codec == Codec::Opus)
                    .map(str0m::format::PayloadParams::pt)
                    .unwrap();
                writer
                    .audio_level(VOICE_LEVEL_DBOV, true)
                    .write(
                        payload_type,
                        now,
                        MediaTime::new(rtp_time, Frequency::FORTY_EIGHT_KHZ),
                        vec![marker; FRAME_BYTES],
                    )
                    .unwrap();
            }
            rtp_time += OPUS_SAMPLES_PER_FRAME;
            next_frame_at += FRAME_INTERVAL;
            continue;
        }
        let wake_at = if is_connected {
            timeout.min(next_frame_at)
        } else {
            timeout
        };
        let wait = wake_at
            .saturating_duration_since(Instant::now())
            .clamp(Duration::from_millis(1), Duration::from_millis(50));
        socket.set_read_timeout(Some(wait)).unwrap();
        let input = match socket.recv_from(&mut buffer) {
            Ok((length, source)) => Input::Receive(
                Instant::now(),
                Receive {
                    proto: Protocol::Udp,
                    source,
                    destination: local_address,
                    contents: buffer[..length].try_into().unwrap(),
                },
            ),
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                Input::Timeout(Instant::now())
            }
            Err(error) => panic!("bot {marker}: {error}"),
        };
        rtc.handle_input(input).unwrap();
    }
}

async fn send_join(socket: &mut ClientSocket, channel_id: Snowflake, offer: &str) {
    send(
        socket,
        &ClientFrame::VoiceJoin {
            channel_id,
            offer_id: 1,
            sdp_offer: offer.to_owned(),
            self_mute: false,
            self_deaf: false,
        },
    )
    .await;
}

/// Joins `channel_id` and returns the server's SDP answer.
async fn join(socket: &mut ClientSocket, channel_id: Snowflake, offer: &str) -> String {
    send_join(socket, channel_id, offer).await;
    match receive_dispatch(socket, |event| {
        matches!(
            event,
            GatewayEvent::VoiceSessionDescription { .. } | GatewayEvent::VoiceJoinRefused { .. }
        )
    })
    .await
    .1
    {
        GatewayEvent::VoiceSessionDescription {
            sdp_answer,
            offer_id,
            ..
        } => {
            assert_eq!(offer_id, 1, "the answer names the offer it answers");
            sdp_answer
        }
        other => panic!("join refused: {other:?}"),
    }
}

async fn join_and_start(socket: &mut ClientSocket, channel_id: Snowflake, marker: u8) -> Bot {
    let bot = pending_bot();
    let answer = join(socket, channel_id, &bot.offer).await;
    start_bot(bot, &answer, marker)
}

async fn next_voice_state_update(socket: &mut ClientSocket, user_id: Snowflake) -> VoiceState {
    match receive_dispatch(socket, |event| {
        matches!(event, GatewayEvent::VoiceStateUpdate { voice_state } if voice_state.user_id == user_id)
    })
    .await
    .1
    {
        GatewayEvent::VoiceStateUpdate { voice_state } => voice_state,
        other => unreachable!("{other:?}"),
    }
}

async fn wait_for_voice_state_delete(socket: &mut ClientSocket, user_id: Snowflake) {
    receive_dispatch(socket, |event| {
        matches!(event, GatewayEvent::VoiceStateDelete { user_id: deleted, .. } if *deleted == user_id)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn participants_hear_each_other_and_a_member_without_speak_is_suppressed() {
    let fixture = fixture().await;
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (mut member, _) = identified(fixture.address, fixture.token("member")).await;
    let (mut muted, _) = identified(fixture.address, fixture.token("muted")).await;

    let owner_bot = join_and_start(&mut owner, fixture.lounge, 1).await;
    let member_bot = join_and_start(&mut member, fixture.lounge, 2).await;
    let muted_bot = join_and_start(&mut muted, fixture.lounge, 3).await;
    owner_bot.wait_until_hearing(2).await;
    member_bot.wait_until_hearing(1).await;
    muted_bot.wait_until_hearing(1).await;
    muted_bot.wait_until_hearing(2).await;
    // Speaking edges and the muted member's arrival race: read until both were seen.
    let (member_id, muted_id) = (fixture.user_id("member"), fixture.user_id("muted"));
    let (mut muted_state, mut speaking) = (None, None);
    while muted_state.is_none() || speaking.is_none() {
        match receive_dispatch(&mut owner, |_| true).await.1 {
            GatewayEvent::VoiceStateUpdate { voice_state } if voice_state.user_id == muted_id => {
                muted_state = Some(voice_state);
            }
            event @ GatewayEvent::Speaking { user_id, .. } if user_id == member_id => {
                speaking = Some(event);
            }
            _ => {}
        }
    }
    let (muted_state, speaking) = (muted_state.unwrap(), speaking.unwrap());
    // The muted bot has been sending for a while: still nothing of it reached anyone.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let heard_from_muted = (owner_bot.heard_from(3), member_bot.heard_from(3));
    drop((owner_bot, member_bot, muted_bot));
    let lounge = fixture.lounge;
    fixture.close().await;

    assert!(muted_state.suppressed, "{muted_state:?}");
    assert_eq!(muted_state.channel_id, lounge);
    assert!(matches!(speaking, GatewayEvent::Speaking { channel_id, .. } if channel_id == lounge));
    assert_eq!(
        heard_from_muted,
        (0, 0),
        "suppressed audio is not forwarded"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn voice_states_reach_viewers_only_and_leave_or_session_end_remove_them() {
    let fixture = fixture().await;
    let (mut member, _) = identified(fixture.address, fixture.token("member")).await;
    let (mut outsider, _) = identified(fixture.address, fixture.token("outsider")).await;
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let owner_id = fixture.user_id("owner");
    join(&mut owner, fixture.lounge, &pending_bot().offer).await;

    let seen_by_member = next_voice_state_update(&mut member, owner_id).await;
    // A message to everyone after the join: the outsider gets it, and nothing about voice before.
    let sentinel = fixture
        .state
        .messages
        .send(
            owner_id,
            fixture.general,
            SendMessageRequest {
                content: "after the join".into(),
                reply_to_id: None,
                nonce: Some("voice-sentinel".into()),
                attachment_ids: Vec::new(),
            },
            now(),
        )
        .await
        .unwrap();
    let first_for_outsider = receive_dispatch(&mut outsider, |event| {
        matches!(
            event,
            GatewayEvent::MessageCreate(_)
                | GatewayEvent::VoiceStateUpdate { .. }
                | GatewayEvent::Speaking { .. }
        )
    })
    .await
    .1;
    let (_, ready) = identified(fixture.address, fixture.token("member")).await;
    let (_, outsider_ready) = identified(fixture.address, fixture.token("outsider")).await;

    send(&mut member, &ClientFrame::VoiceLeave).await;
    join(&mut member, fixture.lounge, &pending_bot().offer).await;
    send(&mut member, &ClientFrame::VoiceLeave).await;
    wait_for_voice_state_delete(&mut owner, fixture.user_id("member")).await;
    // Ending the owner's session (here: signing out) ends their voice connection.
    let owner_token = session_token_sha256(fixture.token("owner")).unwrap();
    fixture.state.events.disconnect_token(&owner_token);
    wait_for_voice_state_delete(&mut member, owner_id).await;
    let remaining = fixture
        .state
        .voice
        .voice_states_in(&HashSet::from([fixture.lounge]))
        .await;
    let lounge = fixture.lounge;
    fixture.close().await;

    assert_eq!(seen_by_member.channel_id, lounge);
    assert!(!seen_by_member.suppressed && !seen_by_member.self_mute);
    assert!(
        matches!(first_for_outsider, GatewayEvent::MessageCreate(message) if message.id == sentinel.id)
    );
    assert_eq!(ready.voice_states, [seen_by_member]);
    assert_eq!(outsider_ready.voice_states, []);
    assert_eq!(remaining, []);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_channels_refuse_and_members_losing_connect_are_disconnected() {
    let fixture = fixture().await;
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (mut member, _) = identified(fixture.address, fixture.token("member")).await;
    let (mut outsider, _) = identified(fixture.address, fixture.token("outsider")).await;
    join(&mut outsider, fixture.booth, &pending_bot().offer).await;
    send_join(&mut member, fixture.booth, &pending_bot().offer).await;
    let full = receive_dispatch(&mut member, |event| {
        matches!(event, GatewayEvent::VoiceJoinRefused { .. })
    })
    .await
    .1;
    send_join(&mut outsider, fixture.lounge, &pending_bot().offer).await;
    let hidden = receive_dispatch(&mut outsider, |event| {
        matches!(event, GatewayEvent::VoiceJoinRefused { .. })
    })
    .await
    .1;
    send_join(&mut member, fixture.lounge, "v=0\r\nm=video 9 RTP 96\r\n").await;
    let invalid = receive_dispatch(&mut member, |event| {
        matches!(event, GatewayEvent::VoiceJoinRefused { .. })
    })
    .await
    .1;

    join(&mut member, fixture.lounge, &pending_bot().offer).await;
    next_voice_state_update(&mut owner, fixture.user_id("member")).await;
    fixture.deny_in_lounge("member", Permissions::CONNECT).await;
    let disconnected = receive_dispatch(&mut member, |event| {
        matches!(event, GatewayEvent::VoiceDisconnected { .. })
    })
    .await
    .1;
    wait_for_voice_state_delete(&mut owner, fixture.user_id("member")).await;
    let lounge = fixture.lounge;
    fixture.close().await;

    let refusal = |reason| GatewayEvent::VoiceJoinRefused {
        channel_id: Snowflake::from_raw(0),
        offer_id: 1,
        reason,
    };
    let without_channel = |event: GatewayEvent| match event {
        GatewayEvent::VoiceJoinRefused { reason, .. } => refusal(reason),
        other => other,
    };
    assert_eq!(
        without_channel(full),
        refusal(VoiceJoinRefusal::ChannelFull)
    );
    assert_eq!(
        without_channel(hidden),
        refusal(VoiceJoinRefusal::MissingPermissions)
    );
    assert_eq!(
        without_channel(invalid),
        refusal(VoiceJoinRefusal::InvalidOffer)
    );
    assert_eq!(
        disconnected,
        GatewayEvent::VoiceDisconnected {
            channel_id: lounge,
            reason: VoiceDisconnectReason::PermissionsChanged,
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_move_is_one_update_and_rejoining_the_same_channel_announces_nothing() {
    let fixture = fixture().await;
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (mut member, ready) = identified(fixture.address, fixture.token("member")).await;
    let owner_id = fixture.user_id("owner");
    let is_voice_event = |event: &GatewayEvent| {
        matches!(
            event,
            GatewayEvent::VoiceStateUpdate { .. } | GatewayEvent::VoiceStateDelete { .. }
        )
    };
    join(&mut owner, fixture.lounge, &pending_bot().offer).await;
    let joined = receive_dispatch(&mut member, is_voice_event).await.1;
    join(&mut owner, fixture.lounge, &pending_bot().offer).await;
    join(&mut owner, fixture.booth, &pending_bot().offer).await;
    let moved = receive_dispatch(&mut member, is_voice_event).await.1;
    // The same user from another session takes the connection over.
    let (mut other_device, _) = identified(fixture.address, fixture.token("owner")).await;
    join(&mut other_device, fixture.booth, &pending_bot().offer).await;
    let taken_over = receive_dispatch(&mut owner, |event| {
        matches!(event, GatewayEvent::VoiceDisconnected { .. })
    })
    .await
    .1;
    let (lounge, booth) = (fixture.lounge, fixture.booth);
    fixture.close().await;

    assert!(ready.voice_enabled);
    assert!(
        matches!(joined, GatewayEvent::VoiceStateUpdate { voice_state } if voice_state.channel_id == lounge)
    );
    assert!(
        matches!(&moved, GatewayEvent::VoiceStateUpdate { voice_state } if voice_state.channel_id == booth && voice_state.user_id == owner_id),
        "the rejoin announced nothing, the move one update: {moved:?}"
    );
    assert_eq!(
        taken_over,
        GatewayEvent::VoiceDisconnected {
            channel_id: booth,
            reason: VoiceDisconnectReason::MovedElsewhere,
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_join_racing_a_revocation_or_the_end_of_its_session_is_not_admitted() {
    let fixture = fixture().await;
    let voice = fixture.state.voice.clone();
    let request = |user_id, session| VoiceJoinRequest {
        user_id,
        session,
        channel_id: fixture.lounge,
        offer_id: 1,
        sdp_offer: pending_bot().offer,
        self_mute: false,
        self_deaf: false,
        host_header: None,
    };
    let member_id = fixture.user_id("member");
    let (member_session, mut member_events) = fixture
        .state
        .events
        .register_session(member_id, [1; 32], 10)
        .unwrap();
    let prepared = voice
        .prepare_join(&request(member_id, member_session))
        .await
        .unwrap();
    // CONNECT is taken away while the offer is being answered.
    fixture.deny_in_lounge("member", Permissions::CONNECT).await;
    voice
        .admit(request(member_id, member_session), prepared)
        .await;
    let refusal = member_events.try_recv().unwrap().payload_json.to_string();

    let owner_id = fixture.user_id("owner");
    let (owner_session, _owner_events) = fixture
        .state
        .events
        .register_session(owner_id, [2; 32], 10)
        .unwrap();
    let prepared = voice
        .prepare_join(&request(owner_id, owner_session))
        .await
        .unwrap();
    // The gateway session ends (and `end_session` runs) while the offer is being answered.
    fixture.state.events.unregister_session(owner_session);
    voice.end_session(owner_session).await;
    voice
        .admit(request(owner_id, owner_session), prepared)
        .await;
    let remaining = voice
        .voice_states_in(&HashSet::from([fixture.lounge]))
        .await;
    fixture.close().await;

    assert!(refusal.contains("missing_permissions"), "{refusal}");
    assert_eq!(remaining, []);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_member_joining_the_guild_learns_who_is_in_its_voice_channels() {
    let fixture = fixture().await;
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (mut newcomer, _) = identified(fixture.address, fixture.token("newcomer")).await;
    join(&mut owner, fixture.lounge, &pending_bot().offer).await;
    let owner_id = fixture.user_id("owner");
    let guild_id = fixture
        .state
        .guilds
        .voice_access(owner_id, fixture.lounge)
        .await
        .map(|access| match access {
            VoiceAccess::Allowed { guild_id, .. } => guild_id,
            other => panic!("{other:?}"),
        })
        .unwrap();
    let invite = fixture
        .state
        .guilds
        .create_invite(owner_id, guild_id, CreateInviteRequest::default(), now())
        .await
        .unwrap();
    fixture
        .state
        .guilds
        .join_with_invite(fixture.user_id("newcomer"), &invite.code, now())
        .await
        .unwrap();
    let first = receive_dispatch(&mut newcomer, |event| {
        matches!(
            event,
            GatewayEvent::GuildCreate(_) | GatewayEvent::VoiceStateUpdate { .. }
        )
    })
    .await
    .1;
    let seen = next_voice_state_update(&mut newcomer, owner_id).await;
    let lounge = fixture.lounge;
    fixture.close().await;

    assert!(matches!(first, GatewayEvent::GuildCreate(_)), "{first:?}");
    assert_eq!(seen.channel_id, lounge);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reconnection_mid_sentence_stops_speaking_and_announces_a_changed_state() {
    let fixture = fixture().await;
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (mut member, _) = identified(fixture.address, fixture.token("member")).await;
    let owner_id = fixture.user_id("owner");
    let owner_bot = join_and_start(&mut owner, fixture.lounge, 1).await;
    receive_dispatch(&mut member, |event| {
        matches!(event, GatewayEvent::Speaking { user_id, speaking: true, .. } if *user_id == owner_id)
    })
    .await;
    // The same session offers again (a network change), now muted.
    send(
        &mut owner,
        &ClientFrame::VoiceJoin {
            channel_id: fixture.lounge,
            offer_id: 2,
            sdp_offer: pending_bot().offer,
            self_mute: true,
            self_deaf: false,
        },
    )
    .await;
    let (mut has_stopped_speaking, mut muted_state) = (false, None);
    while !has_stopped_speaking || muted_state.is_none() {
        match receive_dispatch(&mut member, |_| true).await.1 {
            GatewayEvent::Speaking {
                user_id,
                speaking: false,
                ..
            } if user_id == owner_id => has_stopped_speaking = true,
            GatewayEvent::VoiceStateUpdate { voice_state } if voice_state.user_id == owner_id => {
                muted_state = Some(voice_state);
            }
            _ => {}
        }
    }
    drop(owner_bot);
    fixture.close().await;

    assert!(muted_state.is_some_and(|state| state.self_mute));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_move_to_another_guilds_channel_removes_the_state_for_the_old_viewers() {
    let fixture = fixture().await;
    let owner_id = fixture.user_id("owner");
    let other_guild = fixture
        .state
        .guilds
        .create_guild(
            owner_id,
            false,
            CreateGuildRequest {
                name: "Elsewhere".into(),
                initial_categories: None,
            },
            now(),
        )
        .await
        .unwrap();
    let elsewhere = fixture
        .state
        .guilds
        .create_channel(
            owner_id,
            other_guild.id,
            CreateChannelRequest {
                kind: ChannelKind::Voice,
                name: "Elsewhere".into(),
                topic: String::new(),
                parent_id: None,
                user_limit: None,
            },
            now(),
        )
        .await
        .unwrap();
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (mut member, _) = identified(fixture.address, fixture.token("member")).await;
    join(&mut owner, fixture.lounge, &pending_bot().offer).await;
    next_voice_state_update(&mut member, owner_id).await;
    join(&mut owner, elsewhere.id, &pending_bot().offer).await;
    wait_for_voice_state_delete(&mut member, owner_id).await;
    fixture.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_listener_learns_which_of_its_slots_carries_each_voice() {
    let fixture = fixture().await;
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (mut member, _) = identified(fixture.address, fixture.token("member")).await;

    let owner_bot = join_and_start(&mut owner, fixture.lounge, 1).await;
    let member_answer_bot = pending_bot();
    let member_answer = join(&mut member, fixture.lounge, &member_answer_bot.offer).await;
    let member_bot = start_bot(member_answer_bot, &member_answer, 2);
    member_bot.wait_until_hearing(1).await;
    let owner_id = fixture.user_id("owner");
    let assigned = receive_dispatch(&mut member, |event| {
        matches!(event, GatewayEvent::VoiceSlotAssigned { user_id, .. } if *user_id == owner_id)
    })
    .await
    .1;
    drop((owner_bot, member_bot));
    let lounge = fixture.lounge;
    fixture.close().await;

    let GatewayEvent::VoiceSlotAssigned {
        channel_id, mid, ..
    } = assigned
    else {
        unreachable!("{assigned:?}");
    };
    assert_eq!(channel_id, lounge);
    assert!(
        member_answer.contains(&format!("a=mid:{mid}\r\n")),
        "the slot is one of the listener's m-lines"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sounds_reach_the_channel_two_seconds_apart_and_joins_announce_the_join_sound() {
    let fixture = fixture().await;
    let (owner_id, member_id) = (fixture.user_id("owner"), fixture.user_id("member"));
    let guild_id = fixture
        .state
        .guilds
        .voice_access(owner_id, fixture.lounge)
        .await
        .map(|access| match access {
            VoiceAccess::Allowed { guild_id, .. } => guild_id,
            other => panic!("{other:?}"),
        })
        .unwrap();
    let sound = fixture
        .state
        .guilds
        .create_sound(
            &fixture.state.profile_images,
            owner_id,
            guild_id,
            "Horn",
            None,
            crate::domain::sound_rules::tests::canonical_wav(480),
            now(),
        )
        .await
        .unwrap();
    fixture
        .state
        .guilds
        .set_join_sound(member_id, guild_id, Some(sound.id))
        .await
        .unwrap();
    let (mut owner, _) = identified(fixture.address, fixture.token("owner")).await;
    let (mut member, _) = identified(fixture.address, fixture.token("member")).await;
    let (mut muted, _) = identified(fixture.address, fixture.token("muted")).await;
    join(&mut owner, fixture.lounge, &pending_bot().offer).await;
    join(&mut muted, fixture.lounge, &pending_bot().offer).await;
    join(&mut member, fixture.lounge, &pending_bot().offer).await;
    let member_state = next_voice_state_update(&mut owner, member_id).await;
    let play = ClientFrame::VoicePlaySound { sound_id: sound.id };
    // Suppressed in the lounge: ignored. The member's second sound comes too soon: ignored.
    send(&mut muted, &play).await;
    send(&mut member, &play).await;
    send(&mut member, &play).await;
    tokio::time::sleep(Duration::from_millis(2_100)).await;
    send(&mut owner, &play).await;
    let is_sound = |event: &GatewayEvent| matches!(event, GatewayEvent::VoiceSoundPlayed { .. });
    let heard_by_owner = [
        receive_dispatch(&mut owner, is_sound).await.1,
        receive_dispatch(&mut owner, is_sound).await.1,
    ];
    let heard_by_member = receive_dispatch(&mut member, is_sound).await.1;
    let lounge = fixture.lounge;
    fixture.close().await;

    assert_eq!(member_state.join_sound_id, Some(sound.id));
    let played_by = |user_id| GatewayEvent::VoiceSoundPlayed {
        channel_id: lounge,
        user_id,
        sound_id: sound.id,
    };
    assert_eq!(heard_by_owner, [played_by(member_id), played_by(owner_id)]);
    assert_eq!(
        heard_by_member,
        played_by(member_id),
        "the player hears it too"
    );
}
