//! Video files on macOS through `AVFoundation` (`objc2` bindings, as the camera): an
//! `AVAssetReader` decodes the video track (H.264, HEVC, `ProRes`: whatever `VideoToolbox`
//! decodes) to BGRA already scaled to the shown size, in display order. `AVFoundation` reads
//! assets from files only, so the downloaded bytes go to a private temporary file for as long as
//! the video is open, removed when it closes. A seek starts a new reader at that time, as
//! `AVAssetReader` cannot move once started. The sound is read by the clip player
//! (`clip_decoder::open_mp4_sound_track`).

use std::fs::OpenOptions;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_av_foundation::{
    AVAssetReader, AVAssetReaderStatus, AVAssetReaderTrackOutput, AVAssetTrack, AVMediaTypeVideo,
    AVURLAsset,
};
use objc2_core_media::{CMTime, CMTimeRange, kCMTimePositiveInfinity};
use objc2_core_video::{
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
    CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress, kCVPixelBufferHeightKey,
    kCVPixelBufferPixelFormatTypeKey, kCVPixelBufferWidthKey, kCVPixelFormatType_32BGRA,
    kCVReturnSuccess,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};

use crate::clip_decoder::ClipError;
use crate::video_file::{
    MAXIMUM_SHOWN_VIDEO_SIZE, MAXIMUM_SOURCE_PIXELS, TimedPicture, VideoTrackDecoder,
    quarter_turns_from_counterclockwise_degrees, turned_bgra,
};
use crate::video_frame::{DecodedPixelFormat, DecodedVideoFrame, fit_within};

const BGRA_BYTES_PER_PIXEL: usize = 4;
/// Seek targets are expressed in microseconds.
const SEEK_TIMESCALE: i32 = 1_000_000;
/// Only the owner reads and writes the temporary copy.
const PRIVATE_FILE_MODE: u32 = 0o600;

/// A copy of the downloaded file for `AVFoundation`, removed on drop.
struct TemporaryFile(PathBuf);

