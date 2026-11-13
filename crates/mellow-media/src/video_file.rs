//! Video attachments and Klipy's MP4 GIFs played in a conversation (ADR-0038, ADR-0042), decoded
//! by the system: `AVFoundation` on macOS (MP4 and `QuickTime`: H.264, HEVC, `ProRes`), the system
//! FFmpeg on Linux (MP4, `QuickTime` and `WebM`: whatever decoders that FFmpeg was built with,
//! found out when the file is opened). Windows decodes no video yet: its readers get the file
//! instead. Files are untrusted: the type and first bytes are checked before a decoder sees
//! them, pictures larger than [`MAXIMUM_SOURCE_PIXELS`] are refused before they are allocated,
//! and decoding runs on its own thread with a panic guard.
//!
//! The sound of a video plays through [`crate::ClipPlayer`] like any audio attachment (one at a
//! time, the remembered volume, the voice output), and its pictures follow the player's
//! position ([`VideoTiming::Follow`]); a video without sound plays a silent track of its length
//! so the same clock drives it. GIFs loop silently on their own clock ([`VideoTiming::Loop`]).
//! At most two decoded pictures wait on the thread (the next one and the one to show), and the
//! interface takes only the newest.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::clip_decoder::{ClipDecoder, ClipError};
use crate::video_frame::{DecodedPixelFormat, DecodedVideoFrame};

/// The video types the instance serves inline (ADR-0038's `PLAYABLE_MEDIA_TYPES`).
pub const PLAYABLE_ATTACHMENT_VIDEO_TYPES: [&str; 3] =
    ["video/mp4", "video/quicktime", "video/webm"];
/// Pictures are shrunk to fit this before they reach the interface: twice the largest preview
/// box (400 × 300 points on a Retina screen), and a sharp enough enlarged view.
pub const MAXIMUM_SHOWN_VIDEO_SIZE: (u32, u32) = (1_280, 720);
/// A larger picture is refused before it is allocated: 4K phone recordings fit, a hostile header
/// declaring 65 535 × 65 535 does not.
// Windows decodes no video file yet.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
pub(crate) const MAXIMUM_SOURCE_PIXELS: u64 = 4_096 * 2_304;
/// How often a waiting thread looks at its clock again: a seek while paused shows within this.
const POLL_PERIOD: Duration = Duration::from_millis(20);
/// The sound's position wobbles by a few milliseconds as its output queue fills and drains: a
/// move back smaller than this is not a seek.
const BACKWARD_TOLERANCE: Duration = Duration::from_millis(50);
/// A picture this close to its time is shown now rather than after one more wait.
const EARLY_TOLERANCE: Duration = Duration::from_millis(5);
/// Behind by more than this, the decoder jumps (to the keyframe before the position) instead of
/// decoding every picture in between.
const FORWARD_JUMP: Duration = Duration::from_secs(1);
/// A loop whose container states no length restarts this long after its last picture.
const LOOP_GAP: Duration = Duration::from_millis(40);
/// Silent tracks: 48 kHz mono, handed out in chunks of this many frames.
const SILENT_SAMPLE_RATE: u32 = 48_000;
const SILENT_CHUNK_FRAMES: u64 = 1_024;

/// A container the instance sniffed (ADR-0038).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoContainer {
    /// ISO-BMFF: MP4 and `QuickTime`.
    Mp4,
    WebM,
}

/// The container of a file served as `content_type`, when its first bytes agree: a file whose
/// type was set by something else is never handed to a decoder.
#[must_use]
pub fn video_container(bytes: &[u8], content_type: &str) -> Option<VideoContainer> {
    const EBML_MAGIC: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];
    match content_type {
        "video/mp4" | "video/quicktime" if bytes.get(4..8) == Some(b"ftyp") => {
            Some(VideoContainer::Mp4)
        }
        "video/webm" if bytes.starts_with(&EBML_MAGIC) => Some(VideoContainer::WebM),
        _ => None,
    }
}

/// Whether this system plays videos of `content_type` (before anything is downloaded: the
/// codec inside is only known once the file is opened). macOS's `AVFoundation` reads no
/// `WebM`; Windows has no video decoding yet.
#[must_use]
pub fn plays_video_type(content_type: &str) -> bool {
    let is_mp4 = matches!(content_type, "video/mp4" | "video/quicktime");
    let is_webm = content_type == "video/webm";
    (is_mp4 && cfg!(any(target_os = "linux", target_os = "macos")))
        || (is_webm && cfg!(target_os = "linux"))
}

/// A decoded picture and when it shows, from the start of the video.
// Windows decodes no video file yet.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
pub(crate) struct TimedPicture {
    pub(crate) time: Duration,
    pub(crate) frame: DecodedVideoFrame,
}

