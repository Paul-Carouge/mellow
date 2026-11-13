//! Video compression on macOS (ADR-0046 through `AVFoundation`): an `AVAssetReader` decodes (the
//! video through a composition that bakes the track's rotation into the frames, as the web
//! does), an `AVAssetWriter` encodes H.264 with `VideoToolbox` and AAC, scales to the planned size
//! and writes a faststart MP4 (`shouldOptimizeForNetworkUse`) with no metadata of the source's.
//! `AVFoundation` reads MP4 and `QuickTime` only: `WebM` and Matroska keep their original here.
//! Opus in a source is encoded to AAC rather than copied: `AVAssetWriter` does not pass Opus
//! through into MP4.

use std::path::Path;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::sel;
use objc2_av_foundation::{
    AVAsset, AVAssetReader, AVAssetReaderOutput, AVAssetReaderStatus, AVAssetReaderTrackOutput,
    AVAssetReaderVideoCompositionOutput, AVAssetTrack, AVAssetWriter, AVAssetWriterInput,
    AVAssetWriterStatus, AVFileTypeMPEG4, AVMediaCharacteristicContainsHDRVideo, AVMediaTypeAudio,
    AVMediaTypeVideo, AVMutableVideoComposition, AVURLAsset, AVVideoAverageBitRateKey,
    AVVideoCodecKey, AVVideoCodecTypeH264, AVVideoCompressionPropertiesKey,
    AVVideoExpectedSourceFrameRateKey, AVVideoHeightKey, AVVideoMaxKeyFrameIntervalDurationKey,
    AVVideoProfileLevelH264HighAutoLevel, AVVideoProfileLevelKey, AVVideoScalingModeKey,
    AVVideoScalingModeResize, AVVideoWidthKey,
};
use objc2_core_audio_types::{kAudioFormatLinearPCM, kAudioFormatMPEG4AAC};
use objc2_core_foundation::CFRetained;
use objc2_core_media::{
    CMAudioFormatDescriptionGetStreamBasicDescription, CMFormatDescription, CMTime,
    kCMPersistentTrackID_Invalid, kCMVideoCodecType_H264,
};
use objc2_core_video::{
    kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSNumber, NSObjectProtocol as _, NSString, NSURL,
};

use crate::video_compression::{
    AudioEncoderCodec, AudioPlan, BackendError, EncoderSupport, KEY_FRAME_INTERVAL_SECONDS,
    MAXIMUM_OUTPUT_FRAME_RATE, Measured, ProgressCallback, SourceAudioCodec, VideoCompressionPlan,
    VideoSource,
};

/// AAC's output rate: every source rate converts to it.
const AUDIO_SAMPLE_RATE: f64 = 48_000.0;
/// How long the copy loop waits when the writer takes nothing: it is waiting for the encoder.
const WRITER_BUSY_PAUSE: Duration = Duration::from_millis(1);

// `AVFoundation`'s audio settings keys (AVFAudio's `AVAudioSettings.h`) are strings equal to
// their own names; spelt out so the AVFAudio bindings are not linked for four keys.
const FORMAT_ID_KEY: &str = "AVFormatIDKey";
const SAMPLE_RATE_KEY: &str = "AVSampleRateKey";
const CHANNEL_COUNT_KEY: &str = "AVNumberOfChannelsKey";
const ENCODER_BITRATE_KEY: &str = "AVEncoderBitRateKey";

fn failed(what: &str) -> BackendError {
    BackendError::Failed(what.to_owned())
}

fn ns_failure(what: &str, error: &NSError) -> BackendError {
    BackendError::Failed(format!(
        "{what}: error {} ({})",
        error.code(),
        error.domain()
    ))
}

fn constant<T: ?Sized>(value: Option<&'static T>, name: &str) -> Result<&'static T, BackendError> {
    value.ok_or_else(|| BackendError::Failed(format!("{name} is missing")))
}

fn open_asset(path: &Path) -> Result<Retained<AVURLAsset>, BackendError> {
    let path = path
        .to_str()
        .ok_or_else(|| failed("a path that is not UTF-8"))?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    // SAFETY: a file URL and no options.
    Ok(unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) })
}

