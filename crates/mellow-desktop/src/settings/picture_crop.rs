//! The crop of an icon, an avatar or a banner (`picture-crop.ts`, DESIGN.md §6.54): pure maths
//! over the picture's size, in its own pixels, so the crop dialog only draws what these functions
//! return. The kept part has a fixed shape, `aspect` being its width over its height: square for
//! icons and avatars, the profile card's band for banners. The instance cuts the same part
//! (`crop_x`, `crop_y` and `crop_size` or `crop_width`); without one it keeps the largest centred
//! part of that shape.

use image::{ImageDecoder as _, ImageReader, metadata::Orientation};
use mellow_protocol::{BannerCropQuery, IconCropQuery};

use crate::pictures::has_bounded_metadata;

/// Icons and avatars keep a square.
pub const SQUARE_ASPECT: f32 = 1.0;
/// Banners keep the band the profile card shows, stored at 960 × 384 by the instance (ADR-0033).
pub const BANNER_ASPECT: f32 = 960.0 / 384.0;
/// Zooming in further than this shows a handful of pixels: the picture would be a blur.
pub const MAXIMUM_CROP_ZOOM: f32 = 8.0;
/// The smallest short side worth keeping, in picture pixels, whatever the zoom allows.
const MINIMUM_CROP_SIDE: f32 = 16.0;
/// The instance's limits on an uploaded picture (`image_processing.rs` on the server), checked
/// before sending so the refusal names them.
pub const MAXIMUM_PICTURE_SIDE_PIXELS: u32 = 8_192;
pub const MAXIMUM_PICTURE_MEGAPIXELS: u32 = 24;
/// Under half a pixel from the centred part counts as untouched.
const UNTOUCHED_TOLERANCE: f32 = 0.5;

/// A picture's size in its own pixels, EXIF orientation applied, as the instance measures it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PictureSize {
    pub width: f32,
    pub height: f32,
}

/// A part of the picture in its pixels, `aspect` times wider than tall; fractional while the
/// person drags, rounded only when sent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropArea {
    pub x: f32,
    pub y: f32,
    pub width: f32,
}

impl CropArea {
    #[must_use]
    pub fn height(self, aspect: f32) -> f32 {
        self.width / aspect
    }

    fn centre(self, aspect: f32) -> (f32, f32) {
        (
            self.x + self.width / 2.0,
            self.y + self.height(aspect) / 2.0,
        )
    }
}

/// The crop in whole picture pixels, as the query sends it; its height follows from the aspect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SentCrop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
}

/// The icon and avatar query: `None` leaves the centring to the instance.
#[must_use]
pub fn icon_crop_query(crop: Option<SentCrop>) -> IconCropQuery {
    crop.map_or_else(IconCropQuery::default, |crop| IconCropQuery {
        crop_x: Some(crop.x),
        crop_y: Some(crop.y),
        crop_size: Some(crop.width),
    })
}

/// The banner query: `None` leaves the centring to the instance.
#[must_use]
pub fn banner_crop_query(crop: Option<SentCrop>) -> BannerCropQuery {
    crop.map_or_else(BannerCropQuery::default, |crop| BannerCropQuery {
        crop_x: Some(crop.x),
        crop_y: Some(crop.y),
        crop_width: Some(crop.width),
    })
}

/// What the instance keeps when no crop is sent, and where the dialog starts.
#[must_use]
pub fn largest_centred_crop(picture: PictureSize, aspect: f32) -> CropArea {
    let width = picture.width.min(picture.height * aspect);
    CropArea {
        x: (picture.width - width) / 2.0,
        y: (picture.height - width / aspect) / 2.0,
        width,
    }
}

/// At least 1 (the largest centred part); a tiny picture cannot be zoomed into.
#[must_use]
pub fn maximum_zoom(picture: PictureSize, aspect: f32) -> f32 {
    let largest = largest_centred_crop(picture, aspect);
    let short_side = largest.width.min(largest.height(aspect));
    (short_side / MINIMUM_CROP_SIDE).clamp(1.0, MAXIMUM_CROP_ZOOM.max(1.0))
}

/// 1 when the crop is the largest part of its shape.
#[must_use]
pub fn zoom_of(crop: CropArea, picture: PictureSize, aspect: f32) -> f32 {
    largest_centred_crop(picture, aspect).width / crop.width
}