/// A video track being decoded, in display order.
pub(crate) trait VideoTrackDecoder: Send {
    /// The whole video's length, when the container says.
    fn duration(&self) -> Option<Duration>;
    /// The next picture, BGRA, upright and within [`MAXIMUM_SHOWN_VIDEO_SIZE`]; `None` at the
    /// end.
    fn next_picture(&mut self) -> Result<Option<TimedPicture>, ClipError>;
    /// The next pictures start at the keyframe at or before `position`.
    fn seek(&mut self, position: Duration) -> Result<(), ClipError>;
}

/// A video file's sound: its own track, or none (the length is then played as silence).
pub(crate) enum SoundTrack {
    Sound(Box<dyn ClipDecoder>),
    Silent { duration: Option<Duration> },
}

pub(crate) fn open_video_track(
    bytes: Arc<[u8]>,
    container: VideoContainer,
) -> Result<Box<dyn VideoTrackDecoder>, ClipError> {
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(crate::video_file_ffmpeg::FfmpegVideoTrack::open(
            bytes, container,
        )?))
    }
    #[cfg(target_os = "macos")]
    {
        if container != VideoContainer::Mp4 {
            return Err(ClipError::Unsupported);
        }
        Ok(Box::new(crate::video_file_macos::AssetVideoTrack::open(
            bytes,
        )?))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (bytes, container);
        Err(ClipError::Unsupported)
    }
}

/// The sound of a video file for the clip player: its track, or silence of its length.
pub(crate) fn open_video_sound(
    bytes: Arc<[u8]>,
    content_type: &str,
) -> Result<Box<dyn ClipDecoder>, ClipError> {
    let container = video_container(&bytes, content_type).ok_or(ClipError::Unsupported)?;
    if !plays_video_type(content_type) {
        return Err(ClipError::Unsupported);
    }
    #[cfg(target_os = "linux")]
    let track = { crate::video_file_ffmpeg::open_sound_track(bytes, container)? };
    #[cfg(not(target_os = "linux"))]
    let track = {
        let _ = container;
        crate::clip_decoder::open_mp4_sound_track(bytes)?
    };
    match track {
        SoundTrack::Sound(decoder) => Ok(decoder),
        SoundTrack::Silent { duration } => Ok(Box::new(SilentClip::new(
            duration.ok_or(ClipError::Unsupported)?,
        ))),
    }
}

/// Silence the length of a video without sound, so the clip player's position drives its
/// pictures as it does any other video's.
struct SilentClip {
    duration_frames: u64,
    position: u64,
}

impl SilentClip {
    fn new(duration: Duration) -> Self {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // A video's length in seconds times 48 000 stays far below 2^53, and is never negative.
        let duration_frames = (duration.as_secs_f64() * f64::from(SILENT_SAMPLE_RATE)) as u64;
        Self {
            duration_frames,
            position: 0,
        }
    }
}

impl ClipDecoder for SilentClip {
    fn sample_rate(&self) -> u32 {
        SILENT_SAMPLE_RATE
    }

    fn channels(&self) -> usize {
        1
    }

    fn duration_frames(&self) -> Option<u64> {
        Some(self.duration_frames)
    }

    fn decode_next(&mut self, samples: &mut Vec<f32>) -> Result<bool, ClipError> {
        let frames = SILENT_CHUNK_FRAMES.min(self.duration_frames.saturating_sub(self.position));
        samples.resize(
            samples.len() + usize::try_from(frames).unwrap_or(usize::MAX),
            0.0,
        );
        self.position += frames;
        Ok(self.position < self.duration_frames)
    }

    fn seek(&mut self, frame: u64) -> Result<(), ClipError> {
        self.position = frame.min(self.duration_frames);
        Ok(())
    }
}

/// Where the pictures should be: read by the decoding thread between pictures.
pub trait VideoClock: Send {
    fn position(&self) -> Duration;
}

/// How a video's pictures are timed.
pub enum VideoTiming {
    /// Follows a clock: an attachment's sound in the clip player.
    Follow(Box<dyn VideoClock>),
    /// Loops silently on its own (an MP4 GIF); paused, it holds its picture (the first one
    /// before it ever played).
    Loop { is_playing: bool },
}

impl std::fmt::Debug for VideoTiming {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Follow(_) => formatter.write_str("Follow"),
            Self::Loop { is_playing } => formatter
                .debug_struct("Loop")
                .field("is_playing", is_playing)
                .finish(),
        }
    }
}

