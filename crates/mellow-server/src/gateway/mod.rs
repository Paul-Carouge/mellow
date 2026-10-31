//! `GET /gateway?v=1`: the real-time WebSocket (ARCHITECTURE.md §4.8).

mod session;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::extract::ws::{Message as WebSocketMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::Response;
use mellow_protocol::{
    ChannelKind, ClientFrame, CurrentUser, GATEWAY_VERSION, Guild, Hello, Ready, RelationshipKind,
    ServerFrame, Snowflake, VoiceJoinRefusal, VoiceState,
};
use serde::Deserialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio::time::Instant;
use zeroize::Zeroizing;

use crate::domain::clock::now_unix_milliseconds;
use crate::domain::guilds::GuildError;
use crate::domain::session_token::session_token_sha256;
use crate::events::SessionKey;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiQuery};
use crate::http::authentication::{is_from_this_origin, session_cookie};
use crate::http::client_address::client_address_key;
use crate::http::rate_limit::{
    GATEWAY_CONNECTIONS_PER_ADDRESS, VOICE_JOINS_PER_USER, VOICE_STATE_CHANGES_PER_USER,
};
use crate::voice::VoiceJoinRequest;
use session::{
    FrameText, OUTGOING_QUEUE_FRAMES, SessionCommand, SessionEnd, SessionTask, frame_text,
};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
/// A client is dropped after missing this long without a heartbeat (one and a half intervals).
const HEARTBEAT_DEADLINE: Duration = Duration::from_secs(45);
/// Heartbeats faster than this are acknowledged without touching the database again.
const MINIMUM_INTERVAL_BETWEEN_REVALIDATIONS: Duration = Duration::from_secs(15);
const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(10);
/// A write that cannot be flushed in this time means the client stopped reading: drop it, so a
/// non-reading peer cannot pin a task and its queue forever.
const SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// Bytes buffered towards one socket before writes wait on the network.
const MAXIMUM_WRITE_BUFFER_BYTES: usize = 256 * 1024;
/// Client frames are tiny (identify, resume, heartbeat) except `voice_join`, whose SDP offer may
/// take up to `MAXIMUM_VOICE_OFFER_BYTES`, escaped in JSON (`\r\n` doubles each line ending);
/// anything bigger is abuse.
const MAXIMUM_CLIENT_FRAME_BYTES: usize = 96 * 1024;
const MAXIMUM_SESSIONS_PER_USER: usize = 10;
/// Upper bound on simultaneous `WebSocket` connections, authenticated or not (file descriptors, tasks).
const MAXIMUM_GATEWAY_CONNECTIONS: usize = 10_000;
const SESSION_COMMAND_QUEUE_CAPACITY: usize = 8;
const SESSION_ID_RANDOM_BYTES: usize = 16;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GatewayQuery {
    v: u32,
}

/// What a reconnecting client needs to resume its session.
#[derive(Debug, Clone)]
struct ResumableSession {
    user_id: Snowflake,
    hub_key: SessionKey,
    commands: mpsc::Sender<SessionCommand>,
}

/// Live sessions by id, so a reconnecting client can resume, and the connection budget.
#[derive(Debug)]
pub(crate) struct SessionRegistry {
    sessions: Mutex<HashMap<String, ResumableSession>>,
    next_attachment_id: AtomicU64,
    connection_permits: Arc<Semaphore>,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self {
            sessions: Mutex::default(),
            next_attachment_id: AtomicU64::default(),
            connection_permits: Arc::new(Semaphore::new(MAXIMUM_GATEWAY_CONNECTIONS)),
        }
    }
}

impl SessionRegistry {
    fn sessions(&self) -> std::sync::MutexGuard<'_, HashMap<String, ResumableSession>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next_attachment_id(&self) -> u64 {
        self.next_attachment_id.fetch_add(1, Ordering::Relaxed)
    }
}

