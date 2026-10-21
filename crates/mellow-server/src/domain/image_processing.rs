//! Lossless compression of uploaded pictures (ADR-0021). Pure functions over bytes, run on a
//! blocking thread by the caller: untrusted bytes are decoded in Rust (`image`), within limits
//! checked first; libwebp only ever encodes decoded pixels.

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
use image::imageops::FilterType;
use image::{
    AnimationDecoder as _, DynamicImage, ImageDecoder as _, ImageFormat, ImageReader, Limits,
    RgbaImage,
};
use webp_animation::{EncoderOptions, EncodingConfig, EncodingType};

/// Larger pictures are kept as uploaded and never processed (decompression bombs).
const MAXIMUM_SIDE_PIXELS: u32 = 8_192;
const MAXIMUM_TOTAL_PIXELS: u64 = 64_000_000;
const MAXIMUM_FRAMES: usize = 600;
/// Largest display sizes: nothing is ever shown bigger (ADR-0021).
const AVATAR_SIDE_PIXELS: u32 = 512;
const BANNER_WIDTH_PIXELS: u32 = 1_920;
/// libwebp's effort for lossless encoding: 0 (fast) to 100 (smallest); the pixels are the same
/// whatever the effort. Measured in release on a 480 × 480, 60-frame GIF: 2.4 s at 60/4, 2.2 s at
/// 25/1, 1.5 s at 10/0, decoding included; it runs after the upload is answered.
const LOSSLESS_EFFORT: f32 = 25.0;
const LOSSLESS_METHOD: usize = 1;
/// oxipng's preset: 2 is its fast default, most of the gain of the slow ones.
const PNG_OPTIMIZATION_PRESET: u8 = 2;

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

/// A picture ready to store, with its served type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessedImage {
    pub(crate) bytes: Vec<u8>,
    pub(crate) content_type: &'static str,
    pub(crate) is_animated: bool,
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAXIMUM_SIDE_PIXELS);
    limits.max_image_height = Some(MAXIMUM_SIDE_PIXELS);
    // RGBA, 4 bytes per pixel, for one frame at a time.
    limits.max_alloc = Some(MAXIMUM_TOTAL_PIXELS * 4);
    limits
}

/// The frames of a picture with their duration in milliseconds; one frame for still pictures.
fn decode_frames(bytes: &[u8]) -> Result<Vec<(RgbaImage, u32)>, ImageProcessingError> {
    let format = image::guess_format(bytes).map_err(|_| ImageProcessingError::Unsupported)?;
    if format == ImageFormat::Gif {
        let mut decoder =
            GifDecoder::new(Cursor::new(bytes)).map_err(|_| ImageProcessingError::Unsupported)?;
        decoder
            .set_limits(limits())
            .map_err(|_| ImageProcessingError::TooLarge)?;
        let (width, height) = decoder.dimensions();
        let mut frames = Vec::new();
        let mut total_pixels = 0u64;
        for frame in decoder.into_frames() {
            let frame = frame.map_err(|_| ImageProcessingError::Unsupported)?;
            total_pixels += u64::from(width) * u64::from(height);
            if frames.len() >= MAXIMUM_FRAMES || total_pixels > MAXIMUM_TOTAL_PIXELS {
                return Err(ImageProcessingError::TooLarge);
            }
            let (numerator, denominator) = frame.delay().numer_denom_ms();
            let delay = numerator.checked_div(denominator).unwrap_or(0).max(20);
            frames.push((frame.into_buffer(), delay));
        }
        if frames.is_empty() {
            return Err(ImageProcessingError::Unsupported);
        }
        return Ok(frames);
    }
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err(ImageProcessingError::Unsupported);
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits());
    let image = reader.decode().map_err(|error| match error {
        image::ImageError::Limits(_) => ImageProcessingError::TooLarge,
        _ => ImageProcessingError::Unsupported,
    })?;
    Ok(vec![(image.to_rgba8(), 0)])
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
        .to_rgba8()
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
/// when the source is.
pub(crate) fn process_profile_image(
    bytes: &[u8],
    kind: ProfileImageKind,
) -> Result<ProcessedImage, ImageProcessingError> {
    let frames: Vec<(RgbaImage, u32)> = decode_frames(bytes)?
        .into_iter()
        .map(|(frame, delay)| (fit(frame, kind), delay))
        .collect();
    let is_animated = frames.len() > 1;
    Ok(ProcessedImage {
        bytes: encode_webp(&frames)?,
        content_type: "image/webp",
        is_animated,
    })
}

/// The first frame of an animated WebP made by `process_profile_image`, as a still lossless WebP.
pub(crate) fn first_frame_webp(animated: &[u8]) -> Result<Vec<u8>, ImageProcessingError> {
    let decoder =
        webp_animation::Decoder::new(animated).map_err(|_| ImageProcessingError::Encoding)?;
    let frame = decoder
        .into_iter()
        .next()
        .ok_or(ImageProcessingError::Encoding)?;
    let (width, height) = frame.dimensions();
    let still = RgbaImage::from_raw(width, height, frame.data().to_vec())
        .ok_or(ImageProcessingError::Encoding)?;
    encode_webp(&[(still, 0)])
}