/// The asset's tracks of `media_type`, in order.
fn tracks_of(asset: &AVAsset, media_type: &NSString) -> Vec<Retained<AVAssetTrack>> {
    // SAFETY: plain property reads; this thread may block while they load.
    unsafe { asset.tracks() }
        .iter()
        .filter(|track| &*unsafe { track.mediaType() } == media_type)
        .collect()
}

/// The track's first format description: its codec and, for audio, its channels.
fn format_description(track: &AVAssetTrack) -> Option<CFRetained<CMFormatDescription>> {
    // SAFETY: `formatDescriptions` holds `CMFormatDescription`s (a CoreFoundation type, toll-free
    // bridged), so the first object is one; it is retained again before the array's hold ends.
    unsafe {
        let first = track.formatDescriptions().firstObject()?;
        let description = NonNull::new(
            Retained::as_ptr(&first)
                .cast_mut()
                .cast::<CMFormatDescription>(),
        )?;
        Some(CFRetained::retain(description))
    }
}

/// The source's channel count, or `None` when its description says nothing.
fn channel_count(track: &AVAssetTrack) -> Option<u32> {
    let description = format_description(track)?;
    // SAFETY: an audio track's description; the pointer, when not null, lives as long as it.
    let basic = unsafe { CMAudioFormatDescriptionGetStreamBasicDescription(&description) };
    // SAFETY: checked for null; read at once.
    (!basic.is_null()).then(|| unsafe { (*basic).mChannelsPerFrame })
}

pub(crate) fn measure(path: &Path, byte_size: u64) -> Result<Option<Measured>, BackendError> {
    let video_type = constant(unsafe { AVMediaTypeVideo }, "AVMediaTypeVideo")?;
    let audio_type = constant(unsafe { AVMediaTypeAudio }, "AVMediaTypeAudio")?;
    let hdr = constant(
        unsafe { AVMediaCharacteristicContainsHDRVideo },
        "AVMediaCharacteristicContainsHDRVideo",
    )?;
    let asset = open_asset(path)?;
    let Some(video) = tracks_of(&asset, video_type).into_iter().next() else {
        return Ok(None);
    };
    let audio = tracks_of(&asset, audio_type).into_iter().next();
    // SAFETY: plain property reads of a loaded asset and its tracks.
    let (natural, transform, frame_rate, duration, is_playable, is_hdr) = unsafe {
        (
            video.naturalSize(),
            video.preferredTransform(),
            video.nominalFrameRate(),
            asset.duration().seconds(),
            asset.isReadable() && video.isPlayable(),
            video.hasMediaCharacteristic(hdr),
        )
    };
    // The rotation applied to the natural size: what the reader sees.
    let shown_width = (transform.a * natural.width + transform.c * natural.height).abs();
    let shown_height = (transform.b * natural.width + transform.d * natural.height).abs();
    let video_subtype = format_description(&video).map(|description| {
        // SAFETY: a video track's description.
        unsafe { description.media_sub_type() }
    });
    let audio_codec = audio.as_deref().map(|track| {
        let subtype = format_description(track).map(|description| {
            // SAFETY: an audio track's description.
            unsafe { description.media_sub_type() }
        });
        if subtype == Some(kAudioFormatMPEG4AAC) {
            SourceAudioCodec::Aac
        } else {
            SourceAudioCodec::Other
        }
    });
    // Pixel sizes of real videos fit a u32.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let source = VideoSource {
        byte_size,
        duration_seconds: duration,
        display_width: shown_width.round() as u32,
        display_height: shown_height.round() as u32,
        frame_rate: f64::from(frame_rate),
        is_h264: video_subtype == Some(kCMVideoCodecType_H264),
        can_decode_video: is_playable,
        is_high_dynamic_range: is_hdr,
        audio_codec,
    };
    Ok(Some(Measured {
        source,
        // `VideoToolbox` encodes H.264 and Core Audio AAC on every Mac.
        support: EncoderSupport {
            can_encode_h264: true,
            audio_codec: Some(AudioEncoderCodec::Aac),
        },
    }))
}

fn number_u32(value: u32) -> Retained<NSNumber> {
    NSNumber::numberWithUnsignedInt(value)
}