pub(crate) async fn upgrade(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiQuery(query): ApiQuery<GatewayQuery>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if query.v != GATEWAY_VERSION {
        return Err(ApiError::validation(format!(
            "unsupported gateway version {}; this server speaks v={GATEWAY_VERSION}",
            query.v
        )));
    }
    // A page of another site must not open a gateway with this instance's session cookie (CSWSH).
    if !is_from_this_origin(&headers) {
        return Err(ApiError::unauthorized());
    }
    // The web client identifies with an empty token: its session is this cookie (ADR-0019).
    let cookie_token = session_cookie(&headers).map(|token| Zeroizing::new(token.to_owned()));
    let address = client_address_key(peer, &headers, &state.trusted_proxy_networks);
    // Where the client reached the instance: the voice candidate when no public address is set.
    // Checked by the voice service before any use, and never logged.
    let host_header = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if !state.gateway_rate_limiter.record_event(
        &format!("gateway:{address}"),
        GATEWAY_CONNECTIONS_PER_ADDRESS,
        std::time::Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    let Ok(permit) = state
        .gateway_sessions
        .connection_permits
        .clone()
        .try_acquire_owned()
    else {
        return Err(ApiError::server_busy());
    };
    Ok(upgrade
        .max_message_size(MAXIMUM_CLIENT_FRAME_BYTES)
        .max_frame_size(MAXIMUM_CLIENT_FRAME_BYTES)
        .max_write_buffer_size(MAXIMUM_WRITE_BUFFER_BYTES)
        .on_upgrade(move |socket| run_connection(socket, state, permit, cookie_token, host_header)))
}

/// Every write goes through here: it fails rather than wait on a client that stopped reading.
async fn send_text(socket: &mut WebSocket, text: &str) -> bool {
    matches!(
        tokio::time::timeout(
            SOCKET_WRITE_TIMEOUT,
            socket.send(WebSocketMessage::Text(text.into())),
        )
        .await,
        Ok(Ok(()))
    )
}

async fn send_frame(socket: &mut WebSocket, frame: &ServerFrame) -> bool {
    match frame_text(frame) {
        Some(text) => send_text(socket, &text).await,
        None => false,
    }
}

async fn close(mut socket: WebSocket) {
    let _ = tokio::time::timeout(
        SOCKET_WRITE_TIMEOUT,
        socket.send(WebSocketMessage::Close(None)),
    )
    .await;
}

/// The next client frame, or `None` on close, error, binary data or malformed JSON.
async fn next_client_frame(socket: &mut WebSocket) -> Option<ClientFrame> {
    loop {
        match socket.recv().await? {
            Ok(WebSocketMessage::Text(text)) => return serde_json::from_str(&text).ok(),
            Ok(WebSocketMessage::Ping(_) | WebSocketMessage::Pong(_)) => {}
            Ok(WebSocketMessage::Binary(_) | WebSocketMessage::Close(_)) | Err(_) => return None,
        }
    }
}

async fn refuse(mut socket: WebSocket) {
    send_frame(&mut socket, &ServerFrame::InvalidSession).await;
    close(socket).await;
}

/// Holds one of the `MAXIMUM_GATEWAY_CONNECTIONS` permits for its whole life.
async fn run_connection(
    mut socket: WebSocket,
    state: AppState,
    _permit: OwnedSemaphorePermit,
    cookie_token: Option<Zeroizing<String>>,
    host_header: Option<String>,
) {
    let session_token = |token: String| match cookie_token.as_ref() {
        Some(cookie_token) if token.is_empty() => cookie_token.clone(),
        _ => Zeroizing::new(token),
    };
    let hello = ServerFrame::Hello {
        d: Hello {
            heartbeat_interval_milliseconds: u64::try_from(HEARTBEAT_INTERVAL.as_millis())
                .unwrap_or(u64::MAX),
        },
    };
    if !send_frame(&mut socket, &hello).await {
        return;
    }
    let first_frame = tokio::time::timeout(IDENTIFY_TIMEOUT, next_client_frame(&mut socket)).await;
    let (token, user, commands, hub_key, after_sequence, is_resume) = match first_frame {
        Ok(Some(ClientFrame::Identify { token })) => {
            let token = session_token(token);
            let Some((user, commands, hub_key)) = identify(&mut socket, &state, &token).await
            else {
                return refuse(socket).await;
            };
            (token, user, commands, hub_key, 0, false)
        }
        Ok(Some(ClientFrame::Resume {
            token,
            session_id,
            last_sequence,
        })) => {
            let token = session_token(token);
            let Ok(user) = state
                .accounts
                .authenticate(&token, now_unix_milliseconds())
                .await
            else {
                return refuse(socket).await;
            };
            let resumable = state.gateway_sessions.sessions().get(&session_id).cloned();
            match resumable {
                Some(resumable) if resumable.user_id == user.id => (
                    token,
                    user,
                    resumable.commands,
                    resumable.hub_key,
                    last_sequence,
                    true,
                ),
                _ => return refuse(socket).await,
            }
        }
        _ => return refuse(socket).await,
    };

    let attachment_id = state.gateway_sessions.next_attachment_id();
    let (outgoing, outgoing_frames) = mpsc::channel(OUTGOING_QUEUE_FRAMES);
    let (reply, answer) = oneshot::channel();
    let attach = SessionCommand::Attach {
        attachment_id,
        outgoing,
        after_sequence,
        is_resume,
        reply,
    };
    if commands.send(attach).await.is_err() || !matches!(answer.await, Ok(Ok(()))) {
        return refuse(socket).await;
    }
    let attached = AttachedSession {
        token: &token,
        user: &user,
        hub_key,
        host_header,
    };
    run_attached(socket, &state, &attached, outgoing_frames).await;
    let _ = commands
        .send(SessionCommand::Detach { attachment_id })
        .await;
}

/// Authenticates, starts a session and sends `Ready`. Returns `None` to refuse the client.
async fn identify(
    socket: &mut WebSocket,
    state: &AppState,
    token: &str,
) -> Option<(CurrentUser, mpsc::Sender<SessionCommand>, SessionKey)> {
    let user = state
        .accounts
        .authenticate(token, now_unix_milliseconds())
        .await
        .ok()?;
    let token_sha256 = session_token_sha256(token)?;
    // Registered (atomically against the per-user cap) before reading the guilds, so a guild
    // joined meanwhile is subscribed by `subscribe_user_to_guild` and nothing published after
    // this point is lost: it waits in the session's replay buffer until the socket attaches.
    let (hub_key, events) = state
        .events
        .register_session(user.id, token_sha256, MAXIMUM_SESSIONS_PER_USER)
        .ok()?;
    // A logout or password change committed between the check above and the registration found no
    // hub session to cut: check the token again now that this one is registered.
    let Ok(user) = state
        .accounts
        .authenticate(token, now_unix_milliseconds())
        .await
    else {
        state.events.unregister_session(hub_key);
        return None;
    };
    if let Err(error) = state.accounts.load_presence_into_events(user.id).await {
        tracing::warn!(%error, "gateway identify: cannot read the presence");
        state.events.unregister_session(hub_key);
        return None;
    }
    // Before activation, which announces the user to their friends too (ADR-0025); ordered
    // against relationship changes, so a friendship ended meanwhile is not restored.
    match state.relationships.load_friends_into_events(user.id).await {
        Ok(()) => {}
        Err(error) => {
            tracing::warn!(%error, "gateway identify: cannot list friends");
            state.events.unregister_session(hub_key);
            return None;
        }
    }
    let guild_ids = match state.guilds.list_my_guilds(user.id).await {
        Ok(guilds) => guilds.iter().map(|guild| guild.id).collect::<Vec<_>>(),
        Err(error) => {
            tracing::warn!(%error, "gateway identify: cannot list guilds");
            state.events.unregister_session(hub_key);
            return None;
        }
    };
    state.events.activate_session(hub_key, &guild_ids);
    // A removal (leave, kick, ban) committed between the read above and the subscription found
    // nothing to unsubscribe: read again, so a guild the user is no longer in stays silent.
    let current_guild_ids: HashSet<Snowflake> = match state.guilds.list_my_guilds(user.id).await {
        Ok(guilds) => guilds.iter().map(|guild| guild.id).collect(),
        Err(error) => {
            tracing::warn!(%error, "gateway identify: cannot list guilds");
            state.events.unregister_session(hub_key);
            return None;
        }
    };
    for removed_guild_id in guild_ids
        .iter()
        .filter(|guild_id| !current_guild_ids.contains(guild_id))
    {
        state
            .events
            .unsubscribe_user_from_guild(user.id, *removed_guild_id);
    }
    let Some(session_id) = generate_session_id() else {
        state.events.unregister_session(hub_key);
        return None;
    };
    let (commands_sender, commands) = mpsc::channel(SESSION_COMMAND_QUEUE_CAPACITY);
    state.gateway_sessions.sessions().insert(
        session_id.clone(),
        ResumableSession {
            user_id: user.id,
            hub_key,
            commands: commands_sender.clone(),
        },
    );
    tokio::spawn(run_session(
        state.clone(),
        session_id.clone(),
        hub_key,
        SessionTask { events, commands },
    ));

    let Some(ready) = build_ready(state, &user, session_id.clone(), &guild_ids).await else {
        // Dropping the registry's sender (and ours) ends the session task at once.
        state.gateway_sessions.sessions().remove(&session_id);
        return None;
    };
    if send_frame(socket, &ServerFrame::Ready { d: Box::new(ready) }).await {
        Some((user, commands_sender, hub_key))
    } else {
        state.gateway_sessions.sessions().remove(&session_id);
        None
    }
}

async fn build_ready(
    state: &AppState,
    user: &CurrentUser,
    session_id: String,
    guild_ids: &[Snowflake],
) -> Option<Ready> {
    let mut guilds = Vec::with_capacity(guild_ids.len());
    let mut presences = BTreeMap::new();
    for guild_id in guild_ids {
        match state.guilds.get_guild(user.id, *guild_id).await {
            Ok(guild) => {
                guilds.push(guild);
                // From the hub, not the member list: that list stops at its first 1 000 members.
                presences.extend(
                    state
                        .events
                        .visible_presences_of_guild(*guild_id)
                        .into_iter()
                        .map(|presence| (presence.user_id, presence)),
                );
            }
            // Left meanwhile: its GUILD_DELETE is already queued for this session.
            Err(GuildError::NotFound) => {}
            Err(error) => {
                tracing::warn!(%error, "gateway identify: cannot build ready");
                return None;
            }
        }
    }
    let direct_message_channels = match state.messages.list_direct_message_channels(user.id).await {
        Ok(channels) => channels,
        Err(error) => {
            tracing::warn!(%error, "gateway identify: cannot list conversations");
            return None;
        }
    };
    let visible_channel_ids: HashSet<Snowflake> = guilds
        .iter()
        .flat_map(|guild| guild.channels.iter().map(|channel| channel.id))
        .chain(direct_message_channels.iter().map(|channel| channel.id))
        .collect();
    let read_states = match state
        .messages
        .list_my_read_states_in(user.id, &visible_channel_ids)
        .await
    {
        Ok(read_states) => read_states,
        Err(error) => {
            tracing::warn!(%error, "gateway identify: cannot list read states");
            return None;
        }
    };
    let guild_channel_ids: Vec<Snowflake> = guilds
        .iter()
        .flat_map(|guild| guild.channels.iter().map(|channel| channel.id))
        .collect();
    let direct_channel_ids: Vec<Snowflake> = direct_message_channels
        .iter()
        .map(|channel| channel.id)
        .collect();
    let channel_unreads = match state
        .messages
        .list_my_channel_unreads(user.id, &guild_channel_ids, &direct_channel_ids)
        .await
    {
        Ok(channel_unreads) => channel_unreads,
        Err(error) => {
            tracing::warn!(%error, "gateway identify: cannot list unread channels");
            return None;
        }
    };
    let voice_states = visible_voice_states(state, &guilds).await;
    let relationships = match state.relationships.list(user.id).await {
        Ok(relationships) => relationships,
        Err(error) => {
            tracing::warn!(%error, "gateway identify: cannot list relationships");
            return None;
        }
    };
    // Friends show their presence without a shared guild (ADR-0025).
    presences.extend(
        relationships
            .iter()
            .filter(|relationship| relationship.kind == RelationshipKind::Friend)
            .filter_map(|relationship| state.events.visible_presence_of(relationship.user.id))
            .map(|presence| (presence.user_id, presence)),
    );
    Some(Ready {
        session_id,
        user: user.clone(),
        guilds,
        read_states,
        online_user_ids: presences.keys().copied().collect(),
        direct_message_channels,
        channel_unreads,
        presences: presences.into_values().collect(),
        relationships,
        may_create_guilds: state
            .guilds
            .may_create_guilds(user.is_instance_administrator),
        voice_states,
        voice_enabled: state.voice.is_available(),
    })
}

/// Who is connected to the voice channels among `guilds`' channels (those the reader can view).
async fn visible_voice_states(state: &AppState, guilds: &[Guild]) -> Vec<VoiceState> {
    let voice_channel_ids: HashSet<Snowflake> = guilds
        .iter()
        .flat_map(|guild| guild.channels.iter())
        .filter(|channel| channel.kind == ChannelKind::Voice)
        .map(|channel| channel.id)
        .collect();
    state.voice.voice_states_in(&voice_channel_ids).await
}

fn generate_session_id() -> Option<String> {
    let mut bytes = [0u8; SESSION_ID_RANDOM_BYTES];
    getrandom::fill(&mut bytes).ok()?;
    Some(
        bytes
            .iter()
            .flat_map(|byte| [byte >> 4, byte & 0x0F])
            // A nibble is always below 16, so `from_digit` never drops one.
            .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
            .collect(),
    )
}

/// Runs the session until it expires or is cut off, then removes it; the hub announces
/// "offline" if it was the user's last session.
async fn run_session(state: AppState, session_id: String, hub_key: SessionKey, task: SessionTask) {
    let end = task.run().await;
    state.gateway_sessions.sessions().remove(&session_id);
    state.voice.end_session(hub_key).await;
    state.events.unregister_session(hub_key);
    if end == SessionEnd::CutOff {
        tracing::info!("gateway session cut off (too slow or token revoked)");
    }
}

/// What a WebSocket attached to a session acts as.
struct AttachedSession<'a> {
    token: &'a str,
    user: &'a CurrentUser,
    hub_key: SessionKey,
    host_header: Option<String>,
}

