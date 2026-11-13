//! Video compression on Linux (ADR-0046 through the system FFmpeg the desktop already links,
//! ADR-0051's amendment): libavformat reads the file and writes a faststart MP4 without the
//! source's tags, libavcodec decodes, a filter graph turns the picture upright (FFmpeg's decoders
//! leave the display matrix to the player), scales it to the planned size, caps the rate at 60
//! and hands it to H.264: `h264_vaapi` on the GPU when VA-API opens, else the software encoder
//! the system's FFmpeg was built with (`libx264`, `libopenh264`), as the web falls back to
//! `OpenH264`. Audio is copied (AAC, Opus) or encoded to AAC, else Opus. Each step is tried in
//! that order of encoders: a VA-API driver refusing the stream falls back to software.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::marker::PhantomData;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use rusty_ffmpeg::ffi as ff;

use crate::video_compression::{
    AudioEncoderCodec, AudioPlan, BackendError, EncoderSupport, KEY_FRAME_INTERVAL_SECONDS,
    MAXIMUM_OUTPUT_FRAME_RATE, MeasuredVideo, ProgressCallback, SourceAudioCodec,
    VideoCompressionPlan, VideoSource, runs_past_measured_duration,
};
use crate::video_file_ffmpeg::display_rotation_degrees;

/// The demuxers allowed to read a picked file: those of the containers the web's compression
/// reads (MP4 and `QuickTime`, Matroska and `WebM`). Anything else, a DASH or HLS manifest named
/// like a video included, keeps its original, as on the web.
const ALLOWED_DEMUXERS: &CStr = c"mov,matroska";
/// A picked file is read from the disk only: no demuxer may open network or other protocols.
const ALLOWED_PROTOCOLS: &CStr = c"file";
/// Automatic: one thread per core, for the decoders and the software H.264 encoder.
const AUTOMATIC_THREAD_COUNT: c_int = 0;

/// FFmpeg's `AV_TIME_BASE`: container durations are in microseconds.
const MICROSECONDS_PER_SECOND: f64 = 1_000_000.0;
/// The largest picture decoded: 8K, the largest a phone records.
const MAXIMUM_DECODED_PIXELS: i64 = 8_192 * 4_320;
/// AAC's output rate: every source rate converts to it (Opus's own rate too).
const AUDIO_SAMPLE_RATE: c_int = 48_000;
/// The VBR ceiling over the average, so `h264_vaapi` picks VBR (an average alone means CBR).
const PEAK_BITRATE_RATIO: i64 = 2;

fn check(status: c_int, what: &str) -> Result<c_int, BackendError> {
    if status < 0 {
        Err(BackendError::Failed(format!(
            "{what}: {}",
            ff::av_err2str(status)
        )))
    } else {
        Ok(status)
    }
}

/// FFmpeg's flag `#define`s, which bindgen types as `u32`, for its `int` fields.
#[allow(clippy::cast_possible_wrap)] // Single bits below the sign bit.
const fn flag(value: u32) -> c_int {
    value as c_int
}

fn not_null<T>(pointer: *mut T, what: &str) -> Result<*mut T, BackendError> {
    if pointer.is_null() {
        Err(BackendError::Failed(format!("{what} failed")))
    } else {
        Ok(pointer)
    }
}

fn c_string(text: &str) -> Result<CString, BackendError> {
    CString::new(text).map_err(|_| BackendError::Failed("a NUL byte in an argument".into()))
}

fn ratio(rational: ff::AVRational) -> f64 {
    if rational.den == 0 {
        0.0
    } else {
        f64::from(rational.num) / f64::from(rational.den)
    }
}

/// The H.264 encoders tried, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H264Encoder {
    Vaapi,
    Software(&'static CStr),
}

/// Software encoders a distribution's FFmpeg may carry; the release's minimal FFmpeg has none.
const SOFTWARE_H264_ENCODERS: [&CStr; 2] = [c"libx264", c"libopenh264"];

/// An opened VA-API device, or nothing on a machine without one.
struct VaapiDevice(*mut ff::AVBufferRef);

impl VaapiDevice {
    fn open() -> Option<Self> {
        let render_node = vesper_encode::ffmpeg::render_node_c_string().ok()?;
        let mut device = ptr::null_mut();
        // SAFETY: a plain allocation; `device` is ours on success.
        let status = unsafe {
            ff::av_hwdevice_ctx_create(
                &raw mut device,
                ff::AV_HWDEVICE_TYPE_VAAPI,
                render_node.as_ptr(),
                ptr::null_mut(),
                0,
            )
        };
        (status >= 0 && !device.is_null()).then_some(Self(device))
    }
}

impl Drop for VaapiDevice {
    fn drop(&mut self) {
        // SAFETY: owned here.
        unsafe { ff::av_buffer_unref(&raw mut self.0) };
    }
}

fn has_encoder(name: &CStr) -> bool {
    // SAFETY: a lookup in FFmpeg's static codec list.
    !unsafe { ff::avcodec_find_encoder_by_name(name.as_ptr()) }.is_null()
}

fn h264_encoders(has_vaapi_device: bool) -> Vec<H264Encoder> {
    let vaapi = (has_vaapi_device && has_encoder(c"h264_vaapi")).then_some(H264Encoder::Vaapi);
    vaapi
        .into_iter()
        .chain(
            SOFTWARE_H264_ENCODERS
                .into_iter()
                .filter(|name| has_encoder(name))
                .map(H264Encoder::Software),
        )
        .collect()
}

