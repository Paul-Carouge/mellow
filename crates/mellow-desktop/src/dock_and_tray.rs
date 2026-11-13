//! What the app shows outside its window, which GPUI has no API for: the count of what waits for
//! the reader on the Dock icon, and the item that keeps Mellow reachable while its window is closed
//! (ADR-0056): the menu bar item on macOS, a `StatusNotifierItem` over D-Bus on Linux (`ksni`; KDE,
//! Hyprland's and most bars show it, GNOME only with the `AppIndicator` extension), a notification
//! area icon on Windows. Each menu has "Open Mellow", the microphone and headset toggles, and
//! "Quit Mellow"; a click on the icon itself opens Mellow where the system allows it.
//!
//! The count also shows beside the menu bar item, as the Linux item's attention state and in the
//! tray tooltips; a Linux launcher badge (`com.canonical.Unity.LauncherEntry`) and a Windows
//! taskbar overlay are left for later.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod notification_area;

use gpui::{App, Global};
use mellow_client_core::shared_logic::notifications::window_title;

use crate::i18n::strings;
use crate::window_chrome::WINDOW_TITLE;

/// A choice made in the tray's menu, carried to the UI thread.
#[cfg_attr(
    not(any(target_os = "macos", target_os = "linux", windows)),
    allow(dead_code)
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayCommand {
    OpenMellow,
    ToggleMicrophone,
    ToggleDeafened,
    QuitMellow,
}

/// Clicks come at a person's pace: a full queue drops one rather than grow.
#[cfg_attr(
    not(any(target_os = "macos", target_os = "linux", windows)),
    allow(dead_code)
)]
const TRAY_COMMAND_QUEUE_CAPACITY: usize = 8;

/// The words and state the tray shows, rebuilt when the language, the count or the voice
/// toggles change, and applied only when they differ.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TrayMenu {
    open: String,
    microphone: String,
    headset: String,
    quit: String,
    /// "(3) Mellow", as the window's title.
    tooltip: String,
    attention_count: u32,
}

/// What the tray last showed, and the count it shows.
#[derive(Default)]
struct ShownTray {
    attention_count: u32,
    menu: Option<TrayMenu>,
}

impl Global for ShownTray {}

fn tray_menu(attention_count: u32, cx: &App) -> TrayMenu {
    let catalogue = strings(cx);
    let (is_muted, is_deafened) = crate::voice::voice(cx).map_or((false, false), |voice| {
        let toggles = voice.read(cx).toggles();
        (toggles.is_muted(), toggles.is_deafened())
    });
    let user_card = catalogue.user_card();
    TrayMenu {
        open: catalogue.desktop().open_mellow().to_owned(),
        microphone: if is_muted {
            user_card.unmute_microphone()
        } else {
            user_card.mute_microphone()
        }
        .to_owned(),
        headset: if is_deafened {
            user_card.undeafen()
        } else {
            user_card.deafen()
        }
        .to_owned(),
        quit: catalogue.desktop().quit_mellow().to_owned(),
        tooltip: window_title(attention_count, WINDOW_TITLE),
        attention_count,
    }
}

/// Shows the tray's words and state again, if they changed.
fn refresh_tray(cx: &mut App) {
    let attention_count = cx
        .try_global::<ShownTray>()
        .map_or(0, |shown| shown.attention_count);
    let menu = tray_menu(attention_count, cx);
    if cx
        .try_global::<ShownTray>()
        .is_some_and(|shown| shown.menu.as_ref() == Some(&menu))
    {
        return;
    }
    #[cfg(target_os = "macos")]
    macos::show_menu(&menu, cx);
    #[cfg(target_os = "linux")]
    linux::show_menu(&menu, cx);
    #[cfg(windows)]
    notification_area::show_menu(&menu, cx);
    cx.default_global::<ShownTray>().menu = Some(menu);
}

/// Shows `count` on the Dock icon, beside the menu bar item and in the tray; zero clears it.
pub fn show_attention_count(count: u32, cx: &mut App) {
    cx.default_global::<ShownTray>().attention_count = count;
    #[cfg(target_os = "macos")]
    macos::show_attention_count(count, cx);
    refresh_tray(cx);
}

/// Adds the menu bar item or the tray icon; called once at start, after the voice is installed.
pub fn install_menu_bar_item(cx: &mut App) {
    let (sender, mut commands) = tokio::sync::mpsc::channel(TRAY_COMMAND_QUEUE_CAPACITY);
    #[cfg(target_os = "macos")]
    macos::install_menu_bar_item(sender, cx);
    #[cfg(target_os = "linux")]
    linux::install_tray(sender, cx);
    #[cfg(windows)]
    notification_area::install_tray(sender, cx);
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    drop(sender);
    if let Some(voice) = crate::voice::voice(cx) {
        cx.observe(&voice, |_, cx| refresh_tray(cx)).detach();
    }
    refresh_tray(cx);
    cx.spawn(async move |cx| {
        while let Some(command) = commands.recv().await {
            cx.update(|cx| follow_tray_command(command, cx));
        }
    })
    .detach();
}

