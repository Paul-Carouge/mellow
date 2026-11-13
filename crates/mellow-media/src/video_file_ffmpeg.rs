//! Video files on Linux through the system FFmpeg (the libraries the Linux desktop already links
//! for Vesper's encoder and received H.264, ADR-0051's amendment): libavformat reads the file
//! from memory (only the demuxer its sniffed container names), libavcodec decodes whatever that
//! FFmpeg was built with (H.264, HEVC, VP8, VP9, AV1 through libdav1d; a distribution's
//! patent-free build may lack some, found out here and answered as `Unsupported`), libswscale
//! shrinks and converts each picture to BGRA in one pass, and libswresample turns the sound into
//! interleaved floats, surround folded to stereo. The pictures' and the sound's tracks each get
//! their own demuxer over the same bytes, so each seeks on its own.

// FFmpeg's `#define` constants come as `u32` and fit `c_int`; its buffers are allocated with
// `av_malloc`, aligned for any sample type.
#![allow(clippy::cast_possible_wrap, clippy::cast_ptr_alignment)]

use std::ffi::{CStr, c_int, c_void};
use std::ptr;
use std::sync::Arc;
use std::time::Duration;

use rusty_ffmpeg::ffi as ff;

use crate::clip_decoder::{ClipDecoder, ClipError};
use crate::video_file::{
    MAXIMUM_SHOWN_VIDEO_SIZE, MAXIMUM_SOURCE_PIXELS, SoundTrack, TimedPicture, VideoContainer,
    VideoTrackDecoder, quarter_turns_from_counterclockwise_degrees, turned_bgra,
};
use crate::video_frame::{DecodedPixelFormat, DecodedVideoFrame, fit_within};

/// libavformat's read buffer.
const IO_BUFFER_BYTES: c_int = 64 * 1024;
/// Decoding threads per video: GIFs play several at once, and a preview needs no more.
const DECODER_THREADS: c_int = 2;
/// The rates a recorder or a phone writes, as for audio attachments.
const SUPPORTED_SAMPLE_RATES: std::ops::RangeInclusive<c_int> = 8_000..=192_000;
/// Up to 7.1; folded to stereo.
const MAXIMUM_SOURCE_CHANNELS: c_int = 8;
const BGRA_BYTES_PER_PIXEL: usize = 4;
/// From 720 lines up, a stream that does not say its colours is taken as HD (BT.709), as
/// players do; below, as BT.601.
const HIGH_DEFINITION_LINES: c_int = 720;
const MICROSECONDS: ff::AVRational = ff::AVRational {
    num: 1,
    den: 1_000_000,
};

/// The file being read, behind libavformat's opaque pointer.
struct MemoryInput {
    bytes: Arc<[u8]>,
    position: usize,
}

/// libavformat's read callback: copies the next bytes of the file.
unsafe extern "C" fn read_input(opaque: *mut c_void, buffer: *mut u8, size: c_int) -> c_int {
    // SAFETY: `opaque` is the demuxer's boxed input, alive while its context is.
    let input = unsafe { &mut *opaque.cast::<MemoryInput>() };
    let rest = input.bytes.get(input.position..).unwrap_or_default();
    if rest.is_empty() {
        return ff::AVERROR_EOF;
    }
    let count = rest.len().min(usize::try_from(size).unwrap_or(0));
    // SAFETY: libavformat hands a buffer of `size` bytes; `count` is at most that.
    unsafe { ptr::copy_nonoverlapping(rest.as_ptr(), buffer, count) };
    input.position += count;
    c_int::try_from(count).unwrap_or(0)
}

/// libavformat's seek callback, `AVSEEK_SIZE` included.
unsafe extern "C" fn seek_input(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    // SAFETY: as in `read_input`.
    let input = unsafe { &mut *opaque.cast::<MemoryInput>() };
    let length = i64::try_from(input.bytes.len()).unwrap_or(i64::MAX);
    let base = match whence & !(ff::AVSEEK_FORCE as c_int) {
        whence if whence == ff::AVSEEK_SIZE as c_int => return length,
        whence if whence == ff::SEEK_SET as c_int => 0,
        whence if whence == ff::SEEK_CUR as c_int => {
            i64::try_from(input.position).unwrap_or(i64::MAX)
        }
        whence if whence == ff::SEEK_END as c_int => length,
        _ => return -1,
    };
    match base
        .checked_add(offset)
        .filter(|target| (0..=length).contains(target))
        .and_then(|target| usize::try_from(target).ok())
    {
        Some(target) => {
            input.position = target;
            i64::try_from(target).unwrap_or(-1)
        }
        None => -1,
    }
}