fn audio_encoder() -> Option<AudioEncoderCodec> {
    if has_encoder(c"aac") {
        Some(AudioEncoderCodec::Aac)
    } else if has_encoder(c"libopus") {
        Some(AudioEncoderCodec::Opus)
    } else {
        None
    }
}

/// FFmpeg's interrupt callback: blocking reads (probing, the drain) end once the flag is set.
unsafe extern "C" fn is_interrupted(is_cancelled: *mut c_void) -> c_int {
    // SAFETY: the `AtomicBool` the `Input` borrows for as long as its context lives.
    c_int::from(unsafe { &*is_cancelled.cast::<AtomicBool>() }.load(Ordering::Relaxed))
}

/// An opened input file, interrupted by the cancel flag it borrows.
struct Input<'flag>(*mut ff::AVFormatContext, PhantomData<&'flag AtomicBool>);

impl<'flag> Input<'flag> {
    fn open(path: &Path, is_cancelled: &'flag AtomicBool) -> Result<Self, BackendError> {
        let path = path
            .to_str()
            .ok_or_else(|| BackendError::Failed("a path that is not UTF-8".into()))?;
        let path = c_string(path)?;
        // SAFETY: FFmpeg allocates the context; `avformat_open_input` frees it on failure, and the
        // options dictionary is freed on every path. The flag outlives the context (`'flag`).
        let input = unsafe {
            let mut context = not_null(ff::avformat_alloc_context(), "an input context")?;
            (*context).interrupt_callback = ff::AVIOInterruptCB {
                callback: Some(is_interrupted),
                opaque: ptr::from_ref(is_cancelled).cast_mut().cast(),
            };
            let mut options = ptr::null_mut();
            let opened = check(
                ff::av_dict_set(
                    &raw mut options,
                    c"protocol_whitelist".as_ptr(),
                    ALLOWED_PROTOCOLS.as_ptr(),
                    0,
                ),
                "the input's options",
            )
            .and_then(|_| {
                check(
                    ff::av_dict_set(
                        &raw mut options,
                        c"format_whitelist".as_ptr(),
                        ALLOWED_DEMUXERS.as_ptr(),
                        0,
                    ),
                    "the input's options",
                )
            });
            let opened = match opened {
                Ok(_) => check(
                    ff::avformat_open_input(
                        &raw mut context,
                        path.as_ptr(),
                        ptr::null(),
                        &raw mut options,
                    ),
                    "open the file",
                ),
                Err(error) => {
                    ff::avformat_free_context(context);
                    Err(error)
                }
            };
            ff::av_dict_free(&raw mut options);
            opened?;
            Self(context, PhantomData)
        };
        // SAFETY: an opened context.
        unsafe {
            check(
                ff::avformat_find_stream_info(input.0, ptr::null_mut()),
                "read the streams",
            )?;
        }
        Ok(input)
    }

    /// The primary stream of `media_type` (for audio, the one going with `related` video).
    fn best_stream(&self, media_type: ff::AVMediaType, related: c_int) -> Option<c_int> {
        // SAFETY: an opened context; no decoder is asked for.
        let index =
            unsafe { ff::av_find_best_stream(self.0, media_type, -1, related, ptr::null_mut(), 0) };
        (index >= 0).then_some(index)
    }

    /// # Safety
    /// `index` came from [`Self::best_stream`].
    unsafe fn stream(&self, index: c_int) -> *mut ff::AVStream {
        // SAFETY: `index` is a valid stream index of this context.
        unsafe {
            *(*self.0)
                .streams
                .add(usize::try_from(index).unwrap_or_default())
        }
    }
}

impl Drop for Input<'_> {
    fn drop(&mut self) {
        // SAFETY: owned here; accepts a null pointer.
        unsafe { ff::avformat_close_input(&raw mut self.0) };
    }
}

/// The quarter turns the picture needs to stand upright, from the stream's display matrix.
///
/// # Safety
/// `parameters` belongs to an opened stream.
unsafe fn quarter_turns_clockwise(parameters: *const ff::AVCodecParameters) -> u32 {
    // SAFETY: as the caller promises.
    unsafe { display_rotation_degrees(parameters) }.map_or(0, quarter_turns_for_rotation)
}

/// FFmpeg's matrix angle is counter-clockwise: the picture turns the other way, as `ffmpeg`'s
/// `get_rotation` does it.
fn quarter_turns_for_rotation(counter_clockwise_degrees: f64) -> u32 {
    if !counter_clockwise_degrees.is_finite() {
        return 0;
    }
    let clockwise = (-counter_clockwise_degrees).round().rem_euclid(360.0);
    // Rounded to the nearest quarter turn: 0 to 4, 4 being 0.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let quarters = (clockwise / 90.0).round() as u32;
    quarters % 4
}

