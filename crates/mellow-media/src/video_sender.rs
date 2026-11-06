//! The video encoding thread: the latest captured picture in, simulcast VP8 frames out to the
//! network thread. Capture hands pictures over through [`VideoInput`], which keeps only the
//! newest one: a slow encoder skips pictures instead of falling behind.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use mellow_protocol::SimulcastLayer;

use crate::simulcast::{FRAMES_PER_SECOND, SimulcastEncoder, VideoSourceKind};
use crate::video_frame::{CapturedFrame, I420Picture};
use crate::vp8::VIDEO_CLOCK_RATE;

/// Pictures closer together than this are skipped: 30 frames per second, with a little slack
/// for capture jitter.
const SHORTEST_INTERVAL_BETWEEN_FRAMES: Duration =
    Duration::from_micros(1_000_000 / FRAMES_PER_SECOND as u64 * 9 / 10);
/// Without a new picture, the last one is encoded again this often, as Chrome repeats a still
/// screen: a late watcher gets its picture and the server never sees the source as silent.
const LONGEST_INTERVAL_BETWEEN_FRAMES: Duration = Duration::from_secs(1);

/// A layer's encoded frame, stamped for RTP.
#[derive(Debug)]
pub(crate) struct OutgoingVideoFrame {
    pub(crate) layer: SimulcastLayer,
    pub(crate) rtp_timestamp: u64,
    pub(crate) bytes: Vec<u8>,
}

/// What the encoding thread has to do next, filled by the session, the network thread and
/// capture.
#[derive(Debug, Default)]
struct Inbox {
    /// Which source is on; `None` while video is off.
    source: Option<VideoSourceKind>,
    /// Bumped by every start or stop, so a previous source's late pictures are refused.
    generation: u64,
    picture: Option<(CapturedFrame, Instant)>,
    active_layers: Option<Vec<SimulcastLayer>>,
    keyframe_requests: Vec<Option<SimulcastLayer>>,
    is_stopped: bool,
    /// Something above changed since the thread last looked.
    has_news: bool,
}

/// Shared between the encoding thread and everything that feeds it.
#[derive(Debug, Default)]
pub(crate) struct VideoEncoderControl {
    inbox: Mutex<Inbox>,
    has_work: Condvar,
}

impl VideoEncoderControl {
    fn inbox(&self) -> MutexGuard<'_, Inbox> {
        // A panic while holding this lock leaves plain data behind: keep going with it.
        self.inbox
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn update(&self, change: impl FnOnce(&mut Inbox)) {
        let mut inbox = self.inbox();
        change(&mut inbox);
        inbox.has_news = true;
        drop(inbox);
        self.has_work.notify_one();
    }

    /// Turns a source on (replacing any other) and returns where capture puts its pictures.
    pub(crate) fn start(self: &Arc<Self>, kind: VideoSourceKind) -> VideoInput {
        let mut generation = 0;
        self.update(|inbox| {
            inbox.generation += 1;
            generation = inbox.generation;
            inbox.source = Some(kind);
            inbox.picture = None;
        });
        VideoInput {
            control: Arc::clone(self),
            generation,
        }
    }

    /// Changes the source's kind (a new screen quality) without a new input.
    pub(crate) fn change_kind(&self, kind: VideoSourceKind) {
        self.update(|inbox| {
            if inbox.source.is_some() {
                inbox.source = Some(kind);
            }
        });
    }

    pub(crate) fn stop_source(&self) {
        self.update(|inbox| {
            inbox.generation += 1;
            inbox.source = None;
            inbox.picture = None;
        });
    }

    pub(crate) fn set_active_layers(&self, active: Vec<SimulcastLayer>) {
        self.update(|inbox| inbox.active_layers = Some(active));
    }

    pub(crate) fn request_keyframe(&self, layer: Option<SimulcastLayer>) {
        self.update(|inbox| inbox.keyframe_requests.push(layer));
    }

    /// Ends the encoding thread.
    pub(crate) fn shut_down(&self) {
        self.update(|inbox| inbox.is_stopped = true);
    }