/// One demuxer over the file in memory.
struct Demuxer {
    format: *mut ff::AVFormatContext,
    io: *mut ff::AVIOContext,
    /// Boxed: the I/O context holds its address. Dropped after both.
    _input: Box<MemoryInput>,
}

impl Demuxer {
    fn open(bytes: Arc<[u8]>, container: VideoContainer) -> Result<Self, ClipError> {
        let mut input = Box::new(MemoryInput { bytes, position: 0 });
        let demuxer_name: &CStr = match container {
            VideoContainer::Mp4 => c"mov",
            VideoContainer::WebM => c"matroska",
        };
        // SAFETY: allocations and FFmpeg's documented custom I/O setup; on any error `demuxer`
        // drops and frees what was made (each free accepts null).
        unsafe {
            let buffer = ff::av_malloc(IO_BUFFER_BYTES as usize).cast::<u8>();
            if buffer.is_null() {
                return Err(ClipError::Corrupt);
            }
            let io = ff::avio_alloc_context(
                buffer,
                IO_BUFFER_BYTES,
                0,
                ptr::from_mut::<MemoryInput>(&mut input).cast(),
                Some(read_input),
                None,
                Some(seek_input),
            );
            if io.is_null() {
                ff::av_free(buffer.cast());
                return Err(ClipError::Corrupt);
            }
            let mut demuxer = Self {
                format: ff::avformat_alloc_context(),
                io,
                _input: input,
            };
            if demuxer.format.is_null() {
                return Err(ClipError::Corrupt);
            }
            (*demuxer.format).pb = io;
            (*demuxer.format).flags |= ff::AVFMT_FLAG_CUSTOM_IO as c_int;
            // Only the demuxer of the container the instance sniffed, never a guess.
            let input_format = ff::av_find_input_format(demuxer_name.as_ptr());
            if input_format.is_null() {
                return Err(ClipError::Unsupported);
            }
            // On failure FFmpeg frees the context and nulls the pointer.
            if ff::avformat_open_input(
                &raw mut demuxer.format,
                ptr::null(),
                input_format,
                ptr::null_mut(),
            ) < 0
            {
                return Err(ClipError::Corrupt);
            }
            if ff::avformat_find_stream_info(demuxer.format, ptr::null_mut()) < 0 {
                return Err(ClipError::Corrupt);
            }
            Ok(demuxer)
        }
    }

    /// The best stream of `kind`, with its decoder opened, every other stream skipped.
    fn open_stream(&self, kind: ff::AVMediaType) -> Result<OpenStream, ClipError> {
        // SAFETY: an opened format context; the stream pointers come from it.
        unsafe {
            let mut codec: *const ff::AVCodec = ptr::null();
            let index = ff::av_find_best_stream(self.format, kind, -1, -1, &raw mut codec, 0);
            let stream_index = usize::try_from(index).map_err(|_| ClipError::Unsupported)?;
            if codec.is_null() {
                return Err(ClipError::Unsupported);
            }
            let stream_count = usize::try_from((*self.format).nb_streams).unwrap_or(0);
            let streams = std::slice::from_raw_parts((*self.format).streams, stream_count);
            for (other, stream) in streams.iter().enumerate() {
                if other != stream_index {
                    (**stream).discard = ff::AVDISCARD_ALL;
                }
            }
            let stream = *streams.get(stream_index).ok_or(ClipError::Corrupt)?;
            let parameters = (*stream).codecpar;
            let decoder = Decoder {
                context: ff::avcodec_alloc_context3(codec),
                packet: ff::av_packet_alloc(),
                frame: ff::av_frame_alloc(),
            };
            if decoder.context.is_null() || decoder.packet.is_null() || decoder.frame.is_null() {
                return Err(ClipError::Corrupt);
            }
            if ff::avcodec_parameters_to_context(decoder.context, parameters) < 0 {
                return Err(ClipError::Corrupt);
            }
            (*decoder.context).thread_count = DECODER_THREADS;
            (*decoder.context).max_pixels =
                i64::try_from(MAXIMUM_SOURCE_PIXELS).unwrap_or(i64::MAX);
            if ff::avcodec_open2(decoder.context, codec, ptr::null_mut()) < 0 {
                return Err(ClipError::Unsupported);
            }
            Ok(OpenStream {
                index,
                stream,
                decoder,
            })
        }
    }

