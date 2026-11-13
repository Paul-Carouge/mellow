//! Mellow keeps running when its window closes, as Discord does (ADR-0056): the session, the
//! gateway, the voice call and the notifications go on, and the window opens again from the menu
//! bar item or the Dock (macOS), the tray (Linux, Windows), a notification, or launching Mellow
//! again (`single_instance`). Closing really closes the window, on every system: Wayland has no
//! way to hide one, and a closed window frees its views; the next one opens at the size, place
//! and state (maximised, full screen) the last one had, with the drafts kept on disk.
//!
//! Quitting is Cmd/Ctrl+Q, the app menu's or the tray's Quit: the call is left first
//! (`voice::leave_the_call_when_the_app_quits`).

use anyhow::{Context as _, Result};
use gpui::{App, AppContext as _, Bounds, Entity, Global, WindowBounds, WindowHandle};

use crate::dock_and_tray;
use crate::main_window::MainWindow;
use crate::preferences::preferences;
use crate::session::{Session, SessionPhase};
use crate::{theme, window_chrome};

/// What closing the main window does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowCloseOutcome {
    /// Mellow stays in the menu bar, the Dock or the tray.
    KeepRunningInTheBackground,
    Quit,
}

/// What [`window_close_outcome`] decides from.
#[derive(Debug, Clone, Copy)]
pub struct WindowCloseFacts {
    /// "Keep running in the background" turned off in User settings, Appearance.
    pub does_closing_the_window_quit: bool,
    /// The menu bar item, the Dock or a tray icon can bring the window back. A Linux desktop
    /// without a `StatusNotifierItem` host (GNOME without the `AppIndicator` extension) has none.
    pub can_be_reopened_from_the_system: bool,
    pub is_signed_out: bool,
}

/// Mellow keeps running only when it is asked to, can be seen running, and has something to keep
/// doing: hidden with no visible trace, a call would keep the microphone open unseen, and signed
/// out there is no call or notification to keep.
#[must_use]
pub fn window_close_outcome(facts: WindowCloseFacts) -> WindowCloseOutcome {
    if facts.does_closing_the_window_quit
        || !facts.can_be_reopened_from_the_system
        || facts.is_signed_out
    {
        WindowCloseOutcome::Quit
    } else {
        WindowCloseOutcome::KeepRunningInTheBackground
    }
}

/// The app's main window, while it is open, and the session and bounds a new one opens with.
struct MainWindowSlot {
    session: Entity<Session>,
    window: Option<WindowHandle<MainWindow>>,
    /// The last window's, for the next one; kept in memory only, as closing does not quit.
    bounds: Option<WindowBounds>,
}

impl Global for MainWindowSlot {}

/// Opens the main window and keeps the app running when it closes; called once at start.
///
/// # Errors
/// When the window cannot open.
pub fn open_main_window_and_stay_in_background(
    session: Entity<Session>,
    cx: &mut App,
) -> Result<()> {
    cx.set_global(MainWindowSlot {
        session,
        window: None,
        bounds: None,
    });
    cx.on_window_closed(|cx, closed_window_id| {
        let is_main_window = cx
            .try_global::<MainWindowSlot>()
            .and_then(|slot| slot.window)
            .is_some_and(|window| window.window_id() == closed_window_id);
        if is_main_window {
            cx.global_mut::<MainWindowSlot>().window = None;
        }
        if !cx.windows().is_empty() {
            return;
        }
        let is_signed_out = cx
            .try_global::<MainWindowSlot>()
            .is_none_or(|slot| slot.session.read(cx).phase() == SessionPhase::SignedOut);
        let outcome = window_close_outcome(WindowCloseFacts {
            does_closing_the_window_quit: preferences(cx)
                .is_some_and(|preferences| preferences.does_closing_the_window_quit),
            can_be_reopened_from_the_system: dock_and_tray::can_show_mellow_again(cx),
            is_signed_out,
        });
        match outcome {
            WindowCloseOutcome::Quit => cx.quit(),
            WindowCloseOutcome::KeepRunningInTheBackground => {
                tracing::info!("the window closed: Mellow keeps running in the background");
            }
        }
    })
    .detach();
    let window = open_main_window(cx)?;
    cx.global_mut::<MainWindowSlot>().window = Some(window);
    cx.activate(true);
    Ok(())
}

