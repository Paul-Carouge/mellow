//! Colour profiles of uploaded pictures (ADR-0033). AVIF and WebP are written from pixels, so a
//! picture tagged with a wide-gamut profile (a Display P3 iPhone photo) would change colour if its
//! pixels were stored as they are. Such pictures are stored and scored in Display P3, which the
//! AVIF signals (CICP), unless their colours round-trip through sRGB, which lossless WebP carries.

use std::borrow::Cow;
use std::sync::Arc;

use image::RgbaImage;
use moxcms::{ColorProfile, DataColorSpace, Layout, Transform8BitExecutor, TransformOptions};

use super::avif::Primaries;

/// Steps of the colour grid that decides whether a profile behaves like sRGB or Display P3.
const SAMPLED_LEVELS: [u8; 6] = [0, 51, 102, 153, 204, 255];
/// Largest difference, in 8-bit steps, still counted as the same colour: no one sees it.
const EQUIVALENT_PROFILE_TOLERANCE: u8 = 2;

/// Where a picture's pixels are, and how they are stored.
#[derive(Clone)]
pub(super) enum ColourSpace {
    /// sRGB, no profile, or a profile browsers would ignore: stored as they are.
    Srgb,
    /// A grey or CMYK profile (print exports): the decoder's RGB ignores it while browsers apply
    /// it, so these pixels are not what is seen. Written as sRGB only where a picture must be
    /// stored (avatars, icons, emoji), never in place of a file that shows right (attachments).
    Unmanaged,
    /// Stored in Display P3, which an AVIF signals; lossless WebP only for the sRGB form of
    /// pixels that round-trip through it (no profile is written into a WebP).
    WideGamut(WideGamut),
}

impl std::fmt::Debug for ColourSpace {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Srgb => "Srgb",
            Self::Unmanaged => "Unmanaged",
            Self::WideGamut(_) => "WideGamut",
        })
    }
}

/// Transforms of a wide-gamut picture. Those into and out of Display P3 are `None` when it
/// already is Display P3; a missing way back means its colours are never taken as preserved.
#[derive(Clone)]
pub(super) struct WideGamut {
    into_display_p3: Option<Arc<Transform8BitExecutor>>,
    display_p3_into_source: Option<Arc<Transform8BitExecutor>>,
    into_srgb: Arc<Transform8BitExecutor>,
    srgb_into_source: Option<Arc<Transform8BitExecutor>>,
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

fn is_within_tolerance(before: &[u8], after: &[u8]) -> bool {
    before.len() == after.len()
        && before
            .iter()
            .zip(after)
            .all(|(before, after)| before.abs_diff(*after) <= EQUIVALENT_PROFILE_TOLERANCE)
}

/// `source` through `forward`, when `back` brings every pixel within the tolerance of where it
/// was: no colour fell outside the target's gamut.
fn round_tripped(
    source: &RgbaImage,
    forward: &Transform8BitExecutor,
    back: Option<&Transform8BitExecutor>,
) -> Option<RgbaImage> {
    let converted = apply(forward, source)?;
    let returned = apply(back?, &converted)?;
    is_within_tolerance(source.as_raw(), returned.as_raw()).then_some(converted)
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
        && is_within_tolerance(&samples, &converted)
}

/// The colour space of a picture from its embedded ICC profile.
pub(super) fn colour_space_of(icc_profile: Option<&[u8]>) -> ColourSpace {
    // Browsers ignore a profile they cannot read, and so does this: the picture shows as sRGB.
    let Some(source) = icc_profile.and_then(|bytes| ColorProfile::new_from_slice(bytes).ok())
    else {
        return ColourSpace::Srgb;
    };
    if source.color_space != DataColorSpace::Rgb {
        return ColourSpace::Unmanaged;
    }
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
    let is_display_p3 = is_nearly_identity(into_display_p3.as_ref());
    ColourSpace::WideGamut(WideGamut {
        display_p3_into_source: if is_display_p3 {
            None
        } else {
            transform(&display_p3, &source)
        },
        into_display_p3: (!is_display_p3).then_some(into_display_p3),
        srgb_into_source: transform(&srgb, &source),
        into_srgb,
    })
}

impl ColourSpace {
    /// The primaries pixels are stored, signalled and scored in.
    pub(super) fn primaries(&self) -> Primaries {
        match self {
            Self::Srgb | Self::Unmanaged => Primaries::Srgb,
            Self::WideGamut(_) => Primaries::DisplayP3,
        }
    }

