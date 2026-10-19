//! The live connection: identify, heartbeat, resume after drops, reconnect with backoff.

use std::hash::{BuildHasher as _, Hasher as _};
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use mellow_protocol::{ClientFrame, GatewayEvent, Ready, ServerFrame};
use tokio::sync::mpsc;
use tokio::time::{Instant, timeout};
use tokio_tungstenite::tungstenite::Message as WebSocketMessage;
use url::Url;
use zeroize::Zeroizing;

use crate::instance::InstanceAddress;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The server sends `Hello` right away and answers `Identify` within its own 10 s limit.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const FIRST_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAXIMUM_RECONNECT_DELAY: Duration = Duration::from_secs(60);
/// Below the server's 45 s deadline, whatever interval it announces.
const MAXIMUM_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const MINIMUM_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

/// What the gateway reports to the app, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
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
        match run_connection(&url, &token, &mut resume_point, &updates).await {
            ConnectionEnd::UpdatesClosed => return,
            ConnectionEnd::IdentifyRefused => {
                let _ = updates.send(GatewayUpdate::SignedOut).await;
                return;
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

async fn run_connection(
    url: &Url,
    token: &Zeroizing<String>,
    resume_point: &mut Option<ResumePoint>,
    updates: &mpsc::Sender<GatewayUpdate>,
) -> ConnectionEnd {
    let lost = ConnectionEnd::Lost {
        was_connected: false,
    };
    let Ok(Ok((socket, _))) = timeout(
        CONNECT_TIMEOUT,
        tokio_tungstenite::connect_async(url.as_str()),
    )
    .await
    else {
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
                GatewayUpdate::Event(event)
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
                Err(error) => tracing::warn!(%error, "skipping an unknown gateway frame"),
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
    use super::*;

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
