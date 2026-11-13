//! The live connection: identify, heartbeat, resume after drops, reconnect with backoff, and the
//! frames the app sends (voice) while a session is attached.

use std::hash::{BuildHasher as _, Hasher as _};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use mellow_protocol::{ClientFrame, GatewayEvent, Ready, ServerFrame};
use tokio::sync::{Notify, mpsc};
use tokio::time::{Instant, timeout};
use tokio_tungstenite::tungstenite::Message as WebSocketMessage;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use url::Url;
use zeroize::Zeroizing;

use crate::instance::InstanceAddress;
use crate::rest::RestClient;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The server sends `Hello` right away and answers `Identify` within its own 10 s limit.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const FIRST_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAXIMUM_RECONNECT_DELAY: Duration = Duration::from_secs(60);
/// Below the server's 45 s deadline, whatever interval it announces.
const MAXIMUM_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const MINIMUM_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
/// A `Ready` for hundreds of guilds stays well below this; it bounds what a hostile instance can
/// make the client buffer (tungstenite's default is 64 MiB).
const MAXIMUM_GATEWAY_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
/// After `reconnect_now` on a live connection, the socket must acknowledge a heartbeat within
/// this or it is abandoned: a socket that survived a network change may be dead without knowing
/// it (the web client's `NETWORK_CHANGE_HEARTBEAT_ACKNOWLEDGEMENT_TIMEOUT_MILLISECONDS`).
const NETWORK_CHANGE_HEARTBEAT_ACKNOWLEDGEMENT_TIMEOUT: Duration = Duration::from_secs(10);
/// Frames waiting for the connection task; a voice session sends a handful at a time.
const MAXIMUM_QUEUED_FRAMES: usize = 32;

/// What the gateway reports to the app, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "events are nearly every update and apps match on them; `Ready`, rare, is boxed"
)]
pub enum GatewayUpdate {
    /// A new session: replace all state with this (events may have been missed before it).
    Ready(Box<Ready>),
    /// Live change; apply in order.
    Event(GatewayEvent),
    /// The connection came back and nothing was missed.
    Resumed,
    /// The connection dropped; reconnecting in the background.
    Disconnected,
    /// The token was refused (expired, revoked, account disabled): sign in again. Final update.
    SignedOut,
}

/// What the app and the connection task share.
#[derive(Debug, Default)]
struct SharedConnectionState {
    /// `Ready` or `Resumed` came back on the current socket: frames may be sent.
    is_session_attached: AtomicBool,
    /// Bumped each time a session goes: a frame queued under an older one is never sent.
    session_generation: AtomicU64,
    /// The app asked to retry now (the network came back or changed).
    reconnect_requested: Notify,
}

/// The app's side of a running gateway: sends frames on the live session and asks for a
/// reconnection now. Cheap to clone.
#[derive(Debug, Clone)]
pub struct GatewayHandle {
    frames: mpsc::Sender<SessionFrame>,
    shared: Arc<SharedConnectionState>,
}

/// A frame from the app, tagged with the session it was sent on.
#[derive(Debug)]
struct SessionFrame {
    session_generation: u64,
    frame: ClientFrame,
}

/// The connection task's side of a `GatewayHandle`, given to `run_gateway`.
#[derive(Debug)]
pub struct GatewayCommands {
    frames: mpsc::Receiver<SessionFrame>,
    shared: Arc<SharedConnectionState>,
}

impl GatewayHandle {
    /// A handle, and the commands `run_gateway` reads from it.
    #[must_use]
    pub fn new() -> (Self, GatewayCommands) {
        let (sender, receiver) = mpsc::channel(MAXIMUM_QUEUED_FRAMES);
        let shared = Arc::new(SharedConnectionState::default());
        let handle = Self {
            frames: sender,
            shared: Arc::clone(&shared),
        };
        let commands = GatewayCommands {
            frames: receiver,
            shared,
        };
        (handle, commands)
    }

