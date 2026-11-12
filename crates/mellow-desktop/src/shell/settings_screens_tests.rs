//! The settings screens over the shell headless: who they open for, Esc, and the unsaved changes
//! guard.

use gpui::{
    Entity, InputEvent as _, KeyUpEvent, Keystroke, TestAppContext, VisualTestContext,
    WindowAppearance, px, size,
};
use mellow_client_core::state::ClientState;
use mellow_protocol::{GatewayEvent, Permissions};

use super::*;
use crate::components::bind_tab_navigation;
use crate::session::Session;
use crate::shell::bind_shell_keys;
use crate::shell_fixtures::{GUILD_RUST, RUST_GENERAL, sample_state};
use crate::strings_pending_i18n::{ActiveStrings, ENGLISH};
use crate::theme::ActiveTheme;

/// The sample world, where the reader administers Rust Lyon when `is_administrator`.
fn state(is_administrator: bool) -> ClientState {
    let mut state = sample_state();
    if is_administrator {
        let mut guild = state
            .guild(GUILD_RUST)
            .cloned()
            .expect("the sample world has Rust Lyon");
        guild.my_permissions |= Permissions::ADMINISTRATOR;
        state.apply_event(GatewayEvent::GuildCreate(Box::new(guild)), 0);
    }
    state
}

fn open_shell(
    cx: &mut TestAppContext,
    state: ClientState,
) -> (Entity<DuskShell>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(ActiveStrings(&ENGLISH));
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
        bind_tab_navigation(cx);
        bind_shell_keys(cx);
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| *session.state_mut() = state);
    let (shell, cx) = cx.add_window_view(|window, cx| DuskShell::new(session, window, cx));
    cx.simulate_resize(size(px(1280.0), px(800.0)));
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
    let (shell, cx) = open_shell(cx, state(true));
    let is_offered = shell.read_with(cx, |shell, cx| {
        shell
            .state(cx)
            .guild(GUILD_RUST)
            .is_some_and(crate::settings::server::can_open_server_settings)
    });
    assert!(is_offered, "an administrator gets \"Server settings\"");
    open_server_settings(&shell, cx);
    assert!(is_open(&shell, cx));
    press(cx, "escape");
    assert!(!is_open(&shell, cx));
}

#[gpui::test]
fn settings_without_a_page_for_the_reader_close_by_themselves(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, state(false));
    let is_offered = shell.read_with(cx, |shell, cx| {
        crate::settings::channel::can_edit_channel(shell.state(cx), RUST_GENERAL)
    });
    assert!(
        !is_offered,
        "a member without Manage channels gets no \"Edit channel\""
    );
    open_server_settings(&shell, cx);
    assert!(!is_open(&shell, cx));
}

#[gpui::test]
fn a_typed_name_keeps_the_reader_on_the_page_until_it_is_undone(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, state(true));
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_channel_settings(RUST_GENERAL, window, cx);
        });
    });
    cx.run_until_parked();
    assert!(is_open(&shell, cx));
    // From the Overview item, Tab reaches the channel's name.
    press(cx, "tab");
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
