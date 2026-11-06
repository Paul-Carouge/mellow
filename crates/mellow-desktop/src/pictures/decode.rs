//! Bytes to the BGRA frames GPUI draws: AVIF through `avif`, GIF, JPEG, PNG and WebP (animated
//! too) through `image`, each frame scaled down to the size it is shown at. Runs off the main
//! thread: a 1080p AVIF takes milliseconds, far more than a frame's budget.

use std::io::Cursor;

use image::codecs::gif::GifDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::{self, FilterType};
use image::{
    AnimationDecoder, Delay, DynamicImage, Frame, ImageDecoder, ImageFormat, ImageReader, Limits,
    Rgba, RgbaImage,
};

use super::avif::{decode_avif, looks_like_avif};
use super::colour::convert_rgba_to_srgb_with_profile;
use super::{Animation, DisplaySize, PictureDecodeError};

/// The server decodes uploads up to 24 MP (ADR-0033) and keeps larger attachments as uploaded;
/// a little above that, nothing an honest instance serves is refused.
const MAXIMUM_DECODED_PIXELS: u32 = 25_000_000;
/// What one decoding may allocate in `image`, a 25 MP picture with room for its decoder.
const MAXIMUM_DECODER_ALLOCATION_BYTES: u64 = 256 * 1024 * 1024;
/// Frames past this are dropped: the animation then loops early instead of using unbounded memory.
const MAXIMUM_ANIMATION_FRAMES: usize = 500;
/// The scaled-down frames of one animation, together; past it the animation stops early too.
const MAXIMUM_ANIMATION_BYTES: usize = 128 * 1024 * 1024;

fn decoder_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAXIMUM_DECODER_ALLOCATION_BYTES);
    limits
}

/// The size `width` × `height` is drawn at to cover `display`, never larger than it is.
fn scaled_size(width: u32, height: u32, display: DisplaySize) -> (u32, u32) {
    if width == 0 || height == 0 || display.width == 0 || display.height == 0 {
        return (width, height);
    }
    let scale = (f64::from(display.width) / f64::from(width))
        .max(f64::from(display.height) / f64::from(height));
    if scale >= 1.0 {
        return (width, height);
    }
    // Both products are below the original sizes, so they fit in u32.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let scaled = |side: u32| ((f64::from(side) * scale).round() as u32).max(1);
    (scaled(width), scaled(height))
}

/// Scales `picture` down for `display` and swaps it to GPUI's BGRA order.
fn bgra_for_display(picture: RgbaImage, display: DisplaySize) -> RgbaImage {
    let (width, height) = picture.dimensions();
    let (scaled_width, scaled_height) = scaled_size(width, height, display);
    let mut picture = if (scaled_width, scaled_height) == (width, height) {
        picture
    } else {
        imageops::resize(&picture, scaled_width, scaled_height, FilterType::Triangle)
    };
    for pixel in picture.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    picture
}

fn frame_byte_count(frame: &RgbaImage) -> usize {
    frame.as_raw().len()
}

fn animation_frames<'a>(
    frames: impl Iterator<Item = image::ImageResult<Frame>> + 'a,
    animation: Animation,
    display: DisplaySize,
) -> Result<Vec<Frame>, PictureDecodeError> {
    let frame_limit = match animation {
        Animation::Animate => MAXIMUM_ANIMATION_FRAMES,
        Animation::StillFrame => 1,
    };
    let mut decoded = Vec::new();
    let mut byte_count = 0;
    for frame in frames.take(frame_limit) {
        // A broken frame ends the animation; the frames before it still play.
        let Ok(frame) = frame else {
            break;
        };
        let delay = frame.delay();
        let picture = bgra_for_display(frame.into_buffer(), display);
        byte_count += frame_byte_count(&picture);
        if byte_count > MAXIMUM_ANIMATION_BYTES && !decoded.is_empty() {
            break;
        }
        decoded.push(Frame::from_parts(picture, 0, 0, delay));
    }
    if decoded.is_empty() {
        return Err(PictureDecodeError::Undecodable);
    }
    Ok(decoded)
}

fn still_frame(picture: RgbaImage, display: DisplaySize) -> Vec<Frame> {
    vec![Frame::from_parts(
        bgra_for_display(picture, display),
        0,
        0,
        Delay::from_numer_denom_ms(0, 1),
    )]
}

fn decode_still_raster(bytes: &[u8]) -> Result<RgbaImage, PictureDecodeError> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| PictureDecodeError::Undecodable)?;
    reader.limits(decoder_limits());
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| PictureDecodeError::Undecodable)?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > u64::from(MAXIMUM_DECODED_PIXELS) {
        return Err(PictureDecodeError::TooLarge);
    }
    let icc_profile = decoder.icc_profile().ok().flatten();
    let mut picture = DynamicImage::from_decoder(decoder)
        .map_err(|_| PictureDecodeError::Undecodable)?
        .into_rgba8();
    if let Some(icc_profile) = icc_profile {
        convert_rgba_to_srgb_with_profile(&icc_profile, &mut picture);
    }
    Ok(picture)
}