pub(crate) fn measure(
    path: &Path,
    byte_size: u64,
    is_cancelled: &AtomicBool,
) -> Result<Option<MeasuredVideo>, BackendError> {
    let input = Input::open(path, is_cancelled)?;
    let Some(video_index) = input.best_stream(ff::AVMEDIA_TYPE_VIDEO, -1) else {
        return Ok(None);
    };
    let audio_index = input.best_stream(ff::AVMEDIA_TYPE_AUDIO, video_index);
    // SAFETY: streams of the opened input, read while it lives.
    let source = unsafe {
        let stream = input.stream(video_index);
        let parameters = (*stream).codecpar;
        let duration = if (*input.0).duration > 0 {
            #[allow(clippy::cast_precision_loss)] // Microseconds of a real clip.
            let microseconds = (*input.0).duration as f64;
            microseconds / MICROSECONDS_PER_SECOND
        } else {
            #[allow(clippy::cast_precision_loss)]
            let ticks = (*stream).duration.max(0) as f64;
            ticks * ratio((*stream).time_base)
        };
        let aspect = ff::av_guess_sample_aspect_ratio(input.0, stream, ptr::null_mut());
        let pixel_aspect = if aspect.num > 0 && aspect.den > 0 {
            ratio(aspect)
        } else {
            1.0
        };
        let coded_width = f64::from((*parameters).width.max(0)) * pixel_aspect;
        let coded_height = f64::from((*parameters).height.max(0));
        let (width, height) = if quarter_turns_clockwise(parameters) % 2 == 1 {
            (coded_height, coded_width)
        } else {
            (coded_width, coded_height)
        };
        let transfer = (*parameters).color_trc;
        let audio_codec =
            audio_index.map(|index| match (*(*input.stream(index)).codecpar).codec_id {
                ff::AV_CODEC_ID_AAC => SourceAudioCodec::Aac,
                ff::AV_CODEC_ID_OPUS => SourceAudioCodec::Opus,
                _ => SourceAudioCodec::Other,
            });
        // Pixel sizes of real videos fit a u32.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        VideoSource {
            byte_size,
            duration_seconds: duration,
            display_width: width.round() as u32,
            display_height: height.round() as u32,
            frame_rate: ratio(ff::av_guess_frame_rate(input.0, stream, ptr::null_mut())),
            is_h264: (*parameters).codec_id == ff::AV_CODEC_ID_H264,
            can_decode_video: !ff::avcodec_find_decoder((*parameters).codec_id).is_null(),
            is_high_dynamic_range: transfer == ff::AVCOL_TRC_SMPTE2084
                || transfer == ff::AVCOL_TRC_ARIB_STD_B67,
            audio_codec,
        }
    };
    let has_vaapi_device = has_encoder(c"h264_vaapi") && VaapiDevice::open().is_some();
    Ok(Some(MeasuredVideo {
        source,
        support: EncoderSupport {
            can_encode_h264: !h264_encoders(has_vaapi_device).is_empty(),
            audio_codec: audio_encoder(),
        },
    }))
}

/// The video filters between the decoder and `encoder`: upright, scaled to the planned size with
/// square pixels, at most 60 frames a second, in the encoder's pixel format.
fn video_filters(
    quarter_turns: u32,
    width: u32,
    height: u32,
    caps_frame_rate: bool,
    encoder: H264Encoder,
) -> String {
    let mut filters: Vec<String> = Vec::new();
    match quarter_turns {
        1 => filters.push("transpose=clock".into()),
        2 => filters.push("hflip,vflip".into()),
        3 => filters.push("transpose=cclock".into()),
        _ => {}
    }
    filters.push(format!("scale={width}:{height},setsar=1"));
    if caps_frame_rate {
        filters.push(format!("fps={MAXIMUM_OUTPUT_FRAME_RATE}"));
    }
    filters.push(match encoder {
        H264Encoder::Vaapi => "format=nv12,hwupload".into(),
        H264Encoder::Software(_) => "format=yuv420p".into(),
    });
    filters.join(",")
}

/// The audio filters before `codec`'s encoder: 48 kHz, mono or stereo, its sample format.
fn audio_filters(codec: AudioEncoderCodec, channels: c_int) -> String {
    let sample_format = match codec {
        AudioEncoderCodec::Aac => "fltp",
        AudioEncoderCodec::Opus => "flt",
    };
    let layout = if channels == 1 { "mono" } else { "stereo" };
    format!(
        "aresample={AUDIO_SAMPLE_RATE},aformat=sample_fmts={sample_format}:channel_layouts={layout}"
    )
}

/// One stream decoded, filtered and encoded again.
struct Transcoder {
    decoder: *mut ff::AVCodecContext,
    graph: *mut ff::AVFilterGraph,
    source: *mut ff::AVFilterContext,
    sink: *mut ff::AVFilterContext,
    encoder: *mut ff::AVCodecContext,
}

impl Drop for Transcoder {
    fn drop(&mut self) {
        // SAFETY: owned here; each free accepts a null pointer.
        unsafe {
            ff::avcodec_free_context(&raw mut self.decoder);
            ff::avfilter_graph_free(&raw mut self.graph);
            ff::avcodec_free_context(&raw mut self.encoder);
        }
    }
}

/// An input stream going to the output, copied or transcoded.
struct StreamWork {
    input_index: c_int,
    input_time_base: ff::AVRational,
    output_stream: *mut ff::AVStream,
    transcoder: Option<Transcoder>,
}

/// The output file and the buffers the copy loop reuses.
struct Pipeline {
    output: *mut ff::AVFormatContext,
    packet: *mut ff::AVPacket,
    encoded: *mut ff::AVPacket,
    decoded: *mut ff::AVFrame,
    filtered: *mut ff::AVFrame,
    is_file_open: bool,
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        // SAFETY: owned here; each free accepts a null pointer.
        unsafe {
            if self.is_file_open {
                ff::avio_closep(&raw mut (*self.output).pb);
            }
            ff::avformat_free_context(self.output);
            ff::av_packet_free(&raw mut self.packet);
            ff::av_packet_free(&raw mut self.encoded);
            ff::av_frame_free(&raw mut self.decoded);
            ff::av_frame_free(&raw mut self.filtered);
        }
    }
}