/// Where a video's decoding stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoStreamPhase {
    /// The file is being read.
    Opening,
    Showing,
    /// Not played here, or broke part way.
    Failed(ClipError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoStreamStatus {
    pub phase: VideoStreamPhase,
    pub duration: Option<Duration>,
}

struct StreamState {
    newest: Option<DecodedVideoFrame>,
    status: VideoStreamStatus,
    is_stopping: bool,
    is_loop_playing: bool,
}

struct Shared {
    state: Mutex<StreamState>,
    changed: Condvar,
    waker: Mutex<Option<Box<dyn Fn() + Send>>>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, StreamState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wake_interface(&self) {
        if let Some(waker) = self
            .waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            waker();
        }
    }

    fn publish(&self, frame: DecodedVideoFrame) {
        self.lock().newest = Some(frame);
        self.wake_interface();
    }

    fn set_phase(&self, phase: VideoStreamPhase, duration: Option<Duration>) {
        {
            let mut state = self.lock();
            state.status = VideoStreamStatus { phase, duration };
        }
        self.wake_interface();
    }

    /// Waits up to `timeout` or until the interface changes something; whether to stop. A
    /// paused loop's clock stands still, so it waits for the interface alone: tens of paused
    /// GIFs (reduced motion) would otherwise wake the CPU every poll period.
    fn wait(&self, timeout: Duration, is_loop: bool) -> bool {
        let state = self.lock();
        if state.is_stopping {
            return true;
        }
        let state = if is_loop && !state.is_loop_playing {
            self.changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner)
        } else {
            self.changed
                .wait_timeout(state, timeout)
                .unwrap_or_else(PoisonError::into_inner)
                .0
        };
        state.is_stopping
    }
}

/// One video's pictures, decoded on their own thread from a whole file in memory. Dropping it
/// stops the thread after its current step, without waiting for it: it is dropped on the
/// interface thread, and one step can be a whole file opened or a 4K picture decoded.
pub struct VideoStream {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for VideoStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VideoStream")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl VideoStream {
    /// Starts decoding `bytes` (a whole file of `content_type`); the file is checked and opened
    /// on the new thread, never the caller's.
    ///
    /// # Errors
    /// The thread could not be started.
    pub fn start(
        bytes: Arc<[u8]>,
        content_type: &str,
        timing: VideoTiming,
    ) -> std::io::Result<Self> {
        let is_loop_playing = matches!(timing, VideoTiming::Loop { is_playing: true });
        let shared = Arc::new(Shared {
            state: Mutex::new(StreamState {
                newest: None,
                status: VideoStreamStatus {
                    phase: VideoStreamPhase::Opening,
                    duration: None,
                },
                is_stopping: false,
                is_loop_playing,
            }),
            changed: Condvar::new(),
            waker: Mutex::new(None),
        });
        let thread_shared = Arc::clone(&shared);
        let container = video_container(&bytes, content_type);
        let is_played = plays_video_type(content_type);
        // Detached: the stream never joins it (see `VideoStream`), so it reports its own panic.
        std::thread::Builder::new()
            .name("mellow-video-file".into())
            .spawn(move || {
                let decoding = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let opened = container
                        .filter(|_| is_played)
                        .ok_or(ClipError::Unsupported)
                        .and_then(|container| {
                            without_panics(|| open_video_track(bytes, container))
                        });
                    match opened {
                        Ok(decoder) => run(decoder, timing, &thread_shared),
                        Err(error) => {
                            thread_shared.set_phase(VideoStreamPhase::Failed(error), None);
                        }
                    }
                }));
                if decoding.is_err() {
                    tracing::error!("a video's decoding thread panicked");
                }
            })?;
        Ok(Self { shared })
    }

    /// Called from the decoding thread after each new picture or phase.
    pub fn set_waker(&self, waker: impl Fn() + Send + 'static) {
        *self
            .shared
            .waker
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Box::new(waker));
    }

    /// The newest picture since the last call.
    #[must_use]
    pub fn take_new_picture(&self) -> Option<DecodedVideoFrame> {
        self.shared.lock().newest.take()
    }

    #[must_use]
    pub fn status(&self) -> VideoStreamStatus {
        self.shared.lock().status
    }

    /// Plays or pauses a looping video; a followed one ignores it.
    pub fn set_loop_playing(&self, is_playing: bool) {
        self.shared.lock().is_loop_playing = is_playing;
        self.shared.changed.notify_all();
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        self.shared.lock().is_stopping = true;
        self.shared.changed.notify_all();
    }
}

/// A looping video's own clock: wall time since it started playing, modulo its length.
struct LoopClock {
    started: Option<Instant>,
    paused_at: Duration,
    length: Option<Duration>,
}