    /// Runs on the encoding thread until [`VideoEncoderControl::shut_down`].
    pub(crate) fn run(&self, outgoing: &Sender<OutgoingVideoFrame>) {
        let clock_start = Instant::now();
        let mut encoder: Option<SimulcastEncoder> = None;
        let mut last_encoded_at: Option<Instant> = None;
        // Kept to encode again while no new picture comes: screen capture delivers one only
        // when the screen changes, and a still screen must still answer keyframe requests.
        let mut last_picture: Option<(u64, I420Picture)> = None;
        loop {
            let (source, generation, picture, active_layers, keyframe_requests) = {
                let repeat_due_at = last_picture
                    .as_ref()
                    .and(last_encoded_at)
                    .map(|at| at + LONGEST_INTERVAL_BETWEEN_FRAMES);
                let mut inbox = self.wait_for_news(repeat_due_at);
                inbox.has_news = false;
                if inbox.is_stopped {
                    return;
                }
                (
                    inbox.source,
                    inbox.generation,
                    inbox.picture.take(),
                    inbox.active_layers.take(),
                    std::mem::take(&mut inbox.keyframe_requests),
                )
            };
            if last_picture
                .as_ref()
                .is_some_and(|(kept_generation, _)| *kept_generation != generation)
            {
                last_picture = None;
            }
            let Some(kind) = source else {
                encoder = None;
                continue;
            };
            let encoder = encoder.get_or_insert_with(|| SimulcastEncoder::new(kind));
            if encoder.kind() != kind
                && let Err(error) = encoder.set_kind(kind)
            {
                tracing::warn!(%error, "the video encoders could not change source");
            }
            if let Some(active) = active_layers {
                encoder.set_active_layers(&active);
            }
            let is_keyframe_requested = !keyframe_requests.is_empty();
            for layer in keyframe_requests {
                encoder.request_keyframe(layer);
            }
            let now = Instant::now();
            let encoded_at = if let Some((picture, captured_at)) = picture {
                if last_encoded_at.is_some_and(|last| {
                    captured_at.saturating_duration_since(last) < SHORTEST_INTERVAL_BETWEEN_FRAMES
                }) {
                    continue;
                }
                let Some(picture) = picture.to_i420() else {
                    tracing::warn!("a captured picture does not match its size; dropped");
                    continue;
                };
                last_picture = Some((generation, picture));
                captured_at
            } else {
                let is_repeat_due = last_encoded_at.is_some_and(|last| {
                    now.saturating_duration_since(last) >= LONGEST_INTERVAL_BETWEEN_FRAMES
                });
                if last_picture.is_none() || !(is_keyframe_requested || is_repeat_due) {
                    continue;
                }
                now
            };
            let Some((_, picture)) = &last_picture else {
                continue;
            };
            last_encoded_at = Some(encoded_at);
            let rtp_timestamp = rtp_time_since(clock_start, encoded_at);
            match encoder.encode(picture.view(), rtp_timestamp) {
                Ok(frames) => {
                    let mut frames = frames.into_iter().map(|frame| OutgoingVideoFrame {
                        layer: frame.layer,
                        rtp_timestamp,
                        bytes: frame.bytes,
                    });
                    // The network thread gone means the session is ending.
                    if frames.any(|frame| outgoing.send(frame).is_err()) {
                        return;
                    }
                }
                Err(error) => tracing::warn!(%error, "a video picture could not be encoded"),
            }
        }
    }

    /// The inbox once something changed, or once `repeat_due_at` passed.
    fn wait_for_news(&self, repeat_due_at: Option<Instant>) -> MutexGuard<'_, Inbox> {
        let mut inbox = self.inbox();
        while !inbox.has_news {
            let Some(due_at) = repeat_due_at else {
                inbox = self
                    .has_work
                    .wait(inbox)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                continue;
            };
            let wait = due_at.saturating_duration_since(Instant::now());
            if wait.is_zero() {
                break;
            }
            inbox = self
                .has_work
                .wait_timeout(inbox, wait)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        inbox
    }
}

/// `at` on the 90 kHz RTP clock started at `start`.
fn rtp_time_since(start: Instant, at: Instant) -> u64 {
    let elapsed = at.saturating_duration_since(start).as_micros();
    u64::try_from(elapsed * u128::from(VIDEO_CLOCK_RATE) / 1_000_000).unwrap_or(u64::MAX)
}

/// Where a camera or screen capture puts its pictures. Pictures pushed after the source was
/// stopped or replaced are dropped, so an old capture still running cannot leak into the call.
#[derive(Debug, Clone)]
pub struct VideoInput {
    control: Arc<VideoEncoderControl>,
    generation: u64,
}