pub(crate) fn execute(
    path: &Path,
    measured: &MeasuredVideo,
    plan: &VideoCompressionPlan,
    output_path: &Path,
    on_progress: ProgressCallback<'_>,
    is_cancelled: &AtomicBool,
) -> Result<(), BackendError> {
    let VideoCompressionPlan::Transcode(_) = plan else {
        return execute_with(
            path,
            measured,
            plan,
            None,
            output_path,
            on_progress,
            is_cancelled,
        );
    };
    let device = VaapiDevice::open();
    let mut last_error = BackendError::Failed("no H.264 encoder".into());
    for encoder in h264_encoders(device.is_some()) {
        let device = match encoder {
            H264Encoder::Vaapi => device.as_ref(),
            H264Encoder::Software(_) => None,
        };
        match execute_with(
            path,
            measured,
            plan,
            Some((encoder, device)),
            output_path,
            on_progress,
            is_cancelled,
        ) {
            Ok(()) => return Ok(()),
            Err(BackendError::Cancelled) => return Err(BackendError::Cancelled),
            Err(error) => {
                tracing::warn!(?encoder, %error, "an H.264 encoder failed on a video; trying the next");
                last_error = error;
            }
        }
    }
    Err(last_error)
}

#[allow(clippy::too_many_lines)] // One pipeline, set up in the order it runs.
fn execute_with(
    path: &Path,
    measured: &MeasuredVideo,
    plan: &VideoCompressionPlan,
    encoder: Option<(H264Encoder, Option<&VaapiDevice>)>,
    output_path: &Path,
    on_progress: ProgressCallback<'_>,
    is_cancelled: &AtomicBool,
) -> Result<(), BackendError> {
    let input = Input::open(path, is_cancelled)?;
    let video_index = input
        .best_stream(ff::AVMEDIA_TYPE_VIDEO, -1)
        .ok_or_else(|| BackendError::Failed("the video stream is gone".into()))?;
    let audio_index = input.best_stream(ff::AVMEDIA_TYPE_AUDIO, video_index);
    let audio_plan = match plan {
        VideoCompressionPlan::KeepOriginal(_) => return Ok(()),
        VideoCompressionPlan::Remux { audio } => *audio,
        VideoCompressionPlan::Transcode(transcode) => transcode.audio,
    };
    let output_path = output_path
        .to_str()
        .ok_or_else(|| BackendError::Failed("a path that is not UTF-8".into()))?;
    let output_path = c_string(output_path)?;

    // SAFETY: FFmpeg's documented transcoding sequence (its `transcode.c` example): every
    // pointer is allocated here, checked, and freed by `Pipeline`, `Transcoder` and `Input`.
    unsafe {
        let mut output = ptr::null_mut();
        check(
            ff::avformat_alloc_output_context2(
                &raw mut output,
                ptr::null(),
                c"mp4".as_ptr(),
                output_path.as_ptr(),
            ),
            "the MP4 muxer",
        )?;
        let mut pipeline = Pipeline {
            output: not_null(output, "the MP4 muxer")?,
            packet: not_null(ff::av_packet_alloc(), "a packet")?,
            encoded: not_null(ff::av_packet_alloc(), "a packet")?,
            decoded: not_null(ff::av_frame_alloc(), "a frame")?,
            filtered: not_null(ff::av_frame_alloc(), "a frame")?,
            is_file_open: false,
        };
        let needs_global_header =
            (*(*pipeline.output).oformat).flags & flag(ff::AVFMT_GLOBALHEADER) != 0;

        let mut output_streams = Vec::with_capacity(2);
        let video_stream = input.stream(video_index);
        let video_transcoder = match (plan, encoder) {
            (VideoCompressionPlan::Transcode(transcode), Some((encoder, device))) => {
                let filters = video_filters(
                    quarter_turns_clockwise((*video_stream).codecpar),
                    transcode.width,
                    transcode.height,
                    transcode.frame_rate.is_some(),
                    encoder,
                );
                Some(open_video_transcoder(
                    video_stream,
                    ff::av_guess_frame_rate(input.0, video_stream, ptr::null_mut()),
                    &filters,
                    encoder,
                    device,
                    transcode.video_bitrate,
                    needs_global_header,
                )?)
            }
            (VideoCompressionPlan::Transcode(_), None) => {
                return Err(BackendError::Failed("no H.264 encoder".into()));
            }
            _ => None,
        };
        output_streams.push(add_output_stream(
            pipeline.output,
            video_stream,
            video_index,
            video_transcoder,
        )?);
        if let Some(audio_index) = audio_index {
            let audio_stream = input.stream(audio_index);
            let audio_transcoder = match audio_plan {
                AudioPlan::None => None,
                AudioPlan::Copy => Some(None),
                AudioPlan::Encode { codec, bitrate } => Some(Some(open_audio_transcoder(
                    audio_stream,
                    codec,
                    bitrate,
                    needs_global_header,
                )?)),
            };
            if let Some(transcoder) = audio_transcoder {
                output_streams.push(add_output_stream(
                    pipeline.output,
                    audio_stream,
                    audio_index,
                    transcoder,
                )?);
            }
        }

        check(
            ff::avio_open(
                &raw mut (*pipeline.output).pb,
                output_path.as_ptr(),
                flag(ff::AVIO_FLAG_WRITE),
            ),
            "create the output file",
        )?;
        pipeline.is_file_open = true;
        let mut options = ptr::null_mut();
        // Faststart: the index first, so a reader plays before the whole file is downloaded.
        check(
            ff::av_dict_set(
                &raw mut options,
                c"movflags".as_ptr(),
                c"+faststart".as_ptr(),
                0,
            ),
            "the muxer's options",
        )?;
        let written = ff::avformat_write_header(pipeline.output, &raw mut options);
        ff::av_dict_free(&raw mut options);
        check(written, "write the header")?;

        let duration = measured.source.duration_seconds;
        let start = (*input.0).start_time;
        #[allow(clippy::cast_precision_loss)] // Microseconds of a real clip.
        let start_seconds = if start > 0 {
            start as f64 / MICROSECONDS_PER_SECOND
        } else {
            0.0
        };
        loop {
            if is_cancelled.load(Ordering::Relaxed) {
                return Err(BackendError::Cancelled);
            }
            let status = ff::av_read_frame(input.0, pipeline.packet);
            if status == ff::AVERROR_EOF {
                break;
            }
            check(status, "read a packet")?;
            let index = (*pipeline.packet).stream_index;
            let Some(work) = output_streams.iter().find(|work| work.input_index == index) else {
                ff::av_packet_unref(pipeline.packet);
                continue;
            };
            if (*pipeline.packet).pts != ff::AV_NOPTS_VALUE {
                #[allow(clippy::cast_precision_loss)] // Ticks of a real clip.
                let seconds =
                    (*pipeline.packet).pts as f64 * ratio(work.input_time_base) - start_seconds;
                if runs_past_measured_duration(seconds, duration) {
                    ff::av_packet_unref(pipeline.packet);
                    return Err(BackendError::Failed(
                        "the file runs far past its measured duration".into(),
                    ));
                }
                if index == video_index {
                    on_progress(seconds / duration);
                }
            }
            let result = feed(&pipeline, work, pipeline.packet);
            ff::av_packet_unref(pipeline.packet);
            result?;
        }
        // Drain each decoder, filter graph and encoder.
        for work in &output_streams {
            if work.transcoder.is_some() {
                feed(&pipeline, work, ptr::null_mut())?;
            }
        }
        check(ff::av_write_trailer(pipeline.output), "write the index")?;
    }
    on_progress(1.0);
    Ok(())
}

