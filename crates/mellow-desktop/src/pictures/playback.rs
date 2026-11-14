//! Animated pictures, played by the picture store rather than by GPUI's `img`: `img` advances the
//! frames of a picture only when the element has an id (none of Mellow's have one) and never while
//! the system asks to reduce motion, which "Always animate" overrides (User settings,
//! Accessibility). The store keeps one GPUI image per frame and hands `img` the frame due on its
//! own clock, so every picture plays the same way wherever it is drawn.

use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::RenderImage;
use image::Frame;

/// Frame changes are drawn on a shared beat from the animations' clock origin, so every picture
/// on screen changes in the same redraw: at most about 30 redraws a second, however many play.
pub(crate) const FRAME_CHANGE_BEAT: Duration = Duration::from_millis(33);
/// A picture not drawn for this long past its frame change was off screen (an avatar no longer
/// pointed at): it starts again from its first frame, as a browser's `img` does.
const RESTART_AFTER_MISSED_FRAME_BY: Duration = Duration::from_millis(250);

/// How many times an animation plays (GIF's and WebP's loop count); `None` plays forever.
pub(crate) type PlayCount = Option<u32>;

/// A decoded picture: one single-frame GPUI image per frame, with how long each one shows.
#[derive(Debug)]
pub(crate) struct DecodedPicture {
    frames: Vec<Arc<RenderImage>>,
    frame_durations: Vec<Duration>,
    loop_duration: Duration,
    play_count: PlayCount,
    /// When it started playing and when its next frame was due, while it is drawn.
    playback: Cell<Option<(Instant, Option<Instant>)>>,
}

impl DecodedPicture {
    pub(crate) fn new(frames: Vec<Frame>, play_count: PlayCount) -> Self {
        let frame_durations: Vec<Duration> = frames
            .iter()
            .map(|frame| Duration::from(frame.delay()))
            .collect();
        let loop_duration = frame_durations.iter().sum();
        let frames = frames
            .into_iter()
            .map(|frame| Arc::new(RenderImage::new(vec![frame])))
            .collect();
        Self {
            frames,
            frame_durations,
            loop_duration,
            play_count,
            playback: Cell::new(None),
        }
    }

    pub(crate) fn frames(&self) -> &[Arc<RenderImage>] {
        &self.frames
    }

    pub(crate) fn first_frame(&self) -> Option<&Arc<RenderImage>> {
        self.frames.first()
    }

    pub(crate) fn byte_count(&self) -> usize {
        self.frames
            .iter()
            .filter_map(|frame| frame.as_bytes(0))
            .map(<[u8]>::len)
            .sum()
    }

    /// The frame shown `elapsed` after the animation started, and how long it keeps showing;
    /// `None` for a picture that never changes again (still, or done playing).
    pub(crate) fn frame_due_after(&self, elapsed: Duration) -> (usize, Option<Duration>) {
        if self.frames.len() < 2 || self.loop_duration.is_zero() {
            return (0, None);
        }
        let completed_loops = elapsed.as_nanos() / self.loop_duration.as_nanos();
        if self
            .play_count
            .is_some_and(|play_count| completed_loops >= u128::from(play_count))
        {
            return (self.frames.len() - 1, None);
        }
        let mut position_in_loop = Duration::from_nanos(
            u64::try_from(elapsed.as_nanos() % self.loop_duration.as_nanos()).unwrap_or(0),
        );
        for (index, duration) in self.frame_durations.iter().enumerate() {
            if position_in_loop < *duration {
                return (index, Some(duration.saturating_sub(position_in_loop)));
            }
            position_in_loop = position_in_loop.saturating_sub(*duration);
        }
        // Unreachable: the position is below the sum of the durations.
        (0, self.frame_durations.first().copied())
    }

    /// The frame drawn at `now` and when the next one is due: from the first frame when it was
    /// not drawn since its last frame change was due.
    pub(crate) fn frame_drawn_at(&self, now: Instant) -> (usize, Option<Instant>) {
        let started_at = match self.playback.get() {
            Some((started_at, Some(next_due)))
                if now <= next_due + RESTART_AFTER_MISSED_FRAME_BY =>
            {
                started_at
            }
            // Still or done: it keeps its last frame without being drawn.
            Some((started_at, None)) => started_at,
            _ => now,
        };
        let (index, time_left) = self.frame_due_after(now.saturating_duration_since(started_at));
        let next_due = time_left.map(|time_left| now + time_left);
        self.playback.set(Some((started_at, next_due)));
        (index, next_due)
    }
}