    /// The file's length, when its container says.
    fn duration(&self) -> Option<Duration> {
        // SAFETY: an opened format context.
        let duration = unsafe { (*self.format).duration };
        (duration != ff::AV_NOPTS_VALUE && duration > 0)
            .then(|| Duration::from_micros(u64::try_from(duration).unwrap_or(0)))
    }

    /// Reads the next packet of `stream_index` into `packet`; false at the end.
    fn read(&self, stream_index: c_int, packet: *mut ff::AVPacket) -> Result<bool, ClipError> {
        loop {
            // SAFETY: an opened context and an owned packet, unreferenced by the caller.
            let status = unsafe { ff::av_read_frame(self.format, packet) };
            if status == ff::AVERROR_EOF {
                return Ok(false);
            }
            if status < 0 {
                return Err(ClipError::Corrupt);
            }
            // SAFETY: a packet just read.
            if unsafe { (*packet).stream_index } == stream_index {
                return Ok(true);
            }
            // SAFETY: as above.
            unsafe { ff::av_packet_unref(packet) };
        }
    }

    /// Moves to the keyframe at or before `position` of `stream`.
    fn seek(&self, stream: &OpenStream, position: Duration) -> Result<(), ClipError> {
        let microseconds = i64::try_from(position.as_micros()).unwrap_or(i64::MAX);
        // SAFETY: an opened context; one of its streams.
        unsafe {
            let timestamp =
                ff::av_rescale_q(microseconds, MICROSECONDS, (*stream.stream).time_base);
            if ff::av_seek_frame(
                self.format,
                stream.index,
                timestamp,
                ff::AVSEEK_FLAG_BACKWARD as c_int,
            ) < 0
            {
                return Err(ClipError::Corrupt);
            }
            ff::avcodec_flush_buffers(stream.decoder.context);
        }
        Ok(())
    }
}

impl Drop for Demuxer {
    fn drop(&mut self) {
        // SAFETY: owned here; the custom I/O context is not freed by `avformat_close_input`.
        unsafe {
            ff::avformat_close_input(&raw mut self.format);
            if !self.io.is_null() {
                ff::av_freep((&raw mut (*self.io).buffer).cast());
            }
            ff::avio_context_free(&raw mut self.io);
        }
    }
}

struct Decoder {
    context: *mut ff::AVCodecContext,
    packet: *mut ff::AVPacket,
    frame: *mut ff::AVFrame,
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: owned here; each free accepts null.
        unsafe {
            ff::av_frame_free(&raw mut self.frame);
            ff::av_packet_free(&raw mut self.packet);
            ff::avcodec_free_context(&raw mut self.context);
        }
    }
}

struct OpenStream {
    index: c_int,
    stream: *mut ff::AVStream,
    decoder: Decoder,
}

/// What one more decoded frame needs.
enum Received {
    Frame,
    End,
}

/// Decodes until a frame is in `stream.decoder.frame` or the stream ended. A packet the decoder
/// calls invalid is skipped, as players do.
fn receive_frame(demuxer: &Demuxer, stream: &OpenStream) -> Result<Received, ClipError> {
    let decoder = &stream.decoder;
    loop {
        // SAFETY: an opened decoder and owned frame and packet.
        unsafe {
            let status = ff::avcodec_receive_frame(decoder.context, decoder.frame);
            if status == 0 {
                return Ok(Received::Frame);
            }
            if status == ff::AVERROR_EOF {
                return Ok(Received::End);
            }
            if status != ff::AVERROR(ff::EAGAIN) {
                return Err(ClipError::Corrupt);
            }
            ff::av_packet_unref(decoder.packet);
            let packet = if demuxer.read(stream.index, decoder.packet)? {
                decoder.packet
            } else {
                // Drains the frames held back for reordering.
                ptr::null_mut()
            };
            let sent = ff::avcodec_send_packet(decoder.context, packet);
            ff::av_packet_unref(decoder.packet);
            if sent < 0 && sent != ff::AVERROR_INVALIDDATA && sent != ff::AVERROR_EOF {
                return Err(ClipError::Corrupt);
            }
        }
    }
}

