//! One gateway session: numbers events, keeps a replay buffer, and outlives its WebSocket for
//! `RESUME_WINDOW` so a dropped client can `Resume` without missing anything.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use mellow_protocol::ServerFrame;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use crate::events::SharedEvent;

/// How long a disconnected session waits for its client to resume.
pub(crate) const RESUME_WINDOW: Duration = Duration::from_secs(60);
/// Dispatches kept for replay; a client missing more resynchronises through REST.
pub(crate) const REPLAY_BUFFER_FRAMES: usize = 1_024;
/// Also bounded in bytes, so large messages cannot make one session hold megabytes.
pub(crate) const REPLAY_BUFFER_BYTES: usize = 2 * 1024 * 1024;
/// Frames queued towards one WebSocket; holds a full replay plus a margin.
pub(crate) const OUTGOING_QUEUE_FRAMES: usize = REPLAY_BUFFER_FRAMES + 64;

/// A serialized frame, ready for the socket.
pub(crate) type FrameText = Arc<str>;

pub(crate) fn frame_text(frame: &ServerFrame) -> Option<FrameText> {
    match serde_json::to_string(frame) {
        Ok(text) => Some(text.into()),
        Err(error) => {
            tracing::error!(%error, "cannot serialize a gateway frame");
            None
        }
    }
}

/// Builds the same JSON as `ServerFrame::Dispatch` around an event serialized once by the hub.
fn dispatch_frame(sequence: u64, event: &SharedEvent) -> FrameText {
    format!(
        r#"{{"op":"dispatch","s":{sequence},"d":{}}}"#,
        event.payload_json
    )
    .into()
}

#[derive(Debug)]
pub(crate) enum SessionCommand {
    /// Starts sending to `outgoing` everything after `after_sequence` (0 for a new session).
    Attach {
        attachment_id: u64,
        outgoing: mpsc::Sender<FrameText>,
        after_sequence: u64,
        is_resume: bool,
        reply: oneshot::Sender<Result<(), ResumeRefused>>,
    },
    /// The WebSocket `attachment_id` closed; ignored if a newer one is attached.
    Detach { attachment_id: u64 },
}

/// The requested sequence is no longer (or not yet) in the replay buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResumeRefused;

#[derive(Debug)]
pub(crate) struct SessionTask {
    pub(crate) events: mpsc::Receiver<SharedEvent>,
    pub(crate) commands: mpsc::Receiver<SessionCommand>,
}

#[derive(Debug)]
struct Attachment {
    id: u64,
    outgoing: mpsc::Sender<FrameText>,
}

/// Recent dispatches, sharing the hub's serialized events.
#[derive(Debug, Default)]
struct ReplayBuffer {
    entries: VecDeque<(u64, SharedEvent)>,
    bytes: usize,
}

impl ReplayBuffer {
    fn push(&mut self, sequence: u64, event: SharedEvent) {
        self.bytes += event.payload_json.len();
        self.entries.push_back((sequence, event));
        while self.entries.len() > REPLAY_BUFFER_FRAMES || self.bytes > REPLAY_BUFFER_BYTES {
            let Some((_, oldest)) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= oldest.payload_json.len();
        }
    }

    fn oldest_sequence(&self) -> Option<u64> {
        self.entries.front().map(|(sequence, _)| *sequence)
    }

    fn after(&self, sequence: u64) -> impl Iterator<Item = FrameText> + '_ {
        self.entries
            .iter()
            .filter(move |(entry_sequence, _)| *entry_sequence > sequence)
            .map(|(entry_sequence, event)| dispatch_frame(*entry_sequence, event))
    }
}

/// Why the session ended, for the caller's cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionEnd {
    /// No client resumed within `RESUME_WINDOW`, or the control channel closed.
    Expired,
    /// The hub cut the session off (too slow, or its token was revoked); the client was told to
    /// identify again.
    CutOff,
}

impl SessionTask {
    pub(crate) async fn run(mut self) -> SessionEnd {
        let mut next_sequence: u64 = 1;
        let mut replay = ReplayBuffer::default();
        let mut attachment: Option<Attachment> = None;
        // A session that is never attached expires like a detached one.
        let mut detached_since = Some(Instant::now());

        loop {
            let resume_deadline = detached_since.map(|since| since + RESUME_WINDOW);
            tokio::select! {
                event = self.events.recv() => {
                    let Some(event) = event else {
                        if let Some(attachment) = &attachment
                            && let Some(frame) = frame_text(&ServerFrame::InvalidSession)
                        {
                            let _ = attachment.outgoing.try_send(frame);
                        }
                        return SessionEnd::CutOff;
                    };
                    let sequence = next_sequence;
                    next_sequence += 1;
                    if let Some(current) = &attachment
                        && current.outgoing.try_send(dispatch_frame(sequence, &event)).is_err()
                    {
                        // The socket is gone or too slow: keep buffering until it resumes.
                        attachment = None;
                        detached_since = Some(Instant::now());
                    }
                    replay.push(sequence, event);
                }
                command = self.commands.recv() => match command {
                    Some(SessionCommand::Attach { attachment_id, outgoing, after_sequence, is_resume, reply }) => {
                        let oldest_available = replay.oldest_sequence().unwrap_or(next_sequence);
                        let is_replayable = after_sequence < next_sequence
                            && after_sequence.saturating_add(1) >= oldest_available;
                        if !is_replayable {
                            let _ = reply.send(Err(ResumeRefused));
                            continue;
                        }
                        let resumed = is_resume.then(|| frame_text(&ServerFrame::Resumed)).flatten();
                        let delivered = replay
                            .after(after_sequence)
                            .chain(resumed)
                            .all(|frame| outgoing.try_send(frame).is_ok());
                        if delivered {
                            attachment = Some(Attachment { id: attachment_id, outgoing });
                            detached_since = None;
                            let _ = reply.send(Ok(()));
                        } else {
                            let _ = reply.send(Err(ResumeRefused));
                        }
                    }
                    Some(SessionCommand::Detach { attachment_id }) => {
                        if attachment.as_ref().is_some_and(|current| current.id == attachment_id) {
                            attachment = None;
                            detached_since = Some(Instant::now());
                        }
                    }
                    None => return SessionEnd::Expired,
                },
                () = sleep_until_optional(resume_deadline) => return SessionEnd::Expired,
            }
        }
    }
}

