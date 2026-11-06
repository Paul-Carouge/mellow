//! Mellow's native desktop client on GPUI (ADR-0051): the theme generated from the design tokens,
//! the bridge to the Tokio runtime the network runs on, and the Dusk shell.

pub mod shell;
pub mod theme;
pub mod tokio_bridge;
pub mod window_chrome;

use anyhow::{Context as _, Result};
use gpui::{App, AppContext as _, KeyBinding, Menu, MenuItem, actions};
use tokio::runtime::Handle;

use crate::shell::DuskShell;
use crate::window_chrome::WINDOW_TITLE;

actions!(mellow, [Quit]);

#[cfg(target_os = "macos")]
const QUIT_KEYSTROKE: &str = "cmd-q";
#[cfg(not(target_os = "macos"))]
const QUIT_KEYSTROKE: &str = "ctrl-q";

/// Sets the app up and opens the main window; called once from GPUI's `run`.
///
/// # Errors
/// When the embedded fonts cannot be registered or the window cannot open.
pub fn start(network_runtime: Handle, cx: &mut App) -> Result<()> {
    // Its behaviour (text input, overlays, focus) is what Mellow takes from it; it also reads the
    // OS reduced-motion setting into `App::reduce_motion` and follows it.
    gpui_component::init(cx);
    theme::register_embedded_fonts(cx)?;
    tokio_bridge::install_network_runtime(network_runtime, cx);

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new(QUIT_KEYSTROKE, Quit, None)]);
    // shortcut: English menu label; the interface strings come from the web's catalogues later.
    cx.set_menus([Menu::new(WINDOW_TITLE).items([MenuItem::action("Quit Mellow", Quit)])]);
    // One window for now: closing it ends the app, on macOS too.
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();

    cx.open_window(window_chrome::main_window_options(cx), |window, cx| {
        theme::follow_window_appearance(window, cx);
        cx.new(|_| DuskShell::default())
    })
    .context("cannot open the main window")?;
    cx.activate(true);
    Ok(())
}
