//! The crop of an icon (`picture-crop.ts`, DESIGN.md §6.54): pure maths over the picture's size,
//! in its own pixels, so the crop dialog only draws what these functions return. The kept part
//! is a square; the instance cuts the same part (`crop_x`, `crop_y`, `crop_size`), and without
//! one keeps the largest centred square.

use image::{ImageDecoder as _, ImageReader, metadata::Orientation};
use mellow_protocol::IconCropQuery;

/// Zooming in further than this shows a handful of pixels: the picture would be a blur.
pub const MAXIMUM_CROP_ZOOM: f32 = 8.0;
/// The smallest side worth keeping, in picture pixels, whatever the zoom allows.
const MINIMUM_CROP_SIDE: f32 = 16.0;
/// Under half a pixel from the centred square counts as untouched.
const UNTOUCHED_TOLERANCE: f32 = 0.5;

/// A picture's size in its own pixels, EXIF orientation applied, as the instance measures it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PictureSize {
    pub width: f32,
    pub height: f32,
}

/// A square of the picture in its pixels; fractional while the person drags, rounded only when
/// sent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropSquare {
    pub x: f32,
    pub y: f32,
    pub side: f32,
}

impl CropSquare {
    fn centre(self) -> (f32, f32) {
        (self.x + self.side / 2.0, self.y + self.side / 2.0)
    }
}

/// What the instance keeps when no crop is sent, and where the dialog starts.
#[must_use]
pub fn largest_centred_square(picture: PictureSize) -> CropSquare {
    let side = picture.width.min(picture.height);
    CropSquare {
        x: (picture.width - side) / 2.0,
        y: (picture.height - side) / 2.0,
        side,
    }
}

/// At least 1 (the largest centred square); a tiny picture cannot be zoomed into.
#[must_use]
pub fn maximum_zoom(picture: PictureSize) -> f32 {
    let side = largest_centred_square(picture).side;
    (side / MINIMUM_CROP_SIDE).clamp(1.0, MAXIMUM_CROP_ZOOM.max(1.0))
}

/// 1 when the crop is the largest square.
#[must_use]
pub fn zoom_of(crop: CropSquare, picture: PictureSize) -> f32 {
    largest_centred_square(picture).side / crop.side
}

/// The nearest crop that stays inside the picture and within the zoom range.
#[must_use]
pub fn clamp_crop(crop: CropSquare, picture: PictureSize) -> CropSquare {
    let largest = largest_centred_square(picture).side;
    let side = crop.side.clamp(largest / maximum_zoom(picture), largest);
    CropSquare {
        x: crop.x.clamp(0.0, picture.width - side),
        y: crop.y.clamp(0.0, picture.height - side),
        side,
    }
}

/// The crop at `zoom`, keeping the picture point `anchor` (the pointer, or the crop's centre)
/// where it is in the frame, as maps and photo viewers zoom.
#[must_use]
pub fn crop_zoomed_to(
    crop: CropSquare,
    picture: PictureSize,
    zoom: f32,
    anchor: Option<(f32, f32)>,
) -> CropSquare {
    let (anchor_x, anchor_y) = anchor.unwrap_or_else(|| crop.centre());
    let side = largest_centred_square(picture).side / zoom.clamp(1.0, maximum_zoom(picture));
    let ratio = side / crop.side;
    clamp_crop(
        CropSquare {
            x: anchor_x - (anchor_x - crop.x) * ratio,
            y: anchor_y - (anchor_y - crop.y) * ratio,
            side,
        },
        picture,
    )
}

/// Moves the crop by a distance in picture pixels; dragging the picture moves it the other way.
#[must_use]
pub fn crop_moved_by(
    crop: CropSquare,
    picture: PictureSize,
    delta_x: f32,
    delta_y: f32,
) -> CropSquare {
    clamp_crop(
        CropSquare {
            x: crop.x + delta_x,
            y: crop.y + delta_y,
            ..crop
        },
        picture,
    )
}

/// Whole pixels for the query, or the empty query for the largest centred square, so an
/// untouched crop leaves the choice to the instance's own centring.
#[must_use]
pub fn crop_to_send(crop: CropSquare, picture: PictureSize) -> IconCropQuery {
    let centred = largest_centred_square(picture);
    let is_centred = (crop.x - centred.x).abs() < UNTOUCHED_TOLERANCE
        && (crop.y - centred.y).abs() < UNTOUCHED_TOLERANCE
        && (crop.side - centred.side).abs() < UNTOUCHED_TOLERANCE;
    if is_centred {
        return IconCropQuery::default();
    }
    // Every value is clamped into the picture, whose sides are positive and fit in u32.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let whole = |value: f32| value.round().max(0.0) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let largest_side = centred.side.floor().max(1.0) as u32;
    let side = whole(crop.side).clamp(1, largest_side);
    let width = whole(picture.width);
    let height = whole(picture.height);
    IconCropQuery {
        crop_x: Some(whole(crop.x).min(width.saturating_sub(side))),
        crop_y: Some(whole(crop.y).min(height.saturating_sub(side))),
        crop_size: Some(side),
    }
}

