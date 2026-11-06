//! The main window's options and chrome.
//!
//! macOS: the title bar is transparent, so the dusk ground runs under it and the traffic lights sit
//! in a ground band above the cards; `AppKit` still drags the window by that band.
//! Linux: server-side decorations for now (KISS): the system draws a title bar above the ground.
//! Client-side decorations (our own resize edges, shadow and buttons) are later polish.

use gpui::{
    App, Bounds, Pixels, Size, TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions,
    point, px, size,
};

use crate::theme::tokens::{
    SIZE_SHEET_INSET, SIZE_WINDOW_DEFAULT_HEIGHT, SIZE_WINDOW_DEFAULT_WIDTH,
    SIZE_WINDOW_MIN_HEIGHT, SIZE_WINDOW_MIN_WIDTH, SPACE_MD,
};

pub const WINDOW_TITLE: &str = "Mellow";
/// The Linux desktop entry's name, so docks group the window with it.
const APP_ID: &str = "mellow";

/// Height of the ground band the macOS traffic lights sit in; nothing elsewhere.
#[cfg(target_os = "macos")]
pub const TITLE_BAR_BAND_HEIGHT: f32 = crate::theme::tokens::SPACE_XL;
#[cfg(not(target_os = "macos"))]
pub const TITLE_BAR_BAND_HEIGHT: f32 = 0.0;

/// The ground above the cards: twice the inset, as on the web, or the title bar band if taller.
pub const TOP_BAND_HEIGHT: f32 = if TITLE_BAR_BAND_HEIGHT > 2.0 * SIZE_SHEET_INSET {
    TITLE_BAR_BAND_HEIGHT
} else {
    2.0 * SIZE_SHEET_INSET
};

/// The close, minimise and zoom buttons of macOS are this tall; centring them in the band needs it.
const MACOS_WINDOW_BUTTON_HEIGHT: f32 = 14.0;
/// The default window covers at most this share of the screen (DESIGN.md §4.2).
const DEFAULT_WINDOW_SHARE_OF_SCREEN: f32 = 0.8;

#[must_use]
pub fn main_window_options(cx: &App) -> WindowOptions {
    let screen_size = cx.primary_display().map(|display| display.bounds().size);
    let minimum_size = size(px(SIZE_WINDOW_MIN_WIDTH), px(SIZE_WINDOW_MIN_HEIGHT));
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            default_window_size(screen_size),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(WINDOW_TITLE.into()),
            appears_transparent: cfg!(target_os = "macos"),
            traffic_light_position: cfg!(target_os = "macos").then(|| {
                point(
                    px(SPACE_MD),
                    px((TITLE_BAR_BAND_HEIGHT - MACOS_WINDOW_BUTTON_HEIGHT) / 2.0),
                )
            }),
        }),
        app_id: Some(APP_ID.to_owned()),
        window_min_size: Some(minimum_size),
        window_decorations: Some(WindowDecorations::Server),
        ..WindowOptions::default()
    }
}

/// 1280 × 800, or 80 % of a smaller screen, never below the minimum window size.
fn default_window_size(screen_size: Option<Size<Pixels>>) -> Size<Pixels> {
    let fit = |default: f32, minimum: f32, screen: Option<Pixels>| {
        let available = screen.map_or(default, |screen| {
            f32::from(screen) * DEFAULT_WINDOW_SHARE_OF_SCREEN
        });
        px(default.min(available).max(minimum))
    };
    size(
        fit(
            SIZE_WINDOW_DEFAULT_WIDTH,
            SIZE_WINDOW_MIN_WIDTH,
            screen_size.map(|screen| screen.width),
        ),
        fit(
            SIZE_WINDOW_DEFAULT_HEIGHT,
            SIZE_WINDOW_MIN_HEIGHT,
            screen_size.map(|screen| screen.height),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_window_fits_small_screens_but_never_below_the_minimum() {
        assert_eq!(
            default_window_size(None),
            size(
                px(SIZE_WINDOW_DEFAULT_WIDTH),
                px(SIZE_WINDOW_DEFAULT_HEIGHT)
            )
        );
        assert_eq!(
            default_window_size(Some(size(px(2560.0), px(1440.0)))),
            size(px(1280.0), px(800.0))
        );
        let small_screen = default_window_size(Some(size(px(1366.0), px(768.0))));
        assert!((f32::from(small_screen.width) - 1092.8).abs() < 0.01);
        assert!((f32::from(small_screen.height) - 614.4).abs() < 0.01);
        assert_eq!(
            default_window_size(Some(size(px(400.0), px(400.0)))),
            size(px(SIZE_WINDOW_MIN_WIDTH), px(SIZE_WINDOW_MIN_HEIGHT))
        );
    }
}
