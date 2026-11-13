//! Screen sharing on macOS through `ScreenCaptureKit` (ADR-0054), with the `objc2` bindings GPUI
//! already brings: an in-app picker fed by `SCShareableContent` (thumbnails from
//! `SCScreenshotManager` on macOS 14), an `SCStream` delivering NV12 pictures copied into the
//! session's recycled buffers, and the stream's own audio (macOS 13) without Mellow's sounds.

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, ProtocolObject};
use objc2::{AllocAnyThread as _, DefinedClass as _, define_class, msg_send, sel};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBaseAddressOfPlane,
    CVPixelBufferGetBytesPerRow, CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeight,
    CVPixelBufferGetHeightOfPlane, CVPixelBufferGetPixelFormatType, CVPixelBufferGetPlaneCount,
    CVPixelBufferGetWidth, CVPixelBufferGetWidthOfPlane, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress, kCVPixelFormatType_32BGRA,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVReturnSuccess,
};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCRunningApplication, SCScreenshotManager, SCShareableContent,
    SCStream, SCStreamConfiguration, SCStreamDelegate, SCStreamErrorCode, SCStreamOutput,
    SCStreamOutputType, SCWindow,
};

use crate::screen_capture::{
    RunningScreenCapture, ScreenCaptureError, ScreenChoice, ScreenPicker, ScreenShareContext,
    ScreenSourceDescription, ScreenSourceKind, ScreenSources, ShareSound, ShareSoundState,
    SystemScreenSources, THUMBNAIL_LONGEST_SIDE, pack_bgra_rows, thumbnail_of,
};
use crate::screen_sound::{
    PcmSoundEncoder, PcmSoundWriter, SCREEN_SOUND_CHANNELS, SCREEN_SOUND_SAMPLE_RATE,
};
use crate::session::ScreenShareEnd;
use crate::simulcast::FRAMES_PER_SECOND;
use crate::video_frame::{CapturedFrame, CapturedPixelFormat, fit_within};
use crate::video_sender::VideoInput;

/// How long the system may take to list sources, take a thumbnail or start a stream.
const SYSTEM_ANSWER_TIMEOUT: Duration = Duration::from_secs(10);
/// Captured sizes are asked in pixels; `ScreenCaptureKit` gives sources in points. A Retina
/// screen has two pixels per point; a larger request on another screen is only fitted down.
const PIXELS_PER_POINT: u32 = 2;
/// Audio frames copied per call; `ScreenCaptureKit` hands about 1 024 at a time.
const AUDIO_SCRATCH_FRAMES: usize = 4_096;
const DISPLAY_ID_PREFIX: &str = "display:";
const WINDOW_ID_PREFIX: &str = "window:";

/// `ScreenCaptureKit`'s content, moved from its completion queue to the caller.
struct SendableContent(Retained<SCShareableContent>);

// SAFETY: `SCShareableContent` and the objects it lists are immutable snapshots that Apple
// documents as usable from any queue; nothing here mutates them.
unsafe impl Send for SendableContent {}

fn error_of(error: &NSError) -> ScreenCaptureError {
    if error.code() == SCStreamErrorCode::UserDeclined.0 {
        ScreenCaptureError::PermissionDenied
    } else {
        ScreenCaptureError::System(error.localizedDescription().to_string())
    }
}

/// The displays and on-screen windows, asking for the Screen Recording permission the first
/// time (the system shows its prompt).
fn shareable_content() -> Result<Retained<SCShareableContent>, ScreenCaptureError> {
    let (answer, answered) = mpsc::channel();
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: ScreenCaptureKit passes valid objects or null, retained by it for the
            // call; `retain` takes our own reference.
            let result = match (unsafe { Retained::retain(content) }, unsafe {
                error.as_ref()
            }) {
                (Some(content), _) => Ok(SendableContent(content)),
                (None, Some(error)) => Err(error_of(error)),
                (None, None) => Err(ScreenCaptureError::Unavailable),
            };
            // The caller stopped waiting only if it gave up.
            let _ = answer.send(result);
        },
    );
    // SAFETY: a class method with a block ScreenCaptureKit keeps until it calls it once.
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true, true, &handler,
        );
    }
    match answered.recv_timeout(SYSTEM_ANSWER_TIMEOUT) {
        Ok(result) => result.map(|content| content.0),
        Err(_) => Err(ScreenCaptureError::System(
            "the system did not list the screens in time".into(),
        )),
    }
}