impl LoopClock {
    fn position(&mut self, is_playing: bool) -> Duration {
        match (is_playing, self.started) {
            (true, None) => self.started = Instant::now().checked_sub(self.paused_at),
            (false, Some(started)) => {
                self.paused_at = started.elapsed();
                self.started = None;
            }
            _ => {}
        }
        let elapsed = self
            .started
            .map_or(self.paused_at, |started| started.elapsed());
        looped(elapsed, self.length)
    }
}

/// `elapsed` within a loop of `length` (as it is without one).
fn looped(elapsed: Duration, length: Option<Duration>) -> Duration {
    match length.filter(|length| !length.is_zero()) {
        Some(length) => {
            let nanoseconds = elapsed.as_nanos() % length.as_nanos();
            Duration::from_nanos(u64::try_from(nanoseconds).unwrap_or(u64::MAX))
        }
        None => elapsed,
    }
}

/// What the decoding thread does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Decode the next picture.
    Decode,
    /// The pending picture's time has come: it becomes the one to show.
    Take,
    /// Show what was taken, then wait this long.
    Wait(Duration),
    /// The clock moved away: start again from the keyframe before this position.
    Seek(Duration),
}

/// The decoding thread's timing rules, free of the decoder and the clock.
#[derive(Debug, Default)]
struct Schedule {
    /// The time of the last picture taken.
    shown: Option<Duration>,
    /// The position of the last seek, until the pictures caught up with it.
    sought: Option<Duration>,
    /// When a picture last went to the interface.
    published: Option<Instant>,
}

impl Schedule {
    fn step(&self, position: Duration, pending: Option<Duration>, is_at_end: bool) -> Step {
        let latest = self.shown.or(self.sought);
        if latest.is_some_and(|latest| position + BACKWARD_TOLERANCE < latest) {
            return Step::Seek(position);
        }
        match pending {
            None if is_at_end => Step::Wait(POLL_PERIOD),
            None => Step::Decode,
            // The first picture since the start or a seek shows at once, even early: a paused
            // video shows where it stands.
            Some(_) if self.shown.is_none() => Step::Take,
            Some(time) if time > position + EARLY_TOLERANCE => {
                Step::Wait(time.saturating_sub(position).min(POLL_PERIOD))
            }
            // Far behind with the pictures caught up once: jump rather than decode each one.
            // Right after a seek the decoder starts at the keyframe before, maybe seconds back.
            Some(time) if self.sought.is_none() && time + FORWARD_JUMP < position => {
                Step::Seek(position)
            }
            Some(_) => Step::Take,
        }
    }

    fn took(&mut self, time: Duration) {
        self.shown = Some(time);
        if self.sought.is_some_and(|sought| time >= sought) {
            self.sought = None;
        }
    }

    /// Whether a picture just taken goes to the interface now rather than at the next wait: a
    /// decoder slower than the clock takes picture after picture and never waits, and would
    /// otherwise leave one picture on screen under the sound.
    fn is_publish_due(&self, now: Instant) -> bool {
        self.published
            .is_none_or(|published| now.saturating_duration_since(published) >= POLL_PERIOD)
    }

    fn published(&mut self, now: Instant) {
        self.published = Some(now);
    }

    fn waited(&mut self) {
        self.sought = None;
    }

    fn sought(&mut self, position: Duration) {
        self.shown = None;
        self.sought = Some(position);
    }
}

enum Clock {
    Follow(Box<dyn VideoClock>),
    Loop(LoopClock),
}

impl Clock {
    fn position(&mut self, shared: &Shared) -> Duration {
        match self {
            Self::Follow(clock) => clock.position(),
            Self::Loop(clock) => {
                let is_playing = shared.lock().is_loop_playing;
                clock.position(is_playing)
            }
        }
    }
}

