//! Screens and windows to share, behind one trait (ADR-0054): `ScreenCaptureKit` on macOS, the
//! ScreenCast portal and PipeWire on Wayland, X11 itself on an X11 session, nothing yet on
//! Windows, and a synthetic screen for tests. A capture starts its picture (and its sound) through
//! the [`ScreenShareContext`] the session hands it, once the person has chosen and the system
//! delivers: a cancelled picker never turns the share on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mellow_protocol::VideoCodec;

use crate::camera::synthetic_frame;
use crate::screen_sound::{OutgoingScreenSoundPacket, PcmSoundEncoder, ScreenSoundInput};
use crate::session::ScreenShareEnd;
use crate::simulcast::{FRAMES_PER_SECOND, ScreenShareQuality, VideoSourceKind};
use crate::transport::NetworkCommand;
use crate::video_frame::{CapturedFrame, CapturedPixelFormat};
use crate::video_sender::{EncodedVideoInput, OutgoingVideoFrame, VideoEncoderControl, VideoInput};

/// The longest side of a source's thumbnail in the in-app picker.
pub(crate) const THUMBNAIL_LONGEST_SIDE: u32 = 320;

/// How the person picks what to share on this system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenPicker {
    /// Mellow lists the sources ([`ScreenSources::sources`]) and the person picks one.
    InApp,
    /// The system shows its own dialog when the share starts ([`ScreenChoice::SystemPicker`]):
    /// the Wayland portal.
    System,
    /// This system cannot share a screen yet (Windows, or no display server).
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenSourceKind {
    Display,
    Window,
}

/// A screen or window the in-app picker offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenSourceDescription {
    /// Pass it back in [`ScreenChoice::Source`]; valid while the source exists.
    pub id: String,
    pub kind: ScreenSourceKind,
    /// The window's title, or the display's name. Never logged.
    pub title: String,
    /// The application owning a window.
    pub application_name: Option<String>,
    /// In pixels, as captured.
    pub width: u32,
    pub height: u32,
    /// A small BGRA picture of it, when asked for and the system allows it.
    pub thumbnail: Option<CapturedFrame>,
}

/// What to share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenChoice {
    /// A source of [`ScreenSources::sources`].
    Source(String),
    /// Let the system's dialog ask ([`ScreenPicker::System`]).
    SystemPicker,
}

/// Which sound goes with the picture.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ShareSound {
    /// The picture alone.
    Off,
    /// What the source plays: a window's application, or the whole system without Mellow's own
    /// sounds for a display. Where the system cannot tell a window's application (Wayland), the
    /// share goes without sound and says so ([`ShareSoundState::NotAvailable`]).
    #[default]
    FromSource,
    /// The playback of one application of [`ScreenSources::sound_applications`].
    Application(String),
}

/// An application playing sound, to choose as a share's sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundApplication {
    /// Pass it back in [`ShareSound::Application`].
    pub id: String,
    pub name: String,
}

/// Whether a running share carries sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareSoundState {
    Captured,
    /// Not asked for.
    Off,
    /// Asked for, but this system or source cannot give it.
    NotAvailable,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScreenCaptureError {
    #[error("screen sharing is not available on this system")]
    Unavailable,
    #[error("the system refused to let Mellow record the screen")]
    PermissionDenied,
    #[error("no source was chosen")]
    Cancelled,
    #[error("the chosen screen or window is gone")]
    SourceGone,
    #[error("screen capture failed: {0}")]
    System(String),
}

/// Keeps a capture running; dropping it stops it.
pub trait RunningScreenCapture: Send {
    fn sound(&self) -> ShareSoundState;
}

/// Lists and captures screens and windows.
pub trait ScreenSources: Send + Sync {
    fn picker(&self) -> ScreenPicker;

    /// The displays and windows the in-app picker offers (none with a system picker), with
    /// thumbnails when `with_thumbnails`. Can take a while (thumbnails): call it off the
    /// interface's thread.
    ///
    /// # Errors
    /// The system refused (macOS's Screen Recording permission) or failed.
    fn sources(
        &self,
        with_thumbnails: bool,
    ) -> Result<Vec<ScreenSourceDescription>, ScreenCaptureError>;

    /// The applications playing sound that can be the share's sound instead of its source's.
    fn sound_applications(&self) -> Vec<SoundApplication> {
        Vec::new()
    }

