//! The menus of places headless: opened by the keyboard and the pointer, and what their items do.

use gpui::{
    Entity, InputEvent as _, KeyUpEvent, Keystroke, TestAppContext, VisualTestContext,
    WindowAppearance, px, size,
};
use mellow_client_core::shared_logic::notification_settings::{
    MuteEnd, NotificationSettings, PlaceNotificationSettings, with_place_settings,
};

use super::*;
use crate::components::bind_tab_navigation;
use crate::shell::bind_shell_keys;
use crate::shell_fixtures::{GUILD_RUST, ME, RUST_GENERAL, RUST_TALK, sample_state};
use crate::strings_pending_i18n::{ActiveStrings, ENGLISH};
use crate::theme::ActiveTheme;
use crate::theme::tokens::BREAKPOINT_NAVIGATOR;

fn open_shell(cx: &mut TestAppContext, width: f32) -> (Entity<DuskShell>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(ActiveStrings(&ENGLISH));
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
        bind_tab_navigation(cx);
        bind_shell_keys(cx);
    });
    let session = cx.new(|_| crate::session::Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| *session.state_mut() = sample_state());
    let (shell, cx) = cx.add_window_view(|window, cx| DuskShell::new(session, window, cx));
    cx.simulate_resize(size(px(width), px(800.0)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (shell, cx)
}

/// A full key press: GPUI activates a focused control on the release, as browsers do.
fn press(cx: &mut VisualTestContext, key: &'static str) {
    cx.simulate_keystrokes(key);
    let keystroke = Keystroke::parse(key).expect("a valid keystroke in a test");
    cx.update(|window, cx| {
        window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
    });
    cx.run_until_parked();
}

fn my_settings(cx: &mut VisualTestContext) -> NotificationSettings {
    cx.update(|_, cx| notification_settings_of(ME, cx))
}

fn open_target(shell: &Entity<DuskShell>, cx: &mut VisualTestContext) -> Option<PlaceMenuTarget> {
    shell.read_with(cx, |shell, _| {
        shell
            .place_menus
            .menu
            .as_ref()
            .map(|(_, _, target)| *target)
    })
}

fn open_menu_at_a_point(
    shell: &Entity<DuskShell>,
    target: PlaceMenuTarget,
    cx: &mut VisualTestContext,
) {
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            let origin = MenuOrigin::Point { x: 10.0, y: 10.0 };
            shell.open_place_menu(target, origin, None, window, cx);
        });
    });
    cx.run_until_parked();
}

#[test]
fn the_menu_key_and_shift_f10_open_menus_but_not_with_other_modifiers() {
    let key = |text: &'static str| Keystroke::parse(text).expect("a valid keystroke in a test");
    assert!(crate::components::is_context_menu_keystroke(&key("menu")));
    assert!(crate::components::is_context_menu_keystroke(&key(
        "shift-f10"
    )));
    assert!(!crate::components::is_context_menu_keystroke(&key("f10")));
    assert!(!crate::components::is_context_menu_keystroke(&key(
        "ctrl-shift-f10"
    )));
    assert!(!crate::components::is_context_menu_keystroke(&key(
        "alt-menu"
    )));
}

#[gpui::test]
fn shift_f10_on_a_server_opens_its_menu_and_mute_saves_through_the_dialog(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, 1280.0);
    // F6 lands on the open server in the rail.
    press(cx, "f6");
    press(cx, "shift-f10");
    assert_eq!(
        open_target(&shell, cx),
        Some(PlaceMenuTarget::Server(GUILD_RUST))
    );
    // From the keyboard the first item, "Mute…", has focus.
    press(cx, "enter");
    assert!(open_target(&shell, cx).is_none());
    let dialog = shell.read_with(cx, |shell, _| {
        shell
            .place_menus
            .notifications_dialog
            .as_ref()
            .map(|(dialog, _)| dialog.clone())
    });
    let dialog = dialog.expect("Mute… opens the place's notification dialog");
    // Focus starts on "Don't mute": Down chooses "For 15 minutes".
    press(cx, "down");
    dialog.update(cx, ModalDialog::submit_now);
    cx.run_until_parked();
    let mute = my_settings(cx)
        .places
        .get(&GUILD_RUST)
        .and_then(|place| place.muted_until);
    assert!(matches!(mute, Some(MuteEnd::At(_))), "{mute:?}");
    assert!(shell.read_with(cx, |shell, _| {
        shell.place_menus.notifications_dialog.is_none()
    }));
}

