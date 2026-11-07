//! VP8 through libvpx, built from source by `build.rs` and reached through `src/vp8_shim.c`, so
//! only opaque pointers and integers cross the boundary. The server and browsers speak VP8 only
//! (ADR-0035); no common GPU encodes it, so this is the CPU path for cameras and shares.

use std::ffi::{c_int, c_uint, c_ulong};
use std::ptr::NonNull;

use crate::video_frame::I420View;

/// RTP's video clock, which the encoder's presentation times use.
pub(crate) const VIDEO_CLOCK_RATE: u32 = 90_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Vp8Error {
    #[error("libvpx refused the VP8 settings or frame")]
    Codec,
    #[error("libvpx ran out of memory")]
    OutOfMemory,
    #[error("the picture does not match the encoder's size")]
    WrongPictureSize,
}

/// What the picture shows, which changes how the encoder trades quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoContent {
    /// Motion matters: the encoder may drop detail and frames to keep up.
    Camera,
    /// Sharpness matters: text must stay readable (libvpx's screen content mode).
    Screen,
}

/// One encoder's settings: a simulcast layer's size and bit rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Vp8EncoderSettings {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bits_per_second: u32,
    pub(crate) frames_per_second: u32,
    pub(crate) content: VideoContent,
}

#[repr(C)]
struct ShimEncoder {
    _private: [u8; 0],
}

#[repr(C)]
struct ShimDecoder {
    _private: [u8; 0],
}

const SHIM_OK: c_int = 0;
const SHIM_OUT_OF_MEMORY: c_int = -3;
const SHIM_NO_FRAME: c_int = -4;

#[allow(unsafe_code)]
unsafe extern "C" {
    fn mellow_vp8_encoder_new(
        out: *mut *mut ShimEncoder,
        width: c_uint,
        height: c_uint,
        bitrate_kilobits: c_uint,
        frames_per_second: c_uint,
        threads: c_uint,
        cpu_used: c_int,
        is_screen: c_int,
    ) -> c_int;
    fn mellow_vp8_encoder_set_bitrate(encoder: *mut ShimEncoder, bitrate_kilobits: c_uint)
    -> c_int;
    fn mellow_vp8_encode(
        encoder: *mut ShimEncoder,
        luma: *const u8,
        luma_stride: c_int,
        blue_chroma: *const u8,
        red_chroma: *const u8,
        chroma_stride: c_int,
        presentation_time: i64,
        duration: c_ulong,
        force_keyframe: c_int,
        output: *mut u8,
        output_capacity: usize,
        written: *mut usize,
        is_keyframe: *mut c_int,
    ) -> c_int;
    fn mellow_vp8_encoder_free(encoder: *mut ShimEncoder);
    fn mellow_vp8_decoder_new(out: *mut *mut ShimDecoder, threads: c_uint) -> c_int;
    fn mellow_vp8_decode(
        decoder: *mut ShimDecoder,
        frame: *const u8,
        frame_length: usize,
        width: *mut c_uint,
        height: *mut c_uint,
        luma: *mut *const u8,
        luma_stride: *mut c_int,
        blue_chroma: *mut *const u8,
        red_chroma: *mut *const u8,
        chroma_stride: *mut c_int,
    ) -> c_int;
    fn mellow_vp8_decoder_free(decoder: *mut ShimDecoder);
}

fn status(code: c_int) -> Result<(), Vp8Error> {
    match code {
        SHIM_OK => Ok(()),
        SHIM_OUT_OF_MEMORY => Err(Vp8Error::OutOfMemory),
        _ => Err(Vp8Error::Codec),
    }
}

/// Encoder threads by picture size, as Chrome picks them: more only where a frame is large
/// enough to split, never more than the machine has.
fn encoder_threads(width: u32, height: u32) -> u32 {
    let pixels = u64::from(width) * u64::from(height);
    let wanted: u32 = if pixels >= 1920 * 1080 {
        4
    } else if pixels >= 1280 * 720 {
        2
    } else {
        1
    };
    let available = std::thread::available_parallelism()
        .map_or(1, |cores| u32::try_from(cores.get()).unwrap_or(u32::MAX));
    wanted.min(available.max(1))
}

/// libvpx's speed for real time: negative is a fixed speed of its absolute value (positive would
/// let libvpx pick one from the deadline), larger is faster, as Chrome sets it.
/// Small layers are cheap, so they get a slower, better setting (Chrome's choice).
fn encoder_speed(width: u32, height: u32) -> c_int {
    if u64::from(width) * u64::from(height) <= 352 * 288 {
        -4
    } else {
        -6
    }
}

