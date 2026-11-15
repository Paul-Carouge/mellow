//! The viewer's arithmetic (`picture-viewer.ts`): where previous and next go, how a picture fits
//! the room, and where the room looks once a picture shows at its own size. Lengths are token
//! pixels (logical pixels at the default zoom).

/// A width and a height, in token pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PixelSize {
    pub(crate) width: f32,
    pub(crate) height: f32,
}

/// Most videos are landscape: the box a video holds until its own size is known.
pub(crate) const FALLBACK_VIDEO_SIZE: PixelSize = PixelSize {
    width: 16.0,
    height: 9.0,
};

/// Previous and next wrap around, so neither button is ever disabled.
#[must_use]
pub(crate) fn wrapped_index(index: usize, is_forward: bool, total: usize) -> usize {
    if total == 0 {
        return 0;
    }
    if is_forward {
        (index + 1) % total
    } else {
        (index + total - 1) % total
    }
}

/// The picture fitted inside the box, never larger than its own pixels.
#[must_use]
pub(crate) fn fitted_picture_size(picture: PixelSize, area: PixelSize) -> PixelSize {
    scaled(
        picture,
        1.0_f32
            .min(area.width / picture.width)
            .min(area.height / picture.height),
    )
}

/// A video fills the box (grown too, as a player does).
#[must_use]
pub(crate) fn fitted_video_size(video: PixelSize, area: PixelSize) -> PixelSize {
    scaled(
        video,
        (area.width / video.width).min(area.height / video.height),
    )
}

fn scaled(size: PixelSize, scale: f32) -> PixelSize {
    PixelSize {
        width: (size.width * scale).round().max(1.0),
        height: (size.height * scale).round().max(1.0),
    }
}

/// Actual size only means something when the fitted picture is smaller than its own pixels.
#[must_use]
pub(crate) fn is_larger_than(picture: PixelSize, area: PixelSize) -> bool {
    picture.width > area.width || picture.height > area.height
}

/// How far the room is scrolled along one axis once the picture shows at its own size, so the
/// point clicked (at `click_ratio` of the fitted picture) stays under the pointer, `pointer_offset`
/// from the room's start. The picture sits `stage_padding` inside the scrolled content.
#[must_use]
pub(crate) fn actual_size_scroll_start(
    click_ratio: f32,
    natural_length: f32,
    stage_padding: f32,
    pointer_offset: f32,
) -> f32 {
    (click_ratio * natural_length + stage_padding - pointer_offset).max(0.0)
}

/// The furthest the room scrolls along one axis: the picture and its padding past the room.
#[must_use]
pub(crate) fn maximum_scroll(natural_length: f32, stage_padding: f32, stage_length: f32) -> f32 {
    (natural_length + 2.0 * stage_padding - stage_length).max(0.0)
}

/// Where the picture's start sits in the room along one axis: centred when it fits, else at its
/// padding minus the scroll.
#[must_use]
pub(crate) fn picture_start(
    shown_length: f32,
    stage_padding: f32,
    stage_length: f32,
    scroll: f32,
) -> f32 {
    if shown_length + 2.0 * stage_padding <= stage_length {
        (stage_length - shown_length) / 2.0
    } else {
        stage_padding - scroll
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn size(width: f32, height: f32) -> PixelSize {
        PixelSize { width, height }
    }

    #[test]
    fn previous_and_next_wrap_around() {
        assert_eq!(wrapped_index(3, true, 4), 0);
        assert_eq!(wrapped_index(0, false, 4), 3);
        assert_eq!(wrapped_index(1, true, 4), 2);
        assert_eq!(wrapped_index(0, true, 1), 0);
    }

    #[test]
    fn a_picture_fits_without_growing_past_its_pixels() {
        assert_eq!(
            fitted_picture_size(size(4032.0, 3024.0), size(1000.0, 600.0)),
            size(800.0, 600.0)
        );
        assert_eq!(
            fitted_picture_size(size(480.0, 270.0), size(1000.0, 600.0)),
            size(480.0, 270.0)
        );
        // A video grows to the room.
        assert_eq!(
            fitted_video_size(FALLBACK_VIDEO_SIZE, size(1600.0, 600.0)),
            size(1067.0, 600.0)
        );
    }

    #[test]
    fn actual_size_is_offered_only_for_pictures_larger_than_the_room() {
        assert!(is_larger_than(size(2880.0, 1800.0), size(1000.0, 600.0)));
        assert!(!is_larger_than(size(480.0, 270.0), size(1000.0, 600.0)));
    }

    #[test]
    fn the_clicked_point_stays_under_the_pointer() {
        // The picture's middle clicked 500 px into the room: the room scrolls so that the
        // natural picture's middle (1440 px + 16 px padding) lands 500 px in.
        let scroll = actual_size_scroll_start(0.5, 2880.0, 16.0, 500.0);
        assert!((scroll - 956.0).abs() < f32::EPSILON);
        assert!((picture_start(2880.0, 16.0, 1000.0, scroll) + 1440.0 - 500.0).abs() < 0.001);
        assert!(actual_size_scroll_start(0.0, 2880.0, 16.0, 500.0).abs() < f32::EPSILON);
        assert!((maximum_scroll(2880.0, 16.0, 1000.0) - 1912.0).abs() < f32::EPSILON);
        assert!((picture_start(400.0, 16.0, 1000.0, 50.0) - 300.0).abs() < f32::EPSILON);
    }
}
