//! Lossless compression of uploaded pictures (ADR-0021). Pure functions over bytes, run on a
//! blocking thread by the caller: untrusted bytes are decoded in Rust (`image`), within limits
//! checked first; libwebp only ever encodes decoded pixels.

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::{
    AnimationDecoder as _, DynamicImage, ImageDecoder as _, ImageFormat, ImageReader, Limits,
    RgbaImage,
};
use webp_animation::{EncoderOptions, EncodingConfig, EncodingType};

/// Larger pictures are kept as uploaded and never processed (decompression bombs). Checked from
/// the header, before any pixel is allocated; the total bounds the memory of one job.
const MAXIMUM_SIDE_PIXELS: u32 = 8_192;
const MAXIMUM_TOTAL_PIXELS: u64 = 24_000_000;
const MAXIMUM_FRAMES: usize = 600;
/// Largest display sizes: nothing is ever shown bigger (ADR-0021).
const AVATAR_SIDE_PIXELS: u32 = 512;
const BANNER_WIDTH_PIXELS: u32 = 1_920;
/// Icons are shown small (server rail, channel row), so they are stored small: three times their
/// largest display size, for high-density screens.
const GUILD_ICON_SIDE_PIXELS: u32 = 256;
const CHANNEL_ICON_SIDE_PIXELS: u32 = 96;
/// Custom emoji show at most at 48 px (jumbo, alone in a message), so they are stored at 128 px,
/// as on Discord (ADR-0028).
const EMOJI_SIDE_PIXELS: u32 = 128;
/// libwebp's effort for lossless encoding: 0 (fast) to 100 (smallest); the pixels are the same
/// whatever the effort. Measured in release on a 480 × 480, 60-frame GIF: 2.4 s at 60/4, 2.2 s at
/// 25/1, 1.5 s at 10/0, decoding included; it runs after the upload is answered.
const LOSSLESS_EFFORT: f32 = 25.0;
const LOSSLESS_METHOD: usize = 1;
/// oxipng's preset: 2 is its fast default, most of the gain of the slow ones.
const PNG_OPTIMIZATION_PRESET: u8 = 2;
/// oxipng stops trying better compressions past this and keeps its best so far.
const PNG_OPTIMIZATION_TIME_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);
/// Memory allowed to read a picture's header and its metadata, before any pixel.
const HEADER_READ_MAXIMUM_BYTES: u64 = 16 * 1024 * 1024;
/// Bytes per pixel of the widest PNG layout (RGBA, 16 bits a channel).
const PNG_LARGEST_BYTES_PER_PIXEL: u64 = 8;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum ImageProcessingError {
    #[error("not a picture this server reads")]
    Unsupported,
    #[error("the picture is too large to process")]
    TooLarge,
    #[error("the picture could not be encoded")]
    Encoding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileImageKind {
    Avatar,
    Banner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IconKind {
    /// May be animated.
    Guild,
    /// Always still: an animated source keeps its first frame.
    Channel,
    /// A custom emoji (ADR-0028); may be animated.
    Emoji,
}

/// The square of a picture an icon keeps, in pixels of the picture as shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SquareCrop {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) size: u32,
}

/// A picture ready to store, with its served type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessedImage {
    pub(crate) bytes: Vec<u8>,
    pub(crate) content_type: &'static str,
    pub(crate) is_animated: bool,
}

/// An avatar or banner to store: the picture, and its first frame alone when it is animated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessedProfileImage {
    pub(crate) picture: ProcessedImage,
    pub(crate) still: Option<Vec<u8>>,
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAXIMUM_SIDE_PIXELS);
    limits.max_image_height = Some(MAXIMUM_SIDE_PIXELS);
    // RGBA, 4 bytes per pixel, for one frame at a time.
    limits.max_alloc = Some(MAXIMUM_TOTAL_PIXELS * 4);
    limits
}

/// Refuses a picture whose header announces more than the limits, before decoding it.
fn check_dimensions(width: u32, height: u32) -> Result<(), ImageProcessingError> {
    if width > MAXIMUM_SIDE_PIXELS
        || height > MAXIMUM_SIDE_PIXELS
        || u64::from(width) * u64::from(height) > MAXIMUM_TOTAL_PIXELS
    {
        return Err(ImageProcessingError::TooLarge);
    }
    Ok(())
}