/// A VP8 encoder of one fixed size. Not shared between threads: it is moved to its own.
#[derive(Debug)]
pub(crate) struct Vp8Encoder {
    encoder: NonNull<ShimEncoder>,
    settings: Vp8EncoderSettings,
    output: Vec<u8>,
}

// SAFETY: libvpx's encoder context has no thread affinity; `Vp8Encoder` owns it alone and every
// call takes `&mut self`, so it is never used from two threads at once.
#[allow(unsafe_code)]
unsafe impl Send for Vp8Encoder {}

/// An encoded frame of one layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Vp8Frame {
    pub(crate) bytes: Vec<u8>,
    pub(crate) is_keyframe: bool,
}

impl Vp8Encoder {
    pub(crate) fn new(settings: Vp8EncoderSettings) -> Result<Self, Vp8Error> {
        let mut encoder = std::ptr::null_mut();
        // SAFETY: `encoder` is a valid out pointer; the shim fills it only on success.
        #[allow(unsafe_code)]
        let code = unsafe {
            mellow_vp8_encoder_new(
                &raw mut encoder,
                settings.width,
                settings.height,
                (settings.bits_per_second / 1_000).max(1),
                settings.frames_per_second,
                encoder_threads(settings.width, settings.height),
                encoder_speed(settings.width, settings.height),
                c_int::from(settings.content == VideoContent::Screen),
            )
        };
        status(code)?;
        let encoder = NonNull::new(encoder).ok_or(Vp8Error::Codec)?;
        // A VP8 frame never exceeds the raw picture by much; twice the I420 size is a safe cap.
        let capacity = usize::try_from(u64::from(settings.width) * u64::from(settings.height) * 3)
            .map_err(|_| Vp8Error::Codec)?;
        Ok(Self {
            encoder,
            settings,
            output: vec![0; capacity],
        })
    }

    pub(crate) fn settings(&self) -> Vp8EncoderSettings {
        self.settings
    }

    pub(crate) fn set_bits_per_second(&mut self, bits_per_second: u32) -> Result<(), Vp8Error> {
        // SAFETY: the encoder pointer is owned and valid until `drop`.
        #[allow(unsafe_code)]
        let code = unsafe {
            mellow_vp8_encoder_set_bitrate(self.encoder.as_ptr(), (bits_per_second / 1_000).max(1))
        };
        status(code)?;
        self.settings.bits_per_second = bits_per_second;
        Ok(())
    }

    /// Encodes `picture` at `presentation_time` (90 kHz); `None` when the rate control dropped
    /// the frame to stay within the bit rate.
    pub(crate) fn encode(
        &mut self,
        picture: I420View<'_>,
        presentation_time: u64,
        should_be_keyframe: bool,
    ) -> Result<Option<Vp8Frame>, Vp8Error> {
        if picture.width() != self.settings.width || picture.height() != self.settings.height {
            return Err(Vp8Error::WrongPictureSize);
        }
        let frame_duration = VIDEO_CLOCK_RATE / self.settings.frames_per_second.max(1);
        let presentation_time = i64::try_from(presentation_time).map_err(|_| Vp8Error::Codec)?;
        let mut written = 0_usize;
        let mut is_keyframe: c_int = 0;
        let stride = |value: usize| c_int::try_from(value).map_err(|_| Vp8Error::Codec);
        // SAFETY: the planes are borrowed for the call and sized for the encoder's dimensions
        // (checked above, `I420View` guarantees its planes hold `stride × rows` bytes); `output`
        // is a writable buffer of `output.len()` bytes; the out pointers are valid locals.
        #[allow(unsafe_code)]
        let code = unsafe {
            mellow_vp8_encode(
                self.encoder.as_ptr(),
                picture.luma().as_ptr(),
                stride(picture.luma_stride())?,
                picture.blue_chroma().as_ptr(),
                picture.red_chroma().as_ptr(),
                stride(picture.chroma_stride())?,
                presentation_time,
                c_ulong::from(frame_duration),
                c_int::from(should_be_keyframe),
                self.output.as_mut_ptr(),
                self.output.len(),
                &raw mut written,
                &raw mut is_keyframe,
            )
        };
        status(code)?;
        if written == 0 {
            return Ok(None);
        }
        Ok(Some(Vp8Frame {
            bytes: self.output.get(..written).ok_or(Vp8Error::Codec)?.to_vec(),
            is_keyframe: is_keyframe != 0,
        }))
    }
}