/// Mellow's own application in `content`, to leave its windows and sounds out of a display.
fn own_application(content: &SCShareableContent) -> Option<Retained<SCRunningApplication>> {
    let own_process = i32::try_from(std::process::id()).ok()?;
    // SAFETY: plain property reads of an immutable snapshot.
    unsafe { content.applications() }
        .iter()
        .find(|application| unsafe { application.processID() } == own_process)
}

/// The windows worth offering: on screen, at the normal level, titled, of another application.
fn shareable_windows(content: &SCShareableContent) -> Vec<Retained<SCWindow>> {
    let own_process = i32::try_from(std::process::id()).unwrap_or(-1);
    // SAFETY: plain property reads of an immutable snapshot.
    unsafe { content.windows() }
        .iter()
        .filter(|window| unsafe {
            window.isOnScreen()
                && window.windowLayer() == 0
                && window.title().is_some_and(|title| title.length() > 0)
                && window
                    .owningApplication()
                    .is_some_and(|application| application.processID() != own_process)
                && window.frame().size.width >= 1.0
                && window.frame().size.height >= 1.0
        })
        .collect()
}

fn points_to_pixels(points: f64) -> u32 {
    // Screen sizes in points are far below u32::MAX.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let points = points.max(1.0).round() as u32;
    points.saturating_mul(PIXELS_PER_POINT)
}

fn display_filter(display: &SCDisplay, content: &SCShareableContent) -> Retained<SCContentFilter> {
    let excluded: Retained<NSArray<SCRunningApplication>> = own_application(content)
        .map_or_else(NSArray::new, |application| {
            NSArray::from_retained_slice(&[application])
        });
    // SAFETY: valid objects; the filter keeps its own references.
    unsafe {
        SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
            SCContentFilter::alloc(),
            display,
            &excluded,
            &NSArray::new(),
        )
    }
}

fn window_filter(window: &SCWindow) -> Retained<SCContentFilter> {
    // SAFETY: a valid window; the filter keeps its own reference.
    unsafe { SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), window) }
}

/// One BGRA picture of `filter` at most [`THUMBNAIL_LONGEST_SIDE`] wide or high, on macOS 14
/// and later (`SCScreenshotManager`); `None` before or on failure.
fn thumbnail(filter: &SCContentFilter, size: (u32, u32)) -> Option<CapturedFrame> {
    AnyClass::get(c"SCScreenshotManager")?;
    let (width, height) = fit_within(size, (THUMBNAIL_LONGEST_SIDE, THUMBNAIL_LONGEST_SIDE));
    // SAFETY: a configuration made and set up here only.
    let configuration = unsafe {
        let configuration = SCStreamConfiguration::new();
        configuration.setWidth(width as usize);
        configuration.setHeight(height as usize);
        configuration.setPixelFormat(kCVPixelFormatType_32BGRA);
        configuration.setShowsCursor(false);
        configuration
    };
    let (answer, answered) = mpsc::channel();
    let handler = RcBlock::new(move |sample: *mut CMSampleBuffer, _error: *mut NSError| {
        // SAFETY: a valid sample buffer or null, alive for the call.
        let picture = unsafe { sample.as_ref() }.and_then(copy_bgra_picture);
        let _ = answer.send(picture);
    });
    // SAFETY: valid filter and configuration; the block is kept until called once.
    unsafe {
        SCScreenshotManager::captureSampleBufferWithFilter_configuration_completionHandler(
            filter,
            &configuration,
            Some(&handler),
        );
    }
    answered
        .recv_timeout(SYSTEM_ANSWER_TIMEOUT)
        .ok()
        .flatten()
        .and_then(|picture| thumbnail_of(&picture))
}

/// A locked pixel buffer, unlocked when dropped.
struct LockedPixels<'a>(&'a CVPixelBuffer);

impl<'a> LockedPixels<'a> {
    fn lock(pixels: &'a CVPixelBuffer) -> Option<Self> {
        // SAFETY: a valid pixel buffer, unlocked in `drop` with the same flags.
        let status =
            unsafe { CVPixelBufferLockBaseAddress(pixels, CVPixelBufferLockFlags::ReadOnly) };
        (status == kCVReturnSuccess).then_some(Self(pixels))
    }
}

impl Drop for LockedPixels<'_> {
    fn drop(&mut self) {
        // SAFETY: locked in `lock` with the same flags.
        unsafe { CVPixelBufferUnlockBaseAddress(self.0, CVPixelBufferLockFlags::ReadOnly) };
    }
}