fn run(mut decoder: Box<dyn VideoTrackDecoder>, timing: VideoTiming, shared: &Shared) {
    let duration = decoder.duration();
    shared.set_phase(VideoStreamPhase::Showing, duration);
    let mut clock = match timing {
        VideoTiming::Follow(clock) => Clock::Follow(clock),
        VideoTiming::Loop { .. } => Clock::Loop(LoopClock {
            started: None,
            paused_at: Duration::ZERO,
            length: duration,
        }),
    };
    let mut schedule = Schedule::default();
    let mut pending: Option<TimedPicture> = None;
    let mut taken: Option<DecodedVideoFrame> = None;
    let mut is_at_end = false;
    loop {
        let position = clock.position(shared);
        let step = schedule.step(
            position,
            pending.as_ref().map(|picture| picture.time),
            is_at_end,
        );
        let failure = match step {
            Step::Decode => match without_panics(|| decoder.next_picture()) {
                Ok(Some(picture)) => {
                    pending = Some(picture);
                    None
                }
                Ok(None) => {
                    is_at_end = true;
                    if let Clock::Loop(clock) = &mut clock
                        && clock.length.is_none()
                    {
                        clock.length = Some(schedule.shown.unwrap_or_default() + LOOP_GAP);
                    }
                    None
                }
                Err(error) => Some(error),
            },
            Step::Take => {
                if let Some(picture) = pending.take() {
                    schedule.took(picture.time);
                    let now = Instant::now();
                    if schedule.is_publish_due(now) {
                        schedule.published(now);
                        shared.publish(picture.frame);
                        taken = None;
                    } else {
                        taken = Some(picture.frame);
                    }
                }
                None
            }
            Step::Wait(timeout) => {
                schedule.waited();
                if let Some(frame) = taken.take() {
                    schedule.published(Instant::now());
                    shared.publish(frame);
                }
                if shared.wait(timeout, matches!(clock, Clock::Loop(_))) {
                    return;
                }
                None
            }
            Step::Seek(target) => {
                pending = None;
                is_at_end = false;
                schedule.sought(target);
                without_panics(|| decoder.seek(target)).err()
            }
        };
        if let Some(error) = failure {
            if let Some(frame) = taken.take() {
                shared.publish(frame);
            }
            shared.set_phase(VideoStreamPhase::Failed(error), duration);
            return;
        }
        if shared.lock().is_stopping {
            return;
        }
    }
}

/// Runs decoder work so that a panic in a parser fed a hostile file reports the file as damaged
/// instead of ending the thread silently.
fn without_panics<T>(work: impl FnOnce() -> Result<T, ClipError>) -> Result<T, ClipError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|_| {
        tracing::error!("a video's decoder panicked");
        Err(ClipError::Corrupt)
    })
}

/// Packed BGRA `pixels` turned by `quarter_turns` clockwise quarter turns, as a phone held
/// upright records (its container says how to turn it back).
// Windows decodes no video file yet.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
pub(crate) fn turned_bgra(frame: DecodedVideoFrame, quarter_turns: u8) -> DecodedVideoFrame {
    const BYTES_PER_PIXEL: usize = 4;
    let quarter_turns = quarter_turns % 4;
    let (width, height) = (
        usize::try_from(frame.width).unwrap_or(0),
        usize::try_from(frame.height).unwrap_or(0),
    );
    if quarter_turns == 0
        || frame.format != DecodedPixelFormat::Bgra
        || frame.pixels.len() != width * height * BYTES_PER_PIXEL
    {
        return frame;
    }
    let (source, _) = frame.pixels.as_chunks::<BYTES_PER_PIXEL>();
    let mut turned = Vec::with_capacity(frame.pixels.len());
    let (turned_width, turned_height) = if quarter_turns == 2 {
        (width, height)
    } else {
        (height, width)
    };
    for row in 0..turned_height {
        for column in 0..turned_width {
            // The source pixel that lands at (column, row) once turned.
            let (source_column, source_row) = match quarter_turns {
                1 => (row, height - 1 - column),
                2 => (width - 1 - column, height - 1 - row),
                _ => (width - 1 - row, column),
            };
            turned.extend_from_slice(&source[source_row * width + source_column]);
        }
    }
    DecodedVideoFrame {
        width: u32::try_from(turned_width).unwrap_or(0),
        height: u32::try_from(turned_height).unwrap_or(0),
        format: DecodedPixelFormat::Bgra,
        pixels: turned,
    }
}

/// Clockwise quarter turns from a display rotation in degrees, counterclockwise as FFmpeg's
/// display matrix and `AVFoundation`'s transform state it; anything but a right angle is shown
/// as it is.
// Windows decodes no video file yet.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
pub(crate) fn quarter_turns_from_counterclockwise_degrees(degrees: f64) -> u8 {
    let rounded = degrees.round();
    if (degrees - rounded).abs() > 1.0 || rounded.rem_euclid(90.0) != 0.0 {
        return 0;
    }
    // A whole number of quarter turns, reduced to 0..4.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let counterclockwise = (rounded / 90.0).rem_euclid(4.0) as u8;
    (4 - counterclockwise) % 4
}

#[cfg(test)]
pub(crate) mod tests {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    /// Half a second of red, then half a second of blue (H.264 High with B-frames, AAC mono).
    pub(crate) const MP4_WITH_SOUND: &[u8] =
        include_bytes!("../tests/fixtures/video-h264-aac-160x90.mp4");
    /// The same pictures without sound.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) const MP4_SILENT: &[u8] =
        include_bytes!("../tests/fixtures/video-h264-silent-160x90.mp4");
    /// The same pictures in VP9, Opus sound.
    pub(crate) const WEBM_WITH_SOUND: &[u8] =
        include_bytes!("../tests/fixtures/video-vp9-opus-160x90.webm");