/// Adds the output stream for `input_stream`: its parameters copied, or the encoder's.
///
/// # Safety
/// `output` and `input_stream` are live; the transcoder's encoder is open.
unsafe fn add_output_stream(
    output: *mut ff::AVFormatContext,
    input_stream: *mut ff::AVStream,
    input_index: c_int,
    transcoder: Option<Transcoder>,
) -> Result<StreamWork, BackendError> {
    // SAFETY: as the caller promises.
    unsafe {
        let stream = not_null(
            ff::avformat_new_stream(output, ptr::null()),
            "an output stream",
        )?;
        if let Some(transcoder) = &transcoder {
            check(
                ff::avcodec_parameters_from_context((*stream).codecpar, transcoder.encoder),
                "the encoder's parameters",
            )?;
            (*stream).time_base = (*transcoder.encoder).time_base;
        } else {
            check(
                ff::avcodec_parameters_copy((*stream).codecpar, (*input_stream).codecpar),
                "copy the stream's parameters",
            )?;
            // The source container's tag may not exist in MP4: the muxer picks its own.
            (*(*stream).codecpar).codec_tag = 0;
            (*stream).time_base = (*input_stream).time_base;
        }
        Ok(StreamWork {
            input_index,
            input_time_base: (*input_stream).time_base,
            output_stream: stream,
            transcoder,
        })
    }
}

/// Opens the decoder for `stream`, with the decoded picture's size limit.
///
/// # Safety
/// `stream` belongs to an opened input.
unsafe fn open_decoder(stream: *mut ff::AVStream) -> Result<*mut ff::AVCodecContext, BackendError> {
    // SAFETY: as the caller promises; the context is freed by the caller's `Transcoder`, or here
    // on failure.
    unsafe {
        let parameters = (*stream).codecpar;
        let codec = ff::avcodec_find_decoder((*parameters).codec_id);
        if codec.is_null() {
            return Err(BackendError::Failed("no decoder".into()));
        }
        let mut decoder = not_null(ff::avcodec_alloc_context3(codec), "a decoder")?;
        let opened = check(
            ff::avcodec_parameters_to_context(decoder, parameters),
            "the decoder's parameters",
        )
        .and_then(|_| {
            (*decoder).pkt_timebase = (*stream).time_base;
            (*decoder).max_pixels = MAXIMUM_DECODED_PIXELS;
            // FFmpeg decodes on one thread by default: a 4K HEVC clip would take minutes.
            (*decoder).thread_count = AUTOMATIC_THREAD_COUNT;
            check(
                ff::avcodec_open2(decoder, codec, ptr::null_mut()),
                "open the decoder",
            )
        });
        if let Err(error) = opened {
            ff::avcodec_free_context(&raw mut decoder);
            return Err(error);
        }
        Ok(decoder)
    }
}