/// Where to draw the whole picture so that `crop` fills a frame `frame_side` wide: its offset
/// from the frame's top-left corner and its drawn width and height.
#[must_use]
pub fn placement_in_frame(
    crop: CropSquare,
    picture: PictureSize,
    frame_side: f32,
) -> (f32, f32, f32, f32) {
    let scale = frame_side / crop.side;
    (
        -crop.x * scale,
        -crop.y * scale,
        picture.width * scale,
        picture.height * scale,
    )
}

/// The picture's size as the instance measures it: the decoder's size, turned by its EXIF
/// orientation. Reads the header only. `None` for what cannot be decoded.
#[must_use]
pub fn picture_size_of(bytes: &[u8]) -> Option<PictureSize> {
    let mut decoder = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let is_turned_a_quarter = matches!(
        decoder.orientation().unwrap_or(Orientation::NoTransforms),
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    );
    // Pictures are at most 24 MP (ADR-0033): every side is exact in an f32.
    #[allow(clippy::cast_precision_loss)]
    let (width, height) = (width as f32, height as f32);
    Some(if is_turned_a_quarter {
        PictureSize {
            width: height,
            height: width,
        }
    } else {
        PictureSize { width, height }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: PictureSize = PictureSize {
        width: 400.0,
        height: 200.0,
    };

    #[test]
    fn the_crop_starts_on_the_largest_centred_square_and_sends_nothing() {
        let centred = largest_centred_square(WIDE);
        assert_eq!(
            centred,
            CropSquare {
                x: 100.0,
                y: 0.0,
                side: 200.0
            }
        );
        assert_eq!(crop_to_send(centred, WIDE), IconCropQuery::default());
        assert!((zoom_of(centred, WIDE) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_moved_crop_stays_inside_the_picture_and_is_sent_in_whole_pixels() {
        let moved = crop_moved_by(largest_centred_square(WIDE), WIDE, 250.0, 30.0);
        assert_eq!(
            moved,
            CropSquare {
                x: 200.0,
                y: 0.0,
                side: 200.0
            }
        );
        assert_eq!(
            crop_to_send(moved, WIDE),
            IconCropQuery {
                crop_x: Some(200),
                crop_y: Some(0),
                crop_size: Some(200)
            }
        );
    }

    #[test]
    fn zooming_keeps_the_anchor_in_place_and_stops_at_the_limits() {
        let centred = largest_centred_square(WIDE);
        let zoomed = crop_zoomed_to(centred, WIDE, 2.0, None);
        assert_eq!(
            zoomed,
            CropSquare {
                x: 150.0,
                y: 50.0,
                side: 100.0
            }
        );
        // 200 px is 12.5 times the 16 px floor: the limit is that, under the 8× cap.
        assert!((maximum_zoom(WIDE) - MAXIMUM_CROP_ZOOM).abs() < f32::EPSILON);
        let deepest = crop_zoomed_to(centred, WIDE, 100.0, None);
        assert!((deepest.side - 25.0).abs() < f32::EPSILON);
        let widest = crop_zoomed_to(zoomed, WIDE, 0.1, None);
        assert!((widest.side - 200.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_tiny_picture_cannot_be_zoomed_into() {
        let tiny = PictureSize {
            width: 10.0,
            height: 12.0,
        };
        assert!((maximum_zoom(tiny) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_whole_picture_is_drawn_so_the_crop_fills_the_frame() {
        let crop = CropSquare {
            x: 150.0,
            y: 50.0,
            side: 100.0,
        };
        assert_eq!(
            placement_in_frame(crop, WIDE, 300.0),
            (-450.0, -150.0, 1200.0, 600.0)
        );
    }

    #[test]
    fn a_png_is_measured_from_its_header() {
        let mut bytes = Vec::new();
        image::RgbaImage::new(30, 20)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        assert_eq!(
            picture_size_of(&bytes),
            Some(PictureSize {
                width: 30.0,
                height: 20.0
            })
        );
        assert_eq!(picture_size_of(b"not a picture"), None);
    }
}
