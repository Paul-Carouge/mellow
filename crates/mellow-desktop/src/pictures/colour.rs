//! Colour management: GPUI draws sRGB, so wide-gamut pictures are converted with `moxcms`, or
//! their saturated colours would look washed out (ADR-0033 stores iPhone photos in Display P3).

use std::sync::{Arc, OnceLock};

use moxcms::{ColorProfile, DataColorSpace, Layout, Transform16BitExecutor, TransformOptions};

use super::PictureDecodeError;

pub(crate) const TEN_BIT_MAXIMUM: u16 = 1_023;

fn display_p3_to_srgb() -> Result<&'static Arc<Transform16BitExecutor>, PictureDecodeError> {
    static TRANSFORM: OnceLock<Option<Arc<Transform16BitExecutor>>> = OnceLock::new();
    TRANSFORM
        .get_or_init(|| {
            ColorProfile::new_display_p3()
                .create_transform_10bit(
                    Layout::Rgba,
                    &ColorProfile::new_srgb(),
                    Layout::Rgba,
                    TransformOptions::default(),
                )
                .ok()
        })
        .as_ref()
        .ok_or(PictureDecodeError::ColourConversion)
}

/// Converts 10-bit Display P3 RGBA (sRGB transfer) to 10-bit sRGB RGBA; alpha is kept and colours
/// outside sRGB are clipped to its gamut.
pub(crate) fn display_p3_to_srgb_ten_bit(
    display_p3: &[u16],
) -> Result<Vec<u16>, PictureDecodeError> {
    let mut srgb = vec![0; display_p3.len()];
    display_p3_to_srgb()?
        .transform(display_p3, &mut srgb)
        .map_err(|_| PictureDecodeError::ColourConversion)?;
    Ok(srgb)
}

/// Converts 8-bit RGBA tagged with `icc_profile` to sRGB in place. A profile that cannot be read,
/// or describes no RGB space, leaves the pixels as they are: browsers show such files the same.
pub(crate) fn convert_rgba_to_srgb_with_profile(icc_profile: &[u8], rgba: &mut [u8]) {
    let Ok(profile) = ColorProfile::new_from_slice(icc_profile) else {
        return;
    };
    if profile.color_space != DataColorSpace::Rgb {
        return;
    }
    let Ok(transform) = profile.create_transform_8bit(
        Layout::Rgba,
        &ColorProfile::new_srgb(),
        Layout::Rgba,
        TransformOptions::default(),
    ) else {
        return;
    };
    let source = rgba.to_vec();
    // shortcut: a failed transform keeps the untransformed pixels, as an unreadable profile does.
    let _ = transform.transform(&source, rgba);
}

/// `value` of `bit_depth` bits widened or narrowed to 10 bits, rounding to nearest.
pub(crate) fn to_ten_bit(value: u16, bit_depth: u8) -> u16 {
    match bit_depth {
        8 => (value << 2) | (value >> 6),
        10 => value,
        _ => {
            let maximum = (1_u32 << bit_depth) - 1;
            let scaled = (u32::from(value) * u32::from(TEN_BIT_MAXIMUM) + maximum / 2) / maximum;
            u16::try_from(scaled.min(u32::from(TEN_BIT_MAXIMUM))).unwrap_or(TEN_BIT_MAXIMUM)
        }
    }
}

/// A 10-bit value to 8 bits, rounding to nearest.
pub(crate) fn to_eight_bit(value: u16) -> u8 {
    let value = u32::from(value.min(TEN_BIT_MAXIMUM));
    let scaled = (value * 255 + u32::from(TEN_BIT_MAXIMUM) / 2) / u32::from(TEN_BIT_MAXIMUM);
    u8::try_from(scaled).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn srgb_of_display_p3(red: u8, green: u8, blue: u8) -> [u8; 3] {
        let display_p3 = [red, green, blue, 255].map(|value| to_ten_bit(u16::from(value), 8));
        let srgb = display_p3_to_srgb_ten_bit(&display_p3).unwrap();
        [srgb[0], srgb[1], srgb[2]].map(to_eight_bit)
    }

    fn assert_close(actual: [u8; 3], expected: [u8; 3]) {
        for (actual_channel, expected_channel) in actual.iter().zip(expected) {
            assert!(
                actual_channel.abs_diff(expected_channel) <= 2,
                "{actual:?} is not {expected:?}"
            );
        }
    }

    #[test]
    fn display_p3_coordinates_of_the_srgb_primaries_come_back_as_those_primaries() {
        // sRGB red, green and blue written in Display P3 (CSS Color 4 conversion, rounded to
        // 8 bits): a conversion that ignored the primaries would return these values unchanged.
        assert_close(srgb_of_display_p3(234, 51, 35), [255, 0, 0]);
        assert_close(srgb_of_display_p3(117, 251, 76), [0, 255, 0]);
        assert_close(srgb_of_display_p3(0, 0, 245), [0, 0, 255]);
    }

    #[test]
    fn greys_and_alpha_are_unchanged() {
        assert_close(srgb_of_display_p3(128, 128, 128), [128, 128, 128]);
        let srgb = display_p3_to_srgb_ten_bit(&[512, 512, 512, 300]).unwrap();
        assert_eq!(srgb[3], 300);
    }

    #[test]
    fn colours_outside_srgb_are_clipped_to_its_gamut() {
        assert_close(srgb_of_display_p3(255, 0, 0), [255, 0, 0]);
    }

    #[test]
    fn bit_depths_convert_to_the_nearest_value() {
        assert_eq!(to_ten_bit(255, 8), 1_023);
        assert_eq!(to_ten_bit(0, 8), 0);
        assert_eq!(to_ten_bit(4_095, 12), 1_023);
        assert_eq!(to_ten_bit(2_048, 12), 512);
        assert_eq!(to_eight_bit(1_023), 255);
        assert_eq!(to_eight_bit(512), 128);
        assert_eq!(to_eight_bit(2), 0);
        for value in 0..=255_u16 {
            assert_eq!(u16::from(to_eight_bit(to_ten_bit(value, 8))), value);
        }
    }
}