impl VideoInput {
    /// Hands over a picture; an older one still waiting is replaced. Never blocks for long.
    pub fn push(&self, frame: CapturedFrame) {
        let captured_at = Instant::now();
        let generation = self.generation;
        self.control.update(|inbox| {
            if inbox.generation == generation && inbox.source.is_some() {
                inbox.picture = Some((frame, captured_at));
            }
        });
    }

    /// Whether the session still wants this input's pictures; a capture stops when it is not.
    #[must_use]
    pub fn is_open(&self) -> bool {
        let inbox = self.control.inbox();
        inbox.generation == self.generation && inbox.source.is_some() && !inbox.is_stopped
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::video_frame::CapturedPixelFormat;

    fn grey_frame(width: u32, height: u32) -> CapturedFrame {
        CapturedFrame {
            width,
            height,
            format: CapturedPixelFormat::I420,
            bytes: vec![
                128;
                CapturedFrame::expected_length(width, height, CapturedPixelFormat::I420)
            ],
        }
    }

    #[test]
    fn rtp_time_runs_at_ninety_kilohertz() {
        let start = Instant::now();
        assert_eq!(
            rtp_time_since(start, start + Duration::from_secs(2)),
            180_000
        );
        assert_eq!(rtp_time_since(start + Duration::from_secs(1), start), 0);
    }

    #[test]
    fn a_replaced_source_s_input_is_closed_and_its_pictures_dropped() {
        let control = Arc::new(VideoEncoderControl::default());
        let camera = control.start(VideoSourceKind::Camera);
        assert!(camera.is_open());
        let screen = control.start(VideoSourceKind::Screen(
            crate::simulcast::ScreenShareQuality::High,
        ));
        assert!(!camera.is_open() && screen.is_open());
        camera.push(grey_frame(64, 36));
        assert!(control.inbox().picture.is_none());
        screen.push(grey_frame(64, 36));
        assert!(control.inbox().picture.is_some());
        control.stop_source();
        assert!(!screen.is_open());
    }

    #[test]
    fn the_thread_encodes_pushed_pictures_and_stops_when_asked() {
        let control = Arc::new(VideoEncoderControl::default());
        let (outgoing, frames) = mpsc::channel();
        let thread = {
            let control = Arc::clone(&control);
            std::thread::spawn(move || control.run(&outgoing))
        };
        let input = control.start(VideoSourceKind::Camera);
        input.push(grey_frame(640, 360));
        let mut layers = Vec::new();
        while layers.len() < 3 {
            let frame = frames.recv_timeout(Duration::from_secs(10)).unwrap();
            layers.push(frame.layer);
        }
        layers.sort();
        assert_eq!(
            layers,
            [
                SimulcastLayer::Quarter,
                SimulcastLayer::Low,
                SimulcastLayer::High
            ]
        );
        control.shut_down();
        thread.join().unwrap();
    }

    /// A VP8 frame's first bit is 0 on a keyframe (RFC 6386, section 9.1).
    fn is_keyframe(frame: &OutgoingVideoFrame) -> bool {
        frame.bytes.first().is_some_and(|byte| byte & 1 == 0)
    }

    /// Screen capture delivers a picture only when the screen changes: a still screen must still
    /// answer a late watcher's keyframe request and keep sending, or the server turns it off.
    #[test]
    fn a_still_source_answers_keyframe_requests_and_repeats_its_last_picture() {
        let control = Arc::new(VideoEncoderControl::default());
        let (outgoing, frames) = mpsc::channel();
        let thread = {
            let control = Arc::clone(&control);
            std::thread::spawn(move || control.run(&outgoing))
        };
        let input = control.start(VideoSourceKind::Camera);
        input.push(grey_frame(640, 360));
        for _ in 0..3 {
            frames.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        // Repeats of a still picture are delta frames; a requested one is a keyframe.
        let repeated = frames.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(!is_keyframe(&repeated), "a repeat without request");
        while frames.try_recv().is_ok() {}
        control.request_keyframe(Some(SimulcastLayer::High));
        let requested = frames
            .iter()
            .find(|frame| frame.layer == SimulcastLayer::High && is_keyframe(frame));
        assert!(requested.is_some());
        control.shut_down();
        thread.join().unwrap();
    }
}
