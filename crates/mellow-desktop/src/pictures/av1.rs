//! One AV1 still picture decoded by `rav1d`, the pure Rust port of dav1d (ADR-0051).
//!
//! rav1d 1.1 only publishes dav1d's C interface, so this module is the one place in the picture
//! pipeline with `unsafe` code: a context, a data buffer and a picture, each released by a guard,
//! and planes copied out into owned vectors before the picture is released. Everything after that
//! is safe Rust.

use std::ffi::c_void;
use std::io;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

use rav1d::Dav1dResult;
use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dLogger, Dav1dSettings};
use rav1d::include::dav1d::headers::{
    DAV1D_COLOR_PRI_SMPTE432, DAV1D_MC_BT709, DAV1D_MC_IDENTITY, DAV1D_PIXEL_LAYOUT_I400,
    DAV1D_PIXEL_LAYOUT_I420, DAV1D_PIXEL_LAYOUT_I422, DAV1D_PIXEL_LAYOUT_I444,
};
use rav1d::include::dav1d::picture::Dav1dPicture;
use rav1d::src::lib::{
    dav1d_close, dav1d_data_create, dav1d_data_unref, dav1d_default_settings, dav1d_get_picture,
    dav1d_open, dav1d_picture_unref, dav1d_send_data,
};

use super::PictureDecodeError;

/// A still picture is one frame: decoding it on the calling thread avoids starting and stopping a
/// thread pool per picture. Several pictures decode in parallel on different threads instead.
const DECODER_THREADS: i32 = 1;
const DECODER_FRAME_DELAY: i32 = 1;
/// Send and receive rounds before a bitstream that yields no picture is given up; a still needs
/// one or two.
const MAXIMUM_DECODE_ROUNDS: usize = 16;

/// How the samples relate to red, green and blue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatrixCoefficients {
    /// BT.601 (also BT.470 B/G), what the server's encoder writes, and the fallback for unknown.
    Bt601,
    Bt709,
    /// The planes are G, B, R.
    Identity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChromaSubsampling {
    /// 4:4:4, what the server writes.
    None,
    Horizontal,
    HorizontalAndVertical,
    /// A single plane: alpha pictures.
    Monochrome,
}

impl ChromaSubsampling {
    /// How many bits a luma coordinate is shifted to find its chroma sample, `(x, y)`.
    pub(crate) fn shifts(self) -> (u32, u32) {
        match self {
            Self::None | Self::Monochrome => (0, 0),
            Self::Horizontal => (1, 0),
            Self::HorizontalAndVertical => (1, 1),
        }
    }
}

/// The decoded planes, each row-major without padding; samples keep their bit depth.
#[derive(Debug)]
pub(crate) struct Av1Picture {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) bit_depth: u8,
    pub(crate) subsampling: ChromaSubsampling,
    pub(crate) matrix: MatrixCoefficients,
    pub(crate) is_full_range: bool,
    /// Display P3 primaries; otherwise taken as sRGB's (BT.709), which the server writes.
    pub(crate) is_display_p3: bool,
    /// Luma then the two chroma planes (absent when monochrome).
    pub(crate) planes: Vec<Vec<u16>>,
}

impl Av1Picture {
    pub(crate) fn chroma_width(&self) -> usize {
        self.width.div_ceil(1 << self.subsampling.shifts().0)
    }
}

fn is_would_block(result: Dav1dResult) -> bool {
    // dav1d answers `-EAGAIN`, whose value differs per OS; std knows it.
    result.0 < 0 && io::Error::from_raw_os_error(-result.0).kind() == io::ErrorKind::WouldBlock
}

struct Decoder(Option<Dav1dContext>);

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `dav1d_open` and is closed only here; `dav1d_close` takes
        // the context out of the option, so it is never used again.
        unsafe { dav1d_close(Some(NonNull::from(&mut self.0))) };
    }
}

struct Data(Dav1dData);

impl Drop for Data {
    fn drop(&mut self) {
        // SAFETY: the buffer came from `dav1d_data_create`; unref accepts an emptied one too.
        unsafe { dav1d_data_unref(Some(NonNull::from(&mut self.0))) };
    }
}

struct Picture(Dav1dPicture);

impl Drop for Picture {
    fn drop(&mut self) {
        // SAFETY: the picture was written by `dav1d_get_picture` (or is the empty default, which
        // unref ignores) and is not read after this.
        unsafe { dav1d_picture_unref(Some(NonNull::from(&mut self.0))) };
    }
}