    /// Starts capturing `choice` with `sound`, and turns the share on through `share` once
    /// pictures can flow. Blocks while a system picker is open: call it off the interface's
    /// thread.
    ///
    /// # Errors
    /// Cancelled, refused, gone, or the system failed; the share stays off.
    fn start(
        &self,
        choice: &ScreenChoice,
        sound: &ShareSound,
        share: ScreenShareContext,
    ) -> Result<Box<dyn RunningScreenCapture>, ScreenCaptureError>;
}

/// What a capture needs from the voice session: where its pictures and sound go, and how to say
/// it ended. Cheap to clone; usable from any thread.
#[derive(Clone)]
pub struct ScreenShareContext {
    inner: Arc<ShareContextInner>,
}

struct ShareContextInner {
    quality: ScreenShareQuality,
    video_encoder: Arc<VideoEncoderControl>,
    network_commands: Sender<NetworkCommand>,
    outgoing_video: Sender<OutgoingVideoFrame>,
    outgoing_sound: Sender<OutgoingScreenSoundPacket>,
    /// The source start of this share, once it started, and its codec.
    started: Mutex<Option<(u64, VideoCodec)>>,
}

impl std::fmt::Debug for ScreenShareContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScreenShareContext")
            .field("quality", &self.inner.quality)
            .finish_non_exhaustive()
    }
}

impl ScreenShareContext {
    pub(crate) fn new(
        quality: ScreenShareQuality,
        video_encoder: Arc<VideoEncoderControl>,
        network_commands: Sender<NetworkCommand>,
        outgoing_video: Sender<OutgoingVideoFrame>,
        outgoing_sound: Sender<OutgoingScreenSoundPacket>,
    ) -> Self {
        Self {
            inner: Arc::new(ShareContextInner {
                quality,
                video_encoder,
                network_commands,
                outgoing_video,
                outgoing_sound,
                started: Mutex::new(None),
            }),
        }
    }

    /// The quality the person chose when the share started.
    #[must_use]
    pub fn quality(&self) -> ScreenShareQuality {
        self.inner.quality
    }

    fn kind(&self) -> VideoSourceKind {
        VideoSourceKind::Screen(self.inner.quality)
    }

    fn network(&self, command: NetworkCommand) {
        // The network thread gone means the session is ending.
        let _ = self.inner.network_commands.send(command);
    }

    fn started(&self) -> std::sync::MutexGuard<'_, Option<(u64, VideoCodec)>> {
        self.inner
            .started
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Turns the share on with pictures encoded here in VP8 (any CPU capture), replacing the
    /// camera or another screen.
    #[must_use]
    pub fn start_pictures(&self) -> VideoInput {
        self.network(NetworkCommand::SetCameraCodec(VideoCodec::Vp8));
        let input = self.inner.video_encoder.start(self.kind());
        *self.started() = Some((input.generation(), VideoCodec::Vp8));
        self.network(NetworkCommand::SetVideoSource(Some(self.kind())));
        input
    }

    /// Turns the share on with layers encoded by the capture itself in `codec` (a GPU's H.264,
    /// ADR-0054); the session is offered again for that codec.
    #[must_use]
    pub fn start_encoded(&self, codec: VideoCodec) -> EncodedVideoInput {
        self.network(NetworkCommand::SetCameraCodec(codec));
        let input = self.inner.video_encoder.start_encoded(
            self.kind(),
            codec,
            self.inner.outgoing_video.clone(),
        );
        *self.started() = Some((input.generation(), codec));
        self.network(NetworkCommand::SetVideoSource(Some(self.kind())));
        input
    }

    /// Where the share's sound goes, once the picture started; `None` before.
    #[must_use]
    pub fn sound_input(&self) -> Option<ScreenSoundInput> {
        let (generation, _) = (*self.started())?;
        Some(ScreenSoundInput::new(
            Arc::clone(&self.inner.video_encoder),
            generation,
            self.inner.outgoing_sound.clone(),
        ))
    }

    /// The capture ended by itself (the window closed, the system revoked it, it failed): the
    /// share turns off, and the session says so ([`crate::VoiceSessionEvent::ScreenShareEnded`]).
    pub fn end(&self, reason: ScreenShareEnd) {
        if let Some((generation, _)) = *self.started() {
            self.network(NetworkCommand::ScreenShareEnded { generation, reason });
        }
    }

