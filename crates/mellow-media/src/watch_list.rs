//! `voice_video_watch` lists, paced under the server's limit (30 frames per 10 s): a change
//! goes at once when the last list left long enough ago, otherwise the latest change waits and
//! leaves as soon as it may, so the final list always arrives (the web's
//! `WATCH_LIST_MINIMUM_INTERVAL_MILLISECONDS`). The network thread asks every turn.

use std::time::{Duration, Instant};

use mellow_protocol::WatchedCamera;

/// At most 25 lists per 10 s: under the server's 30, with room for the other video frames.
const MINIMUM_INTERVAL_BETWEEN_LISTS: Duration = Duration::from_millis(400);

/// When lists may leave.
#[derive(Debug, Default)]
pub(crate) struct WatchListPacing {
    waiting: Option<Vec<WatchedCamera>>,
    last_sent_at: Option<Instant>,
}

impl WatchListPacing {
    /// `cameras` if it may leave now (then counted as sent), or kept as the waiting list.
    pub(crate) fn offer(
        &mut self,
        cameras: Vec<WatchedCamera>,
        now: Instant,
    ) -> Option<Vec<WatchedCamera>> {
        self.waiting = Some(cameras);
        self.take_due(now)
    }

    /// The waiting list, if it may leave now (then counted as sent).
    pub(crate) fn take_due(&mut self, now: Instant) -> Option<Vec<WatchedCamera>> {
        if self
            .last_sent_at
            .is_some_and(|at| now.saturating_duration_since(at) < MINIMUM_INTERVAL_BETWEEN_LISTS)
        {
            return None;
        }
        let cameras = self.waiting.take()?;
        self.last_sent_at = Some(now);
        Some(cameras)
    }

    /// A list sent outside the pacing (after a join, which forgot the previous one): it
    /// replaces any waiting list.
    pub(crate) fn sent_now(&mut self, now: Instant) {
        self.waiting = None;
        self.last_sent_at = Some(now);
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
        assert_eq!(pacing.take_due(soon), None, "too early");
        let later = start + MINIMUM_INTERVAL_BETWEEN_LISTS;
        assert_eq!(pacing.take_due(later), Some(list(3)), "the last one wins");
        assert_eq!(
            pacing.take_due(later + MINIMUM_INTERVAL_BETWEEN_LISTS),
            None
        );
        let much_later = later + MINIMUM_INTERVAL_BETWEEN_LISTS;
        assert_eq!(pacing.offer(list(4), much_later), Some(list(4)));
    }

    #[test]
    fn a_list_sent_after_a_join_replaces_the_waiting_one() {
        let start = Instant::now();
        let mut pacing = WatchListPacing::default();
        pacing.offer(list(1), start);
        pacing.offer(list(2), start);
        pacing.sent_now(start);
        assert_eq!(
            pacing.take_due(start + MINIMUM_INTERVAL_BETWEEN_LISTS),
            None
        );
    }
}