/// A frame's time from the start, in its stream's time base.
fn frame_time(frame: *const ff::AVFrame, stream: *const ff::AVStream) -> Duration {
    // SAFETY: a decoded frame and its stream.
    unsafe {
        let timestamp = match (*frame).best_effort_timestamp {
            ff::AV_NOPTS_VALUE => (*frame).pts,
            timestamp => timestamp,
        };
        if timestamp == ff::AV_NOPTS_VALUE {
            return Duration::ZERO;
        }
        let start = match (*stream).start_time {
            ff::AV_NOPTS_VALUE => 0,
            start => start,
        };
        let microseconds = ff::av_rescale_q(
            timestamp.saturating_sub(start),
            (*stream).time_base,
            MICROSECONDS,
        );
        Duration::from_micros(u64::try_from(microseconds).unwrap_or(0))
    }
}

pub(crate) struct FfmpegVideoTrack {
    stream: OpenStream,
    demuxer: Demuxer,
    scaler: *mut ff::SwsContext,
    quarter_turns: u8,
    duration: Option<Duration>,
}

// SAFETY: the FFmpeg contexts are owned by this value alone and used from one thread at a time
// (the decoding thread that opened them).
unsafe impl Send for FfmpegVideoTrack {}

impl FfmpegVideoTrack {
    pub(crate) fn open(bytes: Arc<[u8]>, container: VideoContainer) -> Result<Self, ClipError> {
        let demuxer = Demuxer::open(bytes, container)?;
        let stream = demuxer.open_stream(ff::AVMEDIA_TYPE_VIDEO)?;
        // SAFETY: the opened stream's parameters and their side data.
        let quarter_turns = unsafe {
            let parameters = (*stream.stream).codecpar;
            let pixels = i64::from((*parameters).width) * i64::from((*parameters).height);
            if pixels <= 0 || pixels > i64::try_from(MAXIMUM_SOURCE_PIXELS).unwrap_or(i64::MAX) {
                return Err(ClipError::Unsupported);
            }
            let matrix = ff::av_packet_side_data_get(
                (*parameters).coded_side_data,
                (*parameters).nb_coded_side_data,
                ff::AV_PKT_DATA_DISPLAYMATRIX,
            );
            let is_whole_matrix = !matrix.is_null()
                && (*matrix).size >= 9 * std::mem::size_of::<i32>()
                && !(*matrix).data.is_null();
            if is_whole_matrix {
                let degrees =
                    ff::av_display_rotation_get((*matrix).data.cast::<i32>().cast_const());
                if degrees.is_nan() {
                    0
                } else {
                    quarter_turns_from_counterclockwise_degrees(degrees)
                }
            } else {
                0
            }
        };
        let duration = demuxer.duration();
        Ok(Self {
            stream,
            demuxer,
            scaler: ptr::null_mut(),
            quarter_turns,
            duration,
        })
    }

