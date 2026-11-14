//! The settings screens over the shell headless: who they open for, Esc, and the unsaved changes
//! guard.

use gpui::{Entity, InputEvent as _, KeyUpEvent, Keystroke, TestAppContext, VisualTestContext};

use super::*;
use crate::shell_fixtures::{
    GUILD_RUST, RUST_GENERAL, open_shell, sample_state, sample_state_administering_rust,
};

/// A full key press: GPUI activates a focused control on the release, as browsers do.
fn press(cx: &mut VisualTestContext, key: &'static str) {
    cx.simulate_keystrokes(key);
    let keystroke = Keystroke::parse(key).expect("a valid keystroke in a test");
    cx.update(|window, cx| {
        window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
    });
    cx.run_until_parked();
}

fn is_open(shell: &Entity<DuskShell>, cx: &mut VisualTestContext) -> bool {
    shell.read_with(cx, |shell, _| shell.is_settings_screen_open())
}

fn open_server_settings(shell: &Entity<DuskShell>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_server_settings(GUILD_RUST, None, window, cx);
        });
    });
    cx.run_until_parked();
}

#[gpui::test]
fn server_settings_open_for_a_manager_and_esc_closes_them(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state_administering_rust(), 1280.0);
    let is_offered = shell.read_with(cx, |shell, cx| {
        shell
            .state(cx)
            .guild(GUILD_RUST)
            .is_some_and(crate::settings::server::has_server_settings_page_for_reader)
    });
    assert!(is_offered, "an administrator gets \"Server settings\"");
    open_server_settings(&shell, cx);
    assert!(is_open(&shell, cx));
    press(cx, "escape");
    assert!(!is_open(&shell, cx));
}

#[gpui::test]
fn settings_without_a_page_for_the_reader_close_by_themselves(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    let is_offered = shell.read_with(cx, |shell, cx| {
        crate::settings::channel::has_channel_settings_page_for_reader(
            shell.state(cx),
            RUST_GENERAL,
        )
    });
    assert!(
        !is_offered,
        "a member without Manage channels gets no \"Edit channel\""
    );
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_channel_settings(RUST_GENERAL, window, cx);
        });
    });
    cx.run_until_parked();
    assert!(!is_open(&shell, cx));
}

#[gpui::test]
fn a_typed_name_keeps_the_reader_on_the_page_until_it_is_undone(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state_administering_rust(), 1280.0);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_channel_settings(RUST_GENERAL, window, cx);
        });
    });
    cx.run_until_parked();
    assert!(is_open(&shell, cx));
    // From the Overview item, Tab passes Permissions, "Delete channel" and "Upload icon", then
    // reaches the channel's name.
    for _ in 0..4 {
        press(cx, "tab");
    }
    cx.simulate_input("-chat");
    cx.run_until_parked();
    press(cx, "escape");
    assert!(is_open(&shell, cx), "unsaved changes keep the sheet open");
    // Back to the stored name: nothing holds the reader any more.
    for _ in 0.."-chat".len() {
        press(cx, "backspace");
    }
    press(cx, "escape");
    assert!(!is_open(&shell, cx));
}

#[gpui::test]
fn the_shell_keys_wait_while_the_settings_cover_it(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state_administering_rust(), 1280.0);
    open_server_settings(&shell, cx);
    let focused_in_sheet = cx.update(|window, cx| window.focused(cx));
    assert!(focused_in_sheet.is_some(), "the sheet takes focus");
    press(cx, "f6");
    assert_eq!(
        cx.update(|window, cx| window.focused(cx)),
        focused_in_sheet,
        "F6 does not move focus under the sheet"
    );
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| shell.focus_message_search(window, cx));
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update(|window, cx| window.focused(cx)),
        focused_in_sheet,
        "Ctrl+F does not reach the search field under the sheet"
    );
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| shell.toggle_quick_switcher(window, cx));
    });
    cx.run_until_parked();
    assert!(!shell.read_with(cx, |shell, _| shell.is_quick_switcher_open()));
    // Esc still reaches the sheet.
    press(cx, "escape");
    assert!(!is_open(&shell, cx));
}

/// Ctrl+, (Cmd+, on macOS), the platform's settings shortcut.
const USER_SETTINGS_KEY: &str = if cfg!(target_os = "macos") {
    "cmd-,"
} else {
    "ctrl-,"
};

#[gpui::test]
fn user_settings_open_with_the_shortcut_and_esc_closes_them(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    press(cx, USER_SETTINGS_KEY);
    assert!(is_open(&shell, cx));
    press(cx, "escape");
    assert!(!is_open(&shell, cx));
}

#[gpui::test]
fn esc_closes_the_open_account_form_before_the_user_settings(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    press(cx, USER_SETTINGS_KEY);
    // Past the eight sections and "Sign out", Tab reaches the name's Edit button.
    for _ in 0..9 {
        press(cx, "tab");
    }
    press(cx, "enter");
    cx.simulate_input(" B");
    cx.run_until_parked();
    press(cx, "escape");
    assert!(is_open(&shell, cx), "the first Esc closes the form only");
    press(cx, "escape");
    assert!(!is_open(&shell, cx));
}

#[gpui::test]
fn the_close_button_closes_the_user_settings_even_with_a_form_open(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    press(cx, USER_SETTINGS_KEY);
    for _ in 0..9 {
        press(cx, "tab");
    }
    press(cx, "enter");
    let close_button = cx
        .debug_bounds("settings-close-button")
        .expect("the wide sheet shows its close button");
    cx.simulate_click(close_button.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(!is_open(&shell, cx), "the web's close button calls onclose");
}

#[gpui::test]
fn sign_out_closes_the_user_settings_first(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    press(cx, USER_SETTINGS_KEY);
    // "Sign out" is the navigation's last item.
    press(cx, "end");
    press(cx, "enter");
    assert!(!is_open(&shell, cx));
}