/// Decodes `bytes` into frames for GPUI: one for a still picture or `Animation::StillFrame`, up to
/// the frame and memory caps for an animation.
pub(crate) fn decode_frames(
    bytes: &[u8],
    animation: Animation,
    display: DisplaySize,
) -> Result<Vec<Frame>, PictureDecodeError> {
    if looks_like_avif(bytes) {
        return Ok(still_frame(
            decode_avif(bytes, MAXIMUM_DECODED_PIXELS)?,
            display,
        ));
    }
    match image::guess_format(bytes) {
        Ok(ImageFormat::Gif) => {
            let mut decoder =
                GifDecoder::new(Cursor::new(bytes)).map_err(|_| PictureDecodeError::Undecodable)?;
            decoder
                .set_limits(decoder_limits())
                .map_err(|_| PictureDecodeError::TooLarge)?;
            let (width, height) = decoder.dimensions();
            if u64::from(width) * u64::from(height) > u64::from(MAXIMUM_DECODED_PIXELS) {
                return Err(PictureDecodeError::TooLarge);
            }
            animation_frames(decoder.into_frames(), animation, display)
        }
        Ok(ImageFormat::WebP) => {
            let mut decoder = WebPDecoder::new(Cursor::new(bytes))
                .map_err(|_| PictureDecodeError::Undecodable)?;
            if !decoder.has_animation() {
                return Ok(still_frame(decode_still_raster(bytes)?, display));
            }
            decoder
                .set_limits(decoder_limits())
                .map_err(|_| PictureDecodeError::TooLarge)?;
            let (width, height) = decoder.dimensions();
            if u64::from(width) * u64::from(height) > u64::from(MAXIMUM_DECODED_PIXELS) {
                return Err(PictureDecodeError::TooLarge);
            }
            // Transparent where no frame has drawn yet, as browsers show it.
            decoder
                .set_background_color(Rgba([0, 0, 0, 0]))
                .map_err(|_| PictureDecodeError::Undecodable)?;
            animation_frames(decoder.into_frames(), animation, display)
        }
        Ok(ImageFormat::Png | ImageFormat::Jpeg) => {
            Ok(still_frame(decode_still_raster(bytes)?, display))
        }
        _ => Err(PictureDecodeError::Undecodable),
    }
}

#[cfg(test)]
mod tests {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{ImageEncoder as _, codecs::png::PngEncoder};

    use super::*;

    const LARGE_DISPLAY: DisplaySize = DisplaySize {
        width: 1_000,
        height: 1_000,
    };

    fn gif_of(frame_count: u8, side: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder.set_repeat(Repeat::Infinite).unwrap();
            for index in 0..frame_count {
                let picture = RgbaImage::from_pixel(side, side, Rgba([index, 0, 200, 255]));
                encoder
                    .encode_frame(Frame::from_parts(
                        picture,
                        0,
                        0,
                        Delay::from_numer_denom_ms(50, 1),
                    ))
                    .unwrap();
            }
        }
        bytes
    }

    fn png_of(picture: &RgbaImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(
                picture.as_raw(),
                picture.width(),
                picture.height(),
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn pictures_are_scaled_to_cover_the_display_and_never_enlarged() {
        let display = DisplaySize {
            width: 100,
            height: 50,
        };
        assert_eq!(scaled_size(400, 400, display), (100, 100));
        assert_eq!(scaled_size(1_000, 100, display), (500, 50));
        assert_eq!(scaled_size(80, 40, display), (80, 40));
    }

    #[test]
    fn a_still_png_arrives_as_one_bgra_frame_at_the_displayed_size() {
        let picture = RgbaImage::from_pixel(64, 64, Rgba([10, 20, 30, 255]));
        let frames = decode_frames(
            &png_of(&picture),
            Animation::Animate,
            DisplaySize {
                width: 32,
                height: 32,
            },
        )
        .unwrap();
        assert_eq!(frames.len(), 1);
        let buffer = frames[0].buffer();
        assert_eq!(buffer.dimensions(), (32, 32));
        assert_eq!(buffer.get_pixel(5, 5).0, [30, 20, 10, 255], "blue first");
    }

    #[test]
    fn an_animated_gif_keeps_its_frames_and_delays() {
        let frames = decode_frames(&gif_of(3, 8), Animation::Animate, LARGE_DISPLAY).unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[1].delay(), Delay::from_numer_denom_ms(50, 1));
        assert_eq!(
            frames[2].buffer().get_pixel(0, 0).0[2],
            2,
            "frame index in red, now third"
        );
    }

    #[test]
    fn the_still_mode_decodes_only_the_first_frame() {
        let frames = decode_frames(&gif_of(3, 8), Animation::StillFrame, LARGE_DISPLAY).unwrap();
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn an_animation_stops_at_the_frame_cap() {
        let many = std::iter::repeat_with(|| {
            Ok(Frame::new(RgbaImage::from_pixel(
                1,
                1,
                Rgba([0, 0, 0, 255]),
            )))
        })
        .take(MAXIMUM_ANIMATION_FRAMES + 10);
        let capped = animation_frames(many, Animation::Animate, LARGE_DISPLAY).unwrap();
        assert_eq!(capped.len(), MAXIMUM_ANIMATION_FRAMES);
    }

    #[test]
    fn an_animation_stops_at_the_memory_cap() {
        // 4 MiB frames: the cap holds 32 of them.
        let side = 1_024;
        let frames = std::iter::repeat_with(|| Ok(Frame::new(RgbaImage::new(side, side)))).take(40);
        let display = DisplaySize {
            width: side,
            height: side,
        };
        let capped = animation_frames(frames, Animation::Animate, display).unwrap();
        assert_eq!(
            capped.len(),
            MAXIMUM_ANIMATION_BYTES / (side as usize * side as usize * 4)
        );
    }

    #[test]
    fn unknown_bytes_are_refused() {
        assert!(decode_frames(b"not a picture at all", Animation::Animate, LARGE_DISPLAY).is_err());
    }
}
