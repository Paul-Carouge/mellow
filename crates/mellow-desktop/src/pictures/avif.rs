//! Still AVIF, the format the server stores pictures in (ADR-0033): 10-bit 4:4:4 BT.601 full
//! range, Display P3 or sRGB primaries, alpha as a second AV1 item. Other AVIF layouts decode too.

use std::io::Cursor;

use image::RgbaImage;

use super::PictureDecodeError;
use super::av1::{self, Av1Picture, MatrixCoefficients};
use super::colour::{TEN_BIT_MAXIMUM, display_p3_to_srgb_ten_bit, to_eight_bit, to_ten_bit};

/// True when `bytes` start like an ISO-BMFF file of the AVIF brands.
pub(crate) fn looks_like_avif(bytes: &[u8]) -> bool {
    matches!(bytes.get(4..12), Some(brand) if brand == b"ftypavif" || brand == b"ftypavis")
}

/// Kr and Kb luma weights of the matrix.
fn luma_weights(matrix: MatrixCoefficients) -> (f32, f32) {
    match matrix {
        MatrixCoefficients::Bt709 => (0.2126, 0.0722),
        MatrixCoefficients::Bt601 | MatrixCoefficients::Identity => (0.299, 0.114),
    }
}

fn clamp_to_ten_bit(value: f32) -> u16 {
    // Rounded and clamped to 0..=1023 first, so the cast cannot truncate or wrap.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let sample = value.round().clamp(0.0, f32::from(TEN_BIT_MAXIMUM)) as u16;
    sample
}

/// The colour planes as 10-bit RGBA, alpha opaque.
fn ten_bit_rgba(colour: &Av1Picture) -> Result<Vec<u16>, PictureDecodeError> {
    let (luma, blue_difference, red_difference) = match colour.planes.as_slice() {
        [luma, blue, red] => (luma, Some(blue), Some(red)),
        [luma] => (luma, None, None),
        _ => return Err(PictureDecodeError::Undecodable),
    };
    let (red_weight, blue_weight) = luma_weights(colour.matrix);
    let green_weight = 1.0 - red_weight - blue_weight;
    let maximum = f32::from(TEN_BIT_MAXIMUM);
    let chroma_middle = 512.0;
    // Limited range spans 64..=940 for luma and 64..=960 for chroma in 10 bits.
    let (luma_offset, luma_scale, chroma_scale) = if colour.is_full_range {
        (0.0, 1.0, 1.0)
    } else {
        (64.0, maximum / 876.0, maximum / 896.0)
    };
    let (horizontal_shift, vertical_shift) = colour.subsampling.shifts();
    let chroma_width = colour.chroma_width();
    let mut rgba = Vec::with_capacity(colour.width * colour.height * 4);
    for row in 0..colour.height {
        let chroma_row = (row >> vertical_shift) * chroma_width;
        for column in 0..colour.width {
            let sample = |plane: &Vec<u16>, index: usize| {
                plane
                    .get(index)
                    .map_or(0.0, |&value| f32::from(to_ten_bit(value, colour.bit_depth)))
            };
            let luma_value = sample(luma, row * colour.width + column);
            let chroma_index = chroma_row + (column >> horizontal_shift);
            let [red, green, blue] = if let (Some(blue_plane), Some(red_plane)) =
                (blue_difference, red_difference)
            {
                let blue_value = sample(blue_plane, chroma_index);
                let red_value = sample(red_plane, chroma_index);
                if colour.matrix == MatrixCoefficients::Identity {
                    // AV1 stores G, B, R in the Y, U, V planes.
                    [red_value, luma_value, blue_value]
                } else {
                    let luma_value = (luma_value - luma_offset) * luma_scale;
                    let blue_value = (blue_value - chroma_middle) * chroma_scale;
                    let red_value = (red_value - chroma_middle) * chroma_scale;
                    let red = red_value.mul_add(2.0 * (1.0 - red_weight), luma_value);
                    let blue = blue_value.mul_add(2.0 * (1.0 - blue_weight), luma_value);
                    let green = (luma_value - red_weight * red - blue_weight * blue) / green_weight;
                    [red, green, blue]
                }
            } else {
                let grey = (luma_value - luma_offset) * luma_scale;
                [grey, grey, grey]
            };
            rgba.extend([
                clamp_to_ten_bit(red),
                clamp_to_ten_bit(green),
                clamp_to_ten_bit(blue),
                TEN_BIT_MAXIMUM,
            ]);
        }
    }
    Ok(rgba)
}

/// Decodes an AVIF to straight-alpha RGBA in the colour space it was stored in, 10 bits a sample.
pub(crate) fn decode_avif_in_stored_colours(
    bytes: &[u8],
    maximum_pixels: u32,
) -> Result<(Av1Picture, Vec<u16>), PictureDecodeError> {
    let container = avif_parse::read_avif(&mut Cursor::new(bytes))
        .map_err(|_| PictureDecodeError::Undecodable)?;
    let colour = av1::decode_still(&container.primary_item, maximum_pixels)?;
    let mut rgba = ten_bit_rgba(&colour)?;
    if let Some(alpha_item) = &container.alpha_item {
        let alpha = av1::decode_still(alpha_item, maximum_pixels)?;
        let alpha_plane = alpha
            .planes
            .first()
            .ok_or(PictureDecodeError::Undecodable)?;
        if alpha.width != colour.width || alpha.height != colour.height {
            return Err(PictureDecodeError::Undecodable);
        }
        for (pixel, &opacity) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(alpha_plane) {
            let opacity = to_ten_bit(opacity, alpha.bit_depth);
            pixel[3] = opacity;
            if container.premultiplied_alpha && opacity > 0 {
                for channel in pixel.iter_mut().take(3) {
                    let straight =
                        u32::from(*channel) * u32::from(TEN_BIT_MAXIMUM) / u32::from(opacity);
                    *channel = u16::try_from(straight.min(u32::from(TEN_BIT_MAXIMUM)))
                        .unwrap_or(TEN_BIT_MAXIMUM);
                }
            }
        }
    }
    Ok((colour, rgba))
}

