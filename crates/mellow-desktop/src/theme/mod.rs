//! Turns the generated design tokens into GPUI values: the active theme (the one chosen in User
//! settings, Appearance, or the OS appearance), lengths, colours, motion under reduced motion, and the embedded fonts and icons.

mod assets;
mod fonts;
// Generated files: `cargo xtask tokens` writes them already laid out, and `--check` in CI would
// fail if rustfmt rewrote them.
#[rustfmt::skip]
pub mod icons;
#[rustfmt::skip]
pub mod tokens;

use std::time::Duration;

use gpui::{App, Global, Hsla, Rems, Window, WindowAppearance, rems, rgba};

use crate::preferences::{ThemeChoice, preferences};
pub use assets::EmbeddedAssets;
#[cfg(target_os = "linux")]
pub use fonts::redraw_with_system_fonts_once_read;
pub use fonts::{MONOSPACE_FONT_FAMILY, SANS_FONT_FAMILY, interface_font, register_embedded_fonts};
use tokens::{AnimationDuration, DARK_THEME, Delay, LIGHT_THEME, ThemeTokens};

/// Token lengths are logical pixels at 100 % zoom; GPUI's rem is the window's root font size, so
/// lengths given in rems follow a zoom set through `Window::set_rem_size` (DESIGN.md §7.5).
const PIXELS_PER_REM_AT_DEFAULT_ZOOM: f32 = 16.0;

/// The colours and shadows of the theme the window shows.
#[derive(Debug, Clone, Copy)]
pub struct ActiveTheme {
    pub tokens: &'static ThemeTokens,
}

impl Global for ActiveTheme {}

impl ActiveTheme {
    /// Dark is Mellow's default theme (DESIGN.md §2); vibrant variants are macOS's own names for
    /// the same light or dark appearance.
    #[must_use]
    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        let tokens = match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => &LIGHT_THEME,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => &DARK_THEME,
        };
        Self { tokens }
    }
}

/// The OS appearance last seen, so choosing "Same as this computer" again knows it.
#[derive(Debug, Clone, Copy)]
struct SystemAppearance(WindowAppearance);

impl Global for SystemAppearance {}

/// Sets the theme from the reader's choice, else the window's appearance, and keeps following
/// the OS while the window lives.
pub fn follow_window_appearance(window: &mut Window, cx: &mut App) {
    cx.set_global(SystemAppearance(window.appearance()));
    apply_chosen_theme(cx);
    window
        .observe_window_appearance(|window, cx| {
            cx.set_global(SystemAppearance(window.appearance()));
            if apply_chosen_theme(cx) {
                window.refresh();
            }
        })
        .detach();
}

/// The appearance a theme choice shows when the OS shows `system`.
#[must_use]
pub fn chosen_appearance(choice: ThemeChoice, system: WindowAppearance) -> WindowAppearance {
    match choice {
        ThemeChoice::System => system,
        ThemeChoice::Light => WindowAppearance::Light,
        ThemeChoice::Dark => WindowAppearance::Dark,
    }
}

/// Installs the theme the preferences ask for; true when it changed, so the caller redraws.
pub fn apply_chosen_theme(cx: &mut App) -> bool {
    let system = cx
        .try_global::<SystemAppearance>()
        .map_or(WindowAppearance::Dark, |appearance| appearance.0);
    let choice = preferences(cx)
        .map(|preferences| preferences.theme)
        .unwrap_or_default();
    let wanted = ActiveTheme::for_appearance(chosen_appearance(choice, system));
    let is_shown = cx
        .try_global::<ActiveTheme>()
        .is_some_and(|active| std::ptr::eq(active.tokens, wanted.tokens));
    if !is_shown {
        apply_theme(wanted, cx);
    }
    !is_shown
}

/// Installs a theme, including in gpui-component's text input, which paints from its own theme.
pub fn apply_theme(active_theme: ActiveTheme, cx: &mut App) {
    cx.set_global(active_theme);
    crate::components::sync_text_input_colours(active_theme.tokens, cx);
}

/// The theme's tokens; `follow_window_appearance` installs the global before any view renders.
#[must_use]
pub fn theme(cx: &App) -> &'static ThemeTokens {
    cx.global::<ActiveTheme>().tokens
}

