//! Role colours on names and role mention chips (`web/src/components/role-colors.ts`, DESIGN.md
//! §2.3, §5.4): the hue an admin picked, with its OKLCH lightness moved until it reaches 4.5:1 on
//! the surface it is drawn on, separately in each theme. Colours are `0xRRGGBB`.

use mellow_protocol::{Role, Snowflake};

const MINIMUM_NAME_CONTRAST: f64 = 4.5;
const LIGHTNESS_STEP: f64 = 0.005;
/// Below this relative luminance a surface counts as dark: names get lighter on it, not darker.
const DARK_SURFACE_LUMINANCE: f64 = 0.18;
/// How much of the role colour washes a role mention chip's background over the sheet.
const ROLE_MENTION_WASH_WEIGHT: f64 = 0.2;
const WHITE: u32 = 0x00FF_FFFF;
const BLACK: u32 = 0;

type Rgb = [f64; 3];

/// The member's highest role that has a colour (Discord's rule), or `None`.
#[must_use]
pub fn highest_coloured_role<'r>(roles: &'r [Role], role_ids: &[Snowflake]) -> Option<&'r Role> {
    roles
        .iter()
        .filter(|role| role.color != 0 && role_ids.contains(&role.id))
        .max_by_key(|role| role.position)
}

/// The role colour adjusted for 4.5:1 on `surface`.
#[must_use]
pub fn readable_role_colour(colour: u32, surface: u32) -> u32 {
    let picked = rgb_of(colour);
    let surface = rgb_of(surface);
    if contrast_ratio(picked, surface) >= MINIMUM_NAME_CONTRAST {
        return colour & WHITE;
    }
    let direction = if relative_luminance(surface) < DARK_SURFACE_LUMINANCE {
        1.0
    } else {
        -1.0
    };
    let [lightness, a, b] = to_oklab(picked);
    let mut next = lightness;
    while (0.0..=1.0).contains(&next) {
        // Checked after rounding to 8 bits, since that is the colour drawn.
        let candidate = from_oklab(next, a, b).map(|channel| f64::from(to_byte(channel)) / 255.0);
        if contrast_ratio(candidate, surface) >= MINIMUM_NAME_CONTRAST {
            return to_colour(candidate);
        }
        next += direction * LIGHTNESS_STEP;
    }
    // Out of gamut before reaching the ratio: the surface's own text colour family always does.
    if direction > 0.0 { WHITE } else { BLACK }
}

/// A role mention chip's colours: its text is the role colour adjusted to 4.5:1 on its background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleMentionChipColours {
    pub background: u32,
    pub text: u32,
}

/// The chip of a role mention on `surface` (DESIGN.md §5.4): the sheet washed with the role
/// colour, and the role colour made readable on that wash. Hovered, the two swap, which keeps the
/// same contrast.
#[must_use]
pub fn role_mention_chip_colours(colour: u32, surface: u32) -> RoleMentionChipColours {
    let picked = rgb_of(colour);
    let sheet = rgb_of(surface);
    let wash = to_colour(
        [0, 1, 2]
            .map(|index| sheet[index] + (picked[index] - sheet[index]) * ROLE_MENTION_WASH_WEIGHT),
    );
    RoleMentionChipColours {
        background: wash,
        text: readable_role_colour(colour, wash),
    }
}

/// The WCAG contrast ratio of two `0xRRGGBB` colours.
#[must_use]
pub fn contrast_ratio_of_colours(left: u32, right: u32) -> f64 {
    contrast_ratio(rgb_of(left), rgb_of(right))
}

fn contrast_ratio(left: Rgb, right: Rgb) -> f64 {
    let (left, right) = (relative_luminance(left), relative_luminance(right));
    let (lighter, darker) = if left >= right {
        (left, right)
    } else {
        (right, left)
    };
    (lighter + 0.05) / (darker + 0.05)
}

fn rgb_of(colour: u32) -> Rgb {
    [16, 8, 0].map(|shift| f64::from((colour >> shift) & 0xFF) / 255.0)
}

fn to_colour(rgb: Rgb) -> u32 {
    rgb.into_iter().fold(0, |colour, channel| {
        (colour << 8) | u32::from(to_byte(channel))
    })
}

