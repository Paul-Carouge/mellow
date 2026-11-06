//! Mellow's native desktop client on GPUI (ADR-0051): the theme generated from the design tokens,
//! the interface strings generated from the web's catalogues, the bridge to the Tokio runtime the
//! network runs on, and the Dusk shell.

pub mod i18n;
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
    // OS reduced-motion setting into `App::reduce_motion`, but follows it live on Linux only.
    gpui_component::init(cx);
    theme::register_embedded_fonts(cx)?;
    tokio_bridge::install_network_runtime(network_runtime, cx);

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new(QUIT_KEYSTROKE, Quit, None)]);
    i18n::use_language(i18n::Language::from_operating_system(), cx);
    let quit_label = i18n::strings(cx).desktop().quit_mellow();
    cx.set_menus([Menu::new(WINDOW_TITLE).items([MenuItem::action(quit_label, Quit)])]);
    // One window for now: closing it ends the app, on macOS too.
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();

    cx.open_window(window_chrome::main_window_options(cx), |window, cx| {
        theme::follow_window_appearance(window, cx);
        cx.new(|cx| {
            // macOS and Windows post no reduced-motion change gpui-base can follow, so the
            // setting is read again whenever the window comes back to the front.
            cx.observe_window_activation(window, |_, _, cx| {
                gpui_base::apply_system_reduce_motion(cx);
            })
            .detach();
            DuskShell::default()
        })
    })
    .context("cannot open the main window")?;
    cx.activate(true);
    Ok(())
}