impl Drop for Vp8Encoder {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `mellow_vp8_encoder_new` and is freed once, here.
        #[allow(unsafe_code)]
        unsafe {
            mellow_vp8_encoder_free(self.encoder.as_ptr());
        }
    }
}

/// A VP8 decoder; it follows size changes at keyframes (layer switches) by itself.
#[derive(Debug)]
pub(crate) struct Vp8Decoder {
    decoder: NonNull<ShimDecoder>,
}

// SAFETY: as for `Vp8Encoder`: owned alone, used through `&mut self` only.
#[allow(unsafe_code)]
unsafe impl Send for Vp8Decoder {}

impl Vp8Decoder {
    /// `threads`: 1 for small tiles, more for a large share.
    pub(crate) fn new(threads: u32) -> Result<Self, Vp8Error> {
        let mut decoder = std::ptr::null_mut();
        // SAFETY: `decoder` is a valid out pointer; the shim fills it only on success.
        #[allow(unsafe_code)]
        let code = unsafe { mellow_vp8_decoder_new(&raw mut decoder, threads.max(1)) };
        status(code)?;
        Ok(Self {
            decoder: NonNull::new(decoder).ok_or(Vp8Error::Codec)?,
        })
    }

    /// Decodes one frame; `None` when it shows nothing. The picture is libvpx's own buffer, valid
    /// until the next decode.
    ///
    /// # Errors
    /// A corrupt frame, or one that needs a keyframe first: ask the sender for one.
    pub(crate) fn decode(&mut self, frame: &[u8]) -> Result<Option<I420View<'_>>, Vp8Error> {
        let mut width: c_uint = 0;
        let mut height: c_uint = 0;
        let mut luma: *const u8 = std::ptr::null();
        let mut blue_chroma: *const u8 = std::ptr::null();
        let mut red_chroma: *const u8 = std::ptr::null();
        let mut luma_stride: c_int = 0;
        let mut chroma_stride: c_int = 0;
        // SAFETY: `frame` is borrowed for the call; every out pointer is a valid local.
        #[allow(unsafe_code)]
        let code = unsafe {
            mellow_vp8_decode(
                self.decoder.as_ptr(),
                frame.as_ptr(),
                frame.len(),
                &raw mut width,
                &raw mut height,
                &raw mut luma,
                &raw mut luma_stride,
                &raw mut blue_chroma,
                &raw mut red_chroma,
                &raw mut chroma_stride,
            )
        };
        if code == SHIM_NO_FRAME {
            return Ok(None);
        }
        status(code)?;
        let luma_stride = usize::try_from(luma_stride).map_err(|_| Vp8Error::Codec)?;
        let chroma_stride = usize::try_from(chroma_stride).map_err(|_| Vp8Error::Codec)?;
        let height_in_rows = usize::try_from(height).map_err(|_| Vp8Error::Codec)?;
        let chroma_rows = height_in_rows.div_ceil(2);
        if luma.is_null() || blue_chroma.is_null() || red_chroma.is_null() {
            return Err(Vp8Error::Codec);
        }
        // SAFETY: on success the shim describes libvpx's current picture: each plane holds
        // `stride × rows` bytes (libvpx allocates whole aligned rows) and stays valid until the
        // next decode on this decoder, which `&mut self` rules out while these slices live.
        #[allow(unsafe_code)]
        let (luma, blue_chroma, red_chroma) = unsafe {
            (
                std::slice::from_raw_parts(luma, luma_stride * height_in_rows),
                std::slice::from_raw_parts(blue_chroma, chroma_stride * chroma_rows),
                std::slice::from_raw_parts(red_chroma, chroma_stride * chroma_rows),
            )
        };
        I420View::new(
            width,
            height,
            (luma, luma_stride),
            (blue_chroma, red_chroma, chroma_stride),
        )
        .map(Some)
        .ok_or(Vp8Error::Codec)
    }
}

impl Drop for Vp8Decoder {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `mellow_vp8_decoder_new` and is freed once, here.
        #[allow(unsafe_code)]
        unsafe {
            mellow_vp8_decoder_free(self.decoder.as_ptr());
        }
    }
}