/// Voice frames are handled in order, so a leave sent after a join never overtakes it.
async fn handle_voice_frame(state: &AppState, attached: &AttachedSession<'_>, frame: ClientFrame) {
    let user_id = attached.user.id;
    let now = std::time::Instant::now();
    match frame {
        ClientFrame::VoiceJoin {
            channel_id,
            sdp_offer,
            self_mute,
            self_deaf,
        } => {
            let key = format!("voice-join:{user_id}");
            if !state
                .rate_limiter
                .record_event(&key, VOICE_JOINS_PER_USER, now)
            {
                state.voice.refuse_join(
                    attached.hub_key,
                    channel_id,
                    VoiceJoinRefusal::RateLimited,
                );
                return;
            }
            state
                .voice
                .join(VoiceJoinRequest {
                    user_id,
                    session: attached.hub_key,
                    channel_id,
                    sdp_offer,
                    self_mute,
                    self_deaf,
                    host_header: attached.host_header.clone(),
                })
                .await;
        }
        ClientFrame::VoiceLeave => state.voice.leave(user_id, attached.hub_key).await,
        ClientFrame::VoiceState {
            self_mute,
            self_deaf,
        } => {
            let key = format!("voice-state:{user_id}");
            // Over the limit, the change is dropped: the client's next one sets the state anyway.
            if state
                .rate_limiter
                .record_event(&key, VOICE_STATE_CHANGES_PER_USER, now)
            {
                state
                    .voice
                    .set_self_state(user_id, attached.hub_key, self_mute, self_deaf)
                    .await;
            }
        }
        ClientFrame::Identify { .. } | ClientFrame::Resume { .. } | ClientFrame::Heartbeat => {}
    }
}