fn header_dimensions(
    bytes: &[u8],
    format: ImageFormat,
) -> Result<(u32, u32), ImageProcessingError> {
    let mut header_limits = limits();
    // Headers can hold compressed metadata (PNG iCCP) inflated while reading them.
    header_limits.max_alloc = Some(HEADER_READ_MAXIMUM_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(header_limits);
    reader.into_dimensions().map_err(|error| match error {
        image::ImageError::Limits(_) => ImageProcessingError::TooLarge,
        _ => ImageProcessingError::Unsupported,
    })
}

/// The frames of an animation, each through `shrink` as soon as it is decoded, within the frame
/// and total pixel limits.
fn collect_frames(
    frames: image::Frames<'_>,
    (width, height): (u32, u32),
    shrink: impl Fn(RgbaImage) -> RgbaImage,
) -> Result<Vec<(RgbaImage, u32)>, ImageProcessingError> {
    check_dimensions(width, height)?;
    let mut collected = Vec::new();
    let mut total_pixels = 0u64;
    for frame in frames {
        let frame = match frame {
            Ok(frame) => frame,
            Err(image::ImageError::Limits(_)) => return Err(ImageProcessingError::TooLarge),
            // A truncated file keeps the frames read so far, as browsers show it.
            Err(_) if !collected.is_empty() => break,
            Err(_) => return Err(ImageProcessingError::Unsupported),
        };
        total_pixels += u64::from(width) * u64::from(height);
        if collected.len() >= MAXIMUM_FRAMES || total_pixels > MAXIMUM_TOTAL_PIXELS {
            return Err(ImageProcessingError::TooLarge);
        }
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = numerator.checked_div(denominator).unwrap_or(0).max(20);
        collected.push((shrink(frame.into_buffer()), delay));
    }
    if collected.is_empty() {
        return Err(ImageProcessingError::Unsupported);
    }
    Ok(collected)
}

/// Reads only the frame headers of a GIF and refuses it when a frame lies outside the canvas or
/// the frames' own areas add up past the limits: each frame is decompressed at its own size,
/// whatever the canvas, so counting canvas pixels alone would not bound the work.
fn check_gif_frames(bytes: &[u8]) -> Result<(), ImageProcessingError> {
    let mut options = gif::DecodeOptions::new();
    options.skip_frame_decoding(true);
    let mut decoder = options
        .read_info(Cursor::new(bytes))
        .map_err(|_| ImageProcessingError::Unsupported)?;
    let (canvas_width, canvas_height) = (u32::from(decoder.width()), u32::from(decoder.height()));
    check_dimensions(canvas_width, canvas_height)?;
    let mut frame_count = 0usize;
    let mut total_pixels = 0u64;
    loop {
        let frame = match decoder.next_frame_info() {
            Ok(Some(frame)) => frame,
            Ok(None) => return Ok(()),
            // Truncated: the frames before the cut are what will be decoded.
            Err(_) if frame_count > 0 => return Ok(()),
            Err(_) => return Err(ImageProcessingError::Unsupported),
        };
        let is_inside_canvas = u32::from(frame.left) + u32::from(frame.width) <= canvas_width
            && u32::from(frame.top) + u32::from(frame.height) <= canvas_height;
        frame_count += 1;
        total_pixels += u64::from(frame.width) * u64::from(frame.height);
        if !is_inside_canvas || frame_count > MAXIMUM_FRAMES || total_pixels > MAXIMUM_TOTAL_PIXELS
        {
            return Err(ImageProcessingError::TooLarge);
        }
    }
}

/// The frames of a picture with their duration in milliseconds; one frame for still pictures,
/// turned the way its EXIF orientation says. Each frame goes through `shrink` as soon as it is
/// decoded, so only small frames are kept. Animated PNG (APNG) is read as its first frame.
fn decode_frames(
    bytes: &[u8],
    shrink: impl Fn(RgbaImage) -> RgbaImage,
) -> Result<Vec<(RgbaImage, u32)>, ImageProcessingError> {
    let format = image::guess_format(bytes).map_err(|_| ImageProcessingError::Unsupported)?;
    match format {
        ImageFormat::Gif => {
            check_gif_frames(bytes)?;
            let mut decoder = GifDecoder::new(Cursor::new(bytes))
                .map_err(|_| ImageProcessingError::Unsupported)?;
            decoder
                .set_limits(limits())
                .map_err(|_| ImageProcessingError::TooLarge)?;
            let dimensions = decoder.dimensions();
            return collect_frames(decoder.into_frames(), dimensions, shrink);
        }
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(Cursor::new(bytes))
                .map_err(|_| ImageProcessingError::Unsupported)?;
            if decoder.has_animation() {
                decoder
                    .set_limits(limits())
                    .map_err(|_| ImageProcessingError::TooLarge)?;
                let dimensions = decoder.dimensions();
                return collect_frames(decoder.into_frames(), dimensions, shrink);
            }
        }
        ImageFormat::Png | ImageFormat::Jpeg => {}
        _ => return Err(ImageProcessingError::Unsupported),
    }
    let (width, height) = header_dimensions(bytes, format)?;
    check_dimensions(width, height)?;
    let as_processing_error = |error: image::ImageError| match error {
        image::ImageError::Limits(_) => ImageProcessingError::TooLarge,
        _ => ImageProcessingError::Unsupported,
    };
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits());
    let mut decoder = reader.into_decoder().map_err(as_processing_error)?;
    // Only where reading EXIF is bounded: the WebP decoder sizes its EXIF buffer from the declared
    // chunk length, without any limit.
    let orientation = if matches!(format, ImageFormat::Jpeg | ImageFormat::Png) {
        decoder.orientation().map_err(as_processing_error)?
    } else {
        image::metadata::Orientation::NoTransforms
    };
    let mut image = DynamicImage::from_decoder(decoder).map_err(as_processing_error)?;
    image.apply_orientation(orientation);
    Ok(vec![(shrink(image.into_rgba8()), 0)])
}