fn to_byte(channel: f64) -> u8 {
    // Clamped to 0–255 first, so the cast cannot truncate.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    byte
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

fn relative_luminance(rgb: Rgb) -> f64 {
    let [red, green, blue] = rgb.map(to_linear);
    0.2126 * red + 0.7152 * green + 0.0722 * blue
}

// Björn Ottosson's OKLab matrices.
#[allow(
    clippy::unreadable_literal,
    reason = "the published matrices, digit for digit"
)]
fn to_oklab(rgb: Rgb) -> Rgb {
    let [red, green, blue] = rgb.map(to_linear);
    let l = (0.4122214708 * red + 0.5363325363 * green + 0.0514459929 * blue).cbrt();
    let m = (0.2119034982 * red + 0.6806995451 * green + 0.1073969566 * blue).cbrt();
    let s = (0.0883024619 * red + 0.2817188376 * green + 0.6299787005 * blue).cbrt();
    [
        0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
    ]
}

#[allow(
    clippy::unreadable_literal,
    clippy::many_single_char_names,
    reason = "the published matrices and their L, M, S, a and b, digit for digit"
)]
fn from_oklab(lightness: f64, a: f64, b: f64) -> Rgb {
    let l = (lightness + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m = (lightness - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s = (lightness - 0.0894841775 * a - 1.291485548 * b).powi(3);
    [
        from_linear(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
        from_linear(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
        from_linear(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
    ]
}

#[cfg(test)]
mod tests {
    use mellow_protocol::Permissions;

    use super::*;

    const DARK_SHEET: u32 = 0x0022_1B23;
    const WHITE_SHEET: u32 = 0x00FF_FFFF;

    fn role(id: u64, position: u32, color: u32) -> Role {
        Role {
            id: Snowflake::from_raw(id),
            name: id.to_string(),
            color,
            position,
            permissions: Permissions::empty(),
            is_displayed_separately: false,
            is_mentionable: false,
            icon: None,
            unicode_emoji: None,
        }
    }

    #[test]
    fn a_colour_that_already_reads_is_kept() {
        assert_eq!(readable_role_colour(0x004F_B39A, DARK_SHEET), 0x004F_B39A);
    }

    #[test]
    fn names_darken_on_white_and_lighten_on_the_dark_sheet_until_4_5_to_1() {
        let on_white = readable_role_colour(0x004F_B39A, WHITE_SHEET);
        assert!(contrast_ratio_of_colours(on_white, WHITE_SHEET) >= 4.5);
        let on_dark = readable_role_colour(0x003A_1F6B, DARK_SHEET);
        assert!(contrast_ratio_of_colours(on_dark, DARK_SHEET) >= 4.5);
    }

    #[test]
    fn the_highest_role_with_a_colour_among_the_members_roles_wins() {
        let roles = [
            role(1, 9, 0),
            role(2, 1, 0x0011_1111),
            role(3, 5, 0x004F_B39A),
            role(4, 7, 0x0022_2222),
        ];
        let ids = |raw: &[u64]| {
            raw.iter()
                .copied()
                .map(Snowflake::from_raw)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            highest_coloured_role(&roles, &ids(&[1, 2, 3])).map(|role| role.id),
            Some(Snowflake::from_raw(3))
        );
        assert_eq!(highest_coloured_role(&roles, &ids(&[1])), None);
    }

    #[test]
    fn a_chip_washes_the_sheet_and_keeps_4_5_to_1_for_its_text_in_both_themes() {
        for sheet in [DARK_SHEET, WHITE_SHEET] {
            for colour in [
                0x004F_B39A,
                0x003A_1F6B,
                0x00F1_C40F,
                0x00FF_FFFF,
                0x0001_0101,
            ] {
                let chip = role_mention_chip_colours(colour, sheet);
                // A role as light as the white sheet washes nothing; its text still reads.
                if !(colour == 0x00FF_FFFF && sheet == WHITE_SHEET) {
                    assert_ne!(chip.background, sheet, "{colour:06X} on {sheet:06X}");
                }
                assert!(contrast_ratio_of_colours(chip.text, chip.background) >= 4.5);
            }
        }
    }
}