/// Pumps frames to the socket and answers heartbeats until the client goes away.
async fn run_attached(
    mut socket: WebSocket,
    state: &AppState,
    attached: &AttachedSession<'_>,
    mut outgoing_frames: mpsc::Receiver<FrameText>,
) {
    let (token, user) = (attached.token, attached.user);
    let mut heartbeat_deadline = Instant::now() + HEARTBEAT_DEADLINE;
    let mut last_revalidated = Instant::now();
    loop {
        tokio::select! {
            frame = next_client_frame(&mut socket) => match frame {
                Some(ClientFrame::Heartbeat) => {
                    // Heartbeats revalidate the session (logout, disabled account), at most every
                    // 15 s so a client spamming heartbeats cannot load the database.
                    if last_revalidated.elapsed() >= MINIMUM_INTERVAL_BETWEEN_REVALIDATIONS {
                        let still_valid = state
                            .accounts
                            .authenticate(token, now_unix_milliseconds())
                            .await
                            .is_ok_and(|current| current.id == user.id);
                        if !still_valid {
                            return refuse(socket).await;
                        }
                        last_revalidated = Instant::now();
                    }
                    heartbeat_deadline = Instant::now() + HEARTBEAT_DEADLINE;
                    if !send_frame(&mut socket, &ServerFrame::HeartbeatAck).await {
                        return;
                    }
                }
                Some(
                    frame @ (ClientFrame::VoiceJoin { .. }
                    | ClientFrame::VoiceLeave
                    | ClientFrame::VoiceState { .. }),
                ) => handle_voice_frame(state, attached, frame).await,
                // A second identify/resume, malformed JSON or a closed socket ends the connection.
                Some(ClientFrame::Identify { .. } | ClientFrame::Resume { .. }) | None => break,
            },
            frame = outgoing_frames.recv() => match frame {
                Some(text) => {
                    if !send_text(&mut socket, &text).await {
                        return;
                    }
                }
                // The session ended (cut off): its invalid-session frame was already sent.
                None => break,
            },
            () = tokio::time::sleep_until(heartbeat_deadline) => break,
        }
    }
    close(socket).await;
}

