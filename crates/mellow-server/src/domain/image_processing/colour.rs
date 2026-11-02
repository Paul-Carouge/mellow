//! Colour profiles of uploaded pictures (ADR-0033). AVIF and WebP are written from pixels, so a
//! picture tagged with a wide-gamut profile (a Display P3 iPhone photo) would change colour if its
//! pixels were stored as they are. Such pictures are stored in Display P3, which the AVIF signals
//! (CICP), and their quality is scored in the source's own colour space.

use std::borrow::Cow;
use std::sync::Arc;

use image::RgbaImage;
use moxcms::{ColorProfile, DataColorSpace, Layout, Transform8BitExecutor, TransformOptions};

use super::avif::Primaries;

/// Steps of the colour grid that decides whether a profile behaves like sRGB or Display P3.
const SAMPLED_LEVELS: [u8; 6] = [0, 51, 102, 153, 204, 255];
/// Largest difference, in 8-bit steps, still counted as the same colour space: no one sees it.
const EQUIVALENT_PROFILE_TOLERANCE: u8 = 2;

/// Where a picture's pixels are, and how they are stored.
#[derive(Clone)]
pub(super) enum ColourSpace {
    /// sRGB, no profile, or a profile browsers would ignore: stored as they are.
    Srgb,
    /// Stored in Display P3: an AVIF signals it, lossless WebP cannot (no profile is written).
    WideGamut(WideGamut),
}

impl std::fmt::Debug for ColourSpace {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Srgb => "Srgb",
            Self::WideGamut(_) => "WideGamut",
        })
    }
}

/// Transforms of a picture that is not already Display P3; both `None` when it is.
#[derive(Clone)]
pub(super) struct WideGamut {
    into_display_p3: Option<Arc<Transform8BitExecutor>>,
    /// Back into the source's space, to score there; `None` when the profile only reads one way,
    /// and then the scores compare in Display P3.
    back_into_source: Option<Arc<Transform8BitExecutor>>,
}

fn transform(from: &ColorProfile, into: &ColorProfile) -> Option<Arc<Transform8BitExecutor>> {
    from.create_transform_8bit(
        Layout::Rgba,
        into,
        Layout::Rgba,
        TransformOptions::default(),
    )
    .ok()
}

fn apply(transform: &Transform8BitExecutor, image: &RgbaImage) -> Option<RgbaImage> {
    let mut converted = RgbaImage::new(image.width(), image.height());
    transform.transform(image.as_raw(), &mut converted).ok()?;
    Some(converted)
}

/// Whether `transform` leaves every sampled colour within the tolerance.
fn is_nearly_identity(transform: &Transform8BitExecutor) -> bool {
    let samples: Vec<u8> = SAMPLED_LEVELS
        .iter()
        .flat_map(|red| {
            SAMPLED_LEVELS.iter().flat_map(move |green| {
                SAMPLED_LEVELS
                    .iter()
                    .map(move |blue| [*red, *green, *blue, u8::MAX])
            })
        })
        .flatten()
        .collect();
    let mut converted = vec![0; samples.len()];
    transform.transform(&samples, &mut converted).is_ok()
        && samples
            .iter()
            .zip(&converted)
            .all(|(before, after)| before.abs_diff(*after) <= EQUIVALENT_PROFILE_TOLERANCE)
}

/// The colour space of a picture from its embedded ICC profile.
pub(super) fn colour_space_of(icc_profile: Option<&[u8]>) -> ColourSpace {
    // Browsers ignore a profile they cannot read, and so does this: the picture shows as sRGB.
    // shortcut: grey and CMYK profiles are treated as sRGB too; convert them if such uploads show.
    let Some(source) = icc_profile
        .and_then(|bytes| ColorProfile::new_from_slice(bytes).ok())
        .filter(|profile| profile.color_space == DataColorSpace::Rgb)
    else {
        return ColourSpace::Srgb;
    };
    let srgb = ColorProfile::new_srgb();
    let display_p3 = ColorProfile::new_display_p3();
    let (Some(into_srgb), Some(into_display_p3)) =
        (transform(&source, &srgb), transform(&source, &display_p3))
    else {
        return ColourSpace::Srgb;
    };
    if is_nearly_identity(into_srgb.as_ref()) {
        return ColourSpace::Srgb;
    }
    if is_nearly_identity(into_display_p3.as_ref()) {
        return ColourSpace::WideGamut(WideGamut {
            into_display_p3: None,
            back_into_source: None,
        });
    }
    ColourSpace::WideGamut(WideGamut {
        back_into_source: transform(&display_p3, &source),
        into_display_p3: Some(into_display_p3),
    })
}

impl ColourSpace {
    pub(super) fn primaries(&self) -> Primaries {
        match self {
            Self::Srgb => Primaries::Srgb,
            Self::WideGamut(_) => Primaries::DisplayP3,
        }
    }

    /// The pixels as they are encoded, borrowed when they need no conversion.
    pub(super) fn stored<'a>(&self, source: &'a RgbaImage) -> Option<Cow<'a, RgbaImage>> {
        match self {
            Self::WideGamut(WideGamut {
                into_display_p3: Some(into_display_p3),
                ..
            }) => apply(into_display_p3.as_ref(), source).map(Cow::Owned),
            _ => Some(Cow::Borrowed(source)),
        }
    }

    /// What a candidate is scored against, given the source and its stored form.
    pub(super) fn reference<'a>(
        &self,
        source: &'a RgbaImage,
        stored: &'a RgbaImage,
    ) -> &'a RgbaImage {
        match self {
            Self::WideGamut(WideGamut {
                into_display_p3: Some(_),
                back_into_source: None,
            }) => stored,
            _ => source,
        }
    }

    /// A decoded candidate in the space of `reference`.
    pub(super) fn comparable(&self, decoded: RgbaImage) -> Option<RgbaImage> {
        match self {
            Self::WideGamut(WideGamut {
                back_into_source: Some(back_into_source),
                ..
            }) => apply(back_into_source.as_ref(), &decoded),
            _ => Some(decoded),
        }
    }

    pub(super) fn is_srgb(&self) -> bool {
        matches!(self, Self::Srgb)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// The ICC profile bytes of a standard colour space, as a photo would embed them.
    pub(in crate::domain::image_processing) fn icc_bytes(profile: &ColorProfile) -> Vec<u8> {
        profile.encode().unwrap()
    }

    #[test]
    fn srgb_and_display_p3_profiles_are_recognised_and_others_converted() {
        assert!(colour_space_of(None).is_srgb());
        assert!(colour_space_of(Some(b"not a profile")).is_srgb());
        assert!(colour_space_of(Some(&icc_bytes(&ColorProfile::new_srgb()))).is_srgb());

        let ColourSpace::WideGamut(display_p3) =
            colour_space_of(Some(&icc_bytes(&ColorProfile::new_display_p3())))
        else {
            panic!("Display P3 is wide gamut");
        };
        assert!(display_p3.into_display_p3.is_none(), "stored as it is");

        let ColourSpace::WideGamut(adobe) =
            colour_space_of(Some(&icc_bytes(&ColorProfile::new_adobe_rgb())))
        else {
            panic!("Adobe RGB is wide gamut");
        };
        assert!(adobe.into_display_p3.is_some(), "converted into Display P3");
        assert!(adobe.back_into_source.is_some(), "scored in Adobe RGB");
    }
}
