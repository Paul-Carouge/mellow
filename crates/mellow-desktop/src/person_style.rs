//! Colours that belong to a person (DESIGN.md §2.3): the default avatar tint, picked from their id,
//! and their highest role colour, made readable on the sheet in each theme.

use mellow_protocol::{Guild, Snowflake};
use slint::Color;

use crate::generated_ui::RoleColor;

/// `color.avatar.tint-1` to `tint-8`.
const AVATAR_TINT_COUNT: u32 = 8;
const FNV_OFFSET_BASIS: u32 = 0x811C_9DC5;
const FNV_PRIME: u32 = 0x0100_0193;
/// WCAG 2.2 AA for text.
const MINIMUM_NAME_CONTRAST: f64 = 4.5;
/// OKLCH lightness moved per adjustment step.
const LIGHTNESS_STEP: f64 = 0.005;
/// Below this relative luminance a surface counts as dark: names get lighter on it, not darker.
const DARK_SURFACE_LUMINANCE: f64 = 0.18;
/// `color.surface.chat`, where names are drawn, in the dark and light themes; a test keeps them
/// equal to the generated tokens.
const DARK_SHEET_RGB: [u8; 3] = [0x22, 0x1B, 0x23];
const LIGHT_SHEET_RGB: [u8; 3] = [0xFF, 0xFF, 0xFF];

/// The default avatar tint for a person, 0 to 7: FNV-1a (32 bits) over the UTF-8 bytes of their id
/// in decimal, modulo 8, so every client picks the same tint (Snowflake low bits are mostly zero).
pub fn avatar_tint_index(user_id: &str) -> i32 {
    let hash = user_id.bytes().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(FNV_PRIME)
    });
    // Below 8: always fits.
    i32::try_from(hash % AVATAR_TINT_COUNT).unwrap_or_default()
}

pub fn avatar_tint_index_of(user_id: Snowflake) -> i32 {
    avatar_tint_index(&user_id.to_string())
}

/// The colour of the highest role holding one among `role_ids`, as Discord shows names.
pub fn highest_role_color(guild: &Guild, role_ids: &[Snowflake]) -> Option<u32> {
    guild
        .roles
        .iter()
        .filter(|role| role.color != 0 && role_ids.contains(&role.id))
        .max_by_key(|role| role.position)
        .map(|role| role.color)
}

/// `0xRRGGBB` made readable on the sheet in both themes. No colour gives the theme's default name
/// colour.
pub fn role_color(color: Option<u32>) -> RoleColor {
    let Some(color) = color else {
        return RoleColor::default();
    };
    let picked = [channel(color >> 16), channel(color >> 8), channel(color)];
    let dark = readable_on(picked, DARK_SHEET_RGB);
    let light = readable_on(picked, LIGHT_SHEET_RGB);
    RoleColor {
        is_set: true,
        dark: Color::from_rgb_u8(dark[0], dark[1], dark[2]),
        light: Color::from_rgb_u8(light[0], light[1], light[2]),
    }
}

fn channel(value: u32) -> u8 {
    u8::try_from(value & 0xFF).unwrap_or_default()
}

/// The picked hue with its OKLCH lightness moved, step by step, until it reaches 4.5:1 on
/// `surface`: lighter on a dark surface, darker on a light one. Mirrors the web client's
/// `readableRoleColor`, so a name has the same colour in both clients.
fn readable_on(picked: [u8; 3], surface: [u8; 3]) -> [u8; 3] {
    if contrast(picked, surface) >= MINIMUM_NAME_CONTRAST {
        return picked;
    }
    let direction = if relative_luminance(surface) < DARK_SURFACE_LUMINANCE {
        1.0
    } else {
        -1.0
    };
    let [lightness, green_red, blue_yellow] = to_oklab(picked);
    let mut next = lightness;
    while (0.0..=1.0).contains(&next) {
        // Checked after rounding to 8 bits, since that is the colour drawn.
        let candidate = from_oklab(next, green_red, blue_yellow);
        if contrast(candidate, surface) >= MINIMUM_NAME_CONTRAST {
            return candidate;
        }
        next += direction * LIGHTNESS_STEP;
    }
    // Out of gamut before reaching the ratio: the surface's own text colour family always does.
    if direction > 0.0 {
        [0xFF; 3]
    } else {
        [0x00; 3]
    }
}

