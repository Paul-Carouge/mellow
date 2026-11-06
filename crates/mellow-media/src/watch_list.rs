//! `voice_video_watch` lists, paced under the server's limit (30 frames per 10 s): a change
//! goes at once when the last list left long enough ago, otherwise the latest change waits and
//! leaves as soon as it may, so the final list always arrives (the web's
//! `WATCH_LIST_MINIMUM_INTERVAL_MILLISECONDS`).

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use mellow_protocol::{ClientFrame, WatchedCamera};

use crate::session::VoiceSignalling;

/// At most 25 lists per 10 s: under the server's 30, with room for the other video frames.
const MINIMUM_INTERVAL_BETWEEN_LISTS: Duration = Duration::from_millis(400);

/// The signalling, shared by the session and the watch-list thread.
pub(crate) type SharedSignalling = Arc<Mutex<Box<dyn VoiceSignalling>>>;

pub(crate) fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic while holding the lock leaves plain data behind: keep going with it.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// When lists may leave; pure, so the timing is tested without a clock.
#[derive(Debug, Default)]
struct WatchListPacing {
    waiting: Option<Vec<WatchedCamera>>,
    last_sent_at: Option<Instant>,
    is_stopped: bool,
}

impl WatchListPacing {
    fn may_send_at(&self) -> Option<Instant> {
        self.last_sent_at
            .map(|at| at + MINIMUM_INTERVAL_BETWEEN_LISTS)
    }

    /// `cameras` if it may leave now (then counted as sent), or kept as the waiting list.
    fn offer(&mut self, cameras: Vec<WatchedCamera>, now: Instant) -> Option<Vec<WatchedCamera>> {
        self.waiting = Some(cameras);
        self.take_due(now)
    }

    /// The waiting list, if it may leave now (then counted as sent).
    fn take_due(&mut self, now: Instant) -> Option<Vec<WatchedCamera>> {
        if self.is_stopped || self.may_send_at().is_some_and(|at| now < at) {
            return None;
        }
        let cameras = self.waiting.take()?;
        self.last_sent_at = Some(now);
        Some(cameras)
    }

    /// When the waiting list may leave; `None` when nothing waits.
    fn wake_at(&self) -> Option<Instant> {
        self.waiting
            .as_ref()
            .map(|_| self.may_send_at().unwrap_or_else(Instant::now))
    }
}

/// The session's end: lists in, frames out through the signalling, from the caller's thread
/// when they may leave at once and from the watch-list thread otherwise.
#[derive(Debug, Default)]
pub(crate) struct WatchListOutbox {
    pacing: Mutex<WatchListPacing>,
    has_work: Condvar,
}

fn frame_of(cameras: Vec<WatchedCamera>) -> ClientFrame {
    ClientFrame::VoiceVideoWatch { cameras }
}

impl WatchListOutbox {
    /// Sends `cameras` now if it may leave, or leaves it to the thread.
    pub(crate) fn send(&self, cameras: Vec<WatchedCamera>, signalling: &SharedSignalling) {
        let due = lock(&self.pacing).offer(cameras, Instant::now());
        match due {
            Some(cameras) => lock(signalling).send_frame(frame_of(cameras)),
            None => self.has_work.notify_one(),
        }
    }

    /// Sends `cameras` now whatever the pacing: a new join forgot the previous list.
    pub(crate) fn send_now(&self, cameras: Vec<WatchedCamera>, signalling: &SharedSignalling) {
        let mut pacing = lock(&self.pacing);
        pacing.waiting = None;
        pacing.last_sent_at = Some(Instant::now());
        drop(pacing);
        lock(signalling).send_frame(frame_of(cameras));
    }

    /// Drops any waiting list and ends the thread; nothing is sent after this returns.
    pub(crate) fn stop(&self) {
        lock(&self.pacing).is_stopped = true;
        self.has_work.notify_one();
    }

    /// Runs on the watch-list thread until [`WatchListOutbox::stop`].
    pub(crate) fn run(&self, signalling: &SharedSignalling) {
        loop {
            let mut pacing = lock(&self.pacing);
            loop {
                if pacing.is_stopped {
                    return;
                }
                let Some(wake_at) = pacing.wake_at() else {
                    pacing = self
                        .has_work
                        .wait(pacing)
                        .unwrap_or_else(PoisonError::into_inner);
                    continue;
                };
                let wait = wake_at.saturating_duration_since(Instant::now());
                if wait.is_zero() {
                    break;
                }
                pacing = self
                    .has_work
                    .wait_timeout(pacing, wait)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
            drop(pacing);
            // The signalling first, then the pacing, as `stop` then `voice_leave` take them: a
            // list taken here cannot leave after the session's `voice_leave`.
            let signalling = lock(signalling);
            if let Some(cameras) = lock(&self.pacing).take_due(Instant::now()) {
                signalling.send_frame(frame_of(cameras));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{Snowflake, VideoTileSize};

    use super::*;

    fn list(user: u64) -> Vec<WatchedCamera> {
        vec![WatchedCamera {
            user_id: Snowflake::from_raw(user),
            size: VideoTileSize::Large,
        }]
    }

    #[test]
    fn changes_close_together_send_the_first_at_once_and_only_the_last_after() {
        let start = Instant::now();
        let mut pacing = WatchListPacing::default();
        assert_eq!(pacing.offer(list(1), start), Some(list(1)));
        let soon = start + Duration::from_millis(100);
        assert_eq!(pacing.offer(list(2), soon), None);
        assert_eq!(pacing.offer(list(3), soon), None);
        assert_eq!(
            pacing.wake_at(),
            Some(start + MINIMUM_INTERVAL_BETWEEN_LISTS)
        );
        assert_eq!(pacing.take_due(soon), None, "too early");
        let later = start + MINIMUM_INTERVAL_BETWEEN_LISTS;
        assert_eq!(pacing.take_due(later), Some(list(3)), "the last one wins");
        assert_eq!(pacing.wake_at(), None);
        let much_later = later + MINIMUM_INTERVAL_BETWEEN_LISTS;
        assert_eq!(pacing.offer(list(4), much_later), Some(list(4)));
    }

    #[test]
    fn a_stopped_outbox_sends_nothing_more() {
        let mut pacing = WatchListPacing {
            is_stopped: true,
            ..WatchListPacing::default()
        };
        assert_eq!(pacing.offer(list(1), Instant::now()), None);
    }

    #[test]
    fn the_thread_sends_the_waiting_list_once_it_may() {
        let (frames, sent) = std::sync::mpsc::channel();
        let signalling: SharedSignalling = Arc::new(Mutex::new(Box::new(move |frame| {
            let _ = frames.send(frame);
        })));
        let outbox = Arc::new(WatchListOutbox::default());
        let thread = {
            let (outbox, signalling) = (Arc::clone(&outbox), Arc::clone(&signalling));
            std::thread::spawn(move || outbox.run(&signalling))
        };
        outbox.send(list(1), &signalling);
        outbox.send(list(2), &signalling);
        outbox.send(list(3), &signalling);
        assert_eq!(sent.recv().unwrap(), frame_of(list(1)));
        // However the three calls were spaced, the last list arrives.
        assert!(sent.iter().take(2).any(|frame| frame == frame_of(list(3))));
        outbox.stop();
        thread.join().unwrap();
    }
}