/// Down to the display size, never up; Lanczos keeps edges sharp.
fn fit(image: RgbaImage, kind: ProfileImageKind) -> RgbaImage {
    let (width, height) = image.dimensions();
    let (maximum_width, maximum_height) = match kind {
        ProfileImageKind::Avatar => (AVATAR_SIDE_PIXELS, AVATAR_SIDE_PIXELS),
        ProfileImageKind::Banner => (BANNER_WIDTH_PIXELS, u32::MAX),
    };
    if width <= maximum_width && height <= maximum_height {
        return image;
    }
    DynamicImage::ImageRgba8(image)
        .resize(maximum_width, maximum_height, FilterType::Lanczos3)
        .into_rgba8()
}

/// The `crop` square, cut at the picture's edges, or the centred square when there is none or it
/// starts outside; then down to `maximum_side`, never up.
fn crop_square_and_shrink(
    image: &RgbaImage,
    crop: Option<SquareCrop>,
    maximum_side: u32,
) -> RgbaImage {
    let (width, height) = image.dimensions();
    let (x, y, size) = match crop {
        Some(crop) if crop.size > 0 && crop.x < width && crop.y < height => {
            let size = crop.size.min(width - crop.x).min(height - crop.y);
            (crop.x, crop.y, size)
        }
        _ => {
            let size = width.min(height);
            ((width - size) / 2, (height - size) / 2, size)
        }
    };
    let cropped = image::imageops::crop_imm(image, x, y, size, size).to_image();
    if size <= maximum_side {
        return cropped;
    }
    image::imageops::resize(&cropped, maximum_side, maximum_side, FilterType::Lanczos3)
}

fn lossless_options() -> EncoderOptions {
    EncoderOptions {
        encoding_config: Some(EncodingConfig {
            encoding_type: EncodingType::Lossless,
            quality: LOSSLESS_EFFORT,
            method: LOSSLESS_METHOD,
        }),
        ..EncoderOptions::default()
    }
}

/// One still or animated lossless WebP from frames of the same size.
fn encode_webp(frames: &[(RgbaImage, u32)]) -> Result<Vec<u8>, ImageProcessingError> {
    let Some((first, _)) = frames.first() else {
        return Err(ImageProcessingError::Encoding);
    };
    let dimensions = first.dimensions();
    let mut encoder = webp_animation::Encoder::new_with_options(dimensions, lossless_options())
        .map_err(|_| ImageProcessingError::Encoding)?;
    let mut timestamp: i32 = 0;
    for (frame, delay) in frames {
        encoder
            .add_frame(frame.as_raw(), timestamp)
            .map_err(|_| ImageProcessingError::Encoding)?;
        timestamp = timestamp.saturating_add(i32::try_from((*delay).max(1)).unwrap_or(i32::MAX));
    }
    let data = encoder
        .finalize(timestamp)
        .map_err(|_| ImageProcessingError::Encoding)?;
    Ok(data.to_vec())
}

/// An avatar or a banner: fitted to its display size and stored as lossless WebP, animated
/// when the source is, with its first frame as a still.
pub(crate) fn process_profile_image(
    bytes: &[u8],
    kind: ProfileImageKind,
) -> Result<ProcessedProfileImage, ImageProcessingError> {
    encode_with_still(&decode_frames(bytes, |frame| fit(frame, kind))?)
}

/// The frames as one WebP, and their first alone when there are several.
fn encode_with_still(
    frames: &[(RgbaImage, u32)],
) -> Result<ProcessedProfileImage, ImageProcessingError> {
    let is_animated = frames.len() > 1;
    let still = if is_animated {
        Some(encode_webp(&frames[..1])?)
    } else {
        None
    };
    Ok(ProcessedProfileImage {
        picture: ProcessedImage {
            bytes: encode_webp(frames)?,
            content_type: "image/webp",
            is_animated,
        },
        still,
    })
}

/// Whether processing may find several frames, from the header alone: only GIF and WebP are kept
/// animated (APNG is read as its first frame).
pub(crate) fn may_be_animated(bytes: &[u8]) -> bool {
    matches!(
        image::guess_format(bytes),
        Ok(ImageFormat::Gif | ImageFormat::WebP)
    )
}

/// A guild or channel icon or a custom emoji: the square kept, fitted to its display size and
/// stored as lossless WebP; a guild's icon and an emoji animated when the source is, with its
/// first frame as a still.
pub(crate) fn process_icon(
    bytes: &[u8],
    kind: IconKind,
    crop: Option<SquareCrop>,
) -> Result<ProcessedProfileImage, ImageProcessingError> {
    let side = match kind {
        IconKind::Guild => GUILD_ICON_SIDE_PIXELS,
        IconKind::Channel => CHANNEL_ICON_SIDE_PIXELS,
        IconKind::Emoji => EMOJI_SIDE_PIXELS,
    };
    let mut frames = decode_frames(bytes, |frame| crop_square_and_shrink(&frame, crop, side))?;
    if kind == IconKind::Channel {
        // shortcut: every frame is decoded and shrunk before the first is kept; stop decoding
        // after it if long GIFs as channel icons hit the processing time budget.
        frames.truncate(1);
    }
    encode_with_still(&frames)
}

