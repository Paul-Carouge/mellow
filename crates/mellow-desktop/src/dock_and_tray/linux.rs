//! The Linux tray: a `StatusNotifierItem` over the session bus (`ksni` on the async-io zbus the
//! portals already use). KDE Plasma, Hyprland's and Sway's bars (Waybar), XFCE and Cinnamon host
//! it; GNOME only with the `AppIndicator` extension. Without a host, `is_tray_shown` is false and
//! closing the window quits (ADR-0055).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{App, Global};
use ksni::menu::StandardItem;
use ksni::{Handle, Icon, MenuItem, Status, ToolTip, TrayMethods as _};
use tokio::sync::mpsc::Sender;

use super::{TrayCommand, TrayMenu, tray_icon_rgba};
use crate::APP_IDENTIFIER;
use crate::tokio_bridge::spawn_on_network_runtime;
use crate::window_chrome::WINDOW_TITLE;

/// Hosts scale the closest size; a panel icon is 16 to 48 pixels.
const TRAY_ICON_SIZES: [u32; 2] = [32, 64];

struct MellowTray {
    menu: TrayMenu,
    icons: Vec<Icon>,
    commands: Sender<TrayCommand>,
    /// Whether a host shows the item: it goes when the bar restarts or the extension is off.
    is_shown: Arc<AtomicBool>,
}

impl MellowTray {
    fn send(&self, command: TrayCommand) {
        // Full: the person clicked faster than the app follows; this click is dropped.
        let _ = self.commands.try_send(command);
    }
}

/// A label's underscores are access keys to the host: doubled, they show as written.
fn menu_label(text: &str) -> String {
    text.replace('_', "__")
}

fn menu_entry(label: &str, command: TrayCommand) -> MenuItem<MellowTray> {
    StandardItem {
        label: menu_label(label),
        activate: Box::new(move |tray: &mut MellowTray| tray.send(command)),
        ..StandardItem::default()
    }
    .into()
}

impl ksni::Tray for MellowTray {
    fn id(&self) -> String {
        APP_IDENTIFIER.to_owned()
    }

    fn title(&self) -> String {
        WINDOW_TITLE.to_owned()
    }

    fn status(&self) -> Status {
        if self.menu.attention_count > 0 {
            Status::NeedsAttention
        } else {
            Status::Active
        }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icons.clone()
    }

    // Hosts that show the attention state with its own icon get the same one.
    fn attention_icon_pixmap(&self) -> Vec<Icon> {
        self.icons.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self.menu.tooltip.clone(),
            ..ToolTip::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::OpenMellow);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            menu_entry(&self.menu.open, TrayCommand::OpenMellow),
            MenuItem::Separator,
            menu_entry(&self.menu.microphone, TrayCommand::ToggleMicrophone),
            menu_entry(&self.menu.headset, TrayCommand::ToggleDeafened),
            MenuItem::Separator,
            menu_entry(&self.menu.quit, TrayCommand::QuitMellow),
        ]
    }

    fn watcher_online(&self) {
        self.is_shown.store(true, Ordering::SeqCst);
    }

    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        tracing::info!(?reason, "the system tray went away");
        self.is_shown.store(false, Ordering::SeqCst);
        // Kept running: the bar may come back (a restart of the shell).
        true
    }
}

/// The running tray, once a host took it.
struct LinuxTray {
    handle: Handle<MellowTray>,
    is_shown: Arc<AtomicBool>,
}

impl Global for LinuxTray {}

/// ARGB32 in network byte order, as the specification wants it.
fn pixmap(size: u32) -> Option<Icon> {
    let rgba = tray_icon_rgba(size)?;
    let data = rgba
        .pixels()
        .flat_map(|pixel| {
            let [red, green, blue, alpha] = pixel.0;
            [alpha, red, green, blue]
        })
        .collect();
    let edge = i32::try_from(size).ok()?;
    Some(Icon {
        width: edge,
        height: edge,
        data,
    })
}

pub(super) fn install_tray(commands: Sender<TrayCommand>, cx: &mut App) {
    let is_shown = Arc::new(AtomicBool::new(false));
    let tray_is_shown = Arc::clone(&is_shown);
    // Registering talks to the session bus: off the UI thread.
    let registration = spawn_on_network_runtime(cx, async move {
        let tray = MellowTray {
            menu: TrayMenu::default(),
            icons: TRAY_ICON_SIZES.into_iter().filter_map(pixmap).collect(),
            commands,
            is_shown: tray_is_shown,
        };
        tray.spawn().await
    });
    cx.spawn(async move |cx| {
        let handle = match registration.await {
            Ok(Ok(handle)) => handle,
            Ok(Err(error)) => {
                tracing::info!(%error, "no system tray: closing the window quits Mellow");
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "the system tray's registration stopped");
                return;
            }
        };
        is_shown.store(true, Ordering::SeqCst);
        cx.update(|cx| {
            cx.set_global(LinuxTray { handle, is_shown });
            // What changed while it registered.
            if let Some(menu) = cx
                .try_global::<super::ShownTray>()
                .and_then(|shown| shown.menu.clone())
            {
                show_menu(&menu, cx);
            }
        });
    })
    .detach();
}

pub(super) fn show_menu(menu: &TrayMenu, cx: &mut App) {
    let Some(tray) = cx.try_global::<LinuxTray>() else {
        return;
    };
    let handle = tray.handle.clone();
    let menu = menu.clone();
    // The update answers once the bus took it; nothing waits for that.
    spawn_on_network_runtime(cx, async move {
        handle.update(move |tray| tray.menu = menu).await;
    })
    .detach();
}

pub(super) fn is_tray_shown(cx: &App) -> bool {
    cx.try_global::<LinuxTray>()
        .is_some_and(|tray| tray.is_shown.load(Ordering::SeqCst))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn underscores_show_as_written_and_the_icon_is_argb() {
        assert_eq!(menu_label("Open_Mellow"), "Open__Mellow");
        let icon = pixmap(32).unwrap();
        assert_eq!((icon.width, icon.height), (32, 32));
        assert_eq!(icon.data.len(), 32 * 32 * 4);
    }
}