/// The nearest crop that stays inside the picture and within the zoom range.
#[must_use]
pub fn clamp_crop(crop: CropArea, picture: PictureSize, aspect: f32) -> CropArea {
    let largest = largest_centred_crop(picture, aspect).width;
    let width = crop
        .width
        .clamp(largest / maximum_zoom(picture, aspect), largest);
    CropArea {
        x: crop.x.clamp(0.0, picture.width - width),
        y: crop.y.clamp(0.0, picture.height - width / aspect),
        width,
    }
}

/// The crop at `zoom`, keeping the picture point `anchor` (the pointer, or the crop's centre)
/// where it is in the frame, as maps and photo viewers zoom.
#[must_use]
pub fn crop_zoomed_to(
    crop: CropArea,
    picture: PictureSize,
    aspect: f32,
    zoom: f32,
    anchor: Option<(f32, f32)>,
) -> CropArea {
    let (anchor_x, anchor_y) = anchor.unwrap_or_else(|| crop.centre(aspect));
    let width = largest_centred_crop(picture, aspect).width
        / zoom.clamp(1.0, maximum_zoom(picture, aspect));
    let ratio = width / crop.width;
    clamp_crop(
        CropArea {
            x: anchor_x - (anchor_x - crop.x) * ratio,
            y: anchor_y - (anchor_y - crop.y) * ratio,
            width,
        },
        picture,
        aspect,
    )
}

/// Moves the crop by a distance in picture pixels; dragging the picture moves it the other way.
#[must_use]
pub fn crop_moved_by(
    crop: CropArea,
    picture: PictureSize,
    aspect: f32,
    delta_x: f32,
    delta_y: f32,
) -> CropArea {
    clamp_crop(
        CropArea {
            x: crop.x + delta_x,
            y: crop.y + delta_y,
            ..crop
        },
        picture,
        aspect,
    )
}

/// Whole pixels for the query, or `None` for the largest centred part, so an untouched crop
/// leaves the choice to the instance's own centring. The height is rounded up when checking the
/// bottom edge, so the instance never has to narrow the crop to fit it.
#[must_use]
pub fn crop_to_send(crop: CropArea, picture: PictureSize, aspect: f32) -> Option<SentCrop> {
    let centred = largest_centred_crop(picture, aspect);
    let is_centred = (crop.x - centred.x).abs() < UNTOUCHED_TOLERANCE
        && (crop.y - centred.y).abs() < UNTOUCHED_TOLERANCE
        && (crop.width - centred.width).abs() < UNTOUCHED_TOLERANCE;
    if is_centred {
        return None;
    }
    // Every value is clamped into the picture, whose sides are positive and fit in u32.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let whole = |value: f32| value.round().max(0.0) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let largest_width = centred.width.floor().max(1.0) as u32;
    let width = whole(crop.width).clamp(1, largest_width);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let height = (width as f32 / aspect).ceil().max(1.0) as u32;
    Some(SentCrop {
        x: whole(crop.x).min(whole(picture.width).saturating_sub(width)),
        y: whole(crop.y).min(whole(picture.height).saturating_sub(height)),
        width,
    })
}

/// Where to draw the whole picture so that `crop` fills a frame `frame_width` wide: its offset
/// from the frame's top-left corner and its drawn width and height.
#[must_use]
pub fn placement_in_frame(
    crop: CropArea,
    picture: PictureSize,
    frame_width: f32,
) -> (f32, f32, f32, f32) {
    let scale = frame_width / crop.width;
    (
        -crop.x * scale,
        -crop.y * scale,
        picture.width * scale,
        picture.height * scale,
    )
}

/// Where `crop` lies in a decoded copy of the picture `still` pixels big, in its whole pixels:
/// x, y, width and height, inside the copy and at least one pixel. Each axis has its own scale,
/// since the copy's sides were rounded separately: a very narrow picture's short side can be a
/// pixel or two, far from the long side's scale.
#[must_use]
pub fn crop_in_still(
    crop: CropArea,
    aspect: f32,
    picture: PictureSize,
    still: (u32, u32),
) -> (u32, u32, u32, u32) {
    let (still_width, still_height) = still;
    #[allow(clippy::cast_precision_loss)] // Sides of at most a few thousand pixels.
    let (scale_x, scale_y) = (
        still_width as f32 / picture.width,
        still_height as f32 / picture.height,
    );
    // Inside the decoded copy by construction; rounding is kept within it.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let to_pixels = |value: f32, scale: f32| (value * scale).round().max(0.0) as u32;
    let x = to_pixels(crop.x, scale_x).min(still_width.saturating_sub(1));
    let y = to_pixels(crop.y, scale_y).min(still_height.saturating_sub(1));
    let width = to_pixels(crop.width, scale_x)
        .max(1)
        .min(still_width.saturating_sub(x).max(1));
    let height = to_pixels(crop.height(aspect), scale_y)
        .max(1)
        .min(still_height.saturating_sub(y).max(1));
    (x, y, width, height)
}