#[gpui::test]
fn unmute_on_a_muted_channel_ends_the_mute_and_says_so(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, 1280.0);
    cx.update(|_, cx| {
        change_preferences(cx, |preferences| {
            let muted = with_place_settings(
                &NotificationSettings::default(),
                RUST_GENERAL,
                PlaceNotificationSettings {
                    level: None,
                    muted_until: Some(MuteEnd::Never),
                },
            );
            preferences.set_notification_settings(ME, muted);
        });
    });
    open_menu_at_a_point(&shell, PlaceMenuTarget::Channel(RUST_GENERAL), cx);
    // At the pointer nothing has focus yet: Down reaches the first item, "Unmute".
    press(cx, "down");
    press(cx, "enter");
    assert!(my_settings(cx).places.is_empty());
    assert_eq!(
        shell.read_with(cx, |shell, _| shell.place_menus.announcement.clone()),
        "#general unmuted."
    );
}

#[gpui::test]
fn a_category_copies_its_id(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, 1280.0);
    open_menu_at_a_point(&shell, PlaceMenuTarget::Category(RUST_TALK), cx);
    press(cx, "down");
    press(cx, "enter");
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied, Some(RUST_TALK.raw().to_string()));
}

#[gpui::test]
fn leave_server_comes_last_and_asks_first(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, 1280.0);
    open_menu_at_a_point(&shell, PlaceMenuTarget::Server(GUILD_RUST), cx);
    press(cx, "end");
    press(cx, "enter");
    assert!(shell.read_with(cx, |shell, _| shell.place_menus.leave_dialog.is_some()));
    // Cancel has focus: Enter closes without leaving.
    press(cx, "enter");
    assert!(shell.read_with(cx, |shell, _| shell.place_menus.leave_dialog.is_none()));
    assert!(cx.update(|_, cx| shell.read(cx).state(cx).guild(GUILD_RUST).is_some()));
}

#[gpui::test]
fn a_narrow_window_folds_go_to_into_more_options(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, BREAKPOINT_NAVIGATOR - 1.0);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| shell.toggle_chat_header_menu(window, cx));
    });
    cx.run_until_parked();
    assert_eq!(open_target(&shell, cx), Some(PlaceMenuTarget::ChatHeader));
    press(cx, "enter");
    assert!(shell.read_with(cx, |shell, _| shell.is_quick_switcher_open()));
}

#[gpui::test]
fn more_options_records_where_its_button_is_for_the_menu_to_open_under_it(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, BREAKPOINT_NAVIGATOR - 1.0);
    let header = cx
        .debug_bounds("chat-header")
        .expect("the chat header is drawn");
    let button = shell
        .read_with(cx, |shell, _| shell.place_menus.more_options_bounds.get())
        .expect("the button's place is recorded");
    assert!(
        (button.center().y - header.center().y).abs() < px(1.0),
        "{button:?} in {header:?}"
    );
}

#[gpui::test]
fn more_options_pressed_again_closes_its_menu(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, BREAKPOINT_NAVIGATOR - 1.0);
    let button = shell
        .read_with(cx, |shell, _| shell.place_menus.more_options_bounds.get())
        .expect("the button's place is recorded");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(open_target(&shell, cx), Some(PlaceMenuTarget::ChatHeader));
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(open_target(&shell, cx), None);
}