/// An attachment's smaller form, same pixels: PNG recompressed, JPEG without its metadata
/// (orientation kept), animated GIF as animated WebP. `None` when nothing smaller came out.
pub(crate) fn optimize_attachment(
    bytes: &[u8],
    content_type: &str,
) -> Result<Option<ProcessedImage>, ImageProcessingError> {
    let optimized = match content_type {
        "image/png" => {
            let mut options = oxipng::Options::from_preset(PNG_OPTIMIZATION_PRESET);
            options.strip = oxipng::StripChunks::Safe;
            let bytes = oxipng::optimize_from_memory(bytes, &options)
                .map_err(|_| ImageProcessingError::Encoding)?;
            ProcessedImage {
                bytes,
                content_type: "image/png",
                is_animated: false,
            }
        }
        "image/jpeg" => ProcessedImage {
            bytes: strip_jpeg_metadata(bytes)?,
            content_type: "image/jpeg",
            is_animated: false,
        },
        "image/gif" => {
            let frames = decode_frames(bytes)?;
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
const MARKER_APP1: u8 = 0xE1;
const MARKER_APP13: u8 = 0xED;
const MARKER_COMMENT: u8 = 0xFE;
const MARKER_START_OF_SCAN: u8 = 0xDA;
const EXIF_HEADER: &[u8] = b"Exif\0\0";
const ORIENTATION_TAG: u16 = 0x0112;

/// Drops EXIF and XMP (APP1), IPTC (APP13) and comments without touching the compressed image:
/// they hold GPS positions, camera serials and editing history. The EXIF orientation is written
/// back alone, so the photo still shows the right way up. ICC profiles (APP2) and Adobe colour
/// markers (APP14) stay: colours depend on them.
fn strip_jpeg_metadata(bytes: &[u8]) -> Result<Vec<u8>, ImageProcessingError> {
    if !bytes.starts_with(&JPEG_START_OF_IMAGE) {
        return Err(ImageProcessingError::Unsupported);
    }
    let mut output = Vec::with_capacity(bytes.len());
    output.extend_from_slice(&JPEG_START_OF_IMAGE);
    let mut orientation = None;
    let mut position = 2;
    loop {
        if bytes.get(position) != Some(&0xFF) {
            return Err(ImageProcessingError::Unsupported);
        }
        let marker = *bytes
            .get(position + 1)
            .ok_or(ImageProcessingError::Unsupported)?;
        if marker == MARKER_START_OF_SCAN {
            if let Some(orientation) = orientation.filter(|value| *value != 1) {
                output.extend_from_slice(&orientation_segment(orientation));
            }
            // The scan and everything after it are copied as they are.
            output.extend_from_slice(&bytes[position..]);
            return Ok(output);
        }
        let length_bytes = bytes
            .get(position + 2..position + 4)
            .ok_or(ImageProcessingError::Unsupported)?;
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        let segment = bytes
            .get(position..position + 2 + length)
            .ok_or(ImageProcessingError::Unsupported)?;
        let is_metadata = matches!(marker, MARKER_APP1 | MARKER_APP13 | MARKER_COMMENT);
        if marker == MARKER_APP1 {
            orientation = orientation.or_else(|| exif_orientation(&segment[4..]));
        }
        if !is_metadata {
            output.extend_from_slice(segment);
        }
        position += 2 + length;
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
        assert!(avatar.is_animated);
        assert_eq!(avatar.content_type, "image/webp");
        assert!(
            avatar.bytes.len() < gif.len(),
            "{} < {}",
            avatar.bytes.len(),
            gif.len()
        );
        let decoder = webp_animation::Decoder::new(&avatar.bytes).unwrap();
        assert_eq!(decoder.dimensions(), (512, 512));
        assert_eq!(decoder.into_iter().count(), 6);
    }

    #[test]
    fn a_small_still_avatar_is_kept_pixel_for_pixel() {
        let source = png(64, 40);
        let avatar = process_profile_image(&source, ProfileImageKind::Avatar).unwrap();
        assert!(!avatar.is_animated);
        let original = image::load_from_memory(&source).unwrap().to_rgba8();
        let encoded = image::load_from_memory(&avatar.bytes).unwrap().to_rgba8();
        assert_eq!(original, encoded, "lossless");
    }

    #[test]
    fn banners_keep_their_ratio_and_attachments_keep_their_pixels() {
        let banner = process_profile_image(&png(3_840, 1_080), ProfileImageKind::Banner).unwrap();
        let decoded = image::load_from_memory(&banner.bytes).unwrap();
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
            avatar.bytes.len() / 1024,
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