    /// The pixels as they are encoded in `primaries()`, borrowed when they need no conversion.
    pub(super) fn stored<'a>(&self, source: &'a RgbaImage) -> Option<Cow<'a, RgbaImage>> {
        match self {
            Self::WideGamut(WideGamut {
                into_display_p3: Some(into_display_p3),
                ..
            }) => apply(into_display_p3.as_ref(), source).map(Cow::Owned),
            _ => Some(Cow::Borrowed(source)),
        }
    }

    /// The pixels in sRGB, for a format written without a profile: `None` when a colour would
    /// change on the way.
    pub(super) fn srgb_form<'a>(&self, source: &'a RgbaImage) -> Option<Cow<'a, RgbaImage>> {
        match self {
            Self::Srgb | Self::Unmanaged => Some(Cow::Borrowed(source)),
            Self::WideGamut(wide_gamut) => round_tripped(
                source,
                wide_gamut.into_srgb.as_ref(),
                wide_gamut.srgb_into_source.as_deref(),
            )
            .map(Cow::Owned),
        }
    }

    /// Whether what `stored` gives shows the colours `source` is seen with: `false` for an
    /// unmanaged profile, and for colours outside Display P3 (the greenest Adobe RGB greens).
    pub(super) fn keeps_colours(&self, source: &RgbaImage) -> bool {
        match self {
            Self::Unmanaged => false,
            Self::WideGamut(WideGamut {
                into_display_p3: Some(into_display_p3),
                display_p3_into_source,
                ..
            }) => round_tripped(
                source,
                into_display_p3.as_ref(),
                display_p3_into_source.as_deref(),
            )
            .is_some(),
            // sRGB, or Display P3 stored as it is.
            Self::Srgb | Self::WideGamut(_) => true,
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// The ICC profile bytes of a standard colour space, as a photo would embed them.
    pub(in crate::domain::image_processing) fn icc_bytes(profile: &ColorProfile) -> Vec<u8> {
        profile.encode().unwrap()
    }

    /// An RGB profile relabelled as CMYK, as print exports embed: only its header is read here.
    pub(in crate::domain::image_processing) fn cmyk_icc_bytes() -> Vec<u8> {
        let mut bytes = icc_bytes(&ColorProfile::new_srgb());
        // The data colour space signature, bytes 16 to 19 of the header.
        bytes[16..20].copy_from_slice(b"CMYK");
        bytes
    }

    #[test]
    fn srgb_and_display_p3_profiles_are_recognised_and_others_converted() {
        assert!(matches!(colour_space_of(None), ColourSpace::Srgb));
        assert!(matches!(
            colour_space_of(Some(b"not a profile")),
            ColourSpace::Srgb
        ));
        assert!(matches!(
            colour_space_of(Some(&icc_bytes(&ColorProfile::new_srgb()))),
            ColourSpace::Srgb
        ));

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
        assert!(
            adobe.display_p3_into_source.is_some(),
            "its gamut can be checked"
        );
    }

    #[test]
    fn a_cmyk_profile_is_unmanaged_and_its_colours_are_never_taken_as_kept() {
        let cmyk = colour_space_of(Some(&cmyk_icc_bytes()));
        assert!(matches!(cmyk, ColourSpace::Unmanaged), "{cmyk:?}");
        assert!(!cmyk.keeps_colours(&RgbaImage::new(4, 4)));
    }

    #[test]
    fn colours_outside_display_p3_or_srgb_are_never_taken_as_kept() {
        let adobe = colour_space_of(Some(&icc_bytes(&ColorProfile::new_adobe_rgb())));
        let grey = RgbaImage::from_pixel(4, 4, image::Rgba([128, 128, 128, 255]));
        assert!(adobe.keeps_colours(&grey));
        assert!(adobe.srgb_form(&grey).is_some(), "grey is grey in sRGB");
        // The purest Adobe RGB green lies outside Display P3, and so outside sRGB.
        let green = RgbaImage::from_pixel(4, 4, image::Rgba([0, 255, 0, 255]));
        assert!(!adobe.keeps_colours(&green));
        assert!(adobe.srgb_form(&green).is_none());

        let display_p3 = colour_space_of(Some(&icc_bytes(&ColorProfile::new_display_p3())));
        let red = RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
        assert!(display_p3.keeps_colours(&red));
        assert!(display_p3.srgb_form(&red).is_none(), "outside sRGB");
        assert!(display_p3.srgb_form(&grey).is_some());
    }
}