fn open_decoder(maximum_pixels: u32) -> Result<Decoder, PictureDecodeError> {
    let mut settings = MaybeUninit::<Dav1dSettings>::uninit();
    // SAFETY: `dav1d_default_settings` writes a whole `Dav1dSettings` and reads nothing.
    unsafe { dav1d_default_settings(NonNull::from(&mut settings).cast()) };
    // SAFETY: initialised by the call above.
    let mut settings = unsafe { settings.assume_init() };
    settings.n_threads = DECODER_THREADS;
    settings.max_frame_delay = DECODER_FRAME_DELAY;
    // A hostile picture cannot make the decoder allocate more than the caller allows.
    settings.frame_size_limit = maximum_pixels;
    // SAFETY: a logger without a callback is dav1d's documented "no logging"; nothing is called.
    settings.logger = unsafe { Dav1dLogger::new(None, None) };
    let mut decoder = Decoder(None);
    // SAFETY: both pointers are valid for the call; `dav1d_open` writes the context or `None`.
    let result = unsafe {
        dav1d_open(
            Some(NonNull::from(&mut decoder.0)),
            Some(NonNull::from(&mut settings)),
        )
    };
    if result.0 != 0 || decoder.0.is_none() {
        return Err(PictureDecodeError::Undecodable);
    }
    Ok(decoder)
}

fn data_holding(bitstream: &[u8]) -> Result<Data, PictureDecodeError> {
    if bitstream.is_empty() {
        return Err(PictureDecodeError::Undecodable);
    }
    let mut data = Data(Dav1dData::default());
    // SAFETY: `data.0` is valid to write; on success the returned pointer starts a buffer of
    // `bitstream.len()` bytes owned by `data.0`.
    let buffer = unsafe { dav1d_data_create(Some(NonNull::from(&mut data.0)), bitstream.len()) };
    let buffer = NonNull::new(buffer).ok_or(PictureDecodeError::Undecodable)?;
    // SAFETY: `buffer` holds `bitstream.len()` writable bytes that nothing else references yet.
    unsafe { std::ptr::copy_nonoverlapping(bitstream.as_ptr(), buffer.as_ptr(), bitstream.len()) };
    Ok(data)
}

/// Copies `rows` rows of `width` samples starting at `origin`, `stride` bytes apart.
///
/// # Safety
/// `origin` must point at a plane of at least `rows` rows of `stride` bytes holding `width`
/// samples of 8 bits (`high_bit_depth` false) or 16 bits each, alive for the call.
unsafe fn copy_plane(
    origin: NonNull<c_void>,
    stride: isize,
    width: usize,
    rows: usize,
    high_bit_depth: bool,
) -> Vec<u16> {
    let mut samples = Vec::with_capacity(width * rows);
    for row in 0..rows {
        // `row` is below `rows` and `stride` a positive offset within the plane.
        #[allow(clippy::cast_possible_wrap)]
        let row_offset = row as isize * stride;
        // SAFETY: within the plane per the function's contract.
        let row_start = unsafe { origin.as_ptr().cast::<u8>().offset(row_offset) };
        if high_bit_depth {
            // SAFETY: the row holds `width` samples of two bytes; read as bytes, so no alignment
            // is assumed.
            let bytes_of_row = unsafe { std::slice::from_raw_parts(row_start, width * 2) };
            let (pairs, _) = bytes_of_row.as_chunks::<2>();
            samples.extend(pairs.iter().map(|pair| u16::from_ne_bytes(*pair)));
        } else {
            // SAFETY: the row holds `width` bytes.
            let samples_of_row = unsafe { std::slice::from_raw_parts(row_start, width) };
            samples.extend(samples_of_row.iter().map(|&sample| u16::from(sample)));
        }
    }
    samples
}