    /// The decoded frame shrunk into BGRA.
    fn convert(&mut self) -> Result<DecodedVideoFrame, ClipError> {
        let frame = self.stream.decoder.frame;
        // SAFETY: a decoded frame; the scaler is cached here and freed on drop; the target
        // buffer holds `height` rows of `width × 4` bytes, as told to `sws_scale`.
        unsafe {
            let (source_width, source_height) = ((*frame).width, (*frame).height);
            let size = (
                u32::try_from(source_width).map_err(|_| ClipError::Corrupt)?,
                u32::try_from(source_height).map_err(|_| ClipError::Corrupt)?,
            );
            if u64::from(size.0) * u64::from(size.1) > MAXIMUM_SOURCE_PIXELS {
                return Err(ClipError::Unsupported);
            }
            let (width, height) = fit_within(size, MAXIMUM_SHOWN_VIDEO_SIZE);
            let (target_width, target_height) = (
                c_int::try_from(width).map_err(|_| ClipError::Corrupt)?,
                c_int::try_from(height).map_err(|_| ClipError::Corrupt)?,
            );
            self.scaler = ff::sws_getCachedContext(
                self.scaler,
                source_width,
                source_height,
                (*frame).format,
                target_width,
                target_height,
                ff::AV_PIX_FMT_BGRA,
                ff::SWS_BILINEAR as c_int,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            if self.scaler.is_null() {
                return Err(ClipError::Unsupported);
            }
            let colour_space = match (*frame).colorspace {
                ff::AVCOL_SPC_UNSPECIFIED if source_height >= HIGH_DEFINITION_LINES => {
                    ff::AVCOL_SPC_BT709
                }
                colour_space => colour_space,
            };
            let is_full_range = (*frame).color_range == ff::AVCOL_RANGE_JPEG;
            let coefficients = ff::sws_getCoefficients(colour_space as c_int);
            ff::sws_setColorspaceDetails(
                self.scaler,
                coefficients,
                c_int::from(is_full_range),
                coefficients,
                1,
                0,
                1 << 16,
                1 << 16,
            );
            let row_bytes =
                usize::try_from(width).map_err(|_| ClipError::Corrupt)? * BGRA_BYTES_PER_PIXEL;
            let mut pixels = vec![0_u8; row_bytes * usize::try_from(height).unwrap_or(0)];
            let targets: [*mut u8; 4] = [
                pixels.as_mut_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            ];
            let target_strides: [c_int; 4] = [
                c_int::try_from(row_bytes).map_err(|_| ClipError::Corrupt)?,
                0,
                0,
                0,
            ];
            let rows = ff::sws_scale(
                self.scaler,
                (*frame).data.as_ptr().cast(),
                (*frame).linesize.as_ptr(),
                0,
                source_height,
                targets.as_ptr(),
                target_strides.as_ptr(),
            );
            if rows != target_height {
                return Err(ClipError::Corrupt);
            }
            Ok(DecodedVideoFrame {
                width,
                height,
                format: DecodedPixelFormat::Bgra,
                pixels,
            })
        }
    }
}

impl Drop for FfmpegVideoTrack {
    fn drop(&mut self) {
        // SAFETY: owned here; accepts null.
        unsafe { ff::sws_freeContext(self.scaler) };
    }
}

impl VideoTrackDecoder for FfmpegVideoTrack {
    fn duration(&self) -> Option<Duration> {
        self.duration
    }

    fn next_picture(&mut self) -> Result<Option<TimedPicture>, ClipError> {
        match receive_frame(&self.demuxer, &self.stream)? {
            Received::End => Ok(None),
            Received::Frame => {
                let time = frame_time(self.stream.decoder.frame, self.stream.stream);
                let frame = self.convert();
                // SAFETY: an owned frame, read just above.
                unsafe { ff::av_frame_unref(self.stream.decoder.frame) };
                Ok(Some(TimedPicture {
                    time,
                    frame: turned_bgra(frame?, self.quarter_turns),
                }))
            }
        }
    }