/// The sample's BGRA picture, packed.
fn copy_bgra_picture(sample: &CMSampleBuffer) -> Option<CapturedFrame> {
    // SAFETY: the sample buffer is valid for this call; the image buffer is retained.
    let image = unsafe { sample.image_buffer() }?;
    let pixels: &CVPixelBuffer = &image;
    if CVPixelBufferGetPixelFormatType(pixels) != kCVPixelFormatType_32BGRA {
        return None;
    }
    let _locked = LockedPixels::lock(pixels)?;
    let (width, height) = (
        CVPixelBufferGetWidth(pixels),
        CVPixelBufferGetHeight(pixels),
    );
    let stride = CVPixelBufferGetBytesPerRow(pixels);
    let base = CVPixelBufferGetBaseAddress(pixels).cast::<u8>();
    if base.is_null() || height == 0 {
        return None;
    }
    // SAFETY: the buffer is locked and holds `height` rows of `stride` bytes.
    let bytes = unsafe { std::slice::from_raw_parts(base, stride * height) };
    pack_bgra_rows(
        Vec::new(),
        u32::try_from(width).ok()?,
        u32::try_from(height).ok()?,
        stride,
        bytes,
        false,
    )
}

/// The sample's NV12 planes packed into `bytes` (reused), or `None` for any other layout or a
/// sample without a picture (`ScreenCaptureKit` sends some when the screen is idle).
fn copy_nv12_picture(sample: &CMSampleBuffer, mut bytes: Vec<u8>) -> Option<CapturedFrame> {
    // SAFETY: the sample buffer is valid for this callback; the image buffer is retained.
    let image = unsafe { sample.image_buffer() }?;
    let pixels: &CVPixelBuffer = &image;
    if CVPixelBufferGetPixelFormatType(pixels) != kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        || CVPixelBufferGetPlaneCount(pixels) != 2
    {
        return None;
    }
    let _locked = LockedPixels::lock(pixels)?;
    let width = CVPixelBufferGetWidthOfPlane(pixels, 0);
    let height = CVPixelBufferGetHeightOfPlane(pixels, 0);
    let frame_width = u32::try_from(width).ok()?;
    let frame_height = u32::try_from(height).ok()?;
    bytes.clear();
    bytes.reserve(CapturedFrame::expected_length(
        frame_width,
        frame_height,
        CapturedPixelFormat::Nv12,
    ));
    // Luma rows of `width` bytes, then interleaved chroma rows of `2 × ⌈width / 2⌉` bytes.
    for (plane, row_bytes, rows) in [
        (0, width, height),
        (1, 2 * width.div_ceil(2), height.div_ceil(2)),
    ] {
        let base = CVPixelBufferGetBaseAddressOfPlane(pixels, plane).cast::<u8>();
        let stride = CVPixelBufferGetBytesPerRowOfPlane(pixels, plane);
        if base.is_null()
            || stride < row_bytes
            || CVPixelBufferGetHeightOfPlane(pixels, plane) < rows
        {
            return None;
        }
        // SAFETY: the buffer is locked; the plane holds `rows` rows of `stride` bytes (checked
        // against the buffer's own plane height and stride above).
        let plane_bytes = unsafe { std::slice::from_raw_parts(base, stride * rows) };
        for row in plane_bytes.chunks(stride) {
            bytes.extend_from_slice(row.get(..row_bytes)?);
        }
    }
    Some(CapturedFrame {
        width: frame_width,
        height: frame_height,
        format: CapturedPixelFormat::Nv12,
        bytes,
    })
}

/// Two non-interleaved buffers: the layout `ScreenCaptureKit`'s float stereo audio is copied in.
#[repr(C)]
struct StereoBufferList {
    number_buffers: u32,
    buffers: [AudioBuffer; 2],
}

/// The audio side of a stream output: where samples go, and room to copy them.
struct AudioSink {
    writer: PcmSoundWriter,
    left: Vec<f32>,
    right: Vec<f32>,
    // Encodes what `writer` receives; stops with the stream.
    _encoder: PcmSoundEncoder,
}