    /// Whether the share is still on (not stopped, replaced or refused).
    #[must_use]
    pub fn is_on(&self) -> bool {
        self.started()
            .is_some_and(|(generation, _)| self.inner.video_encoder.is_generation_on(generation))
    }

    pub(crate) fn codec(&self) -> Option<VideoCodec> {
        self.started().map(|(_, codec)| codec)
    }

    /// Turns the share off if it is still this one, and tells the server.
    pub(crate) fn stop(&self) {
        if self.is_on() {
            self.inner.video_encoder.stop_source();
            self.network(NetworkCommand::SetVideoSource(None));
        }
    }
}

/// A running share: dropping it stops the capture and turns the share off (if nothing else did
/// first).
pub struct ScreenShare {
    context: ScreenShareContext,
    capture: Option<Box<dyn RunningScreenCapture>>,
}

impl std::fmt::Debug for ScreenShare {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScreenShare")
            .field("context", &self.context)
            .finish_non_exhaustive()
    }
}

impl ScreenShare {
    pub(crate) fn new(context: ScreenShareContext, capture: Box<dyn RunningScreenCapture>) -> Self {
        Self {
            context,
            capture: Some(capture),
        }
    }

    /// The codec watchers receive (H.264 from a GPU encoder, else VP8).
    #[must_use]
    pub fn codec(&self) -> VideoCodec {
        self.context.codec().unwrap_or(VideoCodec::Vp8)
    }

    #[must_use]
    pub fn sound(&self) -> ShareSoundState {
        self.capture
            .as_ref()
            .map_or(ShareSoundState::Off, |capture| capture.sound())
    }

    /// Whether the share is still on (the server or the source may have ended it).
    #[must_use]
    pub fn is_on(&self) -> bool {
        self.context.is_on()
    }
}

impl Drop for ScreenShare {
    fn drop(&mut self) {
        // The inputs close first, so capture threads see it and stop sending.
        self.context.stop();
        self.capture = None;
    }
}

/// Copies 8-bit RGB rows into a packed BGRA picture (`bytes` is reused), swapping red and blue
/// when `is_red_first`. `None` when `pixels` is shorter than the rows it claims.
pub(crate) fn pack_bgra_rows(
    mut bytes: Vec<u8>,
    width: u32,
    height: u32,
    stride: usize,
    pixels: &[u8],
    is_red_first: bool,
) -> Option<CapturedFrame> {
    let row_bytes = usize::try_from(width).ok()?.checked_mul(4)?;
    let rows = usize::try_from(height).ok()?;
    if width == 0
        || rows == 0
        || stride < row_bytes
        || pixels.len() < stride.checked_mul(rows - 1)?.checked_add(row_bytes)?
    {
        return None;
    }
    bytes.clear();
    bytes.reserve(row_bytes * rows);
    for row in pixels.chunks(stride).take(rows) {
        let row = row.get(..row_bytes)?;
        if is_red_first {
            bytes.extend(row.as_chunks::<4>().0.iter().flat_map(
                |[blue_or_red, green, red_or_blue, alpha]| {
                    [*red_or_blue, *green, *blue_or_red, *alpha]
                },
            ));
        } else {
            bytes.extend_from_slice(row);
        }
    }
    Some(CapturedFrame {
        width,
        height,
        format: CapturedPixelFormat::Bgra,
        bytes,
    })
}

/// A packed BGRA picture shrunk (nearest sample) so its longest side is at most
/// [`THUMBNAIL_LONGEST_SIDE`]; pictures already that small come back as they are.
pub(crate) fn thumbnail_of(picture: &CapturedFrame) -> Option<CapturedFrame> {
    if picture.format != CapturedPixelFormat::Bgra
        || picture.bytes.len()
            != CapturedFrame::expected_length(picture.width, picture.height, picture.format)
    {
        return None;
    }
    let longest = picture.width.max(picture.height);
    if longest <= THUMBNAIL_LONGEST_SIDE {
        return Some(picture.clone());
    }
    let shrink = |length: u32| {
        u32::try_from(u64::from(length) * u64::from(THUMBNAIL_LONGEST_SIDE) / u64::from(longest))
            .unwrap_or(1)
            .max(1)
    };
    let (width, height) = (shrink(picture.width), shrink(picture.height));
    let source_width = usize::try_from(picture.width).ok()?;
    let mut bytes = Vec::with_capacity(CapturedFrame::expected_length(
        width,
        height,
        CapturedPixelFormat::Bgra,
    ));
    for y in 0..height {
        let source_y = u64::from(y) * u64::from(picture.height) / u64::from(height);
        for x in 0..width {
            let source_x = u64::from(x) * u64::from(picture.width) / u64::from(width);
            let index = (usize::try_from(source_y).ok()? * source_width
                + usize::try_from(source_x).ok()?)
                * 4;
            bytes.extend_from_slice(picture.bytes.get(index..index + 4)?);
        }
    }
    Some(CapturedFrame {
        width,
        height,
        format: CapturedPixelFormat::Bgra,
        bytes,
    })
}

