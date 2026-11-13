//! Videos compressed before they upload, as the web client does (ADR-0046, ported from
//! `web/src/lib/video-compression.ts`): the same pure decisions (size box, bits per pixel, the
//! instance's limit, what is only remuxed and what is kept), carried out by the system's own
//! media stack instead of `WebCodecs`: `AVFoundation` and `VideoToolbox` on macOS, the system FFmpeg on
//! Linux (VA-API's `h264_vaapi`, else a software H.264 encoder it was built with). The output is
//! what the web sends: H.264 with AAC (or Opus) in a faststart MP4 without the source's tags.
//! Windows sends the original for now (Media Foundation comes with its own change).
//!
//! Pictures and audio go as they are, as on the web: the instance compresses them (ADR-0033,
//! ADR-0045).

// Windows has no backend yet: the decisions are only reached by the tests there.
#![cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Output fits a 1920 × 1080 box in the video's own orientation: portrait clips stay 1080 wide.
pub(crate) const MAXIMUM_OUTPUT_SHORT_SIDE: f64 = 1_080.0;
pub(crate) const MAXIMUM_OUTPUT_LONG_SIDE: f64 = 1_920.0;
/// Faster sources are brought down to this; slower ones keep their rate.
pub(crate) const MAXIMUM_OUTPUT_FRAME_RATE: f64 = 60.0;
/// H.264 bits per pixel per frame aimed at (ADR-0046's measurements).
pub(crate) const TARGET_BITS_PER_PIXEL: f64 = 0.1;
/// Below this, squeezing a long clip under the instance's limit would look smeared.
pub(crate) const MINIMUM_BITS_PER_PIXEL: f64 = 0.05;
/// A source at most this much above the target is only remuxed.
pub(crate) const REMUX_SOURCE_BITRATE_RATIO: f64 = 1.25;
/// A codec other than H.264 is made H.264 at no more than this times its rate.
pub(crate) const COMPATIBILITY_BITRATE_RATIO: f64 = 1.5;
/// A transcode is kept only when it saves at least this share of the original's bytes.
pub(crate) const MINIMUM_TRANSCODE_SAVING: f64 = 0.2;
/// Room left under the instance's limit for container overhead and VBR overshoot.
pub(crate) const LIMIT_BUDGET_SHARE: f64 = 0.9;
pub(crate) const AAC_BITRATE: u32 = 128_000;
pub(crate) const OPUS_BITRATE: u32 = 96_000;
pub(crate) const KEY_FRAME_INTERVAL_SECONDS: f64 = 2.0;
/// How far past its measured duration a file may run before compression gives up: the plan's
/// bitrate comes from that duration, so a file lying about its length would otherwise write an
/// output as long as it really is.
const MAXIMUM_DURATION_OVERRUN_RATIO: f64 = 1.5;
/// Slack for very short clips, whose last frame may end a little after the stated duration.
const DURATION_OVERRUN_SLACK_SECONDS: f64 = 1.0;

/// Whether a sample at `seconds` lies too far past the measured `duration_seconds`.
pub(crate) fn runs_past_measured_duration(seconds: f64, duration_seconds: f64) -> bool {
    seconds > duration_seconds * MAXIMUM_DURATION_OVERRUN_RATIO + DURATION_OVERRUN_SLACK_SECONDS
}

/// The audio codecs MP4 carries that every browser plays: copied as they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceAudioCodec {
    Aac,
    // macOS encodes Opus sources to AAC: `AVAssetWriter` does not copy Opus into MP4.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Opus,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioEncoderCodec {
    Aac,
    // Only an FFmpeg built without its AAC encoder falls back to Opus.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Opus,
}

/// What the backend measured of the file's primary tracks.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VideoSource {
    pub byte_size: u64,
    pub duration_seconds: f64,
    /// After rotation and pixel aspect ratio, as the reader sees it.
    pub display_width: u32,
    pub display_height: u32,
    pub frame_rate: f64,
    pub is_h264: bool,
    pub can_decode_video: bool,
    pub is_high_dynamic_range: bool,
    /// `None` without an audio track.
    pub audio_codec: Option<SourceAudioCodec>,
}

/// What this system can encode, asked once per file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EncoderSupport {
    pub can_encode_h264: bool,
    /// AAC where the system has it, else Opus; `None` when neither.
    pub audio_codec: Option<AudioEncoderCodec>,
}

