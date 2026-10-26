//! The live connection: identify, heartbeat, resume after drops, reconnect with backoff.

use std::hash::{BuildHasher as _, Hasher as _};
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use mellow_protocol::{ClientFrame, GatewayEvent, Ready, ServerFrame};
use tokio::sync::mpsc;
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

/// Keeps a session alive until the token is refused (sends `SignedOut`) or `updates` is dropped.
/// Abort the task to disconnect.
pub async fn run_gateway(
    instance: InstanceAddress,
    token: Zeroizing<String>,
    updates: mpsc::Sender<GatewayUpdate>,
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
        match run_connection(&url, &token, &mut resume_point, &updates).await {
            ConnectionEnd::UpdatesClosed => return,
            ConnectionEnd::IdentifyRefused if is_token_refused(&instance, &token).await => {
                let _ = updates.send(GatewayUpdate::SignedOut).await;
                return;
            }
            // Refused for another reason (session cap, transient server error): retry later.
            ConnectionEnd::IdentifyRefused => {
                tokio::time::sleep(reconnect_delay(consecutive_failures, random_u64())).await;
                consecutive_failures = consecutive_failures.saturating_add(1);
            }
            ConnectionEnd::Lost { was_connected } => {
                if was_connected {
                    consecutive_failures = 0;
                    if updates.send(GatewayUpdate::Disconnected).await.is_err() {
                        return;
                    }
                }
                tokio::time::sleep(reconnect_delay(consecutive_failures, random_u64())).await;
                consecutive_failures = consecutive_failures.saturating_add(1);
            }
        }
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

async fn run_connection(
    url: &Url,
    token: &Zeroizing<String>,
    resume_point: &mut Option<ResumePoint>,
    updates: &mpsc::Sender<GatewayUpdate>,
) -> ConnectionEnd {
    let lost = ConnectionEnd::Lost {
        was_connected: false,
    };
    let Some(socket) = connect(url).await else {
        return lost;
    };
    let (mut sink, mut stream) = socket.split();

    let Ok(Some(ServerFrame::Hello { d: hello })) =
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

    loop {
        let lost = ConnectionEnd::Lost {
            was_connected: is_connected,
        };
        let frame = tokio::select! {
            frame = next_frame(&mut stream) => frame,
            _ = heartbeat.tick() => {
                // No ack since the last beat: the connection is a zombie, start over.
                if !is_heartbeat_acknowledged || send_frame(&mut sink, &ClientFrame::Heartbeat).await.is_err() {
                    return lost;
                }
                is_heartbeat_acknowledged = false;
                continue;
            }
            () = &mut handshake_deadline, if !is_connected => return lost,
        };
        let update = match frame {
            None => return lost,
            Some(ServerFrame::Hello { .. }) => continue,
            Some(ServerFrame::HeartbeatAck) => {
                is_heartbeat_acknowledged = true;
                continue;
            }
            Some(ServerFrame::Ready { d: ready }) => {
                is_connected = true;
                *resume_point = Some(ResumePoint {
                    session_id: ready.session_id.clone(),
                    last_sequence: 0,
                });
                GatewayUpdate::Ready(ready)
            }
            Some(ServerFrame::Resumed) => {
                is_connected = true;
                GatewayUpdate::Resumed
            }
            Some(ServerFrame::Dispatch { sequence, event }) => {
                if let Some(point) = resume_point.as_mut() {
                    point.last_sequence = sequence;
                }
                GatewayUpdate::Event(*event)
            }
            Some(ServerFrame::InvalidSession) => {
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

/// The next server frame; `None` once the connection is closed or broken. Frames this client
/// cannot parse (a newer server) are skipped.
async fn next_frame<S>(stream: &mut S) -> Option<ServerFrame>
where
    S: futures_util::Stream<Item = Result<WebSocketMessage, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
{
    loop {
        match stream.next().await? {
            Ok(WebSocketMessage::Text(text)) => match serde_json::from_str(&text) {
                Ok(frame) => return Some(frame),
                // Never the error text itself: serde quotes the offending value, which may be
                // message content.
                Err(error) => tracing::warn!(
                    category = ?error.classify(),
                    line = error.line(),
                    column = error.column(),
                    "skipping an unknown gateway frame"
                ),
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
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let instance = InstanceAddress::parse(&format!("http://{address}")).unwrap();
        let (sender, updates) = mpsc::channel(16);
        let client = tokio::spawn(run_gateway(
            instance,
            Zeroizing::new("mlw_t".into()),
            sender,
        ));
        (listener, updates, client)
    }

    /// Accepts the next connection and sends `Hello` with the shortest heartbeat the client allows.
    async fn accept(listener: &TcpListener) -> ServerSocket {
        let (stream, _) = timeout(STEP_TIMEOUT, listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let hello = ServerFrame::Hello {
            d: Hello {
                heartbeat_interval_milliseconds: 1_000,
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
                },
                guilds: Vec::new(),
                read_states: Vec::new(),
                online_user_ids: Vec::new(),
                direct_message_channels: Vec::new(),
                channel_unreads: Vec::new(),
                presences: Vec::new(),
                relationships: Vec::new(),
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