impl AudioSink {
    /// Copies the sample's planar float stereo and writes it to the encoder's ring.
    fn take(&mut self, sample: &CMSampleBuffer) {
        // SAFETY: a valid sample buffer for this callback.
        let frames = usize::try_from(unsafe { sample.num_samples() }).unwrap_or(0);
        let mut offset = 0;
        while offset < frames {
            let count = (frames - offset).min(AUDIO_SCRATCH_FRAMES);
            let byte_size = u32::try_from(count * size_of::<f32>()).unwrap_or(0);
            let mut list = StereoBufferList {
                number_buffers: 2,
                buffers: [
                    AudioBuffer {
                        mNumberChannels: 1,
                        mDataByteSize: byte_size,
                        mData: self.left.as_mut_ptr().cast::<c_void>(),
                    },
                    AudioBuffer {
                        mNumberChannels: 1,
                        mDataByteSize: byte_size,
                        mData: self.right.as_mut_ptr().cast::<c_void>(),
                    },
                ],
            };
            let list_pointer = NonNull::from(&mut list).cast::<AudioBufferList>();
            // SAFETY: `list` describes two buffers of `count` floats each (the scratch holds
            // `AUDIO_SCRATCH_FRAMES`), in the layout `AudioBufferList` has with two buffers;
            // the call writes at most `mDataByteSize` bytes into each.
            let status = unsafe {
                sample.copy_pcm_data_into_audio_buffer_list(
                    i32::try_from(offset).unwrap_or(i32::MAX),
                    i32::try_from(count).unwrap_or(0),
                    list_pointer,
                )
            };
            if status != 0 {
                return;
            }
            self.writer
                .write_planar(&self.left[..count], &self.right[..count]);
            offset += count;
        }
    }
}