    #[test]
    fn containers_are_taken_only_when_type_and_bytes_agree() {
        assert_eq!(
            video_container(MP4_WITH_SOUND, "video/mp4"),
            Some(VideoContainer::Mp4)
        );
        assert_eq!(
            video_container(MP4_WITH_SOUND, "video/quicktime"),
            Some(VideoContainer::Mp4)
        );
        assert_eq!(
            video_container(WEBM_WITH_SOUND, "video/webm"),
            Some(VideoContainer::WebM)
        );
        assert_eq!(video_container(WEBM_WITH_SOUND, "video/mp4"), None);
        assert_eq!(video_container(MP4_WITH_SOUND, "video/webm"), None);
        assert_eq!(video_container(MP4_WITH_SOUND, "audio/mp4"), None);
        assert_eq!(video_container(b"\0\0", "video/mp4"), None);
    }

    #[test]
    fn each_system_names_the_types_it_plays() {
        assert_eq!(
            plays_video_type("video/mp4"),
            cfg!(any(target_os = "linux", target_os = "macos"))
        );
        assert_eq!(plays_video_type("video/webm"), cfg!(target_os = "linux"));
        assert!(!plays_video_type("video/x-matroska"));
    }

    fn millis(milliseconds: u64) -> Duration {
        Duration::from_millis(milliseconds)
    }

    #[test]
    fn a_picture_waits_for_its_time_then_is_taken() {
        let mut schedule = Schedule::default();
        assert_eq!(schedule.step(millis(0), None, false), Step::Decode);
        assert_eq!(
            schedule.step(millis(0), Some(millis(40)), false),
            Step::Take,
            "the first picture shows at once"
        );
        schedule.took(millis(0));
        assert_eq!(
            schedule.step(millis(0), Some(millis(10)), false),
            Step::Wait(millis(10))
        );
        assert_eq!(
            schedule.step(millis(0), Some(millis(500)), false),
            Step::Wait(POLL_PERIOD),
            "a long wait still looks at the clock"
        );
        assert_eq!(
            schedule.step(millis(12), Some(millis(10)), false),
            Step::Take
        );
        assert_eq!(
            schedule.step(millis(12), None, true),
            Step::Wait(POLL_PERIOD),
            "at the end it waits for the clock to come back"
        );
    }

    #[test]
    fn the_clock_moving_back_or_far_ahead_seeks() {
        let mut schedule = Schedule::default();
        schedule.took(millis(2_000));
        // Wobble is not a seek; a replay from the start is.
        assert_eq!(
            schedule.step(millis(1_980), Some(millis(2_100)), false),
            Step::Wait(POLL_PERIOD)
        );
        assert_eq!(
            schedule.step(millis(0), Some(millis(2_100)), true),
            Step::Seek(millis(0))
        );
        assert_eq!(
            schedule.step(millis(9_000), Some(millis(2_100)), false),
            Step::Seek(millis(9_000))
        );
        // Right after a seek the keyframe before is decoded up to the position, never sought
        // again.
        schedule.sought(millis(9_000));
        assert_eq!(
            schedule.step(millis(9_000), Some(millis(4_000)), false),
            Step::Take
        );
        schedule.took(millis(4_000));
        assert_eq!(
            schedule.step(millis(9_000), Some(millis(4_100)), false),
            Step::Take
        );
        schedule.waited();
        schedule.took(millis(9_000));
        assert_eq!(
            schedule.step(millis(3_000), None, false),
            Step::Seek(millis(3_000))
        );
    }

    #[test]
    fn a_decoder_slower_than_the_clock_still_shows_pictures_and_jumps_ahead() {
        let start = Instant::now();
        let mut schedule = Schedule::default();
        schedule.took(millis(0));
        schedule.published(start);
        // Every pending picture is already late: no wait ever comes.
        assert_eq!(
            schedule.step(millis(100), Some(millis(40)), false),
            Step::Take
        );
        schedule.took(millis(40));
        assert!(!schedule.is_publish_due(start + millis(5)));
        assert!(schedule.is_publish_due(start + POLL_PERIOD));
        assert_eq!(
            schedule.step(millis(1_500), Some(millis(80)), false),
            Step::Seek(millis(1_500))
        );
        schedule.sought(millis(1_500));
        // From the keyframe before up to the target: no jump meanwhile.
        assert_eq!(
            schedule.step(millis(1_600), Some(millis(1_000)), false),
            Step::Take
        );
        schedule.took(millis(1_000));
        assert_eq!(
            schedule.step(millis(2_700), Some(millis(1_500)), false),
            Step::Take
        );
        schedule.took(millis(1_500));
        // Caught up with the target without ever waiting, and still behind: it jumps again.
        assert_eq!(
            schedule.step(millis(2_800), Some(millis(1_533)), false),
            Step::Seek(millis(2_800))
        );
    }