fn number_f64(value: f64) -> Retained<NSNumber> {
    NSNumber::numberWithDouble(value)
}

fn dictionary(entries: &[(&NSString, &AnyObject)]) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<&NSString> = entries.iter().map(|(key, _)| *key).collect();
    let values: Vec<&AnyObject> = entries.iter().map(|(_, value)| *value).collect();
    NSDictionary::from_slices(&keys, &values)
}

/// One reader output feeding one writer input.
struct TrackCopy {
    output: Retained<AVAssetReaderOutput>,
    input: Retained<AVAssetWriterInput>,
    is_video: bool,
    is_done: bool,
}

#[allow(clippy::too_many_lines)] // One pipeline, set up in the order it runs.
pub(crate) fn execute(
    path: &Path,
    measured: &Measured,
    plan: &VideoCompressionPlan,
    output_path: &Path,
    on_progress: ProgressCallback<'_>,
    is_cancelled: &AtomicBool,
) -> Result<(), BackendError> {
    let video_type = constant(unsafe { AVMediaTypeVideo }, "AVMediaTypeVideo")?;
    let audio_type = constant(unsafe { AVMediaTypeAudio }, "AVMediaTypeAudio")?;
    let asset = open_asset(path)?;
    let video_tracks = tracks_of(&asset, video_type);
    let video = video_tracks
        .first()
        .ok_or_else(|| failed("the video track is gone"))?;
    let audio = tracks_of(&asset, audio_type).into_iter().next();
    let (video_copy_hint, audio_plan) = match plan {
        VideoCompressionPlan::KeepOriginal(_) => return Ok(()),
        VideoCompressionPlan::Remux { audio } => (true, *audio),
        VideoCompressionPlan::Transcode(transcode) => (false, transcode.audio),
    };

    // SAFETY: `AVFoundation` calls on objects created here, with settings dictionaries of the
    // documented keys and value types; this thread is not the main thread, so the blocking
    // property loads are allowed.
    unsafe {
        let reader = AVAssetReader::assetReaderWithAsset_error(&asset)
            .map_err(|error| ns_failure("the reader", &error))?;
        let output_path = output_path
            .to_str()
            .ok_or_else(|| failed("a path that is not UTF-8"))?;
        let writer = AVAssetWriter::assetWriterWithURL_fileType_error(
            &NSURL::fileURLWithPath(&NSString::from_str(output_path)),
            constant(AVFileTypeMPEG4, "AVFileTypeMPEG4")?,
        )
        .map_err(|error| ns_failure("the writer", &error))?;
        // Faststart: the index first, so a reader plays before the whole file is downloaded.
        writer.setShouldOptimizeForNetworkUse(true);

        let mut copies = Vec::with_capacity(2);
        let video_copy = if video_copy_hint {
            let output = AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(
                video, None,
            );
            let input =
                AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings_sourceFormatHint(
                    video_type,
                    None,
                    format_description(video).as_deref(),
                );
            // Packets are copied as they are: the rotation stays a display matrix.
            input.setTransform(video.preferredTransform());
            (Retained::into_super(output), input)
        } else {
            let VideoCompressionPlan::Transcode(transcode) = plan else {
                return Err(failed("a remux reached the transcoder"));
            };
            // Bakes the tracks' rotation into the frames, as the web does: the sizes were
            // planned on the rotated picture.
            #[allow(deprecated)]
            // The completion-handler form needs a run loop; this thread blocks.
            let composition =
                AVMutableVideoComposition::videoCompositionWithPropertiesOfAsset(&asset);
            let fastest = CMTime::new(1, 60);
            if transcode.frame_rate.is_some()
                || composition.frameDuration().seconds() < fastest.seconds()
            {
                composition.setFrameDuration(fastest);
                // From macOS 14 the composition follows the source's own frame timing, which
                // would keep 120 fps: the fixed rate applies only without it.
                if composition.respondsToSelector(sel!(setSourceTrackIDForFrameTiming:)) {
                    composition.setSourceTrackIDForFrameTiming(kCMPersistentTrackID_Invalid);
                }
            }
            let pixel_format_key: &NSString =
                &*std::ptr::from_ref(kCVPixelBufferPixelFormatTypeKey).cast::<NSString>();
            let pixel_format = number_u32(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange);
            let output = AVAssetReaderVideoCompositionOutput::assetReaderVideoCompositionOutputWithVideoTracks_videoSettings(
                &NSArray::from_retained_slice(&video_tracks),
                Some(&dictionary(&[(pixel_format_key, &pixel_format)])),
            );
            output.setVideoComposition(Some(&composition));
            output.setAlwaysCopiesSampleData(false);
            // u64 bitrates of a 1080p60 stream fit f64 exactly.
            #[allow(clippy::cast_precision_loss)]
            let bitrate = number_f64(transcode.video_bitrate as f64);
            let key_frame_interval = number_f64(KEY_FRAME_INTERVAL_SECONDS);
            let frame_rate = number_f64(
                transcode
                    .frame_rate
                    .unwrap_or(measured.source.frame_rate)
                    .min(MAXIMUM_OUTPUT_FRAME_RATE),
            );
            let compression = dictionary(&[
                (
                    constant(AVVideoAverageBitRateKey, "AVVideoAverageBitRateKey")?,
                    &bitrate,
                ),
                (
                    constant(
                        AVVideoMaxKeyFrameIntervalDurationKey,
                        "AVVideoMaxKeyFrameIntervalDurationKey",
                    )?,
                    &key_frame_interval,
                ),
                (
                    constant(AVVideoProfileLevelKey, "AVVideoProfileLevelKey")?,
                    constant(
                        AVVideoProfileLevelH264HighAutoLevel,
                        "AVVideoProfileLevelH264HighAutoLevel",
                    )?,
                ),
                (
                    constant(
                        AVVideoExpectedSourceFrameRateKey,
                        "AVVideoExpectedSourceFrameRateKey",
                    )?,
                    &frame_rate,
                ),
            ]);
            let width = number_u32(transcode.width);
            let height = number_u32(transcode.height);
            let settings = dictionary(&[
                (
                    constant(AVVideoCodecKey, "AVVideoCodecKey")?,
                    constant(AVVideoCodecTypeH264, "AVVideoCodecTypeH264")?,
                ),
                (constant(AVVideoWidthKey, "AVVideoWidthKey")?, &width),
                (constant(AVVideoHeightKey, "AVVideoHeightKey")?, &height),
                // Fills the planned size, as the web's `fit: "fill"`: the aspect is the same
                // but for the rounding to even sides.
                (
                    constant(AVVideoScalingModeKey, "AVVideoScalingModeKey")?,
                    constant(AVVideoScalingModeResize, "AVVideoScalingModeResize")?,
                ),
                (
                    constant(
                        AVVideoCompressionPropertiesKey,
                        "AVVideoCompressionPropertiesKey",
                    )?,
                    &compression,
                ),
            ]);
            let input = AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
                video_type,
                Some(&settings),
            );
            (Retained::into_super(output), input)
        };
        copies.push(TrackCopy {
            output: video_copy.0,
            input: video_copy.1,
            is_video: true,
            is_done: false,
        });

        if let Some(audio) = audio.as_deref() {
            let format_id_key = NSString::from_str(FORMAT_ID_KEY);
            let sample_rate_key = NSString::from_str(SAMPLE_RATE_KEY);
            let channel_count_key = NSString::from_str(CHANNEL_COUNT_KEY);
            let audio_copy = match audio_plan {
                AudioPlan::None => None,
                AudioPlan::Copy => Some((
                    Retained::into_super(
                        AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(audio, None),
                    ),
                    AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings_sourceFormatHint(
                        audio_type,
                        None,
                        format_description(audio).as_deref(),
                    ),
                )),
                AudioPlan::Encode { bitrate, .. } => {
                    let channels = number_u32(channel_count(audio).unwrap_or(2).clamp(1, 2));
                    let sample_rate = number_f64(AUDIO_SAMPLE_RATE);
                    let linear_pcm = number_u32(kAudioFormatLinearPCM);
                    let decoded = dictionary(&[
                        (&format_id_key, &linear_pcm),
                        (&sample_rate_key, &sample_rate),
                        (&channel_count_key, &channels),
                    ]);
                    let aac = number_u32(kAudioFormatMPEG4AAC);
                    let bitrate = number_u32(bitrate);
                    let encoder_bitrate_key = NSString::from_str(ENCODER_BITRATE_KEY);
                    let encoded = dictionary(&[
                        (&format_id_key, &aac),
                        (&sample_rate_key, &sample_rate),
                        (&channel_count_key, &channels),
                        (&encoder_bitrate_key, &bitrate),
                    ]);
                    Some((
                        Retained::into_super(
                            AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(
                                audio,
                                Some(&decoded),
                            ),
                        ),
                        AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
                            audio_type,
                            Some(&encoded),
                        ),
                    ))
                }
            };
            if let Some((output, input)) = audio_copy {
                copies.push(TrackCopy {
                    output,
                    input,
                    is_video: false,
                    is_done: false,
                });
            }
        }

        for copy in &copies {
            if !reader.canAddOutput(&copy.output) || !writer.canAddInput(&copy.input) {
                return Err(failed("the reader or the writer refused a track"));
            }
            reader.addOutput(&copy.output);
            writer.addInput(&copy.input);
        }
        if !reader.startReading() {
            return Err(reader.error().map_or_else(
                || failed("start reading"),
                |error| ns_failure("start reading", &error),
            ));
        }
        if !writer.startWriting() {
            reader.cancelReading();
            return Err(writer.error().map_or_else(
                || failed("start writing"),
                |error| ns_failure("start writing", &error),
            ));
        }
        writer.startSessionAtSourceTime(CMTime::new(0, 1));

        let duration = measured.source.duration_seconds;
        let stop = |error: BackendError| {
            reader.cancelReading();
            writer.cancelWriting();
            error
        };
        loop {
            if is_cancelled.load(Ordering::Relaxed) {
                return Err(stop(BackendError::Cancelled));
            }
            let mut has_moved = false;
            for copy in copies.iter_mut().filter(|copy| !copy.is_done) {
                if !copy.input.isReadyForMoreMediaData() {
                    continue;
                }
                has_moved = true;
                let Some(sample) = copy.output.copyNextSampleBuffer() else {
                    if reader.status() == AVAssetReaderStatus::Failed {
                        let error = reader.error().map_or_else(
                            || failed("read a sample"),
                            |error| ns_failure("read a sample", &error),
                        );
                        return Err(stop(error));
                    }
                    copy.input.markAsFinished();
                    copy.is_done = true;
                    continue;
                };
                if copy.is_video {
                    on_progress(sample.presentation_time_stamp().seconds() / duration);
                }
                if !copy.input.appendSampleBuffer(&sample) {
                    let error = writer.error().map_or_else(
                        || failed("append a sample"),
                        |error| ns_failure("append a sample", &error),
                    );
                    return Err(stop(error));
                }
            }
            if copies.iter().all(|copy| copy.is_done) {
                break;
            }
            if !has_moved {
                std::thread::sleep(WRITER_BUSY_PAUSE);
            }
        }

        let (finished, wait_for_finish) = std::sync::mpsc::sync_channel(1);
        let handler = RcBlock::new(move || {
            // The receiver waits below until this runs.
            let _ = finished.try_send(());
        });
        writer.finishWritingWithCompletionHandler(&handler);
        wait_for_finish
            .recv()
            .map_err(|_| failed("the writer never finished"))?;
        if writer.status() != AVAssetWriterStatus::Completed {
            return Err(writer.error().map_or_else(
                || failed("finish writing"),
                |error| ns_failure("finish writing", &error),
            ));
        }
    }
    on_progress(1.0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_a_quicktime_clip() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mpeg4-pcm-192x108.mov");
        let measured = measure(&path, 1).unwrap().expect("a video track");
        assert_eq!(
            (
                measured.source.display_width,
                measured.source.display_height
            ),
            (192, 108)
        );
        assert!(!measured.source.is_h264);
        assert_eq!(measured.source.audio_codec, Some(SourceAudioCodec::Other));
        assert!((measured.source.frame_rate - 30.0).abs() < 0.5);
        assert!((measured.source.duration_seconds - 1.0).abs() < 0.1);
        assert!(measured.source.can_decode_video);
    }
}