/// Decodes the one picture of an AV1 bitstream (an AVIF item), up to `maximum_pixels`.
pub(crate) fn decode_still(
    bitstream: &[u8],
    maximum_pixels: u32,
) -> Result<Av1Picture, PictureDecodeError> {
    let decoder = open_decoder(maximum_pixels)?;
    let mut data = data_holding(bitstream)?;
    let mut picture = Picture(Dav1dPicture::default());
    let mut has_picture = false;
    for _ in 0..MAXIMUM_DECODE_ROUNDS {
        if data.0.sz > 0 {
            // SAFETY: the context is open; `data.0` is a valid buffer from `dav1d_data_create`,
            // which dav1d advances as it consumes it.
            let sent = unsafe { dav1d_send_data(decoder.0, Some(NonNull::from(&mut data.0))) };
            if sent.0 != 0 && !is_would_block(sent) {
                return Err(PictureDecodeError::Undecodable);
            }
        }
        // SAFETY: the context is open and `picture.0` is valid to write; dav1d leaves it empty
        // when it has no picture yet.
        let received = unsafe { dav1d_get_picture(decoder.0, Some(NonNull::from(&mut picture.0))) };
        if received.0 == 0 {
            has_picture = true;
            break;
        }
        if !is_would_block(received) {
            return Err(PictureDecodeError::Undecodable);
        }
    }
    if !has_picture {
        return Err(PictureDecodeError::Undecodable);
    }
    planes_of(&picture.0)
}

fn planes_of(picture: &Dav1dPicture) -> Result<Av1Picture, PictureDecodeError> {
    let width = usize::try_from(picture.p.w).map_err(|_| PictureDecodeError::Undecodable)?;
    let height = usize::try_from(picture.p.h).map_err(|_| PictureDecodeError::Undecodable)?;
    let bit_depth = u8::try_from(picture.p.bpc).map_err(|_| PictureDecodeError::Undecodable)?;
    let subsampling = match picture.p.layout {
        DAV1D_PIXEL_LAYOUT_I444 => ChromaSubsampling::None,
        DAV1D_PIXEL_LAYOUT_I422 => ChromaSubsampling::Horizontal,
        DAV1D_PIXEL_LAYOUT_I420 => ChromaSubsampling::HorizontalAndVertical,
        DAV1D_PIXEL_LAYOUT_I400 => ChromaSubsampling::Monochrome,
        _ => return Err(PictureDecodeError::Undecodable),
    };
    let header = picture.seq_hdr.ok_or(PictureDecodeError::Undecodable)?;
    // SAFETY: a picture from `dav1d_get_picture` points at its sequence header until unref.
    let header = unsafe { header.as_ref() };
    let matrix = match header.mtrx {
        DAV1D_MC_BT709 => MatrixCoefficients::Bt709,
        DAV1D_MC_IDENTITY => MatrixCoefficients::Identity,
        _ => MatrixCoefficients::Bt601,
    };
    let is_full_range = header.color_range != 0;
    let is_display_p3 = header.pri == DAV1D_COLOR_PRI_SMPTE432;
    if width == 0 || height == 0 || !(8..=12).contains(&bit_depth) {
        return Err(PictureDecodeError::Undecodable);
    }
    let high_bit_depth = bit_depth > 8;
    let (horizontal_shift, vertical_shift) = subsampling.shifts();
    let chroma_size = (
        width.div_ceil(1 << horizontal_shift),
        height.div_ceil(1 << vertical_shift),
    );
    let plane_sizes: &[(usize, usize, usize)] = if subsampling == ChromaSubsampling::Monochrome {
        &[(0, width, height)]
    } else {
        &[
            (0, width, height),
            (1, chroma_size.0, chroma_size.1),
            (2, chroma_size.0, chroma_size.1),
        ]
    };
    let mut planes = Vec::with_capacity(plane_sizes.len());
    for &(index, plane_width, plane_height) in plane_sizes {
        let origin = picture
            .data
            .get(index)
            .copied()
            .flatten()
            .ok_or(PictureDecodeError::Undecodable)?;
        let stride = picture
            .stride
            .get(usize::from(index > 0))
            .copied()
            .ok_or(PictureDecodeError::Undecodable)?;
        let sample_bytes = if high_bit_depth { 2 } else { 1 };
        if usize::try_from(stride).map_or(true, |stride| stride < plane_width * sample_bytes) {
            return Err(PictureDecodeError::Undecodable);
        }
        // SAFETY: dav1d's picture holds `plane_height` rows of `stride` bytes for this plane, of
        // `plane_width` samples sized by the bit depth (checked above), alive until unref.
        planes
            .push(unsafe { copy_plane(origin, stride, plane_width, plane_height, high_bit_depth) });
    }
    Ok(Av1Picture {
        width,
        height,
        bit_depth,
        subsampling,
        matrix,
        is_full_range,
        is_display_p3,
        planes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_is_refused_without_a_panic() {
        assert!(decode_still(&[], 1_000).is_err());
        assert!(decode_still(&[0x12, 0x00, 0x0a, 0x0b, 0xff, 0xff, 0xff], 1_000).is_err());
    }
}