/// `due` moved to the next beat after it, counted from `origin`.
pub(crate) fn on_frame_change_beat(due: Instant, origin: Instant) -> Instant {
    let since_origin = due.saturating_duration_since(origin).as_nanos();
    let beat = FRAME_CHANGE_BEAT.as_nanos();
    let beats = since_origin.div_ceil(beat);
    let offset = u64::try_from(beats * beat).unwrap_or(u64::MAX);
    origin + Duration::from_nanos(offset)
}

#[cfg(test)]
mod tests {
    use image::{Delay, RgbaImage};

    use super::*;

    fn frame_lasting(milliseconds: u32) -> Frame {
        Frame::from_parts(
            RgbaImage::new(2, 2),
            0,
            0,
            Delay::from_numer_denom_ms(milliseconds, 1),
        )
    }

    fn three_frames(play_count: PlayCount) -> DecodedPicture {
        DecodedPicture::new(
            vec![frame_lasting(100), frame_lasting(50), frame_lasting(200)],
            play_count,
        )
    }

    #[test]
    fn an_animation_shows_each_frame_for_its_own_delay_and_loops() {
        let picture = three_frames(None);
        let at = |milliseconds| picture.frame_due_after(Duration::from_millis(milliseconds));
        assert_eq!(at(0), (0, Some(Duration::from_millis(100))));
        assert_eq!(at(99), (0, Some(Duration::from_millis(1))));
        assert_eq!(at(100), (1, Some(Duration::from_millis(50))));
        assert_eq!(at(170), (2, Some(Duration::from_millis(180))));
        assert_eq!(at(350), (0, Some(Duration::from_millis(100))), "loops");
        assert_eq!(at(3_500 + 120), (1, Some(Duration::from_millis(30))));
    }

    #[test]
    fn an_animation_with_a_loop_count_stops_on_its_last_frame() {
        let twice = three_frames(Some(2));
        let at = |milliseconds| twice.frame_due_after(Duration::from_millis(milliseconds));
        assert_eq!(at(350 + 120), (1, Some(Duration::from_millis(30))));
        assert_eq!(at(700), (2, None), "done after two plays");
        assert_eq!(at(9_000), (2, None));
    }

    #[test]
    fn a_still_picture_never_asks_for_another_frame() {
        let still = DecodedPicture::new(vec![frame_lasting(0)], None);
        assert_eq!(still.frame_due_after(Duration::from_secs(9)), (0, None));
        let without_delays = DecodedPicture::new(vec![frame_lasting(0), frame_lasting(0)], None);
        assert_eq!(
            without_delays.frame_due_after(Duration::from_secs(9)),
            (0, None)
        );
    }

    #[test]
    fn an_animation_shown_again_starts_from_its_first_frame() {
        let picture = three_frames(None);
        let start = Instant::now();
        let at = |milliseconds| start + Duration::from_millis(milliseconds);
        assert_eq!(picture.frame_drawn_at(at(0)).0, 0);
        assert_eq!(picture.frame_drawn_at(at(110)).0, 1, "drawn when due");
        assert_eq!(picture.frame_drawn_at(at(160)).0, 2);
        // Not drawn for a while (the pointer left the avatar): back to the first frame.
        assert_eq!(picture.frame_drawn_at(at(5_000)).0, 0);
        assert_eq!(picture.frame_drawn_at(at(5_100)).0, 1);
    }

    #[test]
    fn frame_changes_fall_on_a_shared_beat() {
        let origin = Instant::now();
        let at = |milliseconds| origin + Duration::from_millis(milliseconds);
        assert_eq!(on_frame_change_beat(at(0), origin), at(0));
        assert_eq!(on_frame_change_beat(at(1), origin), at(33));
        assert_eq!(on_frame_change_beat(at(33), origin), at(33));
        assert_eq!(on_frame_change_beat(at(100), origin), at(132));
    }

    #[test]
    fn every_frame_becomes_its_own_single_frame_image() {
        let picture = DecodedPicture::new(vec![frame_lasting(100), frame_lasting(100)], None);
        assert_eq!(picture.frames().len(), 2);
        assert!(
            picture
                .frames()
                .iter()
                .all(|frame| frame.frame_count() == 1)
        );
        assert_eq!(picture.byte_count(), 2 * 2 * 2 * 4);
    }
}