    fn seek(&mut self, position: Duration) -> Result<(), ClipError> {
        self.demuxer.seek(&self.stream, position)
    }
}

/// A video file's sound, or silence when it has none this FFmpeg decodes.
pub(crate) fn open_sound_track(
    bytes: Arc<[u8]>,
    container: VideoContainer,
) -> Result<SoundTrack, ClipError> {
    let demuxer = Demuxer::open(bytes, container)?;
    match demuxer.open_stream(ff::AVMEDIA_TYPE_AUDIO) {
        Ok(stream) => Ok(SoundTrack::Sound(Box::new(FfmpegSound::new(
            demuxer, stream,
        )?))),
        Err(ClipError::Unsupported) => Ok(SoundTrack::Silent {
            duration: demuxer.duration(),
        }),
        Err(error) => Err(error),
    }
}

struct FfmpegSound {
    stream: OpenStream,
    demuxer: Demuxer,
    resampler: *mut ff::SwrContext,
    converted: *mut ff::AVFrame,
    sample_rate: u32,
    channels: usize,
    duration_frames: Option<u64>,
    /// Frames before this one are dropped: the target of the last seek.
    first_wanted_frame: u64,
}

// SAFETY: as `FfmpegVideoTrack`: owned contexts, one thread at a time (the clip player's).
unsafe impl Send for FfmpegSound {}

impl FfmpegSound {
    fn new(demuxer: Demuxer, stream: OpenStream) -> Result<Self, ClipError> {
        // SAFETY: an opened decoder; the resampler and frame are owned by the value built here
        // and freed on its drop.
        unsafe {
            let context = stream.decoder.context;
            let source_channels = (*context).ch_layout.nb_channels;
            let rate = (*context).sample_rate;
            if !SUPPORTED_SAMPLE_RATES.contains(&rate)
                || !(1..=MAXIMUM_SOURCE_CHANNELS).contains(&source_channels)
            {
                return Err(ClipError::Unsupported);
            }
            let channels = source_channels.min(2);
            let mut sound = Self {
                resampler: ptr::null_mut(),
                converted: ff::av_frame_alloc(),
                sample_rate: u32::try_from(rate).map_err(|_| ClipError::Corrupt)?,
                channels: usize::try_from(channels).map_err(|_| ClipError::Corrupt)?,
                duration_frames: demuxer.duration().map(|duration| {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    // Seconds times a rate stay far below 2^53, and are never negative.
                    let frames = (duration.as_secs_f64() * f64::from(rate)) as u64;
                    frames
                }),
                first_wanted_frame: 0,
                stream,
                demuxer,
            };
            if sound.converted.is_null() {
                return Err(ClipError::Corrupt);
            }
            let mut layout: ff::AVChannelLayout = std::mem::zeroed();
            ff::av_channel_layout_default(&raw mut layout, channels);
            let status = ff::swr_alloc_set_opts2(
                &raw mut sound.resampler,
                &raw const layout,
                ff::AV_SAMPLE_FMT_FLT,
                rate,
                &raw const (*context).ch_layout,
                (*context).sample_fmt,
                rate,
                0,
                ptr::null_mut(),
            );
            if status < 0 || ff::swr_init(sound.resampler) < 0 {
                return Err(ClipError::Unsupported);
            }
            (*sound.converted).ch_layout = layout;
            (*sound.converted).sample_rate = rate;
            (*sound.converted).format = ff::AV_SAMPLE_FMT_FLT as c_int;
            Ok(sound)
        }
    }
}

impl Drop for FfmpegSound {
    fn drop(&mut self) {
        // SAFETY: owned here; each free accepts null.
        unsafe {
            ff::av_frame_free(&raw mut self.converted);
            ff::swr_free(&raw mut self.resampler);
        }
    }
}

impl ClipDecoder for FfmpegSound {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn duration_frames(&self) -> Option<u64> {
        self.duration_frames
    }

    fn decode_next(&mut self, samples: &mut Vec<f32>) -> Result<bool, ClipError> {
        let frame = self.stream.decoder.frame;
        if let Received::End = receive_frame(&self.demuxer, &self.stream)? {
            return Ok(false);
        }
        let first_frame = frame_time(frame, self.stream.stream).as_micros()
            * u128::from(self.sample_rate)
            / 1_000_000;
        // SAFETY: a decoded frame and the owned converted frame, whose buffer FFmpeg
        // (re)allocates to `nb_samples` interleaved floats of `channels` channels.
        unsafe {
            ff::av_frame_unref(self.converted);
            let layout = &raw mut (*self.converted).ch_layout;
            ff::av_channel_layout_default(layout, c_int::try_from(self.channels).unwrap_or(2));
            (*self.converted).sample_rate = c_int::try_from(self.sample_rate).unwrap_or(0);
            (*self.converted).format = ff::AV_SAMPLE_FMT_FLT as c_int;
            let status = ff::swr_convert_frame(self.resampler, self.converted, frame);
            ff::av_frame_unref(frame);
            if status < 0 {
                return Err(ClipError::Corrupt);
            }
            let count = usize::try_from((*self.converted).nb_samples).unwrap_or(0) * self.channels;
            let data = (*self.converted).data[0];
            if count > 0 && !data.is_null() {
                let converted = std::slice::from_raw_parts(data.cast::<f32>().cast_const(), count);
                let skipped = u128::from(self.first_wanted_frame).saturating_sub(first_frame);
                let skipped = usize::try_from(skipped)
                    .unwrap_or(usize::MAX)
                    .saturating_mul(self.channels)
                    .min(count);
                samples.extend_from_slice(&converted[skipped..]);
            }
        }
        Ok(true)
    }

