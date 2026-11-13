//! The video encoding thread: the latest captured picture in, simulcast VP8 frames out to the
//! network thread. Capture hands pictures over through [`VideoInput`], which keeps only the
//! newest one: a slow encoder skips pictures instead of falling behind, and recycles the bytes
//! of pictures it is done with so capture need not allocate one per frame. A source encoded
//! elsewhere (a GPU's H.264 encoder, ADR-0054) goes through [`EncodedVideoInput`] instead, which
//! hands it the server's layer and keyframe requests.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use mellow_protocol::{SimulcastLayer, VideoCodec};

use crate::simulcast::{FRAMES_PER_SECOND, LAYERS, SimulcastEncoder, VideoSourceKind};
use crate::uplink::layers_within;
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
    /// Frames of another codec than the media session's camera line are dropped (a source
    /// replaced while the session is offered again).
    pub(crate) codec: VideoCodec,
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
    /// The source is encoded elsewhere ([`EncodedVideoInput`]): its requests wait for it, not
    /// for this thread.
    is_encoded_elsewhere: bool,
    /// The bytes of a picture already encoded, for capture to fill again.
    spare_bytes: Option<Vec<u8>>,
    /// Bumped by every start or stop, so a previous source's late pictures are refused.
    generation: u64,
    picture: Option<(CapturedFrame, Instant)>,
    active_layers: Option<Vec<SimulcastLayer>>,
    /// The highest layer the uplink carries now (`crate::uplink`).
    layer_ceiling: Option<SimulcastLayer>,
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
        let generation = self.start_generation(kind, false);
        VideoInput {
            control: Arc::clone(self),
            generation,
        }
    }

    /// Turns on a source whose layers are encoded elsewhere, in `codec`, and sent to `outgoing`.
    pub(crate) fn start_encoded(
        self: &Arc<Self>,
        kind: VideoSourceKind,
        codec: VideoCodec,
        outgoing: Sender<OutgoingVideoFrame>,
    ) -> EncodedVideoInput {
        let generation = self.start_generation(kind, true);
        EncodedVideoInput {
            control: Arc::clone(self),
            generation,
            codec,
            outgoing,
            clock_start: Instant::now(),
            asked_layers: LAYERS.to_vec(),
            layer_ceiling: SimulcastLayer::High,
            is_first_take: true,
        }
    }

    fn start_generation(&self, kind: VideoSourceKind, is_encoded_elsewhere: bool) -> u64 {
        let mut generation = 0;
        self.update(|inbox| {
            inbox.generation += 1;
            generation = inbox.generation;
            inbox.source = Some(kind);
            inbox.is_encoded_elsewhere = is_encoded_elsewhere;
            inbox.picture = None;
            inbox.active_layers = None;
            inbox.keyframe_requests.clear();
        });
        generation
    }

    /// Whether the source started as `generation` is still on.
    pub(crate) fn is_generation_on(&self, generation: u64) -> bool {
        let inbox = self.inbox();
        inbox.generation == generation && inbox.source.is_some() && !inbox.is_stopped
    }

    /// The source started as `generation`, if it is still on.
    pub(crate) fn source_of_generation(&self, generation: u64) -> Option<VideoSourceKind> {
        let inbox = self.inbox();
        (inbox.generation == generation)
            .then_some(inbox.source)
            .flatten()
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

    pub(crate) fn set_layer_ceiling(&self, ceiling: SimulcastLayer) {
        self.update(|inbox| inbox.layer_ceiling = Some(ceiling));
    }

    pub(crate) fn request_keyframe(&self, layer: Option<SimulcastLayer>) {
        self.update(|inbox| {
            // Bounded: a request for every layer covers the others, and a layer asked twice
            // before the encoder looked is one keyframe.
            if !inbox.keyframe_requests.contains(&None) && !inbox.keyframe_requests.contains(&layer)
            {
                inbox.keyframe_requests.push(layer);
            }
        });
    }

    /// The source on, if any.
    pub(crate) fn source(&self) -> Option<VideoSourceKind> {
        self.inbox().source
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
        // What the server asks for, trimmed to what the uplink carries.
        let mut asked_layers = LAYERS.to_vec();
        let mut layer_ceiling = SimulcastLayer::High;
        let mut encoded_generation = 0;
        loop {
            let (source, generation, picture, active_layers, new_ceiling, keyframe_requests) = {
                let repeat_due_at = last_picture
                    .as_ref()
                    .and(last_encoded_at)
                    .map(|at| at + LONGEST_INTERVAL_BETWEEN_FRAMES);
                let mut inbox = self.wait_for_news(repeat_due_at);
                inbox.has_news = false;
                if inbox.is_stopped {
                    return;
                }
                // A source encoded elsewhere is none of this thread's: its requests stay in the
                // inbox for its own encoder.
                let is_mine = !inbox.is_encoded_elsewhere;
                (
                    inbox.source.filter(|_| is_mine),
                    inbox.generation,
                    inbox.picture.take(),
                    inbox.active_layers.take_if(|_| is_mine),
                    inbox.layer_ceiling.take_if(|_| is_mine),
                    if is_mine {
                        std::mem::take(&mut inbox.keyframe_requests)
                    } else {
                        Vec::new()
                    },
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
            if generation != encoded_generation {
                // A new source starts with every layer asked for, as a browser's does.
                encoded_generation = generation;
                encoder = None;
                asked_layers = LAYERS.to_vec();
            }
            let is_new_encoder = encoder.is_none();
            let encoder = encoder.get_or_insert_with(|| SimulcastEncoder::new(kind));
            if encoder.kind() != kind
                && let Err(error) = encoder.set_kind(kind)
            {
                tracing::warn!(%error, "the video encoders could not change source");
            }
            let has_layers_changed =
                is_new_encoder || active_layers.is_some() || new_ceiling.is_some();
            // An empty list leaves the layers as they are: the session ends the source instead.
            if let Some(active) = active_layers.filter(|active| !active.is_empty()) {
                asked_layers = active;
            }
            layer_ceiling = new_ceiling.unwrap_or(layer_ceiling);
            if has_layers_changed {
                encoder.set_active_layers(&layers_within(&asked_layers, layer_ceiling));
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
                let converted = picture.to_i420();
                self.recycle(picture.bytes);
                let Some(picture) = converted else {
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
            if !encode_and_send(encoder, picture, rtp_timestamp, outgoing) {
                return;
            }
        }
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn take_waiting_picture_for_tests(&self) -> Option<CapturedFrame> {
        self.inbox().picture.take().map(|(picture, _)| picture)
    }

    /// Keeps `bytes` for capture's next picture, unless some are kept already.
    fn recycle(&self, bytes: Vec<u8>) {
        let mut inbox = self.inbox();
        if inbox.spare_bytes.is_none() {
            inbox.spare_bytes = Some(bytes);
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

/// Encodes `picture` on the active layers and sends the frames; `false` once the network
/// thread is gone (the session is ending). A picture that cannot be encoded is skipped.
fn encode_and_send(
    encoder: &mut SimulcastEncoder,
    picture: &I420Picture,
    rtp_timestamp: u64,
    outgoing: &Sender<OutgoingVideoFrame>,
) -> bool {
    match encoder.encode(picture.view(), rtp_timestamp) {
        Ok(frames) => frames.into_iter().all(|frame| {
            outgoing
                .send(OutgoingVideoFrame {
                    codec: VideoCodec::Vp8,
                    layer: frame.layer,
                    rtp_timestamp,
                    bytes: frame.bytes,
                })
                .is_ok()
        }),
        Err(error) => {
            tracing::warn!(%error, "a video picture could not be encoded");
            true
        }
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
                let replaced = inbox.picture.replace((frame, captured_at));
                if let Some((replaced, _)) = replaced
                    && inbox.spare_bytes.is_none()
                {
                    inbox.spare_bytes = Some(replaced.bytes);
                }
            }
        });
    }

    /// An empty byte buffer for the next picture, with the capacity of one the encoder is done
    /// with when there is one: filling it instead of a new `Vec` spares an allocation per frame.
    #[must_use]
    pub fn recycled_bytes(&self) -> Vec<u8> {
        let mut bytes = self.control.inbox().spare_bytes.take().unwrap_or_default();
        bytes.clear();
        bytes
    }

    /// Whether the session still wants this input's pictures; a capture stops when it is not.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.control.is_generation_on(self.generation)
    }

    /// The source start this input belongs to.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
}

/// What the server asked of a source encoded elsewhere since the last look.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EncoderRequests {
    /// The layers to encode now, when they changed (the server's watched layers, trimmed to
    /// what the uplink carries); the others pause.
    pub active_layers: Option<Vec<SimulcastLayer>>,
    /// Keyframes asked for: one layer, or every layer (`None`).
    pub keyframes: Vec<Option<SimulcastLayer>>,
}

/// Where a source encoded outside this crate's VP8 thread (a GPU's H.264 encoder, ADR-0054)
/// sends its layers. Frames pushed after the source was stopped or replaced are dropped.
#[derive(Debug)]
pub struct EncodedVideoInput {
    control: Arc<VideoEncoderControl>,
    generation: u64,
    codec: VideoCodec,
    outgoing: Sender<OutgoingVideoFrame>,
    clock_start: Instant,
    asked_layers: Vec<SimulcastLayer>,
    layer_ceiling: SimulcastLayer,
    is_first_take: bool,
}

impl EncodedVideoInput {
    /// Whether the session still wants this source; its encoder stops when it is not.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.control.is_generation_on(self.generation)
    }

    /// The codec the frames must be in.
    #[must_use]
    pub fn codec(&self) -> VideoCodec {
        self.codec
    }

    /// The source start this input belongs to.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// The source's kind now (a screen's quality may change while it runs), `None` once off.
    #[must_use]
    pub fn kind(&self) -> Option<VideoSourceKind> {
        self.control.source_of_generation(self.generation)
    }

    /// The server's layer and keyframe requests since the last call. The first call gives the
    /// layers to start with (all of them, as a browser starts).
    pub fn take_requests(&mut self) -> EncoderRequests {
        let (asked, ceiling, keyframes) = {
            let mut inbox = self.control.inbox();
            if inbox.generation != self.generation {
                return EncoderRequests::default();
            }
            (
                inbox.active_layers.take(),
                inbox.layer_ceiling.take(),
                std::mem::take(&mut inbox.keyframe_requests),
            )
        };
        let has_changed = asked.is_some() || ceiling.is_some() || self.is_first_take;
        self.is_first_take = false;
        if let Some(asked) = asked.filter(|asked| !asked.is_empty()) {
            self.asked_layers = asked;
        }
        self.layer_ceiling = ceiling.unwrap_or(self.layer_ceiling);
        EncoderRequests {
            active_layers: has_changed
                .then(|| layers_within(&self.asked_layers, self.layer_ceiling)),
            keyframes,
        }
    }

    /// `at` on the source's 90 kHz RTP clock.
    #[must_use]
    pub fn rtp_time_of(&self, at: Instant) -> u64 {
        rtp_time_since(self.clock_start, at)
    }

    /// Sends one layer's encoded frame (an access unit in Annex B form for H.264), stamped
    /// `rtp_timestamp` (all layers of one picture share it).
    pub fn push(&self, layer: SimulcastLayer, rtp_timestamp: u64, bytes: Vec<u8>) {
        if !self.is_open() {
            return;
        }
        // The network thread gone means the session is ending.
        let _ = self.outgoing.send(OutgoingVideoFrame {
            codec: self.codec,
            layer,
            rtp_timestamp,
            bytes,
        });
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

    #[test]
    fn an_encoded_source_gets_the_server_s_requests_and_the_vp8_thread_leaves_them() {
        let control = Arc::new(VideoEncoderControl::default());
        let (outgoing, frames) = mpsc::channel();
        let thread = {
            let control = Arc::clone(&control);
            let outgoing = outgoing.clone();
            std::thread::spawn(move || control.run(&outgoing))
        };
        let mut input = control.start_encoded(
            VideoSourceKind::Screen(crate::simulcast::ScreenShareQuality::High),
            VideoCodec::H264,
            outgoing,
        );
        assert_eq!(input.take_requests().active_layers, Some(LAYERS.to_vec()));
        control.set_active_layers(vec![SimulcastLayer::Quarter, SimulcastLayer::High]);
        control.set_layer_ceiling(SimulcastLayer::Low);
        control.request_keyframe(Some(SimulcastLayer::Quarter));
        control.request_keyframe(Some(SimulcastLayer::Quarter));
        // The VP8 thread wakes for each change and leaves them alone, whenever it runs.
        let requests = input.take_requests();
        assert_eq!(requests.active_layers, Some(vec![SimulcastLayer::Quarter]));
        assert_eq!(requests.keyframes, [Some(SimulcastLayer::Quarter)]);
        assert_eq!(input.take_requests(), EncoderRequests::default());

        input.push(SimulcastLayer::Quarter, 90, vec![1, 2, 3]);
        let frame = frames.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(
            (frame.codec, frame.layer),
            (VideoCodec::H264, SimulcastLayer::Quarter)
        );
        let camera = control.start(VideoSourceKind::Camera);
        assert!(!input.is_open() && camera.is_open());
        assert_eq!(input.kind(), None);
        input.push(SimulcastLayer::Quarter, 180, vec![4]);
        control.shut_down();
        thread.join().unwrap();
        assert!(
            frames
                .try_iter()
                .all(|frame| frame.codec == VideoCodec::Vp8)
        );
    }

    #[test]
    fn the_bytes_of_a_replaced_picture_come_back_to_capture() {
        let control = Arc::new(VideoEncoderControl::default());
        let input = control.start(VideoSourceKind::Camera);
        assert_eq!(input.recycled_bytes().capacity(), 0);
        input.push(grey_frame(64, 36));
        input.push(grey_frame(64, 36));
        let bytes = input.recycled_bytes();
        assert!(bytes.is_empty() && bytes.capacity() >= 64 * 36);
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