async fn sleep_until_optional(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{GatewayEvent, Snowflake};

    use super::*;
    use crate::events::{EventHub, PublishedEvent};

    const GUILD: Snowflake = Snowflake::from_raw(5);
    const USER: Snowflake = Snowflake::from_raw(1);
    const NO_CAP: usize = 100;

    fn typing() -> GatewayEvent {
        GatewayEvent::TypingStart {
            guild_id: Some(GUILD),
            channel_id: Snowflake::from_raw(6),
            user_id: USER,
        }
    }

    async fn attach(
        commands: &mpsc::Sender<SessionCommand>,
        attachment_id: u64,
        after_sequence: u64,
    ) -> (Result<(), ResumeRefused>, mpsc::Receiver<FrameText>) {
        let (outgoing, frames) = mpsc::channel(OUTGOING_QUEUE_FRAMES);
        let (reply, answer) = oneshot::channel();
        commands
            .send(SessionCommand::Attach {
                attachment_id,
                outgoing,
                after_sequence,
                is_resume: after_sequence > 0,
                reply,
            })
            .await
            .unwrap();
        (answer.await.unwrap(), frames)
    }

    #[test]
    fn dispatch_frames_match_the_protocol_serialization() {
        let event = typing();
        let shared = Arc::new(PublishedEvent {
            payload_json: serde_json::to_string(&event).unwrap().into(),
        });
        let expected = frame_text(&ServerFrame::Dispatch {
            sequence: 3,
            event: Box::new(event),
        })
        .unwrap();
        assert_eq!(dispatch_frame(3, &shared), expected);
    }

    #[test]
    fn the_replay_buffer_is_bounded_in_bytes() {
        let mut replay = ReplayBuffer::default();
        let large = Arc::new(PublishedEvent {
            payload_json: "x".repeat(REPLAY_BUFFER_BYTES / 2 + 1).into(),
        });
        replay.push(1, large.clone());
        replay.push(2, large);
        assert_eq!(replay.oldest_sequence(), Some(2));
        assert!(replay.bytes <= REPLAY_BUFFER_BYTES);
    }

    #[tokio::test]
    async fn detached_sessions_buffer_and_replay_missed_dispatches_then_resumed() {
        let hub = EventHub::default();
        let (hub_key, events) = hub.register_session(USER, [0; 32], NO_CAP).unwrap();
        hub.set_presence(
            USER,
            crate::events::ChosenPresence {
                status: mellow_protocol::ChosenStatus::Online,
                custom_status: None,
                version: 0,
            },
        );
        hub.activate_session(hub_key, &[GUILD]);
        let (commands_sender, commands) = mpsc::channel(8);
        let task = tokio::spawn(SessionTask { events, commands }.run());

        let (attached, mut first_socket) = attach(&commands_sender, 1, 0).await;
        assert_eq!(attached, Ok(()));
        hub.publish_to_guild(GUILD, &typing());
        // Sequence 1 is the user's own "online" presence; 2 is the typing event.
        let first: Vec<String> = [
            first_socket.recv().await.unwrap(),
            first_socket.recv().await.unwrap(),
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert!(first[1].contains(r#""s":2"#), "{first:?}");

        commands_sender
            .send(SessionCommand::Detach { attachment_id: 1 })
            .await
            .unwrap();
        hub.publish_to_guild(GUILD, &typing());
        hub.publish_to_guild(GUILD, &typing());
        tokio::task::yield_now().await;

        let (resumed, mut second_socket) = attach(&commands_sender, 2, 2).await;
        assert_eq!(resumed, Ok(()));
        let replayed: Vec<String> = [
            second_socket.recv().await.unwrap(),
            second_socket.recv().await.unwrap(),
            second_socket.recv().await.unwrap(),
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert!(replayed[0].contains(r#""s":3"#) && replayed[1].contains(r#""s":4"#));
        assert_eq!(replayed[2], r#"{"op":"resumed"}"#);

        // A stale detach from the first socket must not detach the second.
        commands_sender
            .send(SessionCommand::Detach { attachment_id: 1 })
            .await
            .unwrap();
        hub.publish_to_guild(GUILD, &typing());
        assert!(second_socket.recv().await.unwrap().contains(r#""s":5"#));

        let (too_old, _) = attach(&commands_sender, 3, 99).await;
        assert_eq!(too_old, Err(ResumeRefused));
        drop(commands_sender);
        assert_eq!(task.await.unwrap(), SessionEnd::Expired);
    }

    #[tokio::test(start_paused = true)]
    async fn an_unattached_session_expires_after_the_resume_window() {
        let hub = EventHub::default();
        let (_hub_key, events) = hub.register_session(USER, [0; 32], NO_CAP).unwrap();
        let (_commands_sender, commands) = mpsc::channel(8);
        let task = tokio::spawn(SessionTask { events, commands }.run());
        tokio::time::advance(RESUME_WINDOW + Duration::from_secs(1)).await;
        assert_eq!(task.await.unwrap(), SessionEnd::Expired);
    }
}