/// Decodes an AVIF to 8-bit sRGB RGBA, converting Display P3 colours.
pub(crate) fn decode_avif(
    bytes: &[u8],
    maximum_pixels: u32,
) -> Result<RgbaImage, PictureDecodeError> {
    let (colour, mut rgba) = decode_avif_in_stored_colours(bytes, maximum_pixels)?;
    if colour.is_display_p3 {
        rgba = display_p3_to_srgb_ten_bit(&rgba)?;
    }
    let eight_bit = rgba.into_iter().map(to_eight_bit).collect();
    let width = u32::try_from(colour.width).map_err(|_| PictureDecodeError::TooLarge)?;
    let height = u32::try_from(colour.height).map_err(|_| PictureDecodeError::TooLarge)?;
    RgbaImage::from_raw(width, height, eight_bit).ok_or(PictureDecodeError::Undecodable)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made by the server's own encoder (`crates/mellow-server/src/domain/image_processing/avif.rs`
    /// at d9f77a32, quality 100, Display P3, 10-bit 4:4:4 with alpha) from a 64 × 48 picture of
    /// four patches holding the Display P3 coordinates of sRGB red, green, blue and mid grey, its
    /// first 8 columns transparent. The PNG is the encoder's own reconstruction, the picture any
    /// decoder must show, in Display P3.
    const PATCHES_AVIF: &[u8] = include_bytes!("fixtures/display-p3-ten-bit-patches.avif");
    const PATCHES_RECONSTRUCTION_PNG: &[u8] =
        include_bytes!("fixtures/display-p3-ten-bit-patches.reconstruction.png");
    const ENOUGH_PIXELS: u32 = 1 << 20;

    #[test]
    fn the_servers_avif_decodes_to_the_encoders_own_reconstruction() {
        let (colour, rgba) = decode_avif_in_stored_colours(PATCHES_AVIF, ENOUGH_PIXELS).unwrap();
        assert!(colour.is_display_p3);
        assert_eq!(
            (colour.width, colour.height, colour.bit_depth),
            (64, 48, 10)
        );
        let reconstruction = image::load_from_memory(PATCHES_RECONSTRUCTION_PNG)
            .unwrap()
            .to_rgba8();
        let decoded: Vec<u8> = rgba.into_iter().map(to_eight_bit).collect();
        let largest_difference = decoded
            .iter()
            .zip(reconstruction.as_raw())
            .map(|(decoded, expected)| decoded.abs_diff(*expected))
            .max()
            .unwrap();
        assert!(largest_difference <= 1, "{largest_difference}");
    }

    #[test]
    fn display_p3_patches_come_out_as_the_srgb_primaries_with_their_transparency() {
        let picture = decode_avif(PATCHES_AVIF, ENOUGH_PIXELS).unwrap();
        assert_eq!(picture.dimensions(), (64, 48));
        assert_eq!(
            picture.get_pixel(2, 2)[3],
            0,
            "the first columns stay transparent"
        );
        for ((x, y), expected) in [
            ((20, 10), [255, 0, 0]),
            ((50, 10), [0, 255, 0]),
            ((20, 40), [0, 0, 255]),
            ((50, 40), [128, 128, 128]),
        ] {
            let pixel = picture.get_pixel(x, y);
            assert_eq!(pixel[3], 255);
            for (actual, expected) in pixel.0.iter().zip(expected) {
                assert!(actual.abs_diff(expected) <= 3, "({x}, {y}): {pixel:?}");
            }
        }
    }

    #[test]
    fn a_picture_over_the_pixel_limit_is_refused() {
        assert!(decode_avif(PATCHES_AVIF, 64 * 48 - 1).is_err());
    }

    #[test]
    fn only_avif_brands_are_taken_for_avif() {
        assert!(looks_like_avif(PATCHES_AVIF));
        assert!(!looks_like_avif(PATCHES_RECONSTRUCTION_PNG));
        assert!(!looks_like_avif(b"short"));
    }

    /// Prints how long a picture takes to decode; asserts nothing about time. Run on a release
    /// build: `MELLOW_AVIF_TO_MEASURE=photo.avif cargo test --release -p mellow-desktop
    /// decode_time -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, run by hand on a release build"]
    #[allow(clippy::print_stderr)]
    fn decode_time_of_an_avif() {
        const RUNS: usize = 11;
        let path = std::env::var("MELLOW_AVIF_TO_MEASURE").unwrap();
        let bytes = std::fs::read(path).unwrap();
        let mut durations: Vec<_> = (0..RUNS)
            .map(|_| {
                let start = std::time::Instant::now();
                let picture = decode_avif(&bytes, u32::MAX).unwrap();
                let elapsed = start.elapsed();
                assert!(picture.width() > 0);
                elapsed
            })
            .collect();
        durations.sort();
        let container = avif_parse::read_avif(&mut Cursor::new(&bytes)).unwrap();
        let mut av1_durations: Vec<_> = (0..RUNS)
            .map(|_| {
                let start = std::time::Instant::now();
                av1::decode_still(&container.primary_item, u32::MAX).unwrap();
                start.elapsed()
            })
            .collect();
        av1_durations.sort();
        eprintln!(
            "whole decode: median {:?}, fastest {:?}, slowest {:?} over {RUNS} runs; \
             of which AV1 alone: median {:?}",
            durations[RUNS / 2],
            durations[0],
            durations[RUNS - 1],
            av1_durations[RUNS / 2]
        );
    }
}
