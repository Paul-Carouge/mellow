//! Spike tool: saves what the window renders, so the prototype can be inspected without
//! screen-recording permissions. Writes uncompressed TGA (no image dependency needed).

use std::path::{Path, PathBuf};
use std::time::Duration;

use slint::ComponentHandle as _;

use crate::generated_ui::AppWindow;

pub const SNAPSHOT_DIRECTORY_VARIABLE: &str = "MELLOW_SPIKE_SNAPSHOT_DIRECTORY";
const DELAY_BEFORE_FIRST_SNAPSHOT: Duration = Duration::from_millis(1500);
const SCROLL_DISTANCE_FOR_HISTORY_SNAPSHOT_PIXELS: f32 = 2400.0;
const TGA_HEADER_LENGTH: usize = 18;
const TGA_UNCOMPRESSED_TRUE_COLOR: u8 = 2;
const TGA_BITS_PER_PIXEL: u8 = 32;
/// 8 alpha bits, origin at the top-left corner.
const TGA_IMAGE_DESCRIPTOR: u8 = 0x28;

pub fn schedule(window: &AppWindow, directory: PathBuf) {
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