/// An attachment's smaller form, same pixels: PNG recompressed, animated GIF as animated WebP.
/// `None` when nothing smaller came out. JPEG goes through `strip_jpeg_metadata` at upload.
pub(crate) fn optimize_attachment(
    bytes: &[u8],
    content_type: &str,
) -> Result<Option<ProcessedImage>, ImageProcessingError> {
    let optimized = match content_type {
        "image/png" => {
            let (width, height) = header_dimensions(bytes, ImageFormat::Png)?;
            check_dimensions(width, height)?;
            let mut options = oxipng::Options::from_preset(PNG_OPTIMIZATION_PRESET);
            options.strip = oxipng::StripChunks::Safe;
            // oxipng inflates to the size the header announces: bounded again on its side.
            options.max_decompressed_size =
                usize::try_from(MAXIMUM_TOTAL_PIXELS * PNG_LARGEST_BYTES_PER_PIXEL).ok();
            options.timeout = Some(PNG_OPTIMIZATION_TIME_BUDGET);
            let bytes = oxipng::optimize_from_memory(bytes, &options)
                .map_err(|_| ImageProcessingError::Encoding)?;
            ProcessedImage {
                bytes,
                content_type: "image/png",
                is_animated: false,
            }
        }
        "image/gif" => {
            let frames = decode_frames(bytes, |frame| frame)?;
            if frames.len() < 2 {
                return Ok(None);
            }
            ProcessedImage {
                bytes: encode_webp(&frames)?,
                content_type: "image/webp",
                is_animated: true,
            }
        }
        _ => return Ok(None),
    };
    Ok((optimized.bytes.len() < bytes.len()).then_some(optimized))
}

const JPEG_START_OF_IMAGE: [u8; 2] = [0xFF, 0xD8];
const MARKER_APP0: u8 = 0xE0;
const MARKER_APP1: u8 = 0xE1;
const MARKER_APP2: u8 = 0xE2;
const MARKER_APP14: u8 = 0xEE;
const MARKER_APP15: u8 = 0xEF;
const MARKER_COMMENT: u8 = 0xFE;
const MARKER_START_OF_SCAN: u8 = 0xDA;
const MARKER_END_OF_IMAGE: u8 = 0xD9;
const MARKER_TEMPORARY: u8 = 0x01;
const ICC_PROFILE_HEADER: &[u8] = b"ICC_PROFILE\0";
const EXIF_HEADER: &[u8] = b"Exif\0\0";
const ORIENTATION_TAG: u16 = 0x0112;

/// Markers with no length that may appear anywhere: restart markers and TEM.
fn is_standalone_marker(marker: u8) -> bool {
    marker == MARKER_TEMPORARY || (0xD0..=0xD7).contains(&marker)
}

/// Metadata segments to drop: every `APPn` but JFIF (APP0), ICC profiles (APP2) and Adobe colour
/// markers (APP14), which colours depend on; and comments. EXIF, XMP, IPTC, MPF and vendor
/// segments hold GPS positions, camera serials and other pictures (depth maps, thumbnails).
fn is_dropped_metadata(marker: u8, payload: &[u8]) -> bool {
    match marker {
        MARKER_APP0 | MARKER_APP14 => false,
        MARKER_APP2 => !payload.starts_with(ICC_PROFILE_HEADER),
        MARKER_APP1..=MARKER_APP15 | MARKER_COMMENT => true,
        _ => false,
    }
}

/// Removes metadata without touching the compressed image, and anything after its end (phones
/// append further pictures there). The EXIF orientation is written back alone, so the photo still
/// shows the right way up. A scan cut short (a truncated file) keeps its data, which holds no
/// marker; any other damage after the first scan ends the file there, and damage before it is
/// refused, since the metadata could not be removed.
pub(crate) fn strip_jpeg_metadata(bytes: &[u8]) -> Result<Vec<u8>, ImageProcessingError> {
    if !bytes.starts_with(&JPEG_START_OF_IMAGE) {
        return Err(ImageProcessingError::Unsupported);
    }
    let mut stripping = JpegStripping {
        output: Vec::with_capacity(bytes.len()),
        position: 2,
        has_passed_first_scan: false,
    };
    stripping.output.extend_from_slice(&JPEG_START_OF_IMAGE);
    match stripping.copy_without_metadata(bytes) {
        Ok(()) => Ok(stripping.output),
        Err(_) if stripping.has_passed_first_scan => Ok(stripping.output),
        Err(error) => Err(error),
    }
}

struct JpegStripping {
    output: Vec<u8>,
    position: usize,
    has_passed_first_scan: bool,
}