    /// Sends `frame` on the live session. False, and nothing is sent, while no session is
    /// attached (connecting, reconnecting, signed out), when too many frames wait, and for the
    /// frames the task owns (`identify`, `resume`, `heartbeat`). A frame is never kept for a
    /// later connection: voice state would be stale there, and a voice session joins again after
    /// each `Ready` anyway.
    #[must_use]
    pub fn send(&self, frame: ClientFrame) -> bool {
        let is_owned_by_the_task = matches!(
            frame,
            ClientFrame::Identify { .. } | ClientFrame::Resume { .. } | ClientFrame::Heartbeat
        );
        if is_owned_by_the_task {
            return false;
        }
        // Generation first: if a detach slips in after this read, the frame carries the old
        // generation and the task drops it (`SeqCst` orders it against the attached flag).
        let session_generation = self.shared.session_generation.load(Ordering::SeqCst);
        self.shared.is_session_attached.load(Ordering::SeqCst)
            && self
                .frames
                .try_send(SessionFrame {
                    session_generation,
                    frame,
                })
                .is_ok()
    }

    /// Whether the connection task has taken every frame sent so far off the queue, so a frame
    /// sent just before the app exits (`voice_leave`) can be waited for.
    #[must_use]
    pub fn has_no_frame_waiting(&self) -> bool {
        self.frames.capacity() == self.frames.max_capacity()
    }

    /// Whether `Ready` or `Resumed` came back on the current connection.
    #[must_use]
    pub fn is_session_attached(&self) -> bool {
        self.shared.is_session_attached.load(Ordering::SeqCst)
    }

    /// The network came back or changed: a task waiting out its backoff reconnects now, with the
    /// backoff reset; a live session checks that its socket still answers a heartbeat. An attempt
    /// under way goes on (a request made during it may cut its next backoff short).
    pub fn reconnect_now(&self) {
        self.shared.reconnect_requested.notify_one();
    }
}

impl GatewayCommands {
    /// The session is gone: `send` refuses frames, and those already queued are dropped rather
    /// than sent on the next connection.
    fn detach_session(&mut self) {
        self.shared
            .is_session_attached
            .store(false, Ordering::SeqCst);
        self.shared
            .session_generation
            .fetch_add(1, Ordering::SeqCst);
        while self.frames.try_recv().is_ok() {}
    }

    fn attach_session(&self) {
        self.shared
            .is_session_attached
            .store(true, Ordering::SeqCst);
    }
}

/// The next frame the app sent on the current session; `None` once every handle is gone.
/// Takes the fields apart so the select loop can still borrow `shared` alongside it.
async fn next_frame_for_current_session(
    frames: &mut mpsc::Receiver<SessionFrame>,
    shared: &SharedConnectionState,
) -> Option<ClientFrame> {
    loop {
        let queued = frames.recv().await?;
        // Queued by a `send` that raced the last detach: meant for a session that is gone.
        if queued.session_generation == shared.session_generation.load(Ordering::SeqCst) {
            return Some(queued.frame);
        }
    }
}

/// Where to pick up after a drop.
#[derive(Debug)]
struct ResumePoint {
    session_id: String,
    last_sequence: u64,
}