impl TemporaryFile {
    fn write(bytes: &[u8]) -> std::io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        // QuickTime's own brand reads as a movie; anything else as MP4.
        let extension = if bytes.get(8..12) == Some(b"qt  ") {
            "mov"
        } else {
            "mp4"
        };
        let path = std::env::temp_dir().join(format!(
            "mellow-video-{}-{}.{extension}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(PRIVATE_FILE_MODE)
            .open(&path)?;
        let file = TemporaryFile(path).keep_open(&mut file, bytes)?;
        Ok(file)
    }

    fn keep_open(self, file: &mut std::fs::File, bytes: &[u8]) -> std::io::Result<Self> {
        // On error `self` drops and removes what was written.
        file.write_all(bytes)?;
        Ok(self)
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if std::fs::remove_file(&self.0).is_err() {
            tracing::warn!("cannot remove a video's temporary copy");
        }
    }
}

pub(crate) struct AssetVideoTrack {
    track: Retained<AVAssetTrack>,
    asset: Retained<AVURLAsset>,
    reader: Retained<AVAssetReader>,
    output: Retained<AVAssetReaderTrackOutput>,
    duration: Option<Duration>,
    /// The size pictures are decoded at, before turning them upright.
    decoded_size: (u32, u32),
    quarter_turns: u8,
    /// Last, so the file goes after the objects reading it.
    _file: TemporaryFile,
}

// SAFETY: the `AVFoundation` objects are created and used on the decoding thread only (the
// trait asks for `Send` because the decoder is boxed there); `AVAssetReader` and its outputs may
// be used from any one thread at a time.
unsafe impl Send for AssetVideoTrack {}

impl AssetVideoTrack {
    pub(crate) fn open(bytes: std::sync::Arc<[u8]>) -> Result<Self, ClipError> {
        let file = TemporaryFile::write(&bytes).map_err(|error| {
            tracing::warn!(%error, "cannot keep a video's temporary copy");
            ClipError::Unsupported
        })?;
        // `AVFoundation` reads the copy on disk: this reference to the memory goes now.
        drop(bytes);
        let path = NSString::from_str(&file.0.to_string_lossy());
        let url = NSURL::fileURLWithPath(&path);
        // SAFETY: a file URL and no options.
        let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) };
        // SAFETY: a static `AVFoundation` constant, set for the process.
        let media_type = unsafe { AVMediaTypeVideo }.ok_or(ClipError::Unsupported)?;
        // The synchronous accessor is fine on the decoding thread; it reads the file's header.
        #[allow(deprecated)]
        // SAFETY: a valid asset and media type.
        let track = unsafe { asset.tracksWithMediaType(media_type) }
            .firstObject()
            .ok_or(ClipError::Unsupported)?;
        // SAFETY: a valid track; plain value getters.
        let (natural_size, transform) =
            unsafe { (track.naturalSize(), track.preferredTransform()) };
        let is_within_limit = natural_size.width >= 1.0
            && natural_size.height >= 1.0
            && natural_size.width * natural_size.height
                <= f64::from(u32::try_from(MAXIMUM_SOURCE_PIXELS).unwrap_or(u32::MAX));
        if !is_within_limit {
            return Err(ClipError::Unsupported);
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // Checked just above: at least 1 and at most a few thousand pixels a side.
        let source_size = (
            natural_size.width.round() as u32,
            natural_size.height.round() as u32,
        );
        let decoded_size = fit_within(source_size, MAXIMUM_SHOWN_VIDEO_SIZE);
        // A rotation by θ is (cos θ, sin θ, −sin θ, cos θ) in the video's top-down
        // coordinates: clockwise on screen, hence the minus for the counterclockwise helper.
        let clockwise_degrees = transform.b.atan2(transform.a).to_degrees();
        let quarter_turns = quarter_turns_from_counterclockwise_degrees(-clockwise_degrees);
        // SAFETY: a valid asset.
        let duration = seconds(unsafe { asset.duration() });
        let (reader, output) = start_reader(&asset, &track, decoded_size, Duration::ZERO)?;
        Ok(Self {
            track,
            asset,
            reader,
            output,
            duration,
            decoded_size,
            quarter_turns,
            _file: file,
        })
    }
}

/// A time in seconds as a duration, when it is a real, non-negative number.
fn seconds(time: CMTime) -> Option<Duration> {
    // SAFETY: a plain conversion of a value.
    let seconds = unsafe { time.seconds() };
    (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds))
}

fn start_reader(
    asset: &AVURLAsset,
    track: &AVAssetTrack,
    (width, height): (u32, u32),
    from: Duration,
) -> Result<(Retained<AVAssetReader>, Retained<AVAssetReaderTrackOutput>), ClipError> {
    // SAFETY: a valid asset.
    let reader = unsafe { AVAssetReader::assetReaderWithAsset_error(asset) }
        .map_err(|_| ClipError::Corrupt)?;
    // The CoreVideo keys are CFStrings, toll-free bridged to NSString.
    let key = |key: &'static objc2_core_foundation::CFString| -> &'static NSString {
        // SAFETY: the same object viewed as its Foundation type; a constant for the process.
        unsafe { &*std::ptr::from_ref(key).cast::<NSString>() }
    };
    let pixel_format = NSNumber::numberWithUnsignedInt(kCVPixelFormatType_32BGRA);
    let width = NSNumber::numberWithUnsignedInt(width);
    let height = NSNumber::numberWithUnsignedInt(height);
    // SAFETY: CoreVideo constants, valid for the process.
    let keys = unsafe {
        [
            key(kCVPixelBufferPixelFormatTypeKey),
            key(kCVPixelBufferWidthKey),
            key(kCVPixelBufferHeightKey),
        ]
    };
    let values: [&AnyObject; 3] = [&pixel_format, &width, &height];
    let settings = NSDictionary::from_slices(&keys, &values);
    // SAFETY: a track of `asset` and uncompressed BGRA output settings.
    let output = unsafe {
        AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(
            track,
            Some(&settings),
        )
    };
    // SAFETY: valid objects; the pictures are read before the next sample is asked for.
    unsafe {
        output.setAlwaysCopiesSampleData(false);
        if !reader.canAddOutput(&output) {
            return Err(ClipError::Unsupported);
        }
        reader.addOutput(&output);
        if !from.is_zero() {
            reader.setTimeRange(CMTimeRange {
                start: CMTime::with_seconds(from.as_secs_f64(), SEEK_TIMESCALE),
                duration: kCMTimePositiveInfinity,
            });
        }
        if !reader.startReading() {
            return Err(ClipError::Unsupported);
        }
    }
    Ok((reader, output))
}

