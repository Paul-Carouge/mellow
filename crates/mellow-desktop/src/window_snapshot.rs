//! Spike tool: saves what the window renders, so the prototype can be inspected without
//! screen-recording permissions. Writes uncompressed TGA (no image dependency needed).

use std::path::{Path, PathBuf};
use std::time::Duration;

use slint::{ComponentHandle as _, SharedString};

use crate::generated_ui::{AppWindow, ThemeChoice};

pub const SNAPSHOT_DIRECTORY_VARIABLE: &str = "MELLOW_SPIKE_SNAPSHOT_DIRECTORY";
const DELAY_BEFORE_FIRST_SNAPSHOT: Duration = Duration::from_millis(1500);
const SCROLL_DISTANCE_FOR_HISTORY_SNAPSHOT_PIXELS: f32 = 2400.0;
const TGA_HEADER_LENGTH: usize = 18;
const TGA_UNCOMPRESSED_TRUE_COLOR: u8 = 2;
const TGA_BITS_PER_PIXEL: u8 = 32;
/// 8 alpha bits, origin at the top-left corner.
const TGA_IMAGE_DESCRIPTOR: u8 = 0x28;

/// Optional window size for snapshots, as `WIDTHxHEIGHT` in logical pixels (e.g. `1440x900`).
const SNAPSHOT_SIZE_VARIABLE: &str = "MELLOW_SPIKE_SNAPSHOT_SIZE";
/// Optional theme for snapshots: `dark` or `light`.
const SNAPSHOT_THEME_VARIABLE: &str = "MELLOW_SPIKE_SNAPSHOT_THEME";
/// Optional state to show before the first snapshot: `members`, `drawer`, `settings`, `typing` or `switcher`.
const SNAPSHOT_STATE_VARIABLE: &str = "MELLOW_SPIKE_SNAPSHOT_STATE";
/// Who types in the `typing` state.
const SNAPSHOT_TYPING_NAME: &str = "Basile";

/// Applies the optional size, theme and state, so the shell can be compared with the reference
/// mockup at the same window sizes (ADR-0022).
fn prepare(window: &AppWindow) {
    if let Some((width, height)) = std::env::var(SNAPSHOT_SIZE_VARIABLE)
        .ok()
        .and_then(|size| parse_size(&size))
    {
        window
            .window()
            .set_size(slint::LogicalSize::new(width, height));
    }
    match std::env::var(SNAPSHOT_THEME_VARIABLE).as_deref() {
        Ok("light") => window.set_theme_choice(ThemeChoice::Light),
        Ok("dark") => window.set_theme_choice(ThemeChoice::Dark),
        _ => {}
    }
    match std::env::var(SNAPSHOT_STATE_VARIABLE).as_deref() {
        Ok("members") => window.set_is_member_list_visible(true),
        Ok("drawer") => window.set_is_navigator_drawer_open(true),
        Ok("settings") => window.set_is_user_settings_open(true),
        Ok("typing") => window.set_typing_line(crate::live_rows::typing_line(&[
            SNAPSHOT_TYPING_NAME.to_owned(),
        ])),
        Ok("switcher") => {
            window.set_is_quick_switcher_open(true);
            window.invoke_quick_switcher_query_changed(SharedString::default());
        }
        _ => {}
    }
}

fn parse_size(size: &str) -> Option<(f32, f32)> {
    let (width, height) = size.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}

pub fn schedule(window: &AppWindow, directory: PathBuf) {
    prepare(window);
    let weak_window = window.as_weak();
    slint::Timer::single_shot(DELAY_BEFORE_FIRST_SNAPSHOT, move || {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        save_snapshot(&window, &directory.join("latest-messages.tga"));
        window.invoke_scroll_up_by(SCROLL_DISTANCE_FOR_HISTORY_SNAPSHOT_PIXELS);
        window.set_is_text_selection_mode(true);
        let weak_window = window.as_weak();
        slint::Timer::single_shot(DELAY_BEFORE_FIRST_SNAPSHOT, move || {
            let Some(window) = weak_window.upgrade() else {
                return;
            };
            save_snapshot(&window, &directory.join("history-selection-mode.tga"));
            let _ = slint::quit_event_loop();
        });
    });
}

fn save_snapshot(window: &AppWindow, path: &Path) {
    let result = window
        .window()
        .take_snapshot()
        .map_err(|error| error.to_string())
        .and_then(|pixels| {
            let width = u16::try_from(pixels.width()).map_err(|error| error.to_string())?;
            let height = u16::try_from(pixels.height()).map_err(|error| error.to_string())?;
            let mut file_bytes = Vec::with_capacity(TGA_HEADER_LENGTH + pixels.as_bytes().len());
            file_bytes.extend_from_slice(&[
                0,
                0,
                TGA_UNCOMPRESSED_TRUE_COLOR,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ]);
            file_bytes.extend_from_slice(&width.to_le_bytes());
            file_bytes.extend_from_slice(&height.to_le_bytes());
            file_bytes.extend_from_slice(&[TGA_BITS_PER_PIXEL, TGA_IMAGE_DESCRIPTOR]);
            for pixel in pixels.as_slice() {
                file_bytes.extend_from_slice(&[pixel.b, pixel.g, pixel.r, pixel.a]);
            }
            std::fs::write(path, file_bytes).map_err(|error| error.to_string())
        });
    if let Err(error) = result {
        eprintln!("snapshot {} failed: {error}", path.display());
    }
}