enum ConnectionEnd {
    /// Nobody listens to updates anymore.
    UpdatesClosed,
    IdentifyRefused,
    Lost {
        was_connected: bool,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum WaitEnd {
    DelayElapsed,
    ReconnectRequested,
}

/// Keeps a session alive until the token is refused (sends `SignedOut`) or `updates` is dropped.
/// Abort the task to disconnect. `commands` comes from `GatewayHandle::new`.
pub async fn run_gateway(
    instance: InstanceAddress,
    token: Zeroizing<String>,
    updates: mpsc::Sender<GatewayUpdate>,
    mut commands: GatewayCommands,
) {
    let url = match instance.gateway_url() {
        Ok(url) => url,
        Err(error) => {
            tracing::error!(%error, "cannot build the gateway URL");
            return;
        }
    };
    let mut resume_point = None;
    let mut consecutive_failures: u32 = 0;
    loop {
        // Noticed even while every connection fails and nothing is sent.
        if updates.is_closed() {
            return;
        }
        let end = run_connection(&url, &token, &mut resume_point, &updates, &mut commands).await;
        commands.detach_session();
        match end {
            ConnectionEnd::UpdatesClosed => return,
            ConnectionEnd::IdentifyRefused if is_token_refused(&instance, &token).await => {
                let _ = updates.send(GatewayUpdate::SignedOut).await;
                return;
            }
            // Refused for another reason (session cap, transient server error): retry later.
            ConnectionEnd::IdentifyRefused => {}
            ConnectionEnd::Lost { was_connected } => {
                if was_connected {
                    consecutive_failures = 0;
                    if updates.send(GatewayUpdate::Disconnected).await.is_err() {
                        return;
                    }
                }
            }
        }
        let delay = reconnect_delay(consecutive_failures, random_u64());
        consecutive_failures = match wait_or_reconnect_now(delay, &commands.shared).await {
            WaitEnd::DelayElapsed => consecutive_failures.saturating_add(1),
            WaitEnd::ReconnectRequested => 0,
        };
    }
}

/// The backoff wait, cut short by `GatewayHandle::reconnect_now`.
async fn wait_or_reconnect_now(delay: Duration, shared: &SharedConnectionState) -> WaitEnd {
    tokio::select! {
        () = tokio::time::sleep(delay) => WaitEnd::DelayElapsed,
        () = shared.reconnect_requested.notified() => WaitEnd::ReconnectRequested,
    }
}

/// The gateway refuses an identify the same way for a bad token and for a full session cap or a
/// server error; only the REST API says which.
async fn is_token_refused(instance: &InstanceAddress, token: &Zeroizing<String>) -> bool {
    let Ok(mut rest) = RestClient::new(instance.clone()) else {
        return false;
    };
    rest.set_token(token.clone());
    rest.current_user()
        .await
        .is_err_and(|error| error.needs_new_login())
}

/// Exponential backoff with full jitter, so clients dropped together do not reconnect together.
fn reconnect_delay(consecutive_failures: u32, random: u64) -> Duration {
    let ceiling = FIRST_RECONNECT_DELAY
        .saturating_mul(2_u32.saturating_pow(consecutive_failures.min(16)))
        .min(MAXIMUM_RECONNECT_DELAY);
    let ceiling_milliseconds = u64::try_from(ceiling.as_millis()).unwrap_or(u64::MAX);
    // At least half the ceiling, so a run of small draws cannot hammer the server.
    let half = ceiling_milliseconds / 2;
    Duration::from_millis(half + random % (ceiling_milliseconds - half + 1))
}

/// The standard library seeds each `RandomState` randomly; enough for jitter, no extra crate.
fn random_u64() -> u64 {
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}

async fn connect(
    url: &Url,
) -> Option<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
> {
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAXIMUM_GATEWAY_MESSAGE_BYTES))
        .max_frame_size(Some(MAXIMUM_GATEWAY_MESSAGE_BYTES));
    let connecting =
        tokio_tungstenite::connect_async_with_config(url.as_str(), Some(config), false);
    match timeout(CONNECT_TIMEOUT, connecting).await {
        Ok(Ok((socket, _))) => Some(socket),
        _ => None,
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one select loop over the socket, the timers and the app's commands reads best whole"
)]
async fn run_connection(
    url: &Url,
    token: &Zeroizing<String>,
    resume_point: &mut Option<ResumePoint>,
    updates: &mpsc::Sender<GatewayUpdate>,
    commands: &mut GatewayCommands,
) -> ConnectionEnd {
    let lost = ConnectionEnd::Lost {
        was_connected: false,
    };
    let Some(socket) = connect(url).await else {
        return lost;
    };
    let (mut sink, mut stream) = socket.split();

    let Ok(Some(IncomingFrame::Known(ServerFrame::Hello { d: hello }))) =
        timeout(HANDSHAKE_TIMEOUT, next_frame(&mut stream)).await
    else {
        return lost;
    };
    let heartbeat_interval = Duration::from_millis(hello.heartbeat_interval_milliseconds)
        .clamp(MINIMUM_HEARTBEAT_INTERVAL, MAXIMUM_HEARTBEAT_INTERVAL);

    let is_resuming = resume_point.is_some();
    let opening = match resume_point {
        Some(point) => ClientFrame::Resume {
            token: token.to_string(),
            session_id: point.session_id.clone(),
            last_sequence: point.last_sequence,
        },
        None => ClientFrame::Identify {
            token: token.to_string(),
        },
    };
    if send_frame(&mut sink, &opening).await.is_err() {
        return lost;
    }

    let mut is_connected = false;
    let mut is_heartbeat_acknowledged = true;
    let mut heartbeat =
        tokio::time::interval_at(Instant::now() + heartbeat_interval, heartbeat_interval);
    let handshake_deadline = tokio::time::sleep(HANDSHAKE_TIMEOUT);
    tokio::pin!(handshake_deadline);
    // Armed by `reconnect_now` on a live session.
    let link_check_deadline = tokio::time::sleep(NETWORK_CHANGE_HEARTBEAT_ACKNOWLEDGEMENT_TIMEOUT);
    tokio::pin!(link_check_deadline);
    let mut is_checking_link = false;
    let mut are_commands_open = true;

    loop {
        let lost = ConnectionEnd::Lost {
            was_connected: is_connected,
        };
        let frame = tokio::select! {
            frame = next_frame(&mut stream) => frame,
            _ = heartbeat.tick() => {
                // A network-change check owns the deadline of the heartbeat it sent.
                if is_checking_link {
                    continue;
                }
                // No ack since the last beat: the connection is a zombie, start over.
                if !is_heartbeat_acknowledged || send_frame(&mut sink, &ClientFrame::Heartbeat).await.is_err() {
                    return lost;
                }
                is_heartbeat_acknowledged = false;
                continue;
            }
            () = &mut handshake_deadline, if !is_connected => return lost,
            command = next_frame_for_current_session(&mut commands.frames, &commands.shared),
                if is_connected && are_commands_open => {
                match command {
                    Some(frame) => {
                        if send_frame(&mut sink, &frame).await.is_err() {
                            return lost;
                        }
                    }
                    // Every handle is gone: nothing more will come.
                    None => are_commands_open = false,
                }
                continue;
            }
            () = commands.shared.reconnect_requested.notified() => {
                // Before the session is attached, the handshake deadline already watches it.
                if is_connected && !is_checking_link {
                    if send_frame(&mut sink, &ClientFrame::Heartbeat).await.is_err() {
                        return lost;
                    }
                    is_heartbeat_acknowledged = false;
                    is_checking_link = true;
                    link_check_deadline
                        .as_mut()
                        .reset(Instant::now() + NETWORK_CHANGE_HEARTBEAT_ACKNOWLEDGEMENT_TIMEOUT);
                }
                continue;
            }
            () = &mut link_check_deadline, if is_checking_link => return lost,
        };
        let frame = match frame {
            None => return lost,
            Some(IncomingFrame::Known(frame)) => frame,
            // A newer server's event: skipped, but counted, so a resume does not replay it.
            Some(IncomingFrame::UnknownDispatch { sequence }) => {
                if let Some(point) = resume_point.as_mut() {
                    point.last_sequence = sequence;
                }
                continue;
            }
        };
        let update = match frame {
            ServerFrame::Hello { .. } => continue,
            ServerFrame::HeartbeatAck => {
                is_heartbeat_acknowledged = true;
                is_checking_link = false;
                continue;
            }
            ServerFrame::Ready { d: ready } => {
                is_connected = true;
                commands.attach_session();
                *resume_point = Some(ResumePoint {
                    session_id: ready.session_id.clone(),
                    last_sequence: 0,
                });
                GatewayUpdate::Ready(ready)
            }
            ServerFrame::Resumed => {
                is_connected = true;
                commands.attach_session();
                GatewayUpdate::Resumed
            }
            ServerFrame::Dispatch { sequence, event } => {
                if let Some(point) = resume_point.as_mut() {
                    point.last_sequence = sequence;
                }
                GatewayUpdate::Event(*event)
            }
            ServerFrame::InvalidSession => {
                *resume_point = None;
                // Refusing a fresh `Identify` means the token itself is no good.
                if !is_resuming && !is_connected {
                    return ConnectionEnd::IdentifyRefused;
                }
                // A refused resume or a cut-off session: identify again on the next connection.
                return lost;
            }
        };
        if updates.send(update).await.is_err() {
            return ConnectionEnd::UpdatesClosed;
        }
    }
}