impl VideoTrackDecoder for AssetVideoTrack {
    fn duration(&self) -> Option<Duration> {
        self.duration
    }

    fn next_picture(&mut self) -> Result<Option<TimedPicture>, ClipError> {
        loop {
            // SAFETY: a started reader's output.
            let Some(sample) = (unsafe { self.output.copyNextSampleBuffer() }) else {
                // SAFETY: a valid reader.
                return match unsafe { self.reader.status() } {
                    AVAssetReaderStatus::Completed => Ok(None),
                    // A codec `VideoToolbox` lacks fails the reader at its first sample.
                    _ => Err(ClipError::Unsupported),
                };
            };
            // SAFETY: a valid sample; plain getters.
            let (time, image) =
                unsafe { (sample.presentation_time_stamp(), sample.image_buffer()) };
            // Samples without a picture (an empty edit) are skipped.
            let (Some(time), Some(image)) = (seconds(time), image) else {
                continue;
            };
            let Some(frame) = copy_bgra(&image) else {
                return Err(ClipError::Corrupt);
            };
            return Ok(Some(TimedPicture {
                time,
                frame: turned_bgra(frame, self.quarter_turns),
            }));
        }
    }

    fn seek(&mut self, position: Duration) -> Result<(), ClipError> {
        // SAFETY: a valid reader; cancelling a finished one is harmless.
        unsafe { self.reader.cancelReading() };
        let (reader, output) = start_reader(&self.asset, &self.track, self.decoded_size, position)?;
        self.reader = reader;
        self.output = output;
        Ok(())
    }
}

/// The BGRA picture's rows, packed. `None` for another layout.
fn copy_bgra(image: &objc2_core_video::CVImageBuffer) -> Option<DecodedVideoFrame> {
    if CVPixelBufferGetPixelFormatType(image) != kCVPixelFormatType_32BGRA {
        return None;
    }
    // SAFETY: a valid pixel buffer, unlocked below with the same flags.
    if unsafe { CVPixelBufferLockBaseAddress(image, CVPixelBufferLockFlags::ReadOnly) }
        != kCVReturnSuccess
    {
        return None;
    }
    let width = CVPixelBufferGetWidth(image);
    let height = CVPixelBufferGetHeight(image);
    let stride = CVPixelBufferGetBytesPerRow(image);
    let base = CVPixelBufferGetBaseAddress(image).cast::<u8>();
    let row_bytes = width * BGRA_BYTES_PER_PIXEL;
    let frame = (!base.is_null() && stride >= row_bytes && width > 0 && height > 0)
        .then(|| {
            // SAFETY: the locked buffer holds `height` rows of `stride` bytes, as it reports.
            let rows = unsafe { std::slice::from_raw_parts(base, stride.checked_mul(height)?) };
            let mut pixels = Vec::with_capacity(row_bytes * height);
            for row in rows.chunks(stride) {
                pixels.extend_from_slice(&row[..row_bytes]);
            }
            Some(DecodedVideoFrame {
                width: u32::try_from(width).ok()?,
                height: u32::try_from(height).ok()?,
                format: DecodedPixelFormat::Bgra,
                pixels,
            })
        })
        .flatten();
    // SAFETY: locked above with the same flags.
    unsafe { CVPixelBufferUnlockBaseAddress(image, CVPixelBufferLockFlags::ReadOnly) };
    frame
}