/// Brings the main window to the front, opening it again if Mellow ran in the background.
pub fn show_main_window(cx: &mut App) {
    let Some(slot) = cx.try_global::<MainWindowSlot>() else {
        return;
    };
    cx.activate(true);
    if let Some(window) = slot.window {
        let shown = window.update(cx, |_, window, _| window.activate_window());
        if shown.is_ok() {
            return;
        }
    }
    match open_main_window(cx) {
        Ok(window) => cx.global_mut::<MainWindowSlot>().window = Some(window),
        Err(error) => tracing::error!("{error:#}"),
    }
}

fn open_main_window(cx: &mut App) -> Result<WindowHandle<MainWindow>> {
    let slot = cx.global::<MainWindowSlot>();
    let session = slot.session.clone();
    let remembered_bounds = slot.bounds;
    let mut options = window_chrome::main_window_options(cx);
    if let Some(bounds) = remembered_bounds {
        options.window_bounds = Some(placed_on_a_display(bounds, cx));
    }
    cx.open_window(options, |window, cx| {
        theme::follow_window_appearance(window, cx);
        cx.new(|cx| {
            // Moves, resizes, maximising and full screen all report here; read only when the
            // next window opens.
            cx.observe_window_bounds(window, |_, window, cx| {
                cx.global_mut::<MainWindowSlot>().bounds = Some(window.window_bounds());
            })
            .detach();
            // macOS and Windows post no reduced-motion change gpui-base can follow, so the
            // setting is read again whenever the window comes back to the front.
            cx.observe_window_activation(window, |_, _, cx| {
                gpui_base::apply_system_reduce_motion(cx);
            })
            .detach();
            MainWindow::new(session, window, cx)
        })
    })
    .context("cannot open the main window")
}

/// The remembered bounds, centred again when their display was unplugged since. Wayland places
/// windows itself and keeps only the size and state.
fn placed_on_a_display(bounds: WindowBounds, cx: &App) -> WindowBounds {
    let restored = bounds.get_bounds();
    let is_on_a_display = cx
        .displays()
        .iter()
        .any(|display| display.bounds().intersects(&restored));
    if is_on_a_display {
        return bounds;
    }
    let centred = Bounds::centered(None, restored.size, cx);
    match bounds {
        WindowBounds::Windowed(_) => WindowBounds::Windowed(centred),
        WindowBounds::Maximized(_) => WindowBounds::Maximized(centred),
        WindowBounds::Fullscreen(_) => WindowBounds::Fullscreen(centred),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> WindowCloseFacts {
        WindowCloseFacts {
            does_closing_the_window_quit: false,
            can_be_reopened_from_the_system: true,
            is_signed_out: false,
        }
    }

    #[test]
    fn closing_the_window_keeps_mellow_running_by_default() {
        assert_eq!(
            window_close_outcome(facts()),
            WindowCloseOutcome::KeepRunningInTheBackground
        );
    }

    #[test]
    fn closing_quits_when_asked_without_a_tray_or_signed_out() {
        let quits_when_asked = WindowCloseFacts {
            does_closing_the_window_quit: true,
            ..facts()
        };
        let without_a_tray = WindowCloseFacts {
            can_be_reopened_from_the_system: false,
            ..facts()
        };
        let signed_out = WindowCloseFacts {
            is_signed_out: true,
            ..facts()
        };
        for facts in [quits_when_asked, without_a_tray, signed_out] {
            assert_eq!(
                window_close_outcome(facts),
                WindowCloseOutcome::Quit,
                "{facts:?}"
            );
        }
    }
}