/// Builds `buffer_name` (with `source_arguments`) -> `filters` -> `sink_name` into `graph`.
///
/// # Safety
/// `transcoder.graph` is a fresh graph; `device`, when given, is a live VA-API device.
unsafe fn build_graph(
    transcoder: &mut Transcoder,
    buffer_name: &CStr,
    source_arguments: &str,
    sink_name: &CStr,
    filters: &str,
    device: Option<&VaapiDevice>,
) -> Result<(), BackendError> {
    let source_arguments = c_string(source_arguments)?;
    let filters = c_string(filters)?;
    // SAFETY: as the caller promises; the in/out lists are freed on every path.
    unsafe {
        check(
            ff::avfilter_graph_create_filter(
                &raw mut transcoder.source,
                ff::avfilter_get_by_name(buffer_name.as_ptr()),
                c"in".as_ptr(),
                source_arguments.as_ptr(),
                ptr::null_mut(),
                transcoder.graph,
            ),
            "the filters' source",
        )?;
        check(
            ff::avfilter_graph_create_filter(
                &raw mut transcoder.sink,
                ff::avfilter_get_by_name(sink_name.as_ptr()),
                c"out".as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                transcoder.graph,
            ),
            "the filters' sink",
        )?;
        let mut outputs = ff::avfilter_inout_alloc();
        let mut inputs = ff::avfilter_inout_alloc();
        let parsed = if outputs.is_null() || inputs.is_null() {
            Err(BackendError::Failed("the filters' links".into()))
        } else {
            (*outputs).name = ff::av_strdup(c"in".as_ptr());
            (*outputs).filter_ctx = transcoder.source;
            (*outputs).pad_idx = 0;
            (*outputs).next = ptr::null_mut();
            (*inputs).name = ff::av_strdup(c"out".as_ptr());
            (*inputs).filter_ctx = transcoder.sink;
            (*inputs).pad_idx = 0;
            (*inputs).next = ptr::null_mut();
            check(
                ff::avfilter_graph_parse_ptr(
                    transcoder.graph,
                    filters.as_ptr(),
                    &raw mut inputs,
                    &raw mut outputs,
                    ptr::null_mut(),
                ),
                "parse the filters",
            )
        };
        ff::avfilter_inout_free(&raw mut inputs);
        ff::avfilter_inout_free(&raw mut outputs);
        parsed?;
        if let Some(device) = device {
            // `hwupload` takes its frames' device from here, as `ffmpeg -filter_hw_device` does.
            let filter_count = usize::try_from((*transcoder.graph).nb_filters).unwrap_or_default();
            for index in 0..filter_count {
                let filter = *(*transcoder.graph).filters.add(index);
                (*filter).hw_device_ctx =
                    not_null(ff::av_buffer_ref(device.0), "a device reference")?;
            }
        }
        check(
            ff::avfilter_graph_config(transcoder.graph, ptr::null_mut()),
            "configure the filters",
        )?;
    }
    Ok(())
}

/// # Safety
/// `stream` belongs to an opened input; `device` is given for [`H264Encoder::Vaapi`].
unsafe fn open_video_transcoder(
    stream: *mut ff::AVStream,
    source_frame_rate: ff::AVRational,
    filters: &str,
    encoder: H264Encoder,
    device: Option<&VaapiDevice>,
    bitrate: u64,
    needs_global_header: bool,
) -> Result<Transcoder, BackendError> {
    // SAFETY: as the caller promises; everything allocated lands in `transcoder`, freed on drop.
    unsafe {
        let mut transcoder = Transcoder {
            decoder: open_decoder(stream)?,
            graph: not_null(ff::avfilter_graph_alloc(), "a filter graph")?,
            source: ptr::null_mut(),
            sink: ptr::null_mut(),
            encoder: ptr::null_mut(),
        };
        let decoder = transcoder.decoder;
        let parameters = (*stream).codecpar;
        let pixel_format = if (*decoder).pix_fmt >= 0 {
            (*decoder).pix_fmt
        } else {
            (*parameters).format
        };
        let aspect = (*parameters).sample_aspect_ratio;
        let (aspect_num, aspect_den) = if aspect.num > 0 && aspect.den > 0 {
            (aspect.num, aspect.den)
        } else {
            (1, 1)
        };
        let time_base = (*stream).time_base;
        let has_source_frame_rate = source_frame_rate.num > 0 && source_frame_rate.den > 0;
        // The encoder's rate comes from the sink: without it `h264_vaapi` would take the time
        // base for one (thousands of frames a second, a tiny bit budget per frame).
        let frame_rate_argument = if has_source_frame_rate {
            format!(
                ":frame_rate={}/{}",
                source_frame_rate.num, source_frame_rate.den
            )
        } else {
            String::new()
        };
        let source_arguments = format!(
            "video_size={}x{}:pix_fmt={pixel_format}:time_base={}/{}:pixel_aspect={aspect_num}/{aspect_den}{frame_rate_argument}",
            (*parameters).width,
            (*parameters).height,
            time_base.num,
            time_base.den,
        );
        build_graph(
            &mut transcoder,
            c"buffer",
            &source_arguments,
            c"buffersink",
            filters,
            device,
        )?;

        let name = match encoder {
            H264Encoder::Vaapi => c"h264_vaapi",
            H264Encoder::Software(name) => name,
        };
        let codec = ff::avcodec_find_encoder_by_name(name.as_ptr());
        if codec.is_null() {
            return Err(BackendError::Failed("the H.264 encoder is gone".into()));
        }
        transcoder.encoder = not_null(ff::avcodec_alloc_context3(codec), "an encoder")?;
        let context = transcoder.encoder;
        let sink = transcoder.sink;
        (*context).width = ff::av_buffersink_get_w(sink);
        (*context).height = ff::av_buffersink_get_h(sink);
        (*context).sample_aspect_ratio = ff::AVRational { num: 1, den: 1 };
        (*context).pix_fmt = ff::av_buffersink_get_format(sink);
        (*context).time_base = ff::av_buffersink_get_time_base(sink);
        let sink_frame_rate = ff::av_buffersink_get_frame_rate(sink);
        let frame_rate = if sink_frame_rate.num > 0 && sink_frame_rate.den > 0 {
            sink_frame_rate
        } else {
            source_frame_rate
        };
        (*context).framerate = frame_rate;
        let bitrate = i64::try_from(bitrate).unwrap_or(i64::MAX);
        (*context).bit_rate = bitrate;
        (*context).rc_max_rate = bitrate.saturating_mul(PEAK_BITRATE_RATIO);
        (*context).rc_buffer_size =
            c_int::try_from(bitrate.saturating_mul(PEAK_BITRATE_RATIO)).unwrap_or(c_int::MAX);
        let frames_per_second = if frame_rate.num > 0 && frame_rate.den > 0 {
            ratio(frame_rate)
        } else {
            MAXIMUM_OUTPUT_FRAME_RATE
        };
        // A few hundred frames at most.
        #[allow(clippy::cast_possible_truncation)]
        let key_frame_interval = (frames_per_second * KEY_FRAME_INTERVAL_SECONDS).round() as c_int;
        (*context).gop_size = key_frame_interval.max(1);
        if needs_global_header {
            (*context).flags |= flag(ff::AV_CODEC_FLAG_GLOBAL_HEADER);
        }
        match encoder {
            H264Encoder::Vaapi => {
                (*context).hw_frames_ctx = not_null(
                    ff::av_buffer_ref(ff::av_buffersink_get_hw_frames_ctx(sink)),
                    "the GPU frames",
                )?;
            }
            // `libx264` copies this into its own threads; the default is one.
            H264Encoder::Software(_) => (*context).thread_count = AUTOMATIC_THREAD_COUNT,
        }
        check(
            ff::avcodec_open2(context, codec, ptr::null_mut()),
            "open the H.264 encoder",
        )?;
        Ok(transcoder)
    }
}

