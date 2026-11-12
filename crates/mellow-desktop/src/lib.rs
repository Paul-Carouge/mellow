//! Mellow's native desktop client on GPUI (ADR-0051): the theme generated from the design tokens,
//! the interface strings generated from the web's catalogues, the bridge to the Tokio runtime the
//! network runs on, the session and its sign-in screens, and the Dusk shell.

// Upstream cause: macOS ld warns once the unit-test binary (which links the whole server for the
// sign-in tests) holds more than 16 MB of `__eh_frame`, which its compact unwind table cannot
// encode; only the speed of unwinding a panic in a test is affected.
#![cfg_attr(all(test, target_os = "macos"), allow(linker_messages))]

pub mod attachment_audio;
pub mod column_widths;
pub mod components;
pub mod composer;
pub mod connection_banner;
pub mod conversation;
pub mod dock_and_tray;
pub mod emoji_picker;
pub mod gif_picker;
pub mod home;
pub mod i18n;
mod local_widgets;
pub mod main_window;
pub mod message_search;
pub mod messages;
pub mod navigation;
pub mod notifier;
pub mod pictures;
pub mod preferences;
pub mod session;
pub mod session_store;
pub mod settings;
pub mod shell;
#[cfg(test)]
mod shell_fixtures;
pub mod sign_in_form;
pub mod sign_in_screen;
pub mod strings_pending_i18n;
pub mod theme;
pub mod tokio_bridge;
pub mod window_chrome;

use anyhow::{Context as _, Result};
use gpui::{App, AppContext as _, KeyBinding, Menu, MenuItem, actions};
use tokio::runtime::Handle;

use crate::main_window::MainWindow;
use crate::session::Session;
use crate::window_chrome::WINDOW_TITLE;

actions!(mellow, [Quit]);

/// The app's identity for the system's notifications, as the secret store names it.
const APP_IDENTIFIER: &str = "org.mellow.desktop";

#[cfg(target_os = "macos")]
const QUIT_KEYSTROKE: &str = "cmd-q";
#[cfg(not(target_os = "macos"))]
const QUIT_KEYSTROKE: &str = "ctrl-q";

/// The app menu, in the language spoken; set again when the language changes.
fn set_app_menus(cx: &mut App) {
    let quit_label = i18n::strings(cx).desktop().quit_mellow();
    // Signing out is in User settings, as the web keeps it out of the menu bar.
    cx.set_menus([Menu::new(WINDOW_TITLE).items([MenuItem::action(quit_label, Quit)])]);
}

/// Sets the app up and opens the main window; called once from GPUI's `run`.
///
/// # Errors
/// When the embedded fonts cannot be registered or the window cannot open.
pub fn start(network_runtime: Handle, cx: &mut App) -> Result<()> {
    // Its behaviour (text input, overlays, focus) is what Mellow takes from it; it also reads the
    // OS reduced-motion setting into `App::reduce_motion`, but follows it live on Linux only.
    gpui_component::init(cx);
    theme::register_embedded_fonts(cx)?;
    components::bind_tab_navigation(cx);
    shell::bind_shell_keys(cx);
    composer::bind_composer_keys(cx);
    emoji_picker::bind_emoji_picker_keys(cx);
    tokio_bridge::install_network_runtime(network_runtime, cx);
    preferences::load_preferences_in_background(cx);
    // Before any notification: Linux names the app with it, Windows groups its toasts by it.
    cx.set_app_identity(APP_IDENTIFIER, WINDOW_TITLE);
    let session = cx.new(Session::new_restoring_stored_session);
    Session::end_connection_when_the_app_quits(&session, cx);

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new(QUIT_KEYSTROKE, Quit, None)]);
    i18n::use_system_language(cx);
    set_app_menus(cx);
    dock_and_tray::install_menu_bar_item(cx);
    // One window for now: closing it ends the app, on macOS too.
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();

    if let Some(forced_appearance) = components::gallery::gallery_requested() {
        cx.open_window(
            components::gallery::gallery_window_options(cx),
            |window, cx| components::gallery::open_gallery(forced_appearance, window, cx),
        )
        .context("cannot open the component gallery")?;
        cx.activate(true);
        return Ok(());
    }
    // The theme and language chosen in User settings, once the preferences are read and at
    // each change; most changes are of something else, so each applies only what differs.
    cx.observe_global::<preferences::PreferencesStore>(|cx| {
        if theme::apply_chosen_theme(cx) {
            cx.refresh_windows();
        }
        if i18n::apply_chosen_language(cx) {
            set_app_menus(cx);
            dock_and_tray::retitle_menu_bar_item(cx);
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
            MainWindow::new(session, window, cx)
        })
    })
    .context("cannot open the main window")?;
    cx.activate(true);
    Ok(())
}
