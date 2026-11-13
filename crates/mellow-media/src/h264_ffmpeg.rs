//! H.264 decoding on Linux through FFmpeg's software decoder (the system FFmpeg the Linux
//! desktop links for Vesper's encoder, ADR-0051's amendment): one picture out per access unit,
//! no frame threads holding pictures back, and a pixel limit set before anything is decoded.

use std::ffi::c_int;
use std::ptr;

use rusty_ffmpeg::ffi as ff;

use crate::h264::{
    H264Error, MAXIMUM_DECODED_HEIGHT, MAXIMUM_DECODED_WIDTH, is_within_maximum_size,
};
use crate::video_frame::I420View;

fn check(status: c_int, what: &str) -> Result<c_int, H264Error> {
    if status < 0 {
        Err(H264Error(format!("{what}: {}", ff::av_err2str(status))))
    } else {
        Ok(status)
    }
}

pub(crate) struct H264Decoder {
    context: *mut ff::AVCodecContext,
    packet: *mut ff::AVPacket,
    frame: *mut ff::AVFrame,
}

impl std::fmt::Debug for H264Decoder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("H264Decoder")
            .finish_non_exhaustive()
    }
}

impl H264Decoder {
    pub(crate) fn new() -> Result<Self, H264Error> {
        // SAFETY: plain allocations; on error, `decoder` is dropped and frees what was made.
        unsafe {
            let mut decoder = Self {
                context: ptr::null_mut(),
                packet: ff::av_packet_alloc(),
                frame: ff::av_frame_alloc(),
            };
            let codec = ff::avcodec_find_decoder(ff::AV_CODEC_ID_H264);
            if codec.is_null() {
                return Err(H264Error("FFmpeg has no H.264 decoder".into()));
            }
            decoder.context = ff::avcodec_alloc_context3(codec);
            if decoder.context.is_null() || decoder.packet.is_null() || decoder.frame.is_null() {
                return Err(H264Error("allocating the H.264 decoder failed".into()));
            }
            // One picture in, one out: frame threads would hold pictures back.
            (*decoder.context).thread_count = 1;
            // A picture larger than the highest layer is refused before it is allocated.
            (*decoder.context).max_pixels =
                i64::from(MAXIMUM_DECODED_WIDTH) * i64::from(MAXIMUM_DECODED_HEIGHT);
            check(
                ff::avcodec_open2(decoder.context, codec, ptr::null_mut()),
                "open the H.264 decoder",
            )?;
            Ok(decoder)
        }
    }

    /// Decodes one access unit; `None` when it holds no picture.
    pub(crate) fn decode(&mut self, frame: &[u8]) -> Result<Option<I420View<'_>>, H264Error> {
        let length =
            c_int::try_from(frame.len()).map_err(|_| H264Error("the frame is too large".into()))?;
        // SAFETY: the context, packet and frame are owned here; the packet gets its own padded
        // copy of `frame` (FFmpeg reads past the end of its data).
        unsafe {
            ff::av_packet_unref(self.packet);
            check(ff::av_new_packet(self.packet, length), "allocate a packet")?;
            ptr::copy_nonoverlapping(frame.as_ptr(), (*self.packet).data, frame.len());
            check(
                ff::avcodec_send_packet(self.context, self.packet),
                "decode an H.264 frame",
            )?;
            ff::av_frame_unref(self.frame);
            let status = ff::avcodec_receive_frame(self.context, self.frame);
            if status == ff::AVERROR(ff::EAGAIN) {
                return Ok(None);
            }
            check(status, "decode an H.264 frame")?;
            Ok(planar_view(&*self.frame))
        }
    }
}

impl Drop for H264Decoder {
    fn drop(&mut self) {
        // SAFETY: owned here; each free accepts a null pointer.
        unsafe {
            ff::av_frame_free(&raw mut self.frame);
            ff::av_packet_free(&raw mut self.packet);
            ff::avcodec_free_context(&raw mut self.context);
        }
    }
}

/// The decoded picture's planes, when it is 4:2:0 planar (every Constrained Baseline stream).
fn planar_view(frame: &ff::AVFrame) -> Option<I420View<'_>> {
    let is_planar_420 = frame.format == ff::AV_PIX_FMT_YUV420P as c_int
        || frame.format == ff::AV_PIX_FMT_YUVJ420P as c_int;
    let width = u32::try_from(frame.width).ok()?;
    let height = u32::try_from(frame.height).ok()?;
    if !is_planar_420 || !is_within_maximum_size(width, height) {
        return None;
    }
    let plane = |index: usize, rows: u32| -> Option<(&[u8], usize)> {
        let stride = usize::try_from(frame.linesize[index]).ok()?;
        let base = frame.data[index];
        if base.is_null() {
            return None;
        }
        let length = stride.checked_mul(usize::try_from(rows).ok()?)?;
        // SAFETY: plane `index` of a decoded frame holds `rows` rows of `linesize` bytes; the
        // frame is kept until the next decode, which needs `&mut` on the decoder.
        Some((unsafe { std::slice::from_raw_parts(base, length) }, stride))
    };
    let chroma_rows = height.div_ceil(2);
    let (luma, luma_stride) = plane(0, height)?;
    let (blue_chroma, blue_stride) = plane(1, chroma_rows)?;
    let (red_chroma, red_stride) = plane(2, chroma_rows)?;
    if blue_stride != red_stride {
        return None;
    }
    I420View::new(
        width,
        height,
        (luma, luma_stride),
        (blue_chroma, red_chroma, blue_stride),
    )
}