    fn seek(&mut self, frame: u64) -> Result<(), ClipError> {
        self.first_wanted_frame = frame;
        let position = Duration::from_secs_f64(
            // Frames of a sound stay far below 2^52.
            #[allow(clippy::cast_precision_loss)]
            {
                frame as f64 / f64::from(self.sample_rate)
            },
        );
        self.demuxer.seek(&self.stream, position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video_file::tests::{MP4_SILENT, MP4_WITH_SOUND, WEBM_WITH_SOUND, is_red};

    fn pictures(bytes: &'static [u8], container: VideoContainer) -> Vec<TimedPicture> {
        let mut track = FfmpegVideoTrack::open(Arc::from(bytes), container).unwrap();
        std::iter::from_fn(|| track.next_picture().unwrap()).collect()
    }

    #[test]
    fn every_picture_comes_in_display_order() {
        for (bytes, container) in [
            (MP4_WITH_SOUND, VideoContainer::Mp4),
            (WEBM_WITH_SOUND, VideoContainer::WebM),
        ] {
            let pictures = pictures(bytes, container);
            assert_eq!(pictures.len(), 10, "{container:?}");
            assert!(pictures.windows(2).all(|pair| pair[0].time < pair[1].time));
            assert!(is_red(&pictures[0].frame) && !is_red(&pictures[9].frame));
        }
    }

    #[test]
    fn a_seek_starts_again_at_the_keyframe_before() {
        let mut track =
            FfmpegVideoTrack::open(Arc::from(MP4_WITH_SOUND), VideoContainer::Mp4).unwrap();
        while track.next_picture().unwrap().is_some() {}
        track.seek(Duration::from_millis(700)).unwrap();
        let picture = track.next_picture().unwrap().unwrap();
        assert!(picture.time <= Duration::from_millis(700));
        assert!(!is_red(&picture.frame), "the second keyframe is blue");
    }

    #[test]
    fn a_turned_recording_is_shrunk_and_upright() {
        const TURNED: &[u8] = include_bytes!("../tests/fixtures/video-h264-turned-2560x1440.mp4");
        let mut track = FfmpegVideoTrack::open(Arc::from(TURNED), VideoContainer::Mp4).unwrap();
        let picture = track.next_picture().unwrap().unwrap().frame;
        assert_eq!((picture.width, picture.height), (720, 1_280));
        // Red above once turned back: the left column of the upright picture.
        let left = &picture.pixels[(640 * 720 + 10) * 4..][..4];
        assert!(left[2] > left[0], "{left:?}");
    }

    #[test]
    fn a_damaged_or_mislabelled_file_is_refused() {
        assert!(
            FfmpegVideoTrack::open(Arc::from(&MP4_WITH_SOUND[..200]), VideoContainer::Mp4).is_err()
        );
        assert!(FfmpegVideoTrack::open(Arc::from(MP4_WITH_SOUND), VideoContainer::WebM).is_err());
    }

    #[test]
    fn the_sound_seeks_to_its_frame_and_a_silent_file_has_none() {
        let SoundTrack::Sound(mut sound) =
            open_sound_track(Arc::from(WEBM_WITH_SOUND), VideoContainer::WebM).unwrap()
        else {
            panic!("the WebM has sound");
        };
        assert_eq!((sound.sample_rate(), sound.channels()), (48_000, 1));
        sound.seek(36_000).unwrap();
        let mut samples = Vec::new();
        while sound.decode_next(&mut samples).unwrap() {}
        #[allow(clippy::cast_precision_loss)] // A few thousand samples.
        let seconds = samples.len() as f64 / 48_000.0;
        assert!((0.2..=0.3).contains(&seconds), "{seconds}");
        assert!(matches!(
            open_sound_track(Arc::from(MP4_SILENT), VideoContainer::Mp4).unwrap(),
            SoundTrack::Silent { duration: Some(_) }
        ));
    }
}