/// Whether the instance would refuse `picture` for its size.
#[must_use]
pub fn is_picture_too_large(picture: PictureSize) -> bool {
    #[allow(clippy::cast_precision_loss)] // Small whole numbers, exact in an f32.
    let (side, total) = (
        MAXIMUM_PICTURE_SIDE_PIXELS as f32,
        MAXIMUM_PICTURE_MEGAPIXELS as f32 * 1_000_000.0,
    );
    picture.width > side || picture.height > side || picture.width * picture.height > total
}

/// The size to decode `picture` at so it fits in a `side`-pixel square ("contain"), never
/// larger than it is: a panorama keeps its whole width within the atlas's limits.
#[must_use]
pub fn contained_size(picture: PictureSize, side: f32) -> (u32, u32) {
    let scale = (side / picture.width).min(side / picture.height).min(1.0);
    // At most `side` by construction, and at least one pixel.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pixels = |length: f32| (length * scale).round().max(1.0) as u32;
    (pixels(picture.width), pixels(picture.height))
}

/// The picture's size as the instance measures it (`displayed_dimensions`): the decoder's size,
/// turned by its EXIF orientation for a JPEG or a PNG, the only formats whose orientation the
/// instance honours. Reads the header only. `None` for what cannot be decoded.
#[must_use]
pub fn picture_size_of(bytes: &[u8]) -> Option<PictureSize> {
    let reader = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let has_bounded_metadata = has_bounded_metadata(reader.format());
    let mut decoder = reader.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return None;
    }
    let orientation = if has_bounded_metadata {
        decoder.orientation().unwrap_or(Orientation::NoTransforms)
    } else {
        Orientation::NoTransforms
    };
    let is_turned_a_quarter = matches!(
        orientation,
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
        let centred = largest_centred_crop(WIDE, SQUARE_ASPECT);
        assert_eq!(
            centred,
            CropArea {
                x: 100.0,
                y: 0.0,
                width: 200.0
            }
        );
        assert_eq!(crop_to_send(centred, WIDE, SQUARE_ASPECT), None);
        assert_eq!(icon_crop_query(None), IconCropQuery::default());
        assert!((zoom_of(centred, WIDE, SQUARE_ASPECT) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_moved_crop_stays_inside_the_picture_and_is_sent_in_whole_pixels() {
        let moved = crop_moved_by(
            largest_centred_crop(WIDE, SQUARE_ASPECT),
            WIDE,
            SQUARE_ASPECT,
            250.0,
            30.0,
        );
        assert_eq!(
            moved,
            CropArea {
                x: 200.0,
                y: 0.0,
                width: 200.0
            }
        );
        assert_eq!(
            icon_crop_query(crop_to_send(moved, WIDE, SQUARE_ASPECT)),
            IconCropQuery {
                crop_x: Some(200),
                crop_y: Some(0),
                crop_size: Some(200)
            }
        );
    }

    #[test]
    fn zooming_keeps_the_anchor_in_place_and_stops_at_the_limits() {
        let centred = largest_centred_crop(WIDE, SQUARE_ASPECT);
        let zoomed = crop_zoomed_to(centred, WIDE, SQUARE_ASPECT, 2.0, None);
        assert_eq!(
            zoomed,
            CropArea {
                x: 150.0,
                y: 50.0,
                width: 100.0
            }
        );
        // 200 px is 12.5 times the 16 px floor: the limit is that, under the 8× cap.
        assert!((maximum_zoom(WIDE, SQUARE_ASPECT) - MAXIMUM_CROP_ZOOM).abs() < f32::EPSILON);
        let deepest = crop_zoomed_to(centred, WIDE, SQUARE_ASPECT, 100.0, None);
        assert!((deepest.width - 25.0).abs() < f32::EPSILON);
        let widest = crop_zoomed_to(zoomed, WIDE, SQUARE_ASPECT, 0.1, None);
        assert!((widest.width - 200.0).abs() < f32::EPSILON);
        // Zooming around a corner keeps that corner where it was.
        let around_corner = crop_zoomed_to(centred, WIDE, SQUARE_ASPECT, 2.0, Some((100.0, 0.0)));
        assert_eq!((around_corner.x, around_corner.y), (100.0, 0.0));
    }

    #[test]
    fn a_tiny_picture_cannot_be_zoomed_into() {
        let tiny = PictureSize {
            width: 10.0,
            height: 12.0,
        };
        assert!((maximum_zoom(tiny, SQUARE_ASPECT) - 1.0).abs() < f32::EPSILON);
        assert!((maximum_zoom(tiny, BANNER_ASPECT) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_banner_keeps_the_cards_band_inside_a_tall_or_wide_picture() {
        let square = PictureSize {
            width: 1000.0,
            height: 1000.0,
        };
        // The whole width, 400 px tall, centred.
        let centred = largest_centred_crop(square, BANNER_ASPECT);
        assert_eq!(
            centred,
            CropArea {
                x: 0.0,
                y: 300.0,
                width: 1000.0
            }
        );
        assert!((centred.height(BANNER_ASPECT) - 400.0).abs() < 0.001);
        // A panorama wider than the band: the whole height, centred sideways.
        let panorama = PictureSize {
            width: 3000.0,
            height: 600.0,
        };
        assert_eq!(
            largest_centred_crop(panorama, BANNER_ASPECT),
            CropArea {
                x: 750.0,
                y: 0.0,
                width: 1500.0
            }
        );
        // Down to the bottom edge, never past it.
        let lowest = crop_moved_by(centred, square, BANNER_ASPECT, 0.0, 10_000.0);
        assert!((lowest.y - 600.0).abs() < 0.001);
        // The zoom floor is on the band's short side: 400 px / 16 px is past the 8× cap.
        assert!((maximum_zoom(square, BANNER_ASPECT) - MAXIMUM_CROP_ZOOM).abs() < f32::EPSILON);
        let zoomed = crop_zoomed_to(lowest, square, BANNER_ASPECT, 2.0, None);
        assert!((zoomed.width - 500.0).abs() < 0.001);
        assert!(zoomed.y + zoomed.height(BANNER_ASPECT) <= 1000.0 + 0.001);
    }

    #[test]
    fn a_banner_crop_is_sent_with_its_height_inside_the_picture() {
        let picture = PictureSize {
            width: 1001.0,
            height: 401.0,
        };
        // 1001 / 2.5 = 400.4 px tall, rounded up to 401: y = 1 would reach past the bottom.
        let crop = CropArea {
            x: 0.0,
            y: 0.9,
            width: 1001.0,
        };
        let sent = crop_to_send(crop, picture, BANNER_ASPECT);
        assert_eq!(
            sent,
            Some(SentCrop {
                x: 0,
                y: 0,
                width: 1001
            })
        );
        assert_eq!(
            banner_crop_query(sent),
            BannerCropQuery {
                crop_x: Some(0),
                crop_y: Some(0),
                crop_width: Some(1001)
            }
        );
        assert_eq!(banner_crop_query(None), BannerCropQuery::default());
    }

    #[test]
    fn a_very_narrow_pictures_crop_is_cut_with_each_axis_own_scale() {
        // 5 × 3000 decodes as 2 × 1024: across, 0.4 still pixels a picture pixel; down, 0.341.
        let picture = PictureSize {
            width: 5.0,
            height: 3000.0,
        };
        let still = contained_size(picture, 1024.0);
        assert_eq!(still, (2, 1024));
        let centred = largest_centred_crop(picture, SQUARE_ASPECT);
        // The middle of the picture, not 599 px down as the across scale would put it.
        assert_eq!(
            crop_in_still(centred, SQUARE_ASPECT, picture, still),
            (0, 511, 2, 2)
        );
        // The bottom stays inside the copy.
        let lowest = crop_moved_by(centred, picture, SQUARE_ASPECT, 0.0, 10_000.0);
        let (_, y, _, height) = crop_in_still(lowest, SQUARE_ASPECT, picture, still);
        assert!(y + height <= 1024);
    }

    #[test]
    fn the_whole_picture_is_drawn_so_the_crop_fills_the_frame() {
        let crop = CropArea {
            x: 150.0,
            y: 50.0,
            width: 100.0,
        };
        assert_eq!(
            placement_in_frame(crop, WIDE, 300.0),
            (-450.0, -150.0, 1200.0, 600.0)
        );
    }

    /// A 40 × 20 JPEG tagged with EXIF orientation 6 (turn right), as a phone writes it.
    fn jpeg_tagged_to_turn_right() -> Vec<u8> {
        use image::ImageEncoder as _;

        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .write_image(
                image::RgbImage::from_pixel(40, 20, image::Rgb([0, 120, 255])).as_raw(),
                40,
                20,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        // Big-endian TIFF with one IFD entry: Orientation (0x0112), SHORT, count 1, value 6.
        let exif: &[u8] =
            b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0";
        let segment_length = u16::try_from(exif.len() + 2).unwrap();
        let mut tagged = jpeg[..2].to_vec();
        tagged.extend_from_slice(&[0xFF, 0xE1]);
        tagged.extend_from_slice(&segment_length.to_be_bytes());
        tagged.extend_from_slice(exif);
        tagged.extend_from_slice(&jpeg[2..]);
        tagged
    }

    /// A lossless 40 × 20 WebP carrying the same EXIF orientation 6, in an extended (VP8X) file.
    fn webp_tagged_to_turn_right() -> Vec<u8> {
        let mut simple = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut simple)
            .encode(
                image::RgbaImage::from_pixel(40, 20, image::Rgba([0, 120, 255, 255])).as_raw(),
                40,
                20,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        let chunk = |name: &[u8], body: &[u8]| {
            let mut chunk = name.to_vec();
            chunk.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
            chunk.extend_from_slice(body);
            if body.len() % 2 == 1 {
                chunk.push(0);
            }
            chunk
        };
        // Flags: EXIF present (0x08); canvas width and height less one, 24-bit little-endian.
        let mut chunks = chunk(b"VP8X", &[0x08, 0, 0, 0, 39, 0, 0, 19, 0, 0]);
        // The simple file's VP8L chunk, as it is.
        chunks.extend_from_slice(&simple[12..]);
        chunks.extend_from_slice(&chunk(
            b"EXIF",
            b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0",
        ));
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&u32::try_from(chunks.len() + 4).unwrap().to_le_bytes());
        webp.extend_from_slice(b"WEBP");
        webp.extend_from_slice(&chunks);
        webp
    }

    #[test]
    fn a_webps_orientation_is_ignored_as_the_instance_ignores_it() {
        let webp = webp_tagged_to_turn_right();
        // The tag is there and readable: only the format rule leaves it out.
        let mut decoder = image::ImageReader::new(std::io::Cursor::new(&webp))
            .with_guessed_format()
            .unwrap()
            .into_decoder()
            .unwrap();
        assert_eq!(decoder.orientation().unwrap(), Orientation::Rotate90);
        let size = picture_size_of(&webp).unwrap();
        assert_eq!(
            size,
            PictureSize {
                width: 40.0,
                height: 20.0
            }
        );
        let (width, height) = contained_size(size, 1024.0);
        let frames = crate::pictures::decode_frames(
            &webp,
            crate::pictures::Animation::StillFrame,
            crate::pictures::DisplaySize { width, height },
        )
        .unwrap();
        assert_eq!(frames[0].buffer().dimensions(), (40, 20));
    }

    #[test]
    fn a_turned_jpeg_measures_as_the_stage_draws_it() {
        let jpeg = jpeg_tagged_to_turn_right();
        let size = picture_size_of(&jpeg).unwrap();
        assert_eq!(
            size,
            PictureSize {
                width: 20.0,
                height: 40.0
            }
        );
        let (width, height) = contained_size(size, 1024.0);
        let frames = crate::pictures::decode_frames(
            &jpeg,
            crate::pictures::Animation::StillFrame,
            crate::pictures::DisplaySize { width, height },
        )
        .unwrap();
        assert_eq!(frames[0].buffer().dimensions(), (20, 40));
    }

    #[test]
    fn a_panorama_is_decoded_whole_within_the_stage_side() {
        let panorama = PictureSize {
            width: 8000.0,
            height: 1000.0,
        };
        assert_eq!(contained_size(panorama, 1024.0), (1024, 128));
        // Never enlarged.
        let small = PictureSize {
            width: 300.0,
            height: 200.0,
        };
        assert_eq!(contained_size(small, 1024.0), (300, 200));
    }

    #[test]
    fn pictures_past_the_instances_limits_are_refused_before_sending() {
        let size = |width, height| PictureSize { width, height };
        assert!(!is_picture_too_large(size(6000.0, 4000.0)));
        assert!(is_picture_too_large(size(8193.0, 100.0)));
        assert!(is_picture_too_large(size(6000.0, 4001.0)));
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