/// The system's screens and windows: `ScreenCaptureKit` on macOS; on Linux the ScreenCast portal
/// under Wayland and X11 itself under X11 (chosen at run time, as GPUI does); nothing on Windows
/// yet (ADR-0054).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemScreenSources;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl ScreenSources for SystemScreenSources {
    fn picker(&self) -> ScreenPicker {
        ScreenPicker::Unavailable
    }

    fn sources(
        &self,
        _with_thumbnails: bool,
    ) -> Result<Vec<ScreenSourceDescription>, ScreenCaptureError> {
        Ok(Vec::new())
    }

    fn start(
        &self,
        _choice: &ScreenChoice,
        _sound: &ShareSound,
        _share: ScreenShareContext,
    ) -> Result<Box<dyn RunningScreenCapture>, ScreenCaptureError> {
        Err(ScreenCaptureError::Unavailable)
    }
}

/// A screen showing a moving test pattern, and a 440 Hz tone as its sound: for tests and headless
/// use. Its only source is a display.
#[derive(Debug, Clone, Copy)]
pub struct SyntheticScreens {
    pub width: u32,
    pub height: u32,
}

impl Default for SyntheticScreens {
    fn default() -> Self {
        Self {
            width: 1_280,
            height: 720,
        }
    }
}

/// The synthetic screen's id.
const SYNTHETIC_SCREEN_ID: &str = "synthetic-screen";
/// The synthetic sound's pitch and loudness.
const SYNTHETIC_TONE_HERTZ: f32 = 440.0;
const SYNTHETIC_TONE_AMPLITUDE: f32 = 0.3;

struct SyntheticCapture {
    should_stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    sound: ShareSoundState,
}

impl RunningScreenCapture for SyntheticCapture {
    fn sound(&self) -> ShareSoundState {
        self.sound
    }
}

impl Drop for SyntheticCapture {
    fn drop(&mut self) {
        self.should_stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the synthetic screen's thread panicked");
        }
    }
}

impl ScreenSources for SyntheticScreens {
    fn picker(&self) -> ScreenPicker {
        ScreenPicker::InApp
    }

    fn sources(
        &self,
        with_thumbnails: bool,
    ) -> Result<Vec<ScreenSourceDescription>, ScreenCaptureError> {
        let thumbnail = with_thumbnails
            .then(|| synthetic_frame(self.width, self.height, 0).to_i420())
            .flatten()
            .and_then(|picture| picture.view().to_bgra())
            .and_then(|bytes| {
                thumbnail_of(&CapturedFrame {
                    width: self.width,
                    height: self.height,
                    format: CapturedPixelFormat::Bgra,
                    bytes,
                })
            });
        Ok(vec![ScreenSourceDescription {
            id: SYNTHETIC_SCREEN_ID.into(),
            kind: ScreenSourceKind::Display,
            title: "Synthetic screen".into(),
            application_name: None,
            width: self.width,
            height: self.height,
            thumbnail,
        }])
    }