fn follow_tray_command(command: TrayCommand, cx: &mut App) {
    match command {
        TrayCommand::OpenMellow => crate::background::show_main_window(cx),
        TrayCommand::ToggleMicrophone => {
            if let Some(voice) = crate::voice::voice(cx) {
                voice.update(cx, crate::voice::Voice::toggle_microphone);
            }
        }
        TrayCommand::ToggleDeafened => {
            if let Some(voice) = crate::voice::voice(cx) {
                voice.update(cx, crate::voice::Voice::toggle_deafened);
            }
        }
        // The call is left on the way out (`voice::leave_the_call_when_the_app_quits`).
        TrayCommand::QuitMellow => cx.quit(),
    }
}

/// Retitles the menu bar item's or the tray's menu in the language now spoken.
pub fn retitle_menu_bar_item(cx: &mut App) {
    refresh_tray(cx);
}

/// Whether something outside the window can bring Mellow back once it closes: the Dock and the
/// menu bar item on macOS, a tray icon a host shows on Linux, the notification area on Windows.
#[must_use]
pub fn can_show_mellow_again(cx: &App) -> bool {
    #[cfg(target_os = "macos")]
    {
        let _ = cx;
        true
    }
    #[cfg(target_os = "linux")]
    {
        linux::is_tray_shown(cx)
    }
    #[cfg(windows)]
    {
        notification_area::is_tray_shown(cx)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = cx;
        false
    }
}

/// "99+" past 99, as the window's title says it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn badge_label(count: u32) -> Option<String> {
    const MAXIMUM_SHOWN_COUNT: u32 = 99;
    match count {
        0 => None,
        count if count > MAXIMUM_SHOWN_COUNT => Some(format!("{MAXIMUM_SHOWN_COUNT}+")),
        count => Some(count.to_string()),
    }
}

/// The app's icon for the Linux and Windows trays, as RGBA rows of `size` × `size`; decoded from
/// the web app's icon, so nothing new is drawn.
#[cfg(any(target_os = "linux", windows))]
fn tray_icon_rgba(size: u32) -> Option<image::RgbaImage> {
    const APP_ICON_PNG: &[u8] = include_bytes!("../../../web/public/icons/mellow-192.png");
    let icon = image::load_from_memory_with_format(APP_ICON_PNG, image::ImageFormat::Png)
        .inspect_err(|error| tracing::warn!(%error, "cannot decode the tray icon"))
        .ok()?;
    Some(image::imageops::resize(
        &icon.to_rgba8(),
        size,
        size,
        image::imageops::FilterType::Lanczos3,
    ))
}