/// Why the original goes as it is; the web's reasons, for logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepOriginalReason {
    NoVideoTrack,
    UndecodableVideo,
    UnknownDuration,
    UnknownFrameRate,
    HighDynamicRange,
    NoH264Encoder,
    NoAudioEncoder,
    TooLongForLimit,
    NotSmaller,
    /// The system cannot compress videos (Windows for now).
    Unsupported,
    /// Reading, encoding or writing failed: logged, never lost.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AudioPlan {
    None,
    Copy,
    Encode {
        codec: AudioEncoderCodec,
        bitrate: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum VideoCompressionPlan {
    KeepOriginal(KeepOriginalReason),
    /// Packets copied into a faststart MP4 without the source's metadata tags.
    Remux {
        audio: AudioPlan,
    },
    Transcode(TranscodePlan),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TranscodePlan {
    pub width: u32,
    pub height: u32,
    /// `None` keeps the source's rate.
    pub frame_rate: Option<f64>,
    pub video_bitrate: u64,
    pub audio: AudioPlan,
    /// The source's codec does not play in every browser: H.264 is worth it even without a saving.
    pub is_for_compatibility: bool,
}

/// Scaled down to fit the 1080p box, never up; even sides, as H.264 encoders want.
pub(crate) fn output_dimensions(width: u32, height: u32) -> (u32, u32) {
    let (width, height) = (f64::from(width), f64::from(height));
    let scale = 1.0_f64
        .min(MAXIMUM_OUTPUT_SHORT_SIDE / width.min(height))
        .min(MAXIMUM_OUTPUT_LONG_SIDE / width.max(height));
    (even_at_most(width * scale), even_at_most(height * scale))
}

fn even_at_most(length: f64) -> u32 {
    // At most the source's side, a u32.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let even = ((length / 2.0).floor() * 2.0) as u32;
    even.max(2)
}

fn bitrate_at(bits_per_pixel: f64, width: u32, height: u32, frame_rate: f64) -> f64 {
    (bits_per_pixel * f64::from(width) * f64::from(height) * frame_rate).round()
}

fn audio_plan_for(source: Option<SourceAudioCodec>, support: EncoderSupport) -> Option<AudioPlan> {
    match source {
        None => Some(AudioPlan::None),
        Some(SourceAudioCodec::Aac | SourceAudioCodec::Opus) => Some(AudioPlan::Copy),
        Some(SourceAudioCodec::Other) => support.audio_codec.map(|codec| AudioPlan::Encode {
            codec,
            bitrate: match codec {
                AudioEncoderCodec::Aac => AAC_BITRATE,
                AudioEncoderCodec::Opus => OPUS_BITRATE,
            },
        }),
    }
}

fn audio_bitrate_of(audio: AudioPlan, source_total_bitrate: f64) -> f64 {
    match audio {
        AudioPlan::None => 0.0,
        // Unknown without reading its packets; AAC's rate is a fair guess and only sizes the budget.
        AudioPlan::Copy => f64::from(AAC_BITRATE).min(source_total_bitrate),
        AudioPlan::Encode { bitrate, .. } => f64::from(bitrate),
    }
}

/// Decides once per file, as the web's `planVideoCompression`: transcode to H.264 at the
/// bits-per-pixel target (and within the instance's limit when it is known), remux an H.264
/// source already near the target and within the limit, or keep the original.
pub(crate) fn plan_video_compression(
    source: &VideoSource,
    support: EncoderSupport,
    maximum_attachment_bytes: Option<u64>,
) -> VideoCompressionPlan {
    use KeepOriginalReason as Keep;
    if !source.can_decode_video {
        return VideoCompressionPlan::KeepOriginal(Keep::UndecodableVideo);
    }
    if source.duration_seconds.is_nan() || source.duration_seconds <= 0.0 {
        return VideoCompressionPlan::KeepOriginal(Keep::UnknownDuration);
    }
    if !(source.frame_rate.is_finite() && source.frame_rate > 0.0) {
        return VideoCompressionPlan::KeepOriginal(Keep::UnknownFrameRate);
    }
    // Tone mapping to SDR H.264 washes HDR out; kept until the ADR handles it.
    if source.is_high_dynamic_range {
        return VideoCompressionPlan::KeepOriginal(Keep::HighDynamicRange);
    }
    let Some(audio) = audio_plan_for(source.audio_codec, support) else {
        return VideoCompressionPlan::KeepOriginal(Keep::NoAudioEncoder);
    };
    let (width, height) = output_dimensions(source.display_width, source.display_height);
    let frame_rate = source.frame_rate.min(MAXIMUM_OUTPUT_FRAME_RATE);
    // Byte counts of real files are far below 2^52.
    #[allow(clippy::cast_precision_loss)]
    let byte_size = source.byte_size as f64;
    let source_total_bitrate = byte_size * 8.0 / source.duration_seconds;
    let audio_bitrate = audio_bitrate_of(audio, source_total_bitrate);
    let target_video_bitrate = bitrate_at(TARGET_BITS_PER_PIXEL, width, height, frame_rate);
    let is_resized = width < even_at_most(f64::from(source.display_width))
        || height < even_at_most(f64::from(source.display_height));
    // Only H.264 is copied: the MP4 must play in Safari and Firefox too.
    let is_for_compatibility = !source.is_h264;
    let is_near_target = !is_resized
        && source.frame_rate <= MAXIMUM_OUTPUT_FRAME_RATE
        && source_total_bitrate
            <= (target_video_bitrate + audio_bitrate) * REMUX_SOURCE_BITRATE_RATIO;
    let is_within_limit =
        maximum_attachment_bytes.is_none_or(|maximum| source.byte_size <= maximum);
    if is_near_target && !is_for_compatibility && is_within_limit {
        return VideoCompressionPlan::Remux { audio };
    }
    if !support.can_encode_h264 {
        return VideoCompressionPlan::KeepOriginal(Keep::NoH264Encoder);
    }
    #[allow(clippy::cast_precision_loss)]
    let limit_video_bitrate = maximum_attachment_bytes.map_or(f64::INFINITY, |maximum| {
        maximum as f64 * 8.0 * LIMIT_BUDGET_SHARE / source.duration_seconds - audio_bitrate
    });
    if target_video_bitrate.min(limit_video_bitrate)
        < bitrate_at(MINIMUM_BITS_PER_PIXEL, width, height, frame_rate)
    {
        return VideoCompressionPlan::KeepOriginal(Keep::TooLongForLimit);
    }
    // A low-rate source is not inflated to the target: its picture has no more detail to keep.
    let compatibility_video_bitrate = if is_for_compatibility {
        source_total_bitrate * COMPATIBILITY_BITRATE_RATIO
    } else {
        f64::INFINITY
    };
    // Positive and far below u64::MAX: the floor check above passed.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let video_bitrate = target_video_bitrate
        .min(limit_video_bitrate)
        .min(compatibility_video_bitrate)
        .floor() as u64;
    VideoCompressionPlan::Transcode(TranscodePlan {
        width,
        height,
        frame_rate: (source.frame_rate > MAXIMUM_OUTPUT_FRAME_RATE)
            .then_some(MAXIMUM_OUTPUT_FRAME_RATE),
        video_bitrate,
        audio,
        is_for_compatibility,
    })
}

/// An original over the limit would be refused: any output that fits goes. Otherwise a transcode
/// must save a fifth to be worth its generation loss, unless it is for compatibility, which only
/// must fit; a remux only must not grow. `is_for_compatibility` is `None` for a remux.
pub(crate) fn is_output_worth_sending(
    transcode_for_compatibility: Option<bool>,
    original_bytes: u64,
    output_bytes: u64,
    maximum_attachment_bytes: Option<u64>,
) -> bool {
    let fits_limit = maximum_attachment_bytes.is_none_or(|maximum| output_bytes <= maximum);
    if maximum_attachment_bytes.is_some_and(|maximum| original_bytes > maximum) {
        return fits_limit;
    }
    match transcode_for_compatibility {
        None => output_bytes <= original_bytes,
        Some(true) => fits_limit,
        // Byte counts of real files are far below 2^52.
        #[allow(clippy::cast_precision_loss)]
        Some(false) => {
            output_bytes as f64 <= original_bytes as f64 * (1.0 - MINIMUM_TRANSCODE_SAVING)
        }
    }
}

/// The compressed file's name: the original's, with the MP4 extension it now has.
#[must_use]
pub fn compressed_video_file_name(original_name: &str) -> String {
    let stem = match original_name.rfind('.') {
        Some(dot) if dot > 0 => &original_name[..dot],
        _ => original_name,
    };
    format!("{stem}.mp4")
}

/// File extensions a browser types `video/*`: the files the web hands its compression. Others
/// (and those the system cannot read) go as they are.
const VIDEO_FILE_EXTENSIONS: [&str; 16] = [
    "3g2", "3gp", "avi", "m2ts", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "mts", "ogv", "qt",
    "ts", "webm", "wmv",
];

/// Whether `file_name` is a video the composer compresses before upload.
#[must_use]
pub fn is_compressible_video_file_name(file_name: &str) -> bool {
    Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            VIDEO_FILE_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}

/// Whether this system compresses videos at all; Windows sends originals for now.
#[must_use]
pub const fn can_compress_videos_here() -> bool {
    cfg!(any(target_os = "macos", target_os = "linux"))
}

/// What became of a video handed to [`compress_video`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoCompressionOutcome {
    /// A faststart MP4 to upload in the original's place, named by [`compressed_video_file_name`].
    Compressed(Vec<u8>),
    /// The original goes as it is.
    KeptOriginal(KeepOriginalReason),
}

/// Compression stopped because [`compress_video`]'s flag was set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the video compression was cancelled")]
pub struct VideoCompressionCancelled;

/// A backend's failure: measured, planned and carried out by the system's media stack.
#[derive(Debug, thiserror::Error)]
pub(crate) enum BackendError {
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Failed(String),
}

/// What a backend measured of the file, or `None` without a video track.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) struct MeasuredVideo {
    pub source: VideoSource,
    pub support: EncoderSupport,
}