    #[test]
    fn a_loop_wraps_at_its_length() {
        assert_eq!(looped(millis(2_500), Some(millis(1_000))), millis(500));
        assert_eq!(looped(millis(2_500), None), millis(2_500));
        assert_eq!(looped(millis(2_500), Some(Duration::ZERO)), millis(2_500));
    }

    fn picture(width: u32, height: u32) -> DecodedVideoFrame {
        // Each pixel holds its index in its first byte.
        let pixels = (0..width * height)
            .flat_map(|index| [u8::try_from(index).unwrap(), 0, 0, 255])
            .collect();
        DecodedVideoFrame {
            width,
            height,
            format: DecodedPixelFormat::Bgra,
            pixels,
        }
    }

    fn first_bytes(frame: &DecodedVideoFrame) -> Vec<u8> {
        frame.pixels.chunks(4).map(|pixel| pixel[0]).collect()
    }

    #[test]
    fn upright_pictures_come_from_turned_recordings() {
        // 0 1 2
        // 3 4 5
        let turned = turned_bgra(picture(3, 2), 1);
        assert_eq!((turned.width, turned.height), (2, 3));
        assert_eq!(first_bytes(&turned), [3, 0, 4, 1, 5, 2]);
        let upside_down = turned_bgra(picture(3, 2), 2);
        assert_eq!(first_bytes(&upside_down), [5, 4, 3, 2, 1, 0]);
        let back = turned_bgra(picture(3, 2), 3);
        assert_eq!(first_bytes(&back), [2, 5, 1, 4, 0, 3]);
        assert_eq!(turned_bgra(picture(3, 2), 4), picture(3, 2));
    }

    #[test]
    fn display_rotations_become_clockwise_quarter_turns() {
        assert_eq!(quarter_turns_from_counterclockwise_degrees(0.0), 0);
        assert_eq!(quarter_turns_from_counterclockwise_degrees(90.0), 3);
        assert_eq!(quarter_turns_from_counterclockwise_degrees(-90.0), 1);
        assert_eq!(quarter_turns_from_counterclockwise_degrees(180.0), 2);
        assert_eq!(quarter_turns_from_counterclockwise_degrees(270.0), 1);
        assert_eq!(quarter_turns_from_counterclockwise_degrees(45.0), 0);
    }

    #[test]
    fn a_silent_track_lasts_the_video() {
        let mut silence = SilentClip::new(millis(50));
        let mut samples = Vec::new();
        while silence.decode_next(&mut samples).unwrap() {}
        assert_eq!(samples.len(), 2_400);
        assert!(samples.iter().all(|sample| *sample == 0.0));
        silence.seek(2_000).unwrap();
        samples.clear();
        assert!(!silence.decode_next(&mut samples).unwrap());
        assert_eq!(samples.len(), 400);
    }

    /// A clock the test moves by hand, in milliseconds.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) struct ManualClock(pub(crate) Arc<AtomicU64>);

    impl VideoClock for ManualClock {
        fn position(&self) -> Duration {
            Duration::from_millis(self.0.load(Ordering::Relaxed))
        }
    }