// AppKit's Objective-C API: every call the safe bindings cannot check is commented.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos {
    use gpui::{App, Global};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, Sel};
    use objc2::{
        AnyThread as _, DefinedClass as _, MainThreadMarker, MainThreadOnly, define_class,
        msg_send, sel,
    };
    use objc2_app_kit::{
        NSApplication, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem,
        NSVariableStatusItemLength,
    };
    use objc2_foundation::{NSData, NSSize, NSString};
    use tokio::sync::mpsc::Sender;

    use super::{TrayCommand, TrayMenu, badge_label};
    use crate::theme::icons::IconName;
    use crate::window_chrome::WINDOW_TITLE;

    /// The menu bar's own icon size, in points.
    const MENU_BAR_ICON_SIZE: f64 = 18.0;

    define_class!(
        // SAFETY: `NSObject` has no subclassing requirements, and this class does not implement
        // `Drop`.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "MellowMenuBarTarget"]
        #[ivars = Sender<TrayCommand>]
        struct MenuBarTarget;

        unsafe impl NSObjectProtocol for MenuBarTarget {}

        impl MenuBarTarget {
            #[unsafe(method(openMellow:))]
            fn open_mellow(&self, _sender: Option<&AnyObject>) {
                let _ = self.ivars().try_send(TrayCommand::OpenMellow);
            }

            #[unsafe(method(toggleMicrophone:))]
            fn toggle_microphone(&self, _sender: Option<&AnyObject>) {
                let _ = self.ivars().try_send(TrayCommand::ToggleMicrophone);
            }

            #[unsafe(method(toggleDeafened:))]
            fn toggle_deafened(&self, _sender: Option<&AnyObject>) {
                let _ = self.ivars().try_send(TrayCommand::ToggleDeafened);
            }

            #[unsafe(method(quitMellow:))]
            fn quit_mellow(&self, _sender: Option<&AnyObject>) {
                let _ = self.ivars().try_send(TrayCommand::QuitMellow);
            }
        }
    );

    impl MenuBarTarget {
        fn new(sender: Sender<TrayCommand>, mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(sender);
            // SAFETY: `init` is `NSObject`'s designated initializer.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Kept for the app's life: the status bar drops an item nobody retains, and menu items
    /// hold their target weakly.
    struct MenuBarItem {
        item: Retained<NSStatusItem>,
        /// Without its icon, the item's title carries the name so it stays visible.
        has_icon: bool,
        open_item: Retained<NSMenuItem>,
        microphone_item: Retained<NSMenuItem>,
        headset_item: Retained<NSMenuItem>,
        quit_item: Retained<NSMenuItem>,
        _menu: Retained<NSMenu>,
        _target: Retained<MenuBarTarget>,
    }

    impl Global for MenuBarItem {}

    pub(super) fn show_attention_count(count: u32, cx: &mut App) {
        // GPUI calls in on the main thread; anything else shows nothing rather than crash.
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let label = badge_label(count);
        let dock_label = label.as_deref().map(NSString::from_str);
        NSApplication::sharedApplication(mtm)
            .dockTile()
            .setBadgeLabel(dock_label.as_deref());
        if let Some(menu_bar_item) = cx.try_global::<MenuBarItem>()
            && let Some(button) = menu_bar_item.item.button(mtm)
        {
            let title = match (menu_bar_item.has_icon, label) {
                (true, label) => label.unwrap_or_default(),
                (false, Some(label)) => format!("{WINDOW_TITLE} {label}"),
                (false, None) => WINDOW_TITLE.to_owned(),
            };
            button.setTitle(&NSString::from_str(&title));
        }
    }

    pub(super) fn show_menu(menu: &TrayMenu, cx: &mut App) {
        if let Some(menu_bar_item) = cx.try_global::<MenuBarItem>() {
            for (item, title) in [
                (&menu_bar_item.open_item, &menu.open),
                (&menu_bar_item.microphone_item, &menu.microphone),
                (&menu_bar_item.headset_item, &menu.headset),
                (&menu_bar_item.quit_item, &menu.quit),
            ] {
                item.setTitle(&NSString::from_str(title));
            }
        }
    }

    fn menu_item(
        action: Sel,
        target: &MenuBarTarget,
        mtm: MainThreadMarker,
    ) -> Retained<NSMenuItem> {
        // SAFETY: the action is one `MenuBarTarget` implements, with the `sender:` argument; the
        // title is set by `show_menu`.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::new(),
                Some(action),
                &NSString::new(),
            )
        };
        // SAFETY: the target is kept alive in `MenuBarItem` for as long as the menu.
        unsafe { item.setTarget(Some(target)) };
        item
    }

    pub(super) fn install_menu_bar_item(sender: Sender<TrayCommand>, cx: &mut App) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let target = MenuBarTarget::new(sender, mtm);
        let menu = NSMenu::new(mtm);
        let open_item = menu_item(sel!(openMellow:), &target, mtm);
        let microphone_item = menu_item(sel!(toggleMicrophone:), &target, mtm);
        let headset_item = menu_item(sel!(toggleDeafened:), &target, mtm);
        let quit_item = menu_item(sel!(quitMellow:), &target, mtm);
        menu.addItem(&open_item);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&microphone_item);
        menu.addItem(&headset_item);
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&quit_item);
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        let mut has_icon = false;
        if let Some(button) = item.button(mtm) {
            let icon = NSImage::initWithData(
                NSImage::alloc(),
                &NSData::with_bytes(IconName::ChatCircle.svg_bytes()),
            );
            let description = NSString::from_str(WINDOW_TITLE);
            match icon {
                Some(icon) => {
                    icon.setSize(NSSize::new(MENU_BAR_ICON_SIZE, MENU_BAR_ICON_SIZE));
                    // A template follows the menu bar's light or dark look.
                    icon.setTemplate(true);
                    icon.setAccessibilityDescription(Some(&description));
                    button.setImage(Some(&icon));
                    has_icon = true;
                }
                // Older macOS without SVG support for `NSImage`: the name stands in.
                None => button.setTitle(&description),
            }
        }
        item.setMenu(Some(&menu));
        cx.set_global(MenuBarItem {
            item,
            has_icon,
            open_item,
            microphone_item,
            headset_item,
            quit_item,
            _menu: menu,
            _target: target,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_badge_says_99_plus_past_99_and_nothing_at_zero() {
        assert_eq!(badge_label(0), None);
        assert_eq!(badge_label(7).as_deref(), Some("7"));
        assert_eq!(badge_label(100).as_deref(), Some("99+"));
    }
}
