//! Whether a compressed picture looks like its source: SSIMULACRA2 (ADR-0033), whose authors read
//! 85 as "not noticeable in an in-place comparison at 1:1".

use image::{GenericImageView as _, Rgba, RgbaImage};
use ssimulacra2::{ColorPrimaries, Rgb, TransferCharacteristic, compute_frame_ssimulacra2};

/// The lowest score stored: 90 ("not noticeable in a flicker test") costs about 22 % more space
/// for attachments and 50 % more for banners, against a difference no one sees in normal use.
pub(super) const MINIMUM_SCORE: f64 = 85.0;
/// Large pictures are scored on this many tiles spread over them: the whole of a 15 MP photo
/// takes 14 s and 2.7 GB, 8 tiles about 0.5 s, choosing a quality within ±2 steps of it. Four
/// tiles chose 50 instead of 65 on a high-density screenshot.
const SCORED_TILES: u32 = 8;
const TILE_SIDE: u32 = 512;
/// SSIMULACRA2 needs at least this many pixels a side.
const MINIMUM_SCORED_SIDE: u32 = 8;
/// Transparent pictures are seen over the chat sheet (token `color.surface.chat`, `#221B23`) and
/// over white (light themes, other apps): the worse score counts.
const BACKGROUNDS: [[u8; 3]; 2] = [[0x22, 0x1B, 0x23], [0xFF, 0xFF, 0xFF]];

/// The areas scored, `(x, y, width, height)`: the whole picture when it holds no more tiles than
/// `SCORED_TILES`, else that many tiles evenly spread over its grid of tiles.
fn scored_areas(width: u32, height: u32) -> Vec<(u32, u32, u32, u32)> {
    let (tile_width, tile_height) = (TILE_SIDE.min(width), TILE_SIDE.min(height));
    let columns = width.div_ceil(tile_width.max(1));
    let grid_size = columns * height.div_ceil(tile_height.max(1));
    if grid_size <= SCORED_TILES {
        return vec![(0, 0, width, height)];
    }
    (0..SCORED_TILES)
        .map(|index| {
            // The middle of each of `SCORED_TILES` equal runs of the grid, in reading order.
            let cell = (2 * index + 1) * grid_size / (2 * SCORED_TILES);
            let (column, row) = (cell % columns, cell / columns);
            (
                (column * tile_width).min(width - tile_width),
                (row * tile_height).min(height - tile_height),
                tile_width,
                tile_height,
            )
        })
        .collect()
}

/// An area over `background`, blended in sRGB as browsers do.
fn flattened(
    image: &RgbaImage,
    (x, y, width, height): (u32, u32, u32, u32),
    background: [u8; 3],
) -> Option<Rgb> {
    let pixels = image::imageops::crop_imm(image, x, y, width, height)
        .pixels()
        .map(|(_, _, Rgba([red, green, blue, alpha]))| {
            let opacity = f32::from(alpha) / 255.0;
            let mut blended = [0.0; 3];
            for ((channel, value), behind) in
                blended.iter_mut().zip([red, green, blue]).zip(background)
            {
                *channel =
                    (f32::from(value) * opacity + f32::from(behind) * (1.0 - opacity)) / 255.0;
            }
            blended
        })
        .collect();
    Rgb::new(
        pixels,
        usize::try_from(width).ok()?,
        usize::try_from(height).ok()?,
        TransferCharacteristic::SRGB,
        ColorPrimaries::BT709,
    )
    .ok()
}

/// The worst score of each candidate frame against its reference frame, over the scored areas
/// and, for transparent pictures, both backgrounds. `None` when it cannot be scored (smaller than
/// 8 pixels a side, sizes that differ): only a lossless result passes then.
pub(super) fn worst_score(pairs: &[(&RgbaImage, &RgbaImage)]) -> Option<f64> {
    let mut worst = f64::INFINITY;
    for (reference, candidate) in pairs {
        let (width, height) = reference.dimensions();
        if candidate.dimensions() != (width, height)
            || width < MINIMUM_SCORED_SIDE
            || height < MINIMUM_SCORED_SIDE
        {
            return None;
        }
        let is_transparent = reference.pixels().any(|pixel| pixel[3] != u8::MAX)
            || candidate.pixels().any(|pixel| pixel[3] != u8::MAX);
        let backgrounds = if is_transparent {
            &BACKGROUNDS[..]
        } else {
            // Opaque: the background never shows.
            &BACKGROUNDS[..1]
        };
        for area in scored_areas(width, height) {
            for background in backgrounds {
                let score = compute_frame_ssimulacra2(
                    flattened(reference, area, *background)?,
                    flattened(candidate, area, *background)?,
                )
                .ok()?;
                worst = worst.min(score);
            }
        }
    }
    worst.is_finite().then_some(worst)
}

pub(super) fn passes(pairs: &[(&RgbaImage, &RgbaImage)]) -> bool {
    worst_score(pairs).is_some_and(|score| score >= MINIMUM_SCORE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_pictures_are_scored_whole_and_large_ones_on_eight_tiles_inside_them() {
        assert_eq!(scored_areas(800, 600), vec![(0, 0, 800, 600)]);
        assert_eq!(scored_areas(4_000, 300), vec![(0, 0, 4_000, 300)]);
        assert!(
            scored_areas(8_000, 300)
                .iter()
                .all(|area| area.2 == TILE_SIDE && area.3 == 300),
            "long strips are tiled too"
        );
        let tiles = scored_areas(4_000, 3_000);
        assert_eq!(tiles.len(), 8);
        let mut distinct = tiles.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), 8, "no tile twice");
        assert!(tiles.iter().all(|(x, y, width, height)| x + width <= 4_000
            && y + height <= 3_000
            && (*width, *height) == (TILE_SIDE, TILE_SIDE)));
        assert!(
            tiles.iter().any(|(_, y, _, _)| *y >= 2_000),
            "the bottom is scored"
        );
    }

    #[test]
    fn identical_pictures_pass_and_a_damaged_one_or_a_lost_transparency_does_not() {
        let picture = RgbaImage::from_fn(64, 64, |x, y| {
            Rgba([
                u8::try_from(x * 4).unwrap(),
                u8::try_from(y * 4).unwrap(),
                128,
                255,
            ])
        });
        assert!(passes(&[(&picture, &picture)]));
        let flat = RgbaImage::from_pixel(64, 64, Rgba([128, 128, 128, 255]));
        assert!(!passes(&[(&picture, &flat)]));

        let transparent = RgbaImage::from_pixel(64, 64, Rgba([0x22, 0x1B, 0x23, 0]));
        let opaque_like_the_sheet = RgbaImage::from_pixel(64, 64, Rgba([0x22, 0x1B, 0x23, 255]));
        assert!(
            !passes(&[(&transparent, &opaque_like_the_sheet)]),
            "white shows the difference"
        );
        let tiny = RgbaImage::new(4, 4);
        assert_eq!(worst_score(&[(&tiny, &tiny)]), None);
    }
}