enum IncomingFrame {
    Known(ServerFrame),
    /// A dispatch whose event this client cannot parse (a newer server, or a type mismatch).
    UnknownDispatch {
        sequence: u64,
    },
}

/// Just enough of a dispatch to count it when its event cannot be parsed.
#[derive(serde::Deserialize)]
struct DispatchSequence {
    op: String,
    s: u64,
}

/// The next server frame; `None` once the connection is closed or broken. Frames this client
/// cannot parse (a newer server) are skipped; a dispatch among them still reports its sequence.
async fn next_frame<S>(stream: &mut S) -> Option<IncomingFrame>
where
    S: futures_util::Stream<Item = Result<WebSocketMessage, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
{
    loop {
        match stream.next().await? {
            Ok(WebSocketMessage::Text(text)) => match serde_json::from_str(&text) {
                Ok(frame) => return Some(IncomingFrame::Known(frame)),
                Err(error) => {
                    // Never the error text itself: serde quotes the offending value, which may
                    // be message content.
                    if let Ok(DispatchSequence { op, s }) = serde_json::from_str(&text)
                        && op == "dispatch"
                    {
                        // `warn`: a known event that no longer parses (a client and server type
                        // mismatch) is lost for good once counted, so it must show by default.
                        tracing::warn!(
                            sequence = s,
                            category = ?error.classify(),
                            line = error.line(),
                            column = error.column(),
                            "skipping a gateway event this client cannot parse"
                        );
                        return Some(IncomingFrame::UnknownDispatch { sequence: s });
                    }
                    tracing::warn!(
                        category = ?error.classify(),
                        line = error.line(),
                        column = error.column(),
                        "skipping an unknown gateway frame"
                    );
                }
            },
            Ok(WebSocketMessage::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

async fn send_frame<S>(sink: &mut S, frame: &ClientFrame) -> Result<(), ()>
where
    S: futures_util::Sink<WebSocketMessage> + Unpin,
{
    // The JSON holds the token: wipe it once sent.
    let text = Zeroizing::new(serde_json::to_string(frame).map_err(|_| ())?);
    sink.send(WebSocketMessage::text(text.as_str()))
        .await
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{CurrentUser, Hello, Snowflake};
    use tokio::net::TcpListener;
    use tokio_tungstenite::WebSocketStream;

    use super::*;

    const STEP_TIMEOUT: Duration = Duration::from_secs(10);

    type ServerSocket = WebSocketStream<tokio::net::TcpStream>;

    /// A scripted gateway: each test drives the server side frame by frame.
    async fn scripted_gateway() -> (
        TcpListener,
        mpsc::Receiver<GatewayUpdate>,
        tokio::task::JoinHandle<()>,
    ) {
        let (listener, updates, client, _) = scripted_gateway_with_handle().await;
        (listener, updates, client)
    }

    async fn scripted_gateway_with_handle() -> (
        TcpListener,
        mpsc::Receiver<GatewayUpdate>,
        tokio::task::JoinHandle<()>,
        GatewayHandle,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let instance = InstanceAddress::parse(&format!("http://{address}")).unwrap();
        let (sender, updates) = mpsc::channel(16);
        let (handle, commands) = GatewayHandle::new();
        let client = tokio::spawn(run_gateway(
            instance,
            Zeroizing::new("mlw_t".into()),
            sender,
            commands,
        ));
        (listener, updates, client, handle)
    }

    /// Accepts the next connection and sends `Hello` with the shortest heartbeat the client allows.
    async fn accept(listener: &TcpListener) -> ServerSocket {
        accept_with_heartbeat(listener, 1_000).await
    }

    async fn accept_with_heartbeat(
        listener: &TcpListener,
        heartbeat_interval_milliseconds: u64,
    ) -> ServerSocket {
        let (stream, _) = timeout(STEP_TIMEOUT, listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let hello = ServerFrame::Hello {
            d: Hello {
                heartbeat_interval_milliseconds,
            },
        };
        send(&mut socket, &hello).await;
        socket
    }

    async fn send(socket: &mut ServerSocket, frame: &ServerFrame) {
        let text = serde_json::to_string(frame).unwrap();
        socket.send(WebSocketMessage::text(text)).await.unwrap();
    }

    async fn receive(socket: &mut ServerSocket) -> ClientFrame {
        loop {
            let message = timeout(STEP_TIMEOUT, socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let WebSocketMessage::Text(text) = message {
                return serde_json::from_str(&text).unwrap();
            }
        }
    }

    fn ready(session_id: &str) -> ServerFrame {
        ServerFrame::Ready {
            d: Box::new(Ready {
                may_create_guilds: true,
                session_id: session_id.into(),
                user: CurrentUser {
                    id: Snowflake::from_raw(1),
                    username: "aurore".into(),
                    display_name: "Aurore".into(),
                    email: "aurore@example.org".into(),
                    is_instance_administrator: false,
                    avatar: None,
                    banner: None,
                    status: mellow_protocol::ChosenStatus::Online,
                    custom_status: None,
                    friend_code: None,
                },
                guilds: Vec::new(),
                read_states: Vec::new(),
                online_user_ids: Vec::new(),
                direct_message_channels: Vec::new(),
                channel_unreads: Vec::new(),
                presences: Vec::new(),
                relationships: Vec::new(),
                voice_states: Vec::new(),
                voice_enabled: false,
                maximum_attachment_bytes: None,
            }),
        }
    }

    async fn next_update(updates: &mut mpsc::Receiver<GatewayUpdate>) -> GatewayUpdate {
        timeout(STEP_TIMEOUT, updates.recv())
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn resumes_from_the_last_sequence_and_identifies_again_when_refused() {
        let (listener, mut updates, client) = scripted_gateway().await;

        let mut first = accept(&listener).await;
        assert!(matches!(
            receive(&mut first).await,
            ClientFrame::Identify { .. }
        ));
        send(&mut first, &ready("session-1")).await;
        let typing = GatewayEvent::TypingStart {
            guild_id: Some(Snowflake::from_raw(5)),
            channel_id: Snowflake::from_raw(6),
            user_id: Snowflake::from_raw(7),
        };
        send(
            &mut first,
            &ServerFrame::Dispatch {
                sequence: 5,
                event: Box::new(typing.clone()),
            },
        )
        .await;
        assert!(matches!(
            next_update(&mut updates).await,
            GatewayUpdate::Ready(_)
        ));
        assert_eq!(
            next_update(&mut updates).await,
            GatewayUpdate::Event(typing)
        );
        drop(first);
        assert_eq!(next_update(&mut updates).await, GatewayUpdate::Disconnected);

        let mut second = accept(&listener).await;
        match receive(&mut second).await {
            ClientFrame::Resume {
                session_id,
                last_sequence,
                ..
            } => {
                assert_eq!((session_id.as_str(), last_sequence), ("session-1", 5));
            }
            other => panic!("expected a resume, got {other:?}"),
        }
        send(&mut second, &ServerFrame::InvalidSession).await;

        // A refused resume is not a sign-out: the client identifies on a new connection.
        let mut third = accept(&listener).await;
        assert!(matches!(
            receive(&mut third).await,
            ClientFrame::Identify { .. }
        ));
        send(&mut third, &ready("session-2")).await;
        assert!(matches!(
            next_update(&mut updates).await,
            GatewayUpdate::Ready(_)
        ));
        client.abort();
    }

    #[tokio::test]
    async fn an_unacknowledged_heartbeat_makes_the_client_reconnect() {
        let (listener, mut updates, client) = scripted_gateway().await;
        let mut silent = accept(&listener).await;
        assert!(matches!(
            receive(&mut silent).await,
            ClientFrame::Identify { .. }
        ));
        send(&mut silent, &ready("session-1")).await;
        assert!(matches!(
            next_update(&mut updates).await,
            GatewayUpdate::Ready(_)
        ));
        assert_eq!(receive(&mut silent).await, ClientFrame::Heartbeat);

        // No ack: at the next beat the client drops this socket and resumes elsewhere.
        assert_eq!(next_update(&mut updates).await, GatewayUpdate::Disconnected);
        let mut next = accept(&listener).await;
        assert!(matches!(
            receive(&mut next).await,
            ClientFrame::Resume { .. }
        ));
        client.abort();
    }

    #[tokio::test]
    async fn the_task_ends_when_nobody_listens() {
        let (listener, updates, client) = scripted_gateway().await;
        drop(updates);
        drop(listener);
        timeout(STEP_TIMEOUT, client).await.unwrap().unwrap();
    }

    const LEAVE: ClientFrame = ClientFrame::VoiceLeave;

    fn play_sound(sound_id: u64) -> ClientFrame {
        ClientFrame::VoicePlaySound {
            sound_id: Snowflake::from_raw(sound_id),
        }
    }

    #[tokio::test]
    async fn frames_go_out_only_on_an_attached_session() {
        // The longest heartbeat, so every frame the server reads is one the app sent.
        let (listener, mut updates, client, handle) = scripted_gateway_with_handle().await;
        assert!(
            !handle.send(LEAVE.clone()),
            "nothing before the first Ready"
        );

        let mut first = accept_with_heartbeat(&listener, 30_000).await;
        assert!(matches!(
            receive(&mut first).await,
            ClientFrame::Identify { .. }
        ));
        assert!(!handle.send(LEAVE.clone()), "nor during the handshake");
        send(&mut first, &ready("session-1")).await;
        assert!(matches!(
            next_update(&mut updates).await,
            GatewayUpdate::Ready(_)
        ));
        assert!(handle.is_session_attached());
        assert!(
            !handle.send(ClientFrame::Heartbeat),
            "the task owns the session frames"
        );
        assert!(handle.send(play_sound(9)));
        assert_eq!(receive(&mut first).await, play_sound(9));

        drop(first);
        assert_eq!(next_update(&mut updates).await, GatewayUpdate::Disconnected);
        assert!(!handle.is_session_attached());
        assert!(!handle.send(play_sound(10)), "refused while reconnecting");

        let mut second = accept_with_heartbeat(&listener, 30_000).await;
        assert!(matches!(
            receive(&mut second).await,
            ClientFrame::Resume { .. }
        ));
        send(&mut second, &ServerFrame::Resumed).await;
        assert_eq!(next_update(&mut updates).await, GatewayUpdate::Resumed);
        assert!(handle.send(LEAVE.clone()));
        assert_eq!(
            receive(&mut second).await,
            LEAVE,
            "the frame refused while reconnecting never comes"
        );
        client.abort();
    }

    #[tokio::test]
    async fn an_unknown_event_is_skipped_but_counted_for_the_resume() {
        let (listener, mut updates, client) = scripted_gateway().await;
        let mut first = accept(&listener).await;
        assert!(matches!(
            receive(&mut first).await,
            ClientFrame::Identify { .. }
        ));
        send(&mut first, &ready("session-1")).await;
        assert!(matches!(
            next_update(&mut updates).await,
            GatewayUpdate::Ready(_)
        ));
        let from_a_newer_server =
            r#"{"op":"dispatch","s":8,"d":{"t":"SOMETHING_NEW","data":{"x":1}}}"#;
        first
            .send(WebSocketMessage::text(from_a_newer_server))
            .await
            .unwrap();
        drop(first);
        assert_eq!(
            next_update(&mut updates).await,
            GatewayUpdate::Disconnected,
            "the unknown event reaches no one"
        );

        let mut second = accept(&listener).await;
        match receive(&mut second).await {
            ClientFrame::Resume { last_sequence, .. } => assert_eq!(last_sequence, 8),
            other => panic!("expected a resume, got {other:?}"),
        }
        client.abort();
    }

    #[test]
    fn frames_queued_on_a_session_are_dropped_when_it_goes() {
        let (handle, mut commands) = GatewayHandle::new();
        commands.attach_session();
        assert!(handle.send(play_sound(1)));
        commands.detach_session();
        commands.attach_session();
        assert!(
            commands.frames.try_recv().is_err(),
            "the old session's frame never reaches the new one"
        );
    }

    #[test]
    fn a_sent_frame_waits_until_the_connection_task_takes_it() {
        let (handle, mut commands) = GatewayHandle::new();
        assert!(handle.has_no_frame_waiting());
        commands.attach_session();
        assert!(handle.send(LEAVE.clone()));
        assert!(!handle.has_no_frame_waiting());
        assert!(commands.frames.try_recv().is_ok());
        assert!(handle.has_no_frame_waiting());
    }

    #[tokio::test]
    async fn a_frame_queued_after_the_detach_drain_is_not_sent_on_the_next_session() {
        let (handle, mut commands) = GatewayHandle::new();
        commands.attach_session();
        let generation_seen_by_a_racing_send =
            commands.shared.session_generation.load(Ordering::SeqCst);
        commands.detach_session();
        // What `send` enqueues when it read the attached flag just before the detach.
        handle
            .frames
            .try_send(SessionFrame {
                session_generation: generation_seen_by_a_racing_send,
                frame: play_sound(1),
            })
            .unwrap();
        commands.attach_session();
        assert!(handle.send(LEAVE.clone()));
        let next = next_frame_for_current_session(&mut commands.frames, &commands.shared);
        assert_eq!(timeout(STEP_TIMEOUT, next).await.unwrap(), Some(LEAVE));
    }

    /// Opens a session and returns the server's socket for it.
    async fn ready_session(
        listener: &TcpListener,
        updates: &mut mpsc::Receiver<GatewayUpdate>,
        heartbeat_interval_milliseconds: u64,
    ) -> ServerSocket {
        let mut socket = accept_with_heartbeat(listener, heartbeat_interval_milliseconds).await;
        assert!(matches!(
            receive(&mut socket).await,
            ClientFrame::Identify { .. }
        ));
        send(&mut socket, &ready("session-1")).await;
        assert!(matches!(
            next_update(updates).await,
            GatewayUpdate::Ready(_)
        ));
        socket
    }

    /// Acknowledges every heartbeat until `duration` has passed; the socket must stay open.
    async fn answer_heartbeats_for(socket: &mut ServerSocket, duration: Duration) {
        let deadline = Instant::now() + duration;
        while let Ok(message) = tokio::time::timeout_at(deadline, socket.next()).await {
            let message = message
                .expect("the client keeps the socket")
                .expect("the socket stays healthy");
            if let WebSocketMessage::Text(text) = message {
                let frame: ClientFrame = serde_json::from_str(&text).unwrap();
                assert_eq!(frame, ClientFrame::Heartbeat);
                send(socket, &ServerFrame::HeartbeatAck).await;
            }
        }
    }

    #[tokio::test]
    async fn a_slow_answer_to_the_link_check_keeps_the_session_across_regular_beats() {
        let (listener, mut updates, client, handle) = scripted_gateway_with_handle().await;
        let mut socket = ready_session(&listener, &mut updates, 1_000).await;
        answer_heartbeats_for(&mut socket, Duration::from_millis(1_500)).await;

        handle.reconnect_now();
        // Slow round trips after a network change: the check's beat waits past regular ticks,
        // well within the check's 10 s.
        tokio::time::sleep(Duration::from_millis(2_500)).await;
        // Past the check's deadline too: an answered check never fires it.
        answer_heartbeats_for(
            &mut socket,
            NETWORK_CHANGE_HEARTBEAT_ACKNOWLEDGEMENT_TIMEOUT,
        )
        .await;
        assert!(updates.try_recv().is_err(), "the session was never dropped");

        assert!(handle.send(LEAVE.clone()));
        loop {
            match receive(&mut socket).await {
                ClientFrame::Heartbeat => send(&mut socket, &ServerFrame::HeartbeatAck).await,
                frame => {
                    assert_eq!(frame, LEAVE);
                    break;
                }
            }
        }
        client.abort();
    }

    #[tokio::test]
    async fn an_unanswered_link_check_drops_the_socket_and_resumes() {
        let (listener, mut updates, client, handle) = scripted_gateway_with_handle().await;
        // The longest heartbeat, so only the check can end this socket.
        let mut socket = ready_session(&listener, &mut updates, 30_000).await;
        handle.reconnect_now();
        assert_eq!(receive(&mut socket).await, ClientFrame::Heartbeat);
        let disconnected = timeout(
            NETWORK_CHANGE_HEARTBEAT_ACKNOWLEDGEMENT_TIMEOUT + STEP_TIMEOUT,
            updates.recv(),
        )
        .await
        .unwrap();
        assert_eq!(disconnected, Some(GatewayUpdate::Disconnected));
        let mut next = accept(&listener).await;
        assert!(matches!(
            receive(&mut next).await,
            ClientFrame::Resume { .. }
        ));
        client.abort();
    }

    #[tokio::test]
    async fn reconnect_now_cuts_the_backoff_wait_short() {
        let shared = SharedConnectionState::default();
        shared.reconnect_requested.notify_one();
        let wait = wait_or_reconnect_now(Duration::from_secs(3_600), &shared);
        assert_eq!(
            timeout(STEP_TIMEOUT, wait).await.unwrap(),
            WaitEnd::ReconnectRequested
        );
    }

    #[test]
    fn reconnect_delays_grow_with_jitter_and_stay_capped() {
        assert_eq!(reconnect_delay(0, 0), Duration::from_millis(500));
        assert_eq!(reconnect_delay(0, 500), Duration::from_secs(1));
        assert_eq!(reconnect_delay(3, 0), Duration::from_secs(4));
        for failures in [10, 1_000, u32::MAX] {
            for random in [0, 7, u64::MAX] {
                let delay = reconnect_delay(failures, random);
                assert!(delay >= MAXIMUM_RECONNECT_DELAY / 2 && delay <= MAXIMUM_RECONNECT_DELAY);
            }
        }
    }
}