impl JpegStripping {
    fn copy_without_metadata(&mut self, bytes: &[u8]) -> Result<(), ImageProcessingError> {
        let mut orientation = None;
        loop {
            let position = self.position;
            if bytes.get(position) != Some(&0xFF) {
                return Err(ImageProcessingError::Unsupported);
            }
            let marker = *bytes
                .get(position + 1)
                .ok_or(ImageProcessingError::Unsupported)?;
            if marker == 0xFF {
                // Fill byte before a marker.
                self.position += 1;
                continue;
            }
            if marker == MARKER_END_OF_IMAGE {
                self.output.extend_from_slice(&[0xFF, MARKER_END_OF_IMAGE]);
                self.position += 2;
                return Ok(());
            }
            if is_standalone_marker(marker) {
                self.output.extend_from_slice(&[0xFF, marker]);
                self.position += 2;
                continue;
            }
            let length_bytes = bytes
                .get(position + 2..position + 4)
                .ok_or(ImageProcessingError::Unsupported)?;
            let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
            if length < 2 {
                return Err(ImageProcessingError::Unsupported);
            }
            let segment = bytes
                .get(position..position + 2 + length)
                .ok_or(ImageProcessingError::Unsupported)?;
            let payload = &segment[4..];
            if marker == MARKER_APP1 {
                orientation = orientation.or_else(|| exif_orientation(payload));
            }
            if marker == MARKER_START_OF_SCAN && !self.has_passed_first_scan {
                if let Some(orientation) = orientation.filter(|value| *value != 1) {
                    self.output
                        .extend_from_slice(&orientation_segment(orientation));
                }
                self.has_passed_first_scan = true;
            }
            if !is_dropped_metadata(marker, payload) {
                self.output.extend_from_slice(segment);
            }
            self.position += 2 + length;
            if marker == MARKER_START_OF_SCAN {
                let Ok(scan_end) = end_of_entropy_coded_data(bytes, self.position) else {
                    // Cut inside the compressed data: it holds no marker, so no metadata.
                    self.output.extend_from_slice(&bytes[self.position..]);
                    return Ok(());
                };
                self.output
                    .extend_from_slice(&bytes[self.position..scan_end]);
                self.position = scan_end;
            }
        }
    }
}

/// Where the compressed data after a scan header stops: the first `0xFF` that starts a real
/// marker (`0xFF 0x00` is an escaped byte; restart markers belong to the data).
fn end_of_entropy_coded_data(bytes: &[u8], start: usize) -> Result<usize, ImageProcessingError> {
    let mut position = start;
    loop {
        let offset = bytes
            .get(position..)
            .and_then(|rest| rest.iter().position(|byte| *byte == 0xFF))
            .ok_or(ImageProcessingError::Unsupported)?;
        position += offset;
        match bytes.get(position + 1) {
            Some(0x00 | 0xD0..=0xD7) => position += 2,
            Some(_) => return Ok(position),
            None => return Err(ImageProcessingError::Unsupported),
        }
    }
}

/// The orientation of an APP1 payload, if it is EXIF and holds one in IFD0.
fn exif_orientation(payload: &[u8]) -> Option<u16> {
    let tiff = payload.strip_prefix(EXIF_HEADER)?;
    let is_little_endian = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let read_u16 = |offset: usize| -> Option<u16> {
        let pair = [*tiff.get(offset)?, *tiff.get(offset + 1)?];
        Some(if is_little_endian {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        })
    };
    let read_u32 = |offset: usize| -> Option<u32> {
        let quad = [
            *tiff.get(offset)?,
            *tiff.get(offset + 1)?,
            *tiff.get(offset + 2)?,
            *tiff.get(offset + 3)?,
        ];
        Some(if is_little_endian {
            u32::from_le_bytes(quad)
        } else {
            u32::from_be_bytes(quad)
        })
    };
    let ifd = usize::try_from(read_u32(4)?).ok()?;
    let entries = usize::from(read_u16(ifd)?);
    (0..entries).find_map(|index| {
        let entry = ifd + 2 + index * 12;
        (read_u16(entry)? == ORIENTATION_TAG)
            .then(|| read_u16(entry + 8))
            .flatten()
            .filter(|value| (1..=8).contains(value))
    })
}

/// A minimal APP1 EXIF segment holding only the orientation.
fn orientation_segment(orientation: u16) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(EXIF_HEADER);
    payload.extend_from_slice(b"II");
    payload.extend_from_slice(&42u16.to_le_bytes());
    payload.extend_from_slice(&8u32.to_le_bytes());
    payload.extend_from_slice(&1u16.to_le_bytes());
    payload.extend_from_slice(&ORIENTATION_TAG.to_le_bytes());
    payload.extend_from_slice(&3u16.to_le_bytes()); // SHORT
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&orientation.to_le_bytes());
    payload.extend_from_slice(&[0, 0]);
    payload.extend_from_slice(&0u32.to_le_bytes());
    let mut segment = vec![0xFF, MARKER_APP1];
    // A few dozen bytes: always fits the 16-bit length.
    let length = u16::try_from(payload.len() + 2).unwrap_or(u16::MAX);
    segment.extend_from_slice(&length.to_be_bytes());
    segment.extend_from_slice(&payload);
    segment
}