/// The progress callback a backend calls, from 0 to 1, as often as it likes.
pub(crate) type ProgressCallback<'a> = &'a mut dyn FnMut(f64);

/// Compresses the video at `path` for an instance accepting `maximum_attachment_bytes` per file
/// (`Ready.maximum_attachment_bytes`; `None` when unknown), as the web's Worker does. Blocks for
/// as long as the encode takes: run it on a thread of its own. `on_progress` gets whole percents,
/// from 0 to 1. Setting `is_cancelled` stops the work at the next frame.
///
/// Never fails short of a cancel: a file the system cannot read or encode, or a failure on the
/// way, keeps the original (logged without the file's name).
///
/// # Errors
/// [`VideoCompressionCancelled`] once `is_cancelled` is set.
pub fn compress_video(
    path: &Path,
    maximum_attachment_bytes: Option<u64>,
    on_progress: &mut dyn FnMut(f64),
    is_cancelled: &AtomicBool,
) -> Result<VideoCompressionOutcome, VideoCompressionCancelled> {
    if is_cancelled.load(Ordering::Relaxed) {
        return Err(VideoCompressionCancelled);
    }
    // The encoders report every frame; the tray shows whole percents, so the interface gets no
    // more. Only increases: packets come in decode order (B-frames) and a failed VA-API attempt
    // restarts the next encoder at 0.
    let mut posted_percent = None;
    let mut whole_percents = |fraction: f64| {
        // From 0 to 100.
        #[allow(clippy::cast_possible_truncation)]
        let percent = (fraction.clamp(0.0, 1.0) * 100.0).floor() as i32;
        if posted_percent.is_none_or(|posted| percent > posted) {
            posted_percent = Some(percent);
            on_progress(fraction.clamp(0.0, 1.0));
        }
    };
    match run_backend(
        path,
        maximum_attachment_bytes,
        &mut whole_percents,
        is_cancelled,
    ) {
        Ok(outcome) => Ok(outcome),
        Err(_) if is_cancelled.load(Ordering::Relaxed) => Err(VideoCompressionCancelled),
        Err(BackendError::Cancelled) => Err(VideoCompressionCancelled),
        Err(BackendError::Failed(error)) => {
            tracing::warn!(%error, "compressing a video failed; the original is sent");
            Ok(VideoCompressionOutcome::KeptOriginal(
                KeepOriginalReason::Failed,
            ))
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn run_backend(
    _path: &Path,
    _maximum_attachment_bytes: Option<u64>,
    _on_progress: ProgressCallback<'_>,
    _is_cancelled: &AtomicBool,
) -> Result<VideoCompressionOutcome, BackendError> {
    Ok(VideoCompressionOutcome::KeptOriginal(
        KeepOriginalReason::Unsupported,
    ))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run_backend(
    path: &Path,
    maximum_attachment_bytes: Option<u64>,
    on_progress: ProgressCallback<'_>,
    is_cancelled: &AtomicBool,
) -> Result<VideoCompressionOutcome, BackendError> {
    #[cfg(target_os = "linux")]
    use crate::video_compression_linux as backend;
    #[cfg(target_os = "macos")]
    use crate::video_compression_macos as backend;

    let byte_size = std::fs::metadata(path)
        .map_err(|error| {
            BackendError::Failed(format!("reading the file's size: {}", error.kind()))
        })?
        .len();
    let Some(measured) = backend::measure(path, byte_size, is_cancelled)? else {
        return Ok(VideoCompressionOutcome::KeptOriginal(
            KeepOriginalReason::NoVideoTrack,
        ));
    };
    let plan = plan_video_compression(&measured.source, measured.support, maximum_attachment_bytes);
    let transcode_for_compatibility = match plan {
        VideoCompressionPlan::KeepOriginal(reason) => {
            return Ok(VideoCompressionOutcome::KeptOriginal(reason));
        }
        VideoCompressionPlan::Remux { .. } => None,
        VideoCompressionPlan::Transcode(transcode) => Some(transcode.is_for_compatibility),
    };
    // A directory of its own: AVAssetWriter refuses to write over an existing file.
    let directory = tempfile::tempdir().map_err(|error| {
        BackendError::Failed(format!("a temporary directory: {}", error.kind()))
    })?;
    let output_path = directory.path().join("compressed.mp4");
    backend::execute(
        path,
        &measured,
        &plan,
        &output_path,
        on_progress,
        is_cancelled,
    )?;
    let output_bytes = std::fs::metadata(&output_path)
        .map_err(|error| {
            BackendError::Failed(format!("reading the output's size: {}", error.kind()))
        })?
        .len();
    if !is_output_worth_sending(
        transcode_for_compatibility,
        byte_size,
        output_bytes,
        maximum_attachment_bytes,
    ) {
        return Ok(VideoCompressionOutcome::KeptOriginal(
            KeepOriginalReason::NotSmaller,
        ));
    }
    // Bounded: no larger than the original or the instance's limit, as checked just above.
    std::fs::read(&output_path)
        .map(VideoCompressionOutcome::Compressed)
        .map_err(|error| BackendError::Failed(format!("reading the output: {}", error.kind())))
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss)] // Test sizes are small.
mod tests {
    use super::*;

    const MEBIBYTE: u64 = 1024 * 1024;
    const FULL_SUPPORT: EncoderSupport = EncoderSupport {
        can_encode_h264: true,
        audio_codec: Some(AudioEncoderCodec::Aac),
    };

    /// A phone clip: 10 s of 4K H.264 at 30 fps with AAC, about 48 Mbit/s.
    fn phone_clip() -> VideoSource {
        VideoSource {
            byte_size: 60 * MEBIBYTE,
            duration_seconds: 10.0,
            display_width: 3840,
            display_height: 2160,
            frame_rate: 30.0,
            is_h264: true,
            can_decode_video: true,
            is_high_dynamic_range: false,
            audio_codec: Some(SourceAudioCodec::Aac),
        }
    }

    fn transcode(plan: VideoCompressionPlan) -> TranscodePlan {
        match plan {
            VideoCompressionPlan::Transcode(transcode) => transcode,
            other => panic!("expected a transcode, got {other:?}"),
        }
    }

    #[test]
    fn output_dimensions_fit_the_1080p_box_never_upscaled_with_even_sides() {
        assert_eq!(output_dimensions(3840, 2160), (1920, 1080));
        assert_eq!(output_dimensions(2160, 3840), (1080, 1920));
        assert_eq!(output_dimensions(640, 360), (640, 360));
        assert_eq!(output_dimensions(5120, 1440), (1920, 540));
        assert_eq!(output_dimensions(3001, 1001), (1920, 640));
    }

    #[test]
    fn transcodes_a_4k_phone_clip_to_1080p_h264_at_the_bits_per_pixel_target() {
        assert_eq!(
            plan_video_compression(&phone_clip(), FULL_SUPPORT, None),
            VideoCompressionPlan::Transcode(TranscodePlan {
                width: 1920,
                height: 1080,
                frame_rate: None,
                video_bitrate: 6_220_800,
                audio: AudioPlan::Copy,
                is_for_compatibility: false,
            })
        );
    }

    #[test]
    fn brings_a_120_fps_clip_down_to_60_and_keeps_60_fps_clips_as_they_are() {
        let fast = VideoSource {
            frame_rate: 120.0,
            ..phone_clip()
        };
        assert_eq!(
            transcode(plan_video_compression(&fast, FULL_SUPPORT, None)).frame_rate,
            Some(MAXIMUM_OUTPUT_FRAME_RATE)
        );
        let sixty = VideoSource {
            frame_rate: 60.0,
            ..phone_clip()
        };
        assert_eq!(
            transcode(plan_video_compression(&sixty, FULL_SUPPORT, None)).frame_rate,
            None
        );
    }

    #[test]
    fn keeps_hdr_sources_undecodable_video_and_files_without_a_duration() {
        let keep = |source: VideoSource| plan_video_compression(&source, FULL_SUPPORT, None);
        assert_eq!(
            keep(VideoSource {
                is_high_dynamic_range: true,
                ..phone_clip()
            }),
            VideoCompressionPlan::KeepOriginal(KeepOriginalReason::HighDynamicRange)
        );
        assert_eq!(
            keep(VideoSource {
                can_decode_video: false,
                ..phone_clip()
            }),
            VideoCompressionPlan::KeepOriginal(KeepOriginalReason::UndecodableVideo)
        );
        assert_eq!(
            keep(VideoSource {
                duration_seconds: 0.0,
                ..phone_clip()
            }),
            VideoCompressionPlan::KeepOriginal(KeepOriginalReason::UnknownDuration)
        );
    }

    #[test]
    fn keeps_the_original_without_an_h264_encoder() {
        let support = EncoderSupport {
            can_encode_h264: false,
            audio_codec: Some(AudioEncoderCodec::Aac),
        };
        assert_eq!(
            plan_video_compression(&phone_clip(), support, None),
            VideoCompressionPlan::KeepOriginal(KeepOriginalReason::NoH264Encoder)
        );
    }

    #[test]
    fn encodes_other_audio_to_aac_or_opus_where_there_is_no_aac_encoder() {
        let pcm = VideoSource {
            audio_codec: Some(SourceAudioCodec::Other),
            ..phone_clip()
        };
        assert_eq!(
            transcode(plan_video_compression(&pcm, FULL_SUPPORT, None)).audio,
            AudioPlan::Encode {
                codec: AudioEncoderCodec::Aac,
                bitrate: AAC_BITRATE
            }
        );
        let opus_only = EncoderSupport {
            can_encode_h264: true,
            audio_codec: Some(AudioEncoderCodec::Opus),
        };
        assert_eq!(
            transcode(plan_video_compression(&pcm, opus_only, None)).audio,
            AudioPlan::Encode {
                codec: AudioEncoderCodec::Opus,
                bitrate: OPUS_BITRATE
            }
        );
        let no_audio_encoder = EncoderSupport {
            can_encode_h264: true,
            audio_codec: None,
        };
        assert_eq!(
            plan_video_compression(&pcm, no_audio_encoder, None),
            VideoCompressionPlan::KeepOriginal(KeepOriginalReason::NoAudioEncoder)
        );
        let silent = VideoSource {
            audio_codec: None,
            ..phone_clip()
        };
        assert_eq!(
            transcode(plan_video_compression(&silent, FULL_SUPPORT, None)).audio,
            AudioPlan::None
        );
    }

    #[test]
    fn only_remuxes_an_h264_source_already_near_the_target() {
        // 1080p30 at 4.2 Mbit/s: under 1.25 × (6.2 Mbit/s + AAC).
        let small = VideoSource {
            display_width: 1920,
            display_height: 1080,
            byte_size: 5 * MEBIBYTE,
            ..phone_clip()
        };
        assert_eq!(
            plan_video_compression(&small, FULL_SUPPORT, None),
            VideoCompressionPlan::Remux {
                audio: AudioPlan::Copy
            }
        );
        // Remuxing needs no encoder.
        let nothing = EncoderSupport {
            can_encode_h264: false,
            audio_codec: None,
        };
        assert!(matches!(
            plan_video_compression(&small, nothing, None),
            VideoCompressionPlan::Remux { .. }
        ));
    }

    #[test]
    fn lowers_the_bitrate_to_fit_the_instances_limit_down_to_the_quality_floor() {
        let limit = 25 * MEBIBYTE;
        // 45 s at the target would be about 35 MB.
        let forty_five_seconds = VideoSource {
            duration_seconds: 45.0,
            byte_size: 270 * MEBIBYTE,
            ..phone_clip()
        };
        let plan = transcode(plan_video_compression(
            &forty_five_seconds,
            FULL_SUPPORT,
            Some(limit),
        ));
        assert!(plan.video_bitrate < 6_220_800);
        assert!((plan.video_bitrate + u64::from(AAC_BITRATE)) * 45 / 8 <= limit);
        // Ten minutes would need a bitrate too low to watch.
        let long = VideoSource {
            duration_seconds: 600.0,
            byte_size: 3600 * MEBIBYTE,
            ..phone_clip()
        };
        assert_eq!(
            plan_video_compression(&long, FULL_SUPPORT, Some(limit)),
            VideoCompressionPlan::KeepOriginal(KeepOriginalReason::TooLongForLimit)
        );
    }

    #[test]
    fn turns_a_small_sdr_hevc_clip_into_h264_at_most_half_again_its_rate() {
        // An iPhone's 1080p30 HEVC: 10 s at about 5.2 Mbit/s, near the H.264 target.
        let hevc = VideoSource {
            display_width: 1920,
            display_height: 1080,
            byte_size: 6_501_171,
            is_h264: false,
            ..phone_clip()
        };
        let plan = transcode(plan_video_compression(&hevc, FULL_SUPPORT, None));
        assert_eq!((plan.width, plan.height), (1920, 1080));
        assert!(plan.is_for_compatibility);
        assert!(
            plan.video_bitrate as f64 <= 6_501_171.0 * 8.0 / 10.0 * COMPATIBILITY_BITRATE_RATIO
        );
        let no_encoder = EncoderSupport {
            can_encode_h264: false,
            audio_codec: Some(AudioEncoderCodec::Aac),
        };
        assert_eq!(
            plan_video_compression(&hevc, no_encoder, None),
            VideoCompressionPlan::KeepOriginal(KeepOriginalReason::NoH264Encoder)
        );
    }

    #[test]
    fn does_not_spend_the_targets_rate_on_a_tiny_vp9_clip() {
        let tiny = VideoSource {
            display_width: 640,
            display_height: 360,
            byte_size: 250_000,
            is_h264: false,
            ..phone_clip()
        };
        let plan = transcode(plan_video_compression(&tiny, FULL_SUPPORT, None));
        assert!(plan.is_for_compatibility);
        assert!(plan.video_bitrate as f64 <= 250_000.0 * 8.0 / 10.0 * COMPATIBILITY_BITRATE_RATIO);
    }

    #[test]
    fn transcodes_a_near_target_h264_clip_over_the_limit_instead_of_remuxing_it() {
        let limit = 25 * MEBIBYTE;
        // 40 s of 1080p30 at 6 Mbit/s: about 30 MB, near the target but over 25 MiB.
        let over_limit = VideoSource {
            display_width: 1920,
            display_height: 1080,
            duration_seconds: 40.0,
            byte_size: 30_600_000,
            ..phone_clip()
        };
        assert!(matches!(
            plan_video_compression(&over_limit, FULL_SUPPORT, None),
            VideoCompressionPlan::Remux { .. }
        ));
        let plan = transcode(plan_video_compression(
            &over_limit,
            FULL_SUPPORT,
            Some(limit),
        ));
        assert!(!plan.is_for_compatibility);
        assert!((plan.video_bitrate + u64::from(AAC_BITRATE)) * 40 / 8 <= limit);
    }

    #[test]
    fn keeps_the_original_when_the_frame_rate_is_unknown() {
        for frame_rate in [0.0, f64::NAN, f64::INFINITY] {
            let source = VideoSource {
                frame_rate,
                ..phone_clip()
            };
            assert_eq!(
                plan_video_compression(&source, FULL_SUPPORT, Some(25 * MEBIBYTE)),
                VideoCompressionPlan::KeepOriginal(KeepOriginalReason::UnknownFrameRate)
            );
        }
    }

    #[test]
    fn wants_a_transcode_at_least_a_fifth_smaller_and_a_remux_that_does_not_grow() {
        assert!(is_output_worth_sending(Some(false), 100, 80, None));
        assert!(!is_output_worth_sending(Some(false), 100, 81, None));
        assert!(is_output_worth_sending(None, 100, 100, None));
        assert!(!is_output_worth_sending(None, 100, 101, None));
    }

    #[test]
    fn takes_any_output_that_fits_when_the_original_is_over_the_limit() {
        assert!(is_output_worth_sending(Some(false), 120, 100, Some(100)));
        assert!(!is_output_worth_sending(Some(false), 120, 101, Some(100)));
        assert!(!is_output_worth_sending(None, 120, 101, Some(100)));
    }

    #[test]
    fn takes_a_transcode_for_compatibility_that_fits_even_a_little_larger() {
        assert!(is_output_worth_sending(Some(true), 100, 130, Some(200)));
        assert!(is_output_worth_sending(Some(true), 100, 130, None));
        assert!(!is_output_worth_sending(Some(true), 100, 201, Some(200)));
    }

    #[test]
    fn gives_the_name_the_mp4_extension() {
        assert_eq!(compressed_video_file_name("IMG_0042.MOV"), "IMG_0042.mp4");
        assert_eq!(
            compressed_video_file_name("clip.final.webm"),
            "clip.final.mp4"
        );
        assert_eq!(compressed_video_file_name("video"), "video.mp4");
        assert_eq!(compressed_video_file_name(".hidden"), ".hidden.mp4");
    }

    /// An MPEG-4 Part 2 clip with PCM sound: a codec browsers do not play, so it is made H.264
    /// with AAC for compatibility, which exercises decoding, scaling, both encoders and the muxer.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn compresses_a_clip_to_a_faststart_mp4_with_progress_and_stops_when_cancelled() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mpeg4-pcm-192x108.mov");
        let mut fractions = Vec::new();
        let outcome = compress_video(
            &path,
            Some(25 * MEBIBYTE),
            &mut |fraction| fractions.push(fraction),
            &AtomicBool::new(false),
        )
        .expect("not cancelled");
        match outcome {
            VideoCompressionOutcome::Compressed(bytes) => {
                let position = |tag: &[u8]| bytes.windows(4).position(|window| window == tag);
                assert_eq!(position(b"ftyp"), Some(4));
                let (moov, mdat) = (position(b"moov"), position(b"mdat"));
                assert!(moov.is_some() && moov < mdat, "faststart: the index first");
                assert!(position(b"avc1").is_some(), "H.264 video");
                assert!(position(b"mp4a").is_some(), "AAC sound");
                assert!(fractions.windows(2).all(|pair| pair[0] < pair[1]));
                assert_eq!(fractions.last(), Some(&1.0));

                // Cancelled while it runs, at the first progress.
                let is_cancelled = AtomicBool::new(false);
                let cancelled = compress_video(
                    &path,
                    None,
                    &mut |_| is_cancelled.store(true, Ordering::Relaxed),
                    &is_cancelled,
                );
                assert_eq!(cancelled, Err(VideoCompressionCancelled));
            }
            // A Linux FFmpeg built without any H.264 encoder (CI images have libx264).
            VideoCompressionOutcome::KeptOriginal(reason) => {
                assert!(
                    cfg!(target_os = "linux") && reason == KeepOriginalReason::NoH264Encoder,
                    "{reason:?}"
                );
            }
        }

        let cancelled = compress_video(&path, None, &mut |_| {}, &AtomicBool::new(true));
        assert_eq!(cancelled, Err(VideoCompressionCancelled));
    }

    #[test]
    fn hands_only_videos_to_compression() {
        assert!(is_compressible_video_file_name("IMG_0042.MOV"));
        assert!(is_compressible_video_file_name("clip.webm"));
        assert!(!is_compressible_video_file_name("photo.jpg"));
        assert!(!is_compressible_video_file_name("take 3.wav"));
        assert!(!is_compressible_video_file_name("mp4"));
    }

    #[test]
    fn stops_a_file_running_far_past_its_measured_duration() {
        assert!(!runs_past_measured_duration(10.4, 10.0));
        assert!(!runs_past_measured_duration(1.7, 0.5));
        assert!(runs_past_measured_duration(3_600.0, 1.0));
    }
}