/// # Safety
/// `stream` belongs to an opened input.
unsafe fn open_audio_transcoder(
    stream: *mut ff::AVStream,
    codec: AudioEncoderCodec,
    bitrate: u32,
    needs_global_header: bool,
) -> Result<Transcoder, BackendError> {
    // SAFETY: as the caller promises; everything allocated lands in `transcoder`, freed on drop.
    unsafe {
        let mut transcoder = Transcoder {
            decoder: open_decoder(stream)?,
            graph: not_null(ff::avfilter_graph_alloc(), "a filter graph")?,
            source: ptr::null_mut(),
            sink: ptr::null_mut(),
            encoder: ptr::null_mut(),
        };
        let decoder = transcoder.decoder;
        if (*decoder).ch_layout.order == ff::AV_CHANNEL_ORDER_UNSPEC {
            let channels = (*decoder).ch_layout.nb_channels;
            ff::av_channel_layout_uninit(&raw mut (*decoder).ch_layout);
            ff::av_channel_layout_default(&raw mut (*decoder).ch_layout, channels);
        }
        let mut layout: [c_char; 64] = [0; 64];
        check(
            ff::av_channel_layout_describe(
                &raw const (*decoder).ch_layout,
                layout.as_mut_ptr(),
                layout.len(),
            ),
            "the channel layout",
        )?;
        let layout = CStr::from_ptr(layout.as_ptr()).to_string_lossy();
        let sample_format = if (*decoder).sample_fmt >= 0 {
            (*decoder).sample_fmt
        } else {
            (*(*stream).codecpar).format
        };
        let sample_format_name = ff::av_get_sample_fmt_name(sample_format);
        if sample_format_name.is_null() {
            return Err(BackendError::Failed("an unknown sample format".into()));
        }
        let time_base = (*stream).time_base;
        let source_arguments = format!(
            "time_base={}/{}:sample_rate={}:sample_fmt={}:channel_layout={layout}",
            time_base.num,
            time_base.den,
            (*decoder).sample_rate,
            CStr::from_ptr(sample_format_name).to_string_lossy(),
        );
        let channels = (*decoder).ch_layout.nb_channels;
        build_graph(
            &mut transcoder,
            c"abuffer",
            &source_arguments,
            c"abuffersink",
            &audio_filters(codec, channels),
            None,
        )?;

        let name = match codec {
            AudioEncoderCodec::Aac => c"aac",
            AudioEncoderCodec::Opus => c"libopus",
        };
        let encoder = ff::avcodec_find_encoder_by_name(name.as_ptr());
        if encoder.is_null() {
            return Err(BackendError::Failed("the audio encoder is gone".into()));
        }
        transcoder.encoder = not_null(ff::avcodec_alloc_context3(encoder), "an encoder")?;
        let context = transcoder.encoder;
        let sink = transcoder.sink;
        (*context).sample_rate = ff::av_buffersink_get_sample_rate(sink);
        (*context).sample_fmt = ff::av_buffersink_get_format(sink);
        check(
            ff::av_buffersink_get_ch_layout(sink, &raw mut (*context).ch_layout),
            "the output channel layout",
        )?;
        (*context).time_base = ff::AVRational {
            num: 1,
            den: (*context).sample_rate,
        };
        (*context).bit_rate = i64::from(bitrate);
        if needs_global_header {
            (*context).flags |= flag(ff::AV_CODEC_FLAG_GLOBAL_HEADER);
        }
        check(
            ff::avcodec_open2(context, encoder, ptr::null_mut()),
            "open the audio encoder",
        )?;
        let has_variable_frame_size =
            (*encoder).capabilities & flag(ff::AV_CODEC_CAP_VARIABLE_FRAME_SIZE) != 0;
        if !has_variable_frame_size {
            let frame_size = u32::try_from((*context).frame_size).unwrap_or_default();
            ff::av_buffersink_set_frame_size(sink, frame_size);
        }
        Ok(transcoder)
    }
}