/// What the stream's callbacks need.
struct StreamState {
    pictures: VideoInput,
    /// Locked only by the stream's own serial queue (`try_lock`: never waits).
    audio: Option<Mutex<AudioSink>>,
    share: ScreenShareContext,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; the class adds no `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "MellowScreenStreamOutput"]
    #[ivars = StreamState]
    struct StreamOutput;

    // SAFETY: `NSObjectProtocol` has no extra requirements.
    unsafe impl NSObjectProtocol for StreamOutput {}

    // SAFETY: the method's signature matches the protocol's.
    unsafe impl SCStreamOutput for StreamOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn stream_did_output_sample_buffer_of_type(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            kind: SCStreamOutputType,
        ) {
            let state = self.ivars();
            if kind == SCStreamOutputType::Screen {
                let pictures = &state.pictures;
                if pictures.is_open()
                    && let Some(frame) = copy_nv12_picture(sample_buffer, pictures.recycled_bytes())
                {
                    pictures.push(frame);
                }
            } else if kind == SCStreamOutputType::Audio
                && let Some(audio) = &state.audio
                && let Ok(mut audio) = audio.try_lock()
            {
                audio.take(sample_buffer);
            }
        }
    }

    // SAFETY: the method's signature matches the protocol's.
    unsafe impl SCStreamDelegate for StreamOutput {
        #[unsafe(method(stream:didStopWithError:))]
        fn stream_did_stop_with_error(&self, _stream: &SCStream, error: &NSError) {
            // The person stopped it from the system's controls, the window closed, or it failed.
            let reason = if error.code() == SCStreamErrorCode::UserStopped.0 {
                ScreenShareEnd::SourceClosed
            } else {
                ScreenShareEnd::Failed(error.localizedDescription().to_string())
            };
            self.ivars().share.end(reason);
        }
    }
);

impl StreamOutput {
    fn new(state: StreamState) -> Retained<Self> {
        let this = Self::alloc().set_ivars(state);
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

struct ScreenCaptureKitCapture {
    stream: Retained<SCStream>,
    // Kept alive while the stream may still call it.
    _output: Retained<StreamOutput>,
    sound: ShareSoundState,
}

// SAFETY: the stream is only stopped from here (`stopCapture` may be called from any thread,
// ScreenCaptureKit's objects are thread-safe), and the output's state is `Send` and `Sync`.
unsafe impl Send for ScreenCaptureKitCapture {}

impl RunningScreenCapture for ScreenCaptureKitCapture {
    fn sound(&self) -> ShareSoundState {
        self.sound
    }
}

impl Drop for ScreenCaptureKitCapture {
    fn drop(&mut self) {
        // SAFETY: a running stream this capture owns; stopping needs no answer.
        unsafe { self.stream.stopCaptureWithCompletionHandler(None) };
    }
}

/// The filter and pixel size of the source `id`.
fn filter_of(
    content: &SCShareableContent,
    id: &str,
) -> Option<(Retained<SCContentFilter>, (u32, u32))> {
    if let Some(display_id) = id.strip_prefix(DISPLAY_ID_PREFIX) {
        // SAFETY: plain property reads of an immutable snapshot.
        let display = unsafe { content.displays() }
            .iter()
            .find(|display| unsafe { display.displayID() }.to_string() == display_id)?;
        // SAFETY: plain property reads.
        let size = unsafe { display.frame() }.size;
        let pixels = (points_to_pixels(size.width), points_to_pixels(size.height));
        return Some((display_filter(&display, content), pixels));
    }
    let window_id = id.strip_prefix(WINDOW_ID_PREFIX)?;
    let window = shareable_windows(content)
        .into_iter()
        .find(|window| unsafe { window.windowID() }.to_string() == window_id)?;
    // SAFETY: plain property reads.
    let size = unsafe { window.frame() }.size;
    Some((
        window_filter(&window),
        (points_to_pixels(size.width), points_to_pixels(size.height)),
    ))
}

impl ScreenSources for SystemScreenSources {
    fn picker(&self) -> ScreenPicker {
        if AnyClass::get(c"SCStream").is_some() {
            ScreenPicker::InApp
        } else {
            // Before macOS 12.3.
            ScreenPicker::Unavailable
        }
    }

    fn sources(
        &self,
        with_thumbnails: bool,
    ) -> Result<Vec<ScreenSourceDescription>, ScreenCaptureError> {
        if self.picker() == ScreenPicker::Unavailable {
            return Ok(Vec::new());
        }
        let content = shareable_content()?;
        let mut sources = Vec::new();
        // SAFETY: plain property reads of an immutable snapshot.
        for display in &unsafe { content.displays() } {
            let size = unsafe { display.frame() }.size;
            let pixels = (points_to_pixels(size.width), points_to_pixels(size.height));
            let thumbnail = with_thumbnails
                .then(|| thumbnail(&display_filter(&display, &content), pixels))
                .flatten();
            sources.push(ScreenSourceDescription {
                id: format!("{DISPLAY_ID_PREFIX}{}", unsafe { display.displayID() }),
                kind: ScreenSourceKind::Display,
                // The interface names displays ("Screen 1"); macOS gives no name here.
                title: String::new(),
                application_name: None,
                width: pixels.0,
                height: pixels.1,
                thumbnail,
            });
        }
        for window in shareable_windows(&content) {
            // SAFETY: plain property reads.
            let (size, title, application) = unsafe {
                (
                    window.frame().size,
                    window
                        .title()
                        .map(|title| title.to_string())
                        .unwrap_or_default(),
                    window
                        .owningApplication()
                        .map(|application| application.applicationName().to_string()),
                )
            };
            let pixels = (points_to_pixels(size.width), points_to_pixels(size.height));
            let thumbnail = with_thumbnails
                .then(|| thumbnail(&window_filter(&window), pixels))
                .flatten();
            sources.push(ScreenSourceDescription {
                id: format!("{WINDOW_ID_PREFIX}{}", unsafe { window.windowID() }),
                kind: ScreenSourceKind::Window,
                title,
                application_name: application,
                width: pixels.0,
                height: pixels.1,
                thumbnail,
            });
        }
        Ok(sources)
    }

    fn start(
        &self,
        choice: &ScreenChoice,
        sound: &ShareSound,
        share: ScreenShareContext,
    ) -> Result<Box<dyn RunningScreenCapture>, ScreenCaptureError> {
        let ScreenChoice::Source(id) = choice else {
            return Err(ScreenCaptureError::Unavailable);
        };
        let content = shareable_content()?;
        let (filter, pixels) = filter_of(&content, id).ok_or(ScreenCaptureError::SourceGone)?;
        let (width, height) = fit_within(pixels, share.quality().top_size());
        // SAFETY: a configuration made and set up here only.
        let configuration = unsafe { SCStreamConfiguration::new() };
        // Audio capture came with macOS 13.
        let can_capture_audio = configuration.respondsToSelector(sel!(setCapturesAudio:));
        let wants_sound = *sound == ShareSound::FromSource;
        let sound_state = match (wants_sound, can_capture_audio) {
            (true, true) => ShareSoundState::Captured,
            (false, _) if *sound == ShareSound::Off => ShareSoundState::Off,
            _ => ShareSoundState::NotAvailable,
        };
        // SAFETY: setters of a configuration only this function uses; the audio ones exist
        // (checked above).
        unsafe {
            configuration.setWidth(width as usize);
            configuration.setHeight(height as usize);
            configuration.setPixelFormat(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange);
            configuration.setMinimumFrameInterval(CMTime::new(
                1,
                i32::try_from(FRAMES_PER_SECOND).unwrap_or(30),
            ));
            configuration.setShowsCursor(true);
            configuration.setScalesToFit(true);
            configuration.setPreservesAspectRatio(true);
            if sound_state == ShareSoundState::Captured {
                configuration.setCapturesAudio(true);
                configuration.setExcludesCurrentProcessAudio(true);
                configuration
                    .setSampleRate(isize::try_from(SCREEN_SOUND_SAMPLE_RATE).unwrap_or(48_000));
                configuration.setChannelCount(SCREEN_SOUND_CHANNELS.cast_signed());
            }
        }
        // The share turns on now: pictures are refused until it is. Any failure below turns it
        // off again, which also ends the sound's encoder.
        let pictures = share.start_pictures();
        let started = start_sound(&share, sound_state).and_then(|audio| {
            let output = StreamOutput::new(StreamState {
                pictures,
                audio,
                share: share.clone(),
            });
            let stream = start_stream(&filter, &configuration, &output, sound_state)?;
            Ok((stream, output))
        });
        match started {
            Ok((stream, output)) => Ok(Box::new(ScreenCaptureKitCapture {
                stream,
                _output: output,
                sound: sound_state,
            })),
            Err(error) => {
                share.stop();
                Err(error)
            }
        }
    }
}

/// The sound's encoder and its scratch buffers, when the share captures sound.
fn start_sound(
    share: &ScreenShareContext,
    sound: ShareSoundState,
) -> Result<Option<Mutex<AudioSink>>, ScreenCaptureError> {
    if sound != ShareSoundState::Captured {
        return Ok(None);
    }
    let input = share.sound_input().ok_or(ScreenCaptureError::Unavailable)?;
    let (encoder, writer) = PcmSoundEncoder::start(input)
        .map_err(|error| ScreenCaptureError::System(error.to_string()))?;
    Ok(Some(Mutex::new(AudioSink {
        writer,
        left: vec![0.0; AUDIO_SCRATCH_FRAMES],
        right: vec![0.0; AUDIO_SCRATCH_FRAMES],
        _encoder: encoder,
    })))
}

/// Makes and starts the stream, waiting for the system's answer.
fn start_stream(
    filter: &SCContentFilter,
    configuration: &SCStreamConfiguration,
    output: &StreamOutput,
    sound: ShareSoundState,
) -> Result<Retained<SCStream>, ScreenCaptureError> {
    let delegate = ProtocolObject::<dyn SCStreamDelegate>::from_ref(output);
    let stream_output = ProtocolObject::<dyn SCStreamOutput>::from_ref(output);
    let queue = dispatch2::DispatchQueue::new("fr.mellow.screen", None);
    // SAFETY: valid filter, configuration and delegate; the output and queue outlive the
    // stream's use of them (the capture keeps the output, the stream retains the queue).
    let stream = unsafe {
        SCStream::initWithFilter_configuration_delegate(
            SCStream::alloc(),
            filter,
            configuration,
            Some(delegate),
        )
    };
    let mut kinds = vec![SCStreamOutputType::Screen];
    if sound == ShareSoundState::Captured {
        kinds.push(SCStreamOutputType::Audio);
    }
    for kind in kinds {
        // SAFETY: as above; one serial queue for both kinds, so callbacks never overlap.
        unsafe {
            stream.addStreamOutput_type_sampleHandlerQueue_error(stream_output, kind, Some(&queue))
        }
        .map_err(|error| error_of(&error))?;
    }
    let (answer, answered) = mpsc::channel();
    let handler = RcBlock::new(move |error: *mut NSError| {
        // SAFETY: a valid error or null, alive for the call.
        let result = unsafe { error.as_ref() }.map_or(Ok(()), |error| Err(error_of(error)));
        let _ = answer.send(result);
    });
    // SAFETY: a stream made above; the block is kept until called once.
    unsafe { stream.startCaptureWithCompletionHandler(Some(&handler)) };
    answered.recv_timeout(SYSTEM_ANSWER_TIMEOUT).map_err(|_| {
        ScreenCaptureError::System("the screen capture did not start in time".into())
    })??;
    Ok(stream)
}