fn to_linear(channel: f64) -> f64 {
    if channel <= 0.040_45 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(channel: f64) -> f64 {
    let channel = channel.clamp(0.0, 1.0);
    if channel <= 0.003_130_8 {
        12.92 * channel
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

fn to_byte(channel: f64) -> u8 {
    // Clamped to 0–255 first, so the cast cannot truncate.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    byte
}

fn linear_rgb(rgb: [u8; 3]) -> [f64; 3] {
    rgb.map(|channel| to_linear(f64::from(channel) / 255.0))
}

// Björn Ottosson's OKLab matrices.
fn to_oklab(rgb: [u8; 3]) -> [f64; 3] {
    let [red, green, blue] = linear_rgb(rgb);
    let l = (0.412_221_470_8 * red + 0.536_332_536_3 * green + 0.051_445_992_9 * blue).cbrt();
    let m = (0.211_903_498_2 * red + 0.680_699_545_1 * green + 0.107_396_956_6 * blue).cbrt();
    let s = (0.088_302_461_9 * red + 0.281_718_837_6 * green + 0.629_978_700_5 * blue).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    ]
}

fn from_oklab(lightness: f64, green_red: f64, blue_yellow: f64) -> [u8; 3] {
    let long = (lightness + 0.396_337_777_4 * green_red + 0.215_803_757_3 * blue_yellow).powi(3);
    let medium = (lightness - 0.105_561_345_8 * green_red - 0.063_854_172_8 * blue_yellow).powi(3);
    let short = (lightness - 0.089_484_177_5 * green_red - 1.291_485_548 * blue_yellow).powi(3);
    [
        to_byte(from_linear(
            4.076_741_662_1 * long - 3.307_711_591_3 * medium + 0.230_969_929_2 * short,
        )),
        to_byte(from_linear(
            -1.268_438_004_6 * long + 2.609_757_401_1 * medium - 0.341_319_396_5 * short,
        )),
        to_byte(from_linear(
            -0.004_196_086_3 * long - 0.703_418_614_7 * medium + 1.707_614_701 * short,
        )),
    ]
}

fn relative_luminance(rgb: [u8; 3]) -> f64 {
    let [red, green, blue] = linear_rgb(rgb);
    0.2126 * red + 0.7152 * green + 0.0722 * blue
}

fn contrast(first: [u8; 3], second: [u8; 3]) -> f64 {
    let (a, b) = (relative_luminance(first), relative_luminance(second));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_colours_match_the_generated_tokens() {
        let tokens = include_str!("../ui/tokens.slint");
        let line = tokens
            .lines()
            .find(|line| line.contains("property <color> surface-chat:"))
            .unwrap();
        let hex = |rgb: [u8; 3]| format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
        assert!(
            line.ends_with(&format!(
                "MellowTheme.is-dark ? {} : {};",
                hex(DARK_SHEET_RGB),
                hex(LIGHT_SHEET_RGB)
            )),
            "{line}"
        );
    }

    #[test]
    fn tints_are_stable_and_spread_over_the_palette() {
        assert_eq!(avatar_tint_index("1"), avatar_tint_index("1"));
        let tints: std::collections::BTreeSet<_> = (1_000_u64..1_064)
            .map(|id| avatar_tint_index(&id.to_string()))
            .collect();
        assert_eq!(tints.len(), 8, "{tints:?}");
        assert!(tints.iter().all(|tint| (0..8).contains(tint)));
    }

    #[test]
    fn role_colours_reach_aa_on_the_sheet_in_both_themes() {
        // The mockup's green already reads on the dark sheet and is kept, as on the web.
        assert_eq!(
            readable_on([0x4F, 0xB3, 0x9A], DARK_SHEET_RGB),
            [0x4F, 0xB3, 0x9A]
        );
        // A dark navy and a pale yellow: each fails on one of the two themes as picked.
        for picked in [0x1F_2A_44, 0xF0_E6_8C, 0x4F_B3_9A] {
            let color = role_color(Some(picked));
            assert!(color.is_set);
            let rgb = |color: Color| [color.red(), color.green(), color.blue()];
            assert!(contrast(rgb(color.dark), DARK_SHEET_RGB) >= MINIMUM_NAME_CONTRAST);
            assert!(contrast(rgb(color.light), LIGHT_SHEET_RGB) >= MINIMUM_NAME_CONTRAST);
        }
        assert!(!role_color(None).is_set);
    }
}