    /// Whether a BGRA picture is mostly red, else mostly blue.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn is_red(frame: &DecodedVideoFrame) -> bool {
        let pixel = &frame.pixels[frame.pixels.len() / 2..][..4];
        pixel[2] > pixel[0]
    }

    /// Waits for the stream to show a picture that `wanted` accepts; the deadline only guards
    /// against a hang.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn wait_for_picture(
        stream: &VideoStream,
        wanted: impl Fn(&DecodedVideoFrame) -> bool,
    ) -> DecodedVideoFrame {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(frame) = stream.take_new_picture()
                && wanted(&frame)
            {
                return frame;
            }
            assert!(Instant::now() < deadline, "{:?}", stream.status());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_file_whose_bytes_are_not_its_type_fails_without_a_decoder() {
        let stream = VideoStream::start(
            Arc::from(&b"not a video at all"[..]),
            "video/mp4",
            VideoTiming::Loop { is_playing: true },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while stream.status().phase == VideoStreamPhase::Opening {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            stream.status().phase,
            VideoStreamPhase::Failed(ClipError::Unsupported)
        );
    }

    /// The platform decoder shows the first picture, follows the clock across the cut from red
    /// to blue, and comes back to red when the clock goes back to the start.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn follows_its_clock(bytes: &'static [u8], content_type: &str) {
        let position = Arc::new(AtomicU64::new(0));
        let stream = VideoStream::start(
            Arc::from(bytes),
            content_type,
            VideoTiming::Follow(Box::new(ManualClock(Arc::clone(&position)))),
        )
        .unwrap();
        let first = wait_for_picture(&stream, |_| true);
        assert_eq!((first.width, first.height), (160, 90));
        assert!(is_red(&first));
        let status = stream.status();
        assert_eq!(status.phase, VideoStreamPhase::Showing);
        let duration = status.duration.unwrap();
        assert!(
            (millis(900)..=millis(1_100)).contains(&duration),
            "{duration:?}"
        );

        position.store(800, Ordering::Relaxed);
        wait_for_picture(&stream, |frame| !is_red(frame));
        position.store(0, Ordering::Relaxed);
        wait_for_picture(&stream, is_red);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn an_mp4_follows_its_clock() {
        follows_its_clock(MP4_WITH_SOUND, "video/mp4");
    }

    /// Red above, blue below, 2560 × 1440, stored with a display rotation of 90° counterclockwise
    /// as a phone held upright records.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn a_large_turned_recording_is_shrunk_and_shown_upright() {
        const TURNED: &[u8] = include_bytes!("../tests/fixtures/video-h264-turned-2560x1440.mp4");
        let stream = VideoStream::start(
            Arc::from(TURNED),
            "video/mp4",
            VideoTiming::Loop { is_playing: false },
        )
        .unwrap();
        let picture = wait_for_picture(&stream, |_| true);
        assert_eq!((picture.width, picture.height), (720, 1_280));
        let pixel = |column: usize| {
            let row = usize::try_from(picture.height / 2).unwrap();
            let width = usize::try_from(picture.width).unwrap();
            let start = (row * width + column) * 4;
            [picture.pixels[start], picture.pixels[start + 2]]
        };
        let [blue, red] = pixel(10);
        assert!(red > blue, "red on the left once upright");
        let [blue, red] = pixel(usize::try_from(picture.width).unwrap() - 10);
        assert!(blue > red, "blue on the right");
    }

    /// 4800 × 4800 coded pictures whose header shows them 75 × 4800 (a sample aspect of 1:64):
    /// the limit holds on the coded size, not the shown one.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn a_picture_coded_over_the_limit_is_refused() {
        const HUGE: &[u8] =
            include_bytes!("../tests/fixtures/video-h264-4800x4800-shown-75x4800.mp4");
        let stream = VideoStream::start(
            Arc::from(HUGE),
            "video/mp4",
            VideoTiming::Loop { is_playing: false },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while stream.status().phase == VideoStreamPhase::Opening {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        // Refused while probing (damaged to FFmpeg) or once opened (unsupported).
        assert!(
            matches!(stream.status().phase, VideoStreamPhase::Failed(_)),
            "{:?}",
            stream.status()
        );
        assert!(stream.take_new_picture().is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_webm_follows_its_clock() {
        follows_its_clock(WEBM_WITH_SOUND, "video/webm");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn a_looping_video_paused_holds_its_first_picture_and_loops_once_played() {
        let stream = VideoStream::start(
            Arc::from(MP4_SILENT),
            "video/mp4",
            VideoTiming::Loop { is_playing: false },
        )
        .unwrap();
        assert!(is_red(&wait_for_picture(&stream, |_| true)));
        stream.set_loop_playing(true);
        wait_for_picture(&stream, |frame| !is_red(frame));
        // Back to red: it looped.
        wait_for_picture(&stream, is_red);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn the_sound_of_a_video_or_its_silence_lasts_the_video() {
        let mut samples = Vec::new();
        for (bytes, content_type) in [(MP4_WITH_SOUND, "video/mp4"), (MP4_SILENT, "video/mp4")] {
            let mut sound = open_video_sound(Arc::from(bytes), content_type).unwrap();
            samples.clear();
            while sound.decode_next(&mut samples).unwrap() {}
            #[allow(clippy::cast_precision_loss)] // A second of samples.
            let seconds =
                samples.len() as f64 / sound.channels() as f64 / f64::from(sound.sample_rate());
            assert!((0.9..=1.1).contains(&seconds), "{content_type}: {seconds}");
        }
        assert_eq!(
            open_video_sound(Arc::from(MP4_WITH_SOUND), "video/webm").err(),
            Some(ClipError::Unsupported)
        );
    }
}