/// A `0xRRGGBBAA` colour token as a GPUI colour.
#[must_use]
pub fn color(token: u32) -> Hsla {
    rgba(token).into()
}

/// A length token in rems, so zoom scales it.
#[must_use]
pub fn length(pixels: f32) -> Rems {
    rems(pixels / PIXELS_PER_REM_AT_DEFAULT_ZOOM)
}

/// The window's width in logical pixels at 100 % zoom, the unit of the breakpoints.
#[must_use]
pub fn window_width_at_default_zoom(window: &Window) -> f32 {
    f32::from(window.viewport_size().width) * PIXELS_PER_REM_AT_DEFAULT_ZOOM
        / f32::from(window.rem_size())
}

/// How long an animation runs: nothing under reduced motion (DESIGN.md §2.8). `gpui_component::init`
/// reads the OS setting into `App::reduce_motion`; `start` re-reads it when the window is activated.
#[must_use]
pub fn animation_duration(token: AnimationDuration, cx: &App) -> Duration {
    if cx.reduce_motion() {
        Duration::ZERO
    } else {
        Duration::from_millis(token.0)
    }
}

/// Delays keep their value under reduced motion, or content would flash.
#[must_use]
pub fn delay(token: Delay) -> Duration {
    Duration::from_millis(token.0)
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    #[test]
    fn the_os_appearance_picks_the_theme() {
        for appearance in [WindowAppearance::Dark, WindowAppearance::VibrantDark] {
            assert_eq!(*ActiveTheme::for_appearance(appearance).tokens, DARK_THEME);
        }
        for appearance in [WindowAppearance::Light, WindowAppearance::VibrantLight] {
            assert_eq!(*ActiveTheme::for_appearance(appearance).tokens, LIGHT_THEME);
        }
        assert_ne!(
            DARK_THEME.color_surface_rail, LIGHT_THEME.color_surface_rail,
            "the ground differs per theme"
        );
    }

    #[gpui::test]
    fn a_chosen_theme_wins_over_the_os_until_the_system_is_chosen_again(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.set_global(SystemAppearance(WindowAppearance::Dark));
            apply_chosen_theme(cx);
            assert_eq!(*theme(cx), DARK_THEME);
            crate::preferences::change_preferences(cx, |preferences| {
                preferences.theme = ThemeChoice::Light;
            });
            assert!(apply_chosen_theme(cx));
            assert_eq!(*theme(cx), LIGHT_THEME);
            assert!(
                !apply_chosen_theme(cx),
                "nothing to redraw when nothing changed"
            );
            crate::preferences::change_preferences(cx, |preferences| {
                preferences.theme = ThemeChoice::System;
            });
            assert!(apply_chosen_theme(cx));
            assert_eq!(*theme(cx), DARK_THEME);
        });
    }

    #[test]
    fn colour_tokens_keep_their_alpha() {
        let scrim = color(0x0804_09B8);
        assert!((scrim.a - f32::from(0xB8_u8) / 255.0).abs() < 1e-6);
    }

    #[test]
    fn lengths_become_rems_of_the_default_root_size() {
        assert_eq!(length(tokens::SIZE_RAIL_WIDTH), rems(4.5));
    }

    #[test]
    fn the_generated_breakpoints_match_the_web_client() {
        assert!((tokens::BREAKPOINT_NAVIGATOR - 816.0).abs() < f32::EPSILON);
        assert!((tokens::BREAKPOINT_SIDE_SHEET - 1064.0).abs() < f32::EPSILON);
    }

    #[gpui::test]
    fn reduced_motion_stops_animations_but_keeps_delays(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settle = tokens::MOTION_DURATION_SETTLE;
            assert_eq!(animation_duration(settle, cx), Duration::from_millis(340));
            assert_eq!(
                delay(tokens::MOTION_DELAY_TOOLTIP),
                Duration::from_millis(500)
            );

            cx.set_reduce_motion(true);
            assert_eq!(animation_duration(settle, cx), Duration::ZERO);
            assert_eq!(
                delay(tokens::MOTION_DELAY_TOOLTIP),
                Duration::from_millis(500)
            );
        });
    }
}
