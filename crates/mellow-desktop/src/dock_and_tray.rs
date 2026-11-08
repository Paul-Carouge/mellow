//! What the app shows outside its window, which GPUI has no API for: the count of what waits for
//! the reader on the Dock icon, and a menu bar item with "Open Mellow" and "Quit Mellow".
//!
//! macOS only for now. On Linux and Windows the count shows in the window's title (and so in the
//! taskbar); a Linux launcher badge (`com.canonical.Unity.LauncherEntry`), a Linux tray
//! (`StatusNotifierItem`) and a Windows taskbar overlay and notification area icon are left for
//! later. Closing the window still quits the app (ADR-0051: one window), so the menu bar item
//! only brings the window to the front or quits.

use gpui::App;

/// Shows `count` on the Dock icon and beside the menu bar item; zero clears it.
pub fn show_attention_count(count: u32, cx: &mut App) {
    #[cfg(target_os = "macos")]
    macos::show_attention_count(count, cx);
    #[cfg(not(target_os = "macos"))]
    let _ = (count, cx);
}

/// Adds the menu bar item; called once at start.
pub fn install_menu_bar_item(cx: &mut App) {
    #[cfg(target_os = "macos")]
    macos::install_menu_bar_item(cx);
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
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

// AppKit's Objective-C API: every call the safe bindings cannot check is commented.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos {
    use gpui::{App, Global};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
    use objc2::{
        AnyThread as _, DefinedClass as _, MainThreadMarker, MainThreadOnly, define_class,
        msg_send, sel,
    };
    use objc2_app_kit::{
        NSApplication, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem,
        NSVariableStatusItemLength,
    };
    use objc2_foundation::{NSData, NSSize, NSString};
    use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

    use super::badge_label;
    use crate::i18n::strings;
    use crate::theme::icons::IconName;
    use crate::window_chrome::WINDOW_TITLE;

    /// The menu bar's own icon size, in points.
    const MENU_BAR_ICON_SIZE: f64 = 18.0;

    #[derive(Debug, Clone, Copy)]
    enum MenuBarCommand {
        OpenMellow,
        QuitMellow,
    }

    define_class!(
        // SAFETY: `NSObject` has no subclassing requirements, and this class does not implement
        // `Drop`.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "MellowMenuBarTarget"]
        #[ivars = UnboundedSender<MenuBarCommand>]
        struct MenuBarTarget;

        unsafe impl NSObjectProtocol for MenuBarTarget {}

        impl MenuBarTarget {
            #[unsafe(method(openMellow:))]
            fn open_mellow(&self, _sender: Option<&AnyObject>) {
                let _ = self.ivars().send(MenuBarCommand::OpenMellow);
            }

            #[unsafe(method(quitMellow:))]
            fn quit_mellow(&self, _sender: Option<&AnyObject>) {
                let _ = self.ivars().send(MenuBarCommand::QuitMellow);
            }
        }
    );

    impl MenuBarTarget {
        fn new(sender: UnboundedSender<MenuBarCommand>, mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(sender);
            // SAFETY: `init` is `NSObject`'s designated initializer.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Kept for the app's life: the status bar drops an item nobody retains, and menu items
    /// hold their target weakly.
    struct MenuBarItem {
        item: Retained<NSStatusItem>,
        _menu: Retained<NSMenu>,
        _target: Retained<MenuBarTarget>,
    }

    impl Global for MenuBarItem {}

    pub(super) fn show_attention_count(count: u32, cx: &mut App) {
        // GPUI calls in on the main thread; anything else shows nothing rather than crash.
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let label = badge_label(count).map(|label| NSString::from_str(&label));
        NSApplication::sharedApplication(mtm)
            .dockTile()
            .setBadgeLabel(label.as_deref());
        if let Some(menu_bar_item) = cx.try_global::<MenuBarItem>()
            && let Some(button) = menu_bar_item.item.button(mtm)
        {
            button.setTitle(&label.unwrap_or_default());
        }
    }

    pub(super) fn install_menu_bar_item(cx: &mut App) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let catalogue = strings(cx).desktop();
        let (sender, mut commands) = unbounded_channel();
        let target = MenuBarTarget::new(sender, mtm);
        let menu = NSMenu::new(mtm);
        for (label, action) in [
            (catalogue.open_mellow(), sel!(openMellow:)),
            (catalogue.quit_mellow(), sel!(quitMellow:)),
        ] {
            if action == sel!(quitMellow:) {
                menu.addItem(&NSMenuItem::separatorItem(mtm));
            }
            // SAFETY: the action is one `MenuBarTarget` implements, with the `sender:` argument.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(label),
                    Some(action),
                    &NSString::new(),
                )
            };
            // SAFETY: the target is kept alive in `MenuBarItem` for as long as the menu.
            unsafe { item.setTarget(Some(&target)) };
            menu.addItem(&item);
        }
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
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
                }
                // Older macOS without SVG support for `NSImage`: the name stands in.
                None => button.setTitle(&description),
            }
        }
        item.setMenu(Some(&menu));
        cx.set_global(MenuBarItem {
            item,
            _menu: menu,
            _target: target,
        });
        cx.spawn(async move |cx| {
            while let Some(command) = commands.recv().await {
                cx.update(|cx| match command {
                    MenuBarCommand::OpenMellow => {
                        cx.activate(true);
                        if let Some(window) = cx.windows().first() {
                            let _ = window.update(cx, |_, window, _| window.activate_window());
                        }
                    }
                    MenuBarCommand::QuitMellow => cx.quit(),
                });
            }
        })
        .detach();
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