fn is_drained(status: c_int) -> bool {
    status == ff::AVERROR(ff::EAGAIN) || status == ff::AVERROR_EOF
}

/// Copies `packet` to the output, or decodes, filters and encodes it; a null packet drains.
///
/// # Safety
/// The pipeline and `work` are live; `packet`, when not null, came from the input.
unsafe fn feed(
    pipeline: &Pipeline,
    work: &StreamWork,
    packet: *mut ff::AVPacket,
) -> Result<(), BackendError> {
    // SAFETY: as the caller promises.
    unsafe {
        let Some(transcoder) = &work.transcoder else {
            ff::av_packet_rescale_ts(
                packet,
                work.input_time_base,
                (*work.output_stream).time_base,
            );
            (*packet).stream_index = (*work.output_stream).index;
            (*packet).pos = -1;
            check(
                ff::av_interleaved_write_frame(pipeline.output, packet),
                "write a packet",
            )?;
            return Ok(());
        };
        let status = ff::avcodec_send_packet(transcoder.decoder, packet);
        // A corrupt packet the decoder refuses is skipped, as players do.
        if status < 0 && status != ff::AVERROR_INVALIDDATA && !is_drained(status) {
            check(status, "decode")?;
        }
        loop {
            let status = ff::avcodec_receive_frame(transcoder.decoder, pipeline.decoded);
            if is_drained(status) {
                break;
            }
            check(status, "decode")?;
            (*pipeline.decoded).pts = (*pipeline.decoded).best_effort_timestamp;
            check(
                ff::av_buffersrc_add_frame_flags(transcoder.source, pipeline.decoded, 0),
                "filter a frame",
            )?;
            filter_and_encode(pipeline, work, transcoder)?;
        }
        if packet.is_null() {
            check(
                ff::av_buffersrc_add_frame_flags(transcoder.source, ptr::null_mut(), 0),
                "drain the filters",
            )?;
            filter_and_encode(pipeline, work, transcoder)?;
            check(
                ff::avcodec_send_frame(transcoder.encoder, ptr::null()),
                "drain the encoder",
            )?;
            write_encoded(pipeline, work, transcoder)?;
        }
        Ok(())
    }
}

/// # Safety
/// As [`feed`].
unsafe fn filter_and_encode(
    pipeline: &Pipeline,
    work: &StreamWork,
    transcoder: &Transcoder,
) -> Result<(), BackendError> {
    // SAFETY: as the caller promises.
    unsafe {
        loop {
            let status = ff::av_buffersink_get_frame(transcoder.sink, pipeline.filtered);
            if is_drained(status) {
                return Ok(());
            }
            check(status, "filter a frame")?;
            // The encoder decides the frame types.
            (*pipeline.filtered).pict_type = ff::AV_PICTURE_TYPE_NONE;
            let sent = ff::avcodec_send_frame(transcoder.encoder, pipeline.filtered);
            ff::av_frame_unref(pipeline.filtered);
            check(sent, "encode a frame")?;
            write_encoded(pipeline, work, transcoder)?;
        }
    }
}

/// # Safety
/// As [`feed`].
unsafe fn write_encoded(
    pipeline: &Pipeline,
    work: &StreamWork,
    transcoder: &Transcoder,
) -> Result<(), BackendError> {
    // SAFETY: as the caller promises.
    unsafe {
        loop {
            let status = ff::avcodec_receive_packet(transcoder.encoder, pipeline.encoded);
            if is_drained(status) {
                return Ok(());
            }
            check(status, "encode")?;
            ff::av_packet_rescale_ts(
                pipeline.encoded,
                (*transcoder.encoder).time_base,
                (*work.output_stream).time_base,
            );
            (*pipeline.encoded).stream_index = (*work.output_stream).index;
            check(
                ff::av_interleaved_write_frame(pipeline.output, pipeline.encoded),
                "write a packet",
            )?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_the_picture_upright_against_the_matrix_angle() {
        assert_eq!(quarter_turns_for_rotation(0.0), 0);
        // A phone held upright records a matrix of -90 degrees: one clockwise turn.
        assert_eq!(quarter_turns_for_rotation(-90.0), 1);
        assert_eq!(quarter_turns_for_rotation(90.0), 3);
        assert_eq!(quarter_turns_for_rotation(180.0), 2);
        assert_eq!(quarter_turns_for_rotation(-180.0), 2);
        assert_eq!(quarter_turns_for_rotation(f64::NAN), 0);
    }

    #[test]
    fn builds_the_filters_for_each_encoder() {
        assert_eq!(
            video_filters(1, 1080, 1920, true, H264Encoder::Vaapi),
            "transpose=clock,scale=1080:1920,setsar=1,fps=60,format=nv12,hwupload"
        );
        assert_eq!(
            video_filters(0, 640, 360, false, H264Encoder::Software(c"libx264")),
            "scale=640:360,setsar=1,format=yuv420p"
        );
        assert_eq!(
            audio_filters(AudioEncoderCodec::Aac, 6),
            "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo"
        );
    }
}