#[cfg(test)]
pub(crate) mod tests {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame, ImageEncoder as _, Rgba};

    use super::*;

    pub(crate) fn animated_gif(width: u32, height: u32, frames: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder.set_repeat(Repeat::Infinite).unwrap();
            for index in 0..frames {
                let image = RgbaImage::from_fn(width, height, |x, y| {
                    let shade = u8::try_from((x + y + u32::from(index) * 8) % 256).unwrap();
                    Rgba([shade, 255 - shade, index.wrapping_mul(30), 255])
                });
                encoder
                    .encode_frame(Frame::from_parts(
                        image,
                        0,
                        0,
                        Delay::from_numer_denom_ms(80, 1),
                    ))
                    .unwrap();
            }
        }
        bytes
    }

    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        let image = RgbaImage::from_fn(width, height, |x, _| {
            Rgba([u8::try_from(x % 256).unwrap(), 40, 90, 255])
        });
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(
                image.as_raw(),
                width,
                height,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn an_animated_gif_avatar_stays_animated_fits_512_and_shrinks() {
        let gif = animated_gif(600, 600, 6);
        let avatar = process_profile_image(&gif, ProfileImageKind::Avatar).unwrap();
        assert!(avatar.picture.is_animated);
        assert_eq!(avatar.picture.content_type, "image/webp");
        assert!(
            avatar.picture.bytes.len() < gif.len(),
            "{} < {}",
            avatar.picture.bytes.len(),
            gif.len()
        );
        let decoder = webp_animation::Decoder::new(&avatar.picture.bytes).unwrap();
        assert_eq!(decoder.dimensions(), (512, 512));
        assert_eq!(decoder.into_iter().count(), 6);
    }

    #[test]
    fn icons_keep_the_centred_square_unless_told_otherwise() {
        // Columns are shaded by x, so the kept square shows in its first pixel.
        let wide = RgbaImage::from_fn(300, 100, |x, _| {
            Rgba([u8::try_from(x % 256).unwrap(), 40, 90, 255])
        });
        let centred = crop_square_and_shrink(&wide, None, 256);
        assert_eq!(centred.dimensions(), (100, 100), "never scaled up");
        assert_eq!(centred.get_pixel(0, 0)[0], 100);

        let chosen = Some(SquareCrop {
            x: 20,
            y: 0,
            size: 100,
        });
        assert_eq!(
            crop_square_and_shrink(&wide, chosen, 256).get_pixel(0, 0)[0],
            20
        );

        let past_the_edge = Some(SquareCrop {
            x: 250,
            y: 10,
            size: 100,
        });
        assert_eq!(
            crop_square_and_shrink(&wide, past_the_edge, 256).dimensions(),
            (50, 50)
        );

        for outside in [
            SquareCrop {
                x: 300,
                y: 0,
                size: 10,
            },
            SquareCrop {
                x: 0,
                y: 0,
                size: 0,
            },
        ] {
            assert_eq!(crop_square_and_shrink(&wide, Some(outside), 256), centred);
        }
    }

    #[test]
    fn guild_icons_stay_animated_and_channel_icons_keep_one_frame_both_small() {
        let gif = animated_gif(600, 400, 4);
        let guild = process_icon(&gif, IconKind::Guild, None).unwrap();
        assert!(guild.picture.is_animated);
        assert!(guild.still.is_some());
        let decoder = webp_animation::Decoder::new(&guild.picture.bytes).unwrap();
        assert_eq!(decoder.dimensions(), (256, 256));
        assert_eq!(decoder.into_iter().count(), 4);

        let channel = process_icon(&gif, IconKind::Channel, None).unwrap();
        assert!(!channel.picture.is_animated);
        assert!(channel.still.is_none());
        let channel_picture = image::load_from_memory(&channel.picture.bytes).unwrap();
        assert_eq!(
            (channel_picture.width(), channel_picture.height()),
            (96, 96)
        );

        let emoji = process_icon(&gif, IconKind::Emoji, None).unwrap();
        assert!(emoji.picture.is_animated);
        assert!(emoji.still.is_some());
        let decoder = webp_animation::Decoder::new(&emoji.picture.bytes).unwrap();
        assert_eq!(decoder.dimensions(), (128, 128));
    }

    #[test]
    fn a_small_still_avatar_is_kept_pixel_for_pixel() {
        let source = png(64, 40);
        let avatar = process_profile_image(&source, ProfileImageKind::Avatar).unwrap();
        assert!(!avatar.picture.is_animated);
        let original = image::load_from_memory(&source).unwrap().to_rgba8();
        let encoded = image::load_from_memory(&avatar.picture.bytes)
            .unwrap()
            .to_rgba8();
        assert_eq!(original, encoded, "lossless");
    }

    #[test]
    fn banners_keep_their_ratio_and_attachments_keep_their_pixels() {
        let banner = process_profile_image(&png(3_840, 1_080), ProfileImageKind::Banner).unwrap();
        let decoded = image::load_from_memory(&banner.picture.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1_920, 540));

        let source = png(300, 300);
        if let Some(optimized) = optimize_attachment(&source, "image/png").unwrap() {
            assert_eq!(
                image::load_from_memory(&optimized.bytes)
                    .unwrap()
                    .to_rgba8(),
                image::load_from_memory(&source).unwrap().to_rgba8()
            );
        }
        let gif = animated_gif(120, 80, 4);
        let as_webp = optimize_attachment(&gif, "image/gif").unwrap().unwrap();
        assert_eq!(as_webp.content_type, "image/webp");
        assert!(
            optimize_attachment(b"just text", "application/octet-stream")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn jpeg_metadata_goes_but_orientation_and_the_image_stay() {
        // SOI, APP1 EXIF with orientation 6 and a GPS-looking comment, COM, SOS, data, EOI.
        let mut exif = EXIF_HEADER.to_vec();
        exif.extend_from_slice(
            b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0",
        );
        exif.extend_from_slice(b"GPS 48.85N 2.35E");
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, MARKER_APP1];
        jpeg.extend_from_slice(&u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
        jpeg.extend_from_slice(&exif);
        jpeg.extend_from_slice(&[0xFF, MARKER_COMMENT, 0x00, 0x07]);
        jpeg.extend_from_slice(b"hello");
        jpeg.extend_from_slice(&[
            0xFF,
            MARKER_START_OF_SCAN,
            0x00,
            0x02,
            0x12,
            0x34,
            0xFF,
            0xD9,
        ]);

        let stripped = strip_jpeg_metadata(&jpeg).unwrap();
        assert!(!stripped.windows(3).any(|window| window == b"GPS"));
        assert!(!stripped.windows(5).any(|window| window == b"hello"));
        assert!(stripped.ends_with(&[
            0xFF,
            MARKER_START_OF_SCAN,
            0x00,
            0x02,
            0x12,
            0x34,
            0xFF,
            0xD9
        ]));
        let kept = strip_jpeg_metadata(&stripped).unwrap();
        assert_eq!(kept, stripped, "the orientation survives a second pass");
        assert_eq!(exif_orientation(&orientation_segment(6)[4..]), Some(6));
        assert!(
            strip_jpeg_metadata(b"\xFF\xD8\xFF\xE1\xFF\xFF").is_err(),
            "truncated"
        );
    }

    #[test]
    fn a_truncated_gif_keeps_the_frames_before_the_cut() {
        let gif = animated_gif(32, 32, 3);
        // Cut inside the last frame's data: its trailer and end are gone.
        let truncated = &gif[..gif.len() - 40];
        let avatar = process_profile_image(truncated, ProfileImageKind::Avatar).unwrap();
        assert!(avatar.picture.is_animated, "two whole frames remain");
    }

    #[test]
    fn oversized_or_unreadable_pictures_are_left_alone() {
        assert_eq!(
            process_profile_image(b"<svg/>", ProfileImageKind::Avatar),
            Err(ImageProcessingError::Unsupported)
        );
        let huge = png(MAXIMUM_SIDE_PIXELS + 1, 1);
        assert_eq!(
            process_profile_image(&huge, ProfileImageKind::Avatar),
            Err(ImageProcessingError::TooLarge)
        );
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn png_chunk(kind: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut chunk = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
        let mut checked = kind.to_vec();
        checked.extend_from_slice(payload);
        chunk.extend_from_slice(&checked);
        chunk.extend_from_slice(&crc32(&checked).to_be_bytes());
        chunk
    }

    #[test]
    fn a_png_announcing_huge_dimensions_is_refused_before_any_allocation() {
        // 200 000 × 200 000 RGBA in about 70 bytes: inflating it would need 160 GB.
        let mut header = Vec::new();
        header.extend_from_slice(&200_000u32.to_be_bytes());
        header.extend_from_slice(&200_000u32.to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut bomb = b"\x89PNG\r\n\x1a\n".to_vec();
        bomb.extend_from_slice(&png_chunk(*b"IHDR", &header));
        bomb.extend_from_slice(&png_chunk(
            *b"IDAT",
            &[0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
        ));
        bomb.extend_from_slice(&png_chunk(*b"IEND", &[]));

        assert_eq!(
            optimize_attachment(&bomb, "image/png"),
            Err(ImageProcessingError::TooLarge)
        );
        assert_eq!(
            process_profile_image(&bomb, ProfileImageKind::Avatar),
            Err(ImageProcessingError::TooLarge)
        );
    }

    #[test]
    fn a_rotated_phone_photo_makes_an_upright_avatar() {
        let image = RgbaImage::from_fn(40, 20, |x, _| {
            Rgba([u8::try_from(x * 6).unwrap(), 0, 0, 255])
        });
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .write_image(
                &DynamicImage::ImageRgba8(image).into_rgb8(),
                40,
                20,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        // Orientation 6: the camera was turned a quarter; viewers rotate it upright.
        let mut rotated = jpeg[..2].to_vec();
        rotated.extend_from_slice(&orientation_segment(6));
        rotated.extend_from_slice(&jpeg[2..]);

        let avatar = process_profile_image(&rotated, ProfileImageKind::Avatar).unwrap();
        let decoded = image::load_from_memory(&avatar.picture.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (20, 40));
    }

    #[test]
    fn an_animated_webp_avatar_stays_animated() {
        let animated_webp =
            process_profile_image(&animated_gif(64, 64, 3), ProfileImageKind::Avatar)
                .unwrap()
                .picture
                .bytes;
        let avatar = process_profile_image(&animated_webp, ProfileImageKind::Avatar).unwrap();
        assert!(avatar.picture.is_animated);
        assert!(avatar.still.is_some());
    }

    #[test]
    fn jpeg_stripping_drops_trailers_and_other_pictures_and_never_panics() {
        let scan = [
            0xFF,
            MARKER_START_OF_SCAN,
            0x00,
            0x02,
            0x12,
            0xFF,
            0x00,
            0xFF,
            0xD0,
            0x34,
        ];
        let mut jpeg = vec![0xFF, 0xD8];
        let icc = [
            &[0xFF, MARKER_APP2, 0x00, 0x11][..],
            ICC_PROFILE_HEADER,
            b"abc",
        ]
        .concat();
        let mpf = [&[0xFF, MARKER_APP2, 0x00, 0x08][..], b"MPF\0xy"].concat();
        jpeg.extend_from_slice(&icc);
        jpeg.extend_from_slice(&mpf);
        jpeg.extend_from_slice(&scan);
        jpeg.extend_from_slice(&[0xFF, MARKER_END_OF_IMAGE]);
        jpeg.extend_from_slice(b"\xFF\xD8 second picture with GPS");

        let stripped = strip_jpeg_metadata(&jpeg).unwrap();
        let expected = [&[0xFF, 0xD8][..], &icc, &scan, &[0xFF, MARKER_END_OF_IMAGE]].concat();
        assert_eq!(stripped, expected);
        assert_eq!(
            strip_jpeg_metadata(b"\xFF\xD8\xFF\xE1\x00\x00\xFF\xDA"),
            Err(ImageProcessingError::Unsupported),
            "a segment length under 2"
        );
    }

    #[test]
    fn a_still_webp_declaring_a_huge_exif_chunk_is_not_read_for_its_orientation() {
        // A real 1×1 lossless WebP, rewrapped as VP8X with an EXIF chunk announcing ~4 GiB that
        // is not there: reading it would allocate the announced size.
        let simple =
            encode_webp(&[(RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255])), 0)]).unwrap();
        let lossless_chunk = &simple[12..];
        let mut extended_header = b"VP8X".to_vec();
        extended_header.extend_from_slice(&10u32.to_le_bytes());
        extended_header.extend_from_slice(&[0x08, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut body = b"WEBP".to_vec();
        body.extend_from_slice(&extended_header);
        body.extend_from_slice(lossless_chunk);
        body.extend_from_slice(b"EXIF");
        body.extend_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        let mut forged = b"RIFF".to_vec();
        forged.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        forged.extend_from_slice(&body);

        let avatar = process_profile_image(&forged, ProfileImageKind::Avatar).unwrap();
        assert!(!avatar.picture.is_animated);
    }

    #[test]
    fn gif_frames_outside_the_canvas_are_refused_before_decoding() {
        let mut gif = animated_gif(64, 64, 2);
        // Canvas shrunk to 1×1: the 64×64 frames now lie outside it.
        gif[6..10].copy_from_slice(&[1, 0, 1, 0]);
        assert_eq!(
            process_profile_image(&gif, ProfileImageKind::Avatar),
            Err(ImageProcessingError::TooLarge)
        );
        assert_eq!(
            optimize_attachment(&gif, "image/gif"),
            Err(ImageProcessingError::TooLarge)
        );
    }

    #[test]
    fn a_photo_cut_after_its_metadata_still_loses_it() {
        let mut exif = EXIF_HEADER.to_vec();
        exif.extend_from_slice(b"MM\0\x2a\0\0\0\x08\0\0GPS 48.85N 2.35E");
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, MARKER_APP1];
        jpeg.extend_from_slice(&u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
        jpeg.extend_from_slice(&exif);
        // A scan whose data stops short: no end-of-image marker.
        jpeg.extend_from_slice(&[0xFF, MARKER_START_OF_SCAN, 0x00, 0x02, 0x12, 0x34, 0x56]);

        let stripped = strip_jpeg_metadata(&jpeg).unwrap();
        assert!(!stripped.windows(3).any(|window| window == b"GPS"));
        assert!(
            stripped.ends_with(&[0x12, 0x34, 0x56]),
            "the image data is kept"
        );
    }

    /// `cargo test --release -p mellow-server image_processing_timing -- --ignored --nocapture`
    #[test]
    #[ignore = "measurement, not a check"]
    #[allow(clippy::print_stdout)]
    fn image_processing_timing() {
        let gif = animated_gif(480, 480, 60);
        let started = std::time::Instant::now();
        let avatar = process_profile_image(&gif, ProfileImageKind::Avatar).unwrap();
        println!(
            "animated avatar: {} KB GIF -> {} KB WebP in {:?}",
            gif.len() / 1024,
            avatar.picture.bytes.len() / 1024,
            started.elapsed()
        );
        let source = png(1_600, 1_200);
        let started = std::time::Instant::now();
        let optimized = optimize_attachment(&source, "image/png").unwrap();
        println!(
            "png attachment: {} KB -> {:?} KB in {:?}",
            source.len() / 1024,
            optimized.map(|image| image.bytes.len() / 1024),
            started.elapsed()
        );
    }
}