#[cfg(test)]
pub(crate) mod tests {
    use std::net::SocketAddr;

    use futures_util::{SinkExt as _, StreamExt as _};
    use mellow_protocol::{
        CreateGuildRequest, CreateInviteRequest, GatewayEvent, Presence, PresenceStatus,
        SendMessageRequest,
    };
    use tokio_tungstenite::tungstenite::Message as ClientMessage;

    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::tests::{migrated_database, register_request};
    use crate::http::tests::test_state;

    pub(crate) type ClientSocket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    const RECEIVE_TIMEOUT: Duration = Duration::from_secs(5);

    struct Fixture {
        _directory: tempfile::TempDir,
        state: AppState,
        address: SocketAddr,
        owner_token: String,
        member_token: String,
        guild_id: Snowflake,
        general: Snowflake,
    }

    /// Real time: the gateway validates sessions against the clock, so fixtures must too.
    fn now() -> u64 {
        now_unix_milliseconds()
    }

    async fn fixture() -> Fixture {
        let (directory, database) = migrated_database().await;
        let state = test_state(database, RegistrationMode::Open);
        let owner = state
            .accounts
            .register(register_request("owner", "owner@example.org"), now())
            .await
            .unwrap();
        let member = state
            .accounts
            .register(register_request("member", "member@example.org"), now())
            .await
            .unwrap();
        let guild = state
            .guilds
            .create_guild(
                owner.user.id,
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
            .create_invite(
                owner.user.id,
                guild.id,
                CreateInviteRequest::default(),
                now(),
            )
            .await
            .unwrap();
        state
            .guilds
            .join_with_invite(member.user.id, &invite.code, now())
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(crate::commands::serve_until_shutdown(
            listener,
            state.clone(),
            std::future::pending(),
        ));
        Fixture {
            _directory: directory,
            state,
            address,
            owner_token: owner.token,
            member_token: member.token,
            guild_id: guild.id,
            general: guild.channels[1].id,
        }
    }

    impl Fixture {
        /// SQLCipher must not be closing connections when the test process exits (ADR-0006).
        async fn close(self) {
            self.state.database.close().await;
        }
    }

    pub(crate) async fn connect(address: SocketAddr) -> ClientSocket {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/gateway?v=1"))
            .await
            .unwrap();
        socket
    }

    pub(crate) async fn send(socket: &mut ClientSocket, frame: &ClientFrame) {
        socket
            .send(ClientMessage::Text(
                serde_json::to_string(frame).unwrap().into(),
            ))
            .await
            .unwrap();
    }

    pub(crate) async fn receive(socket: &mut ClientSocket) -> ServerFrame {
        loop {
            let message = tokio::time::timeout(RECEIVE_TIMEOUT, socket.next())
                .await
                .expect("no frame within the timeout")
                .expect("socket closed")
                .unwrap();
            if let ClientMessage::Text(text) = message {
                return serde_json::from_str(&text).unwrap();
            }
        }
    }

    /// Skips dispatches until `matches` accepts one; returns its sequence and event.
    pub(crate) async fn receive_dispatch(
        socket: &mut ClientSocket,
        matches: impl Fn(&GatewayEvent) -> bool,
    ) -> (u64, GatewayEvent) {
        loop {
            if let ServerFrame::Dispatch {
                sequence: s,
                event: d,
            } = receive(socket).await
                && matches(&d)
            {
                return (s, *d);
            }
        }
    }

    pub(crate) async fn identified(address: SocketAddr, token: &str) -> (ClientSocket, Ready) {
        let mut socket = connect(address).await;
        assert!(matches!(
            receive(&mut socket).await,
            ServerFrame::Hello { .. }
        ));
        send(
            &mut socket,
            &ClientFrame::Identify {
                token: token.into(),
            },
        )
        .await;
        match receive(&mut socket).await {
            ServerFrame::Ready { d } => (socket, *d),
            other => panic!("expected ready, got {other:?}"),
        }
    }

    async fn post_message(
        fixture: &Fixture,
        token: &str,
        content: &str,
    ) -> mellow_protocol::Message {
        let author = fixture
            .state
            .accounts
            .authenticate(token, now())
            .await
            .unwrap();
        fixture
            .state
            .messages
            .send(
                author.id,
                fixture.general,
                SendMessageRequest {
                    content: content.into(),
                    reply_to_id: None,
                    // Unique per message: a repeated nonce returns the first message.
                    nonce: Some(format!("private-{}", content.replace(' ', "-"))),
                    attachment_ids: Vec::new(),
                },
                now(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn identify_returns_ready_then_live_dispatches_without_the_nonce() {
        let fixture = fixture().await;
        let (mut socket, ready) = identified(fixture.address, &fixture.member_token).await;
        assert_eq!(ready.user.username, "member");
        assert_eq!(ready.guilds[0].id, fixture.guild_id);
        assert!(ready.online_user_ids.contains(&ready.user.id));
        // The fixture's instance lets everyone create servers.
        assert!(ready.may_create_guilds);

        let posted = post_message(&fixture, &fixture.owner_token, "hello").await;
        let (sequence, event) = receive_dispatch(&mut socket, |event| {
            matches!(event, GatewayEvent::MessageCreate(_))
        })
        .await;
        assert!(sequence >= 1);
        match event {
            GatewayEvent::MessageCreate(message) => {
                assert_eq!(message.id, posted.id);
                assert_eq!(message.nonce, None, "the nonce stays private to the sender");
            }
            other => panic!("unexpected {other:?}"),
        }
        fixture.close().await;
    }

    #[tokio::test]
    async fn ready_tells_where_each_channel_stands_with_unread_mentions() {
        let fixture = fixture().await;
        post_message(&fixture, &fixture.owner_token, "hi @member").await;
        let posted = post_message(&fixture, &fixture.owner_token, "and @member again").await;
        let (_socket, ready) = identified(fixture.address, &fixture.member_token).await;
        fixture.close().await;

        assert_eq!(
            ready.channel_unreads,
            [mellow_protocol::ChannelUnread {
                channel_id: posted.channel_id,
                last_message_id: posted.id,
                mention_count: 2,
            }]
        );
    }

    #[tokio::test]
    #[cfg_attr(
        windows,
        ignore = "Windows resets the socket instead of delivering the close frame; the server refuses Windows for now"
    )]
    async fn heartbeats_are_acknowledged_until_the_session_is_revoked() {
        let fixture = fixture().await;
        let (mut socket, _) = identified(fixture.address, &fixture.member_token).await;
        send(&mut socket, &ClientFrame::Heartbeat).await;
        loop {
            match receive(&mut socket).await {
                ServerFrame::HeartbeatAck => break,
                ServerFrame::Dispatch { .. } => {}
                other => panic!("unexpected {other:?}"),
            }
        }
        fixture
            .state
            .accounts
            .logout(&fixture.member_token)
            .await
            .unwrap();
        send(&mut socket, &ClientFrame::Heartbeat).await;
        loop {
            match receive(&mut socket).await {
                ServerFrame::InvalidSession => break,
                ServerFrame::Dispatch { .. } => {}
                other => panic!("unexpected {other:?}"),
            }
        }
        fixture.close().await;
    }

    #[tokio::test]
    async fn a_bad_token_or_version_is_refused() {
        let fixture = fixture().await;
        let mut socket = connect(fixture.address).await;
        receive(&mut socket).await;
        send(
            &mut socket,
            &ClientFrame::Identify {
                token: "mlw_nope".into(),
            },
        )
        .await;
        assert_eq!(receive(&mut socket).await, ServerFrame::InvalidSession);

        let wrong_version =
            tokio_tungstenite::connect_async(format!("ws://{}/gateway?v=2", fixture.address)).await;
        assert!(
            wrong_version.is_err(),
            "the upgrade is refused with a JSON 400"
        );
        fixture.close().await;
    }

    #[tokio::test]
    async fn a_dropped_client_resumes_and_receives_what_it_missed() {
        let fixture = fixture().await;
        let (mut socket, ready) = identified(fixture.address, &fixture.member_token).await;
        let first = post_message(&fixture, &fixture.owner_token, "before the drop").await;
        let (last_sequence, _) = receive_dispatch(
            &mut socket,
            |event| matches!(event, GatewayEvent::MessageCreate(message) if message.id == first.id),
        )
        .await;
        drop(socket);
        let missed = post_message(&fixture, &fixture.owner_token, "while away").await;

        let mut resumed = connect(fixture.address).await;
        receive(&mut resumed).await;
        send(
            &mut resumed,
            &ClientFrame::Resume {
                token: fixture.member_token.clone(),
                session_id: ready.session_id.clone(),
                last_sequence,
            },
        )
        .await;
        let (_, event) = receive_dispatch(&mut resumed, |_| true).await;
        assert!(matches!(event, GatewayEvent::MessageCreate(message) if message.id == missed.id));
        assert_eq!(receive(&mut resumed).await, ServerFrame::Resumed);

        // Another user cannot take over the session.
        let mut hijack = connect(fixture.address).await;
        receive(&mut hijack).await;
        send(
            &mut hijack,
            &ClientFrame::Resume {
                token: fixture.owner_token.clone(),
                session_id: ready.session_id,
                last_sequence: 0,
            },
        )
        .await;
        assert_eq!(receive(&mut hijack).await, ServerFrame::InvalidSession);
        fixture.close().await;
    }

    #[tokio::test]
    async fn members_see_each_other_come_online() {
        let fixture = fixture().await;
        let (mut owner_socket, _) = identified(fixture.address, &fixture.owner_token).await;
        let (_member_socket, member_ready) =
            identified(fixture.address, &fixture.member_token).await;
        let (_, event) = receive_dispatch(&mut owner_socket, |event| {
            matches!(event, GatewayEvent::PresenceUpdate(presence) if presence.user_id == member_ready.user.id)
        })
        .await;
        assert!(matches!(
            event,
            GatewayEvent::PresenceUpdate(Presence {
                status: PresenceStatus::Online,
                ..
            })
        ));
        fixture.close().await;
    }

    #[tokio::test]
    async fn an_invisible_member_connecting_is_announced_to_no_one() {
        let fixture = fixture().await;
        let member = fixture
            .state
            .accounts
            .update_presence(
                &fixture.member_token,
                mellow_protocol::UpdatePresenceRequest {
                    status: mellow_protocol::ChosenStatus::Invisible,
                    custom_status: Some("Hidden".into()),
                },
                now(),
            )
            .await
            .unwrap();
        let (mut owner_socket, _) = identified(fixture.address, &fixture.owner_token).await;
        let (_member_socket, member_ready) =
            identified(fixture.address, &fixture.member_token).await;
        assert_eq!(
            member_ready.user.status,
            mellow_protocol::ChosenStatus::Invisible
        );
        // Anything published now reaches the owner after the member's identify.
        let marker = GatewayEvent::TypingStart {
            guild_id: Some(fixture.guild_id),
            channel_id: fixture.general,
            user_id: member.id,
        };
        fixture
            .state
            .events
            .publish_to_guild(fixture.guild_id, &marker);
        let (_, before_marker) = receive_dispatch(&mut owner_socket, |event| {
            *event == marker
                || matches!(event, GatewayEvent::PresenceUpdate(presence) if presence.user_id == member.id)
        })
        .await;
        assert_eq!(before_marker, marker, "the member was announced");
        let (_, owner_ready) = identified(fixture.address, &fixture.owner_token).await;
        assert!(!owner_ready.online_user_ids.contains(&member.id));
        assert!(
            owner_ready
                .presences
                .iter()
                .all(|presence| presence.user_id != member.id)
        );
        fixture.close().await;
    }

    #[tokio::test]
    async fn joining_and_leaving_a_guild_updates_live_sessions() {
        let fixture = fixture().await;
        let newcomer = fixture
            .state
            .accounts
            .register(register_request("newcomer", "newcomer@example.org"), now())
            .await
            .unwrap();
        let (mut newcomer_socket, ready) = identified(fixture.address, &newcomer.token).await;
        assert_eq!(ready.guilds, Vec::new());
        let (mut owner_socket, _) = identified(fixture.address, &fixture.owner_token).await;

        let owner = fixture
            .state
            .accounts
            .authenticate(&fixture.owner_token, now())
            .await
            .unwrap();
        let invite = fixture
            .state
            .guilds
            .create_invite(
                owner.id,
                fixture.guild_id,
                CreateInviteRequest::default(),
                now(),
            )
            .await
            .unwrap();
        fixture
            .state
            .guilds
            .join_with_invite(newcomer.user.id, &invite.code, now())
            .await
            .unwrap();
        receive_dispatch(&mut newcomer_socket, |event| {
            matches!(event, GatewayEvent::GuildCreate(guild) if guild.id == fixture.guild_id)
        })
        .await;
        receive_dispatch(&mut owner_socket, |event| {
            matches!(event, GatewayEvent::GuildMemberAdd { member, .. } if member.user.id == newcomer.user.id)
        })
        .await;

        // The newcomer now receives the guild's messages.
        let posted = post_message(&fixture, &fixture.owner_token, "welcome").await;
        receive_dispatch(&mut newcomer_socket, |event| {
            matches!(event, GatewayEvent::MessageCreate(message) if message.id == posted.id)
        })
        .await;

        fixture
            .state
            .guilds
            .leave_guild(newcomer.user.id, fixture.guild_id)
            .await
            .unwrap();
        receive_dispatch(
            &mut newcomer_socket,
            |event| matches!(event, GatewayEvent::GuildDelete { id } if *id == fixture.guild_id),
        )
        .await;
        receive_dispatch(&mut owner_socket, |event| {
            matches!(event, GatewayEvent::GuildMemberRemove { user_id, .. } if *user_id == newcomer.user.id)
        })
        .await;
        fixture.close().await;
    }

    #[tokio::test]
    async fn role_changes_reach_live_sessions() {
        let fixture = fixture().await;
        let (mut owner_socket, _) = identified(fixture.address, &fixture.owner_token).await;
        let owner = fixture
            .state
            .accounts
            .authenticate(&fixture.owner_token, now())
            .await
            .unwrap();
        let guilds = &fixture.state.guilds;
        let role = guilds
            .create_role(
                owner.id,
                fixture.guild_id,
                mellow_protocol::CreateRoleRequest {
                    name: "Moderator".into(),
                    color: 0,
                    permissions: mellow_protocol::Permissions::KICK_MEMBERS,
                },
                now(),
            )
            .await
            .unwrap();
        receive_dispatch(&mut owner_socket, |event| {
            matches!(event, GatewayEvent::GuildRolesUpdate { roles, .. }
                if roles.iter().any(|listed| listed.id == role.id))
        })
        .await;
        guilds
            .add_member_role(owner.id, fixture.guild_id, owner.id, role.id)
            .await
            .unwrap();
        receive_dispatch(&mut owner_socket, |event| {
            matches!(event, GatewayEvent::GuildMemberUpdate { user_id, role_ids, .. }
                if *user_id == owner.id && *role_ids == [role.id])
        })
        .await;
        let second = guilds
            .create_role(
                owner.id,
                fixture.guild_id,
                mellow_protocol::CreateRoleRequest {
                    name: "Helper".into(),
                    color: 0,
                    permissions: mellow_protocol::Permissions::empty(),
                },
                now(),
            )
            .await
            .unwrap();
        // Helper was created last, so it sits below Moderator: put it on top.
        guilds
            .reorder_roles(
                owner.id,
                fixture.guild_id,
                mellow_protocol::ReorderRolesRequest {
                    role_ids: vec![role.id, second.id],
                },
            )
            .await
            .unwrap();
        receive_dispatch(&mut owner_socket, |event| {
            matches!(event, GatewayEvent::GuildRolesUpdate { roles, .. }
                if roles.last().is_some_and(|top| top.id == second.id))
        })
        .await;
        guilds
            .delete_role(owner.id, fixture.guild_id, role.id)
            .await
            .unwrap();
        receive_dispatch(&mut owner_socket, |event| {
            matches!(event, GatewayEvent::GuildRolesUpdate { roles, .. }
                if roles.iter().all(|listed| listed.id != role.id))
        })
        .await;
        fixture.close().await;
    }

    #[tokio::test]
    async fn a_banned_member_loses_the_guild_on_live_sessions() {
        let fixture = fixture().await;
        let banned = fixture
            .state
            .accounts
            .register(register_request("banned", "banned@example.org"), now())
            .await
            .unwrap();
        let owner = fixture
            .state
            .accounts
            .authenticate(&fixture.owner_token, now())
            .await
            .unwrap();
        let invite = fixture
            .state
            .guilds
            .create_invite(
                owner.id,
                fixture.guild_id,
                CreateInviteRequest::default(),
                now(),
            )
            .await
            .unwrap();
        fixture
            .state
            .guilds
            .join_with_invite(banned.user.id, &invite.code, now())
            .await
            .unwrap();
        let (mut banned_socket, _) = identified(fixture.address, &banned.token).await;
        let (mut owner_socket, _) = identified(fixture.address, &fixture.owner_token).await;
        fixture
            .state
            .guilds
            .ban_member(
                owner.id,
                fixture.guild_id,
                banned.user.id,
                mellow_protocol::CreateBanRequest::default(),
                now(),
            )
            .await
            .unwrap();
        receive_dispatch(
            &mut banned_socket,
            |event| matches!(event, GatewayEvent::GuildDelete { id } if *id == fixture.guild_id),
        )
        .await;
        receive_dispatch(&mut owner_socket, |event| {
            matches!(event, GatewayEvent::GuildMemberRemove { user_id, .. } if *user_id == banned.user.id)
        })
        .await;
        fixture.close().await;
    }

    #[tokio::test]
    async fn logout_ends_the_live_session_immediately() {
        let fixture = fixture().await;
        let (mut socket, _) = identified(fixture.address, &fixture.member_token).await;
        fixture
            .state
            .accounts
            .logout(&fixture.member_token)
            .await
            .unwrap();
        loop {
            match receive(&mut socket).await {
                ServerFrame::InvalidSession => break,
                ServerFrame::Dispatch { .. } => {}
                other => panic!("unexpected {other:?}"),
            }
        }
        fixture.close().await;
    }

    #[tokio::test]
    async fn the_web_client_identifies_with_its_cookie_and_other_sites_are_refused() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        let fixture = fixture().await;
        let request_from = |origin: &str| {
            let mut request = format!("ws://{}/gateway?v=1", fixture.address)
                .into_client_request()
                .unwrap();
            let headers = request.headers_mut();
            headers.insert(
                "cookie",
                format!("__Host-mellow_session={}", fixture.member_token)
                    .parse()
                    .unwrap(),
            );
            headers.insert("origin", origin.parse().unwrap());
            request
        };
        let cross_site =
            tokio_tungstenite::connect_async(request_from("https://evil.example")).await;
        let (mut socket, _) =
            tokio_tungstenite::connect_async(request_from(&format!("http://{}", fixture.address)))
                .await
                .unwrap();
        assert!(matches!(
            receive(&mut socket).await,
            ServerFrame::Hello { .. }
        ));
        send(
            &mut socket,
            &ClientFrame::Identify {
                token: String::new(),
            },
        )
        .await;
        let ready = receive(&mut socket).await;
        fixture.close().await;

        assert!(cross_site.is_err(), "the upgrade is refused");
        assert!(matches!(ready, ServerFrame::Ready { .. }));
    }

    #[tokio::test]
    async fn an_oversized_client_frame_closes_the_connection() {
        let fixture = fixture().await;
        let mut socket = connect(fixture.address).await;
        receive(&mut socket).await;
        let oversized = format!(
            r#"{{"op":"identify","d":{{"token":"{}"}}}}"#,
            "x".repeat(MAXIMUM_CLIENT_FRAME_BYTES)
        );
        let _ = socket.send(ClientMessage::Text(oversized.into())).await;
        assert_eq!(receive(&mut socket).await, ServerFrame::InvalidSession);
        let next = tokio::time::timeout(RECEIVE_TIMEOUT, socket.next())
            .await
            .unwrap();
        assert!(
            matches!(next, None | Some(Err(_) | Ok(ClientMessage::Close(_)))),
            "{next:?}"
        );
        fixture.close().await;
    }
}