    fn start(
        &self,
        choice: &ScreenChoice,
        sound: &ShareSound,
        share: ScreenShareContext,
    ) -> Result<Box<dyn RunningScreenCapture>, ScreenCaptureError> {
        if *choice != ScreenChoice::Source(SYNTHETIC_SCREEN_ID.into()) {
            return Err(ScreenCaptureError::SourceGone);
        }
        let pictures = share.start_pictures();
        let sound_encoder = match sound {
            ShareSound::Off => None,
            ShareSound::FromSource | ShareSound::Application(_) => share
                .sound_input()
                .map(PcmSoundEncoder::start)
                .transpose()
                .map_err(|error| ScreenCaptureError::System(error.to_string()))?,
        };
        let sound_state = if sound_encoder.is_some() {
            ShareSoundState::Captured
        } else {
            ShareSoundState::Off
        };
        let should_stop = Arc::new(AtomicBool::new(false));
        let (width, height) = (self.width, self.height);
        let thread = {
            let should_stop = Arc::clone(&should_stop);
            std::thread::Builder::new()
                .name("mellow-synthetic-screen".into())
                .spawn(move || {
                    let (_encoder, mut writer) = sound_encoder.unzip();
                    let interval = Duration::from_secs(1) / FRAMES_PER_SECOND;
                    let samples_per_frame =
                        crate::screen_sound::SCREEN_SOUND_SAMPLE_RATE / FRAMES_PER_SECOND;
                    let mut tone = Vec::with_capacity(samples_per_frame as usize);
                    let mut next_frame_at = Instant::now();
                    let mut index: u32 = 0;
                    let mut sample_index: u32 = 0;
                    while !should_stop.load(Ordering::Relaxed) && pictures.is_open() {
                        pictures.push(synthetic_frame(width, height, index));
                        if let Some(writer) = writer.as_mut() {
                            tone.clear();
                            tone.extend((0..samples_per_frame).map(|_| {
                                sample_index = (sample_index + 1) % 48_000;
                                // Below 48 000: exact in an f32.
                                #[allow(clippy::cast_precision_loss)]
                                let phase = sample_index as f32
                                    * SYNTHETIC_TONE_HERTZ
                                    * std::f32::consts::TAU
                                    / 48_000.0;
                                phase.sin() * SYNTHETIC_TONE_AMPLITUDE
                            }));
                            writer.write_planar(&tone, &tone);
                        }
                        index = index.wrapping_add(1);
                        next_frame_at += interval;
                        std::thread::sleep(next_frame_at.saturating_duration_since(Instant::now()));
                    }
                })
                .map_err(|error| ScreenCaptureError::System(error.to_string()))?
        };
        Ok(Box::new(SyntheticCapture {
            should_stop,
            thread: Some(thread),
            sound: sound_state,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_with_padding_are_packed_and_red_first_rows_swapped() {
        // Two pixels a row, a stride of 12 bytes, two rows.
        let pixels = [
            1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, //
            9, 10, 11, 12, 13, 14, 15, 16,
        ];
        let packed = pack_bgra_rows(Vec::new(), 2, 2, 12, &pixels, false).unwrap();
        assert_eq!(
            packed.bytes,
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
        let swapped = pack_bgra_rows(packed.bytes, 2, 2, 12, &pixels, true).unwrap();
        assert_eq!(&swapped.bytes[..8], [3, 2, 1, 4, 7, 6, 5, 8]);
        assert!(pack_bgra_rows(Vec::new(), 2, 2, 12, &pixels[..19], false).is_none());
        assert!(pack_bgra_rows(Vec::new(), 4, 2, 12, &pixels, false).is_none());
    }

    #[test]
    fn thumbnails_keep_the_shape_within_the_longest_side() {
        let picture = CapturedFrame {
            width: 1_920,
            height: 1_080,
            format: CapturedPixelFormat::Bgra,
            bytes: vec![7; 1_920 * 1_080 * 4],
        };
        let thumbnail = thumbnail_of(&picture).unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (320, 180));
        assert_eq!(thumbnail.bytes.len(), 320 * 180 * 4);
        let small = CapturedFrame {
            width: 10,
            height: 10,
            format: CapturedPixelFormat::Bgra,
            bytes: vec![1; 400],
        };
        assert_eq!(thumbnail_of(&small).unwrap(), small);
    }

    #[test]
    fn the_synthetic_screen_lists_one_display_with_its_thumbnail() {
        let screens = SyntheticScreens::default();
        assert_eq!(screens.picker(), ScreenPicker::InApp);
        let sources = screens.sources(true).unwrap();
        assert_eq!(sources.len(), 1);
        let thumbnail = sources[0].thumbnail.as_ref().unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (320, 180));
        assert!(screens.sources(false).unwrap()[0].thumbnail.is_none());
    }
}
