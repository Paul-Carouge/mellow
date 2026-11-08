//! The shell headless: its layout from the tokens, and what a keyboard-only person relies on.

use gpui::{
    Bounds, Entity, InputEvent as _, KeyUpEvent, Keystroke, Pixels, TestAppContext,
    VisualTestContext, WindowAppearance, px, size,
};

use super::*;
use crate::components::bind_tab_navigation;
use crate::navigation::Place;
use crate::preferences::change_preferences;
use crate::shell_fixtures::{
    BOOKS_READING, CONVERSATION_ANA, GUILD_BOOKS, GUILD_RUST, RUST_ANNOUNCEMENTS, RUST_GENERAL,
    RUST_HELP, sample_state, sample_state_with_unreads,
};
use crate::strings_pending_i18n::{ActiveStrings, ENGLISH};
use crate::theme::ActiveTheme;
use crate::theme::tokens::{
    BREAKPOINT_NAVIGATOR, BREAKPOINT_SIDE_SHEET, COMPONENT_NAVIGATOR_STRIP_WIDTH, SIZE_DOCK_HEIGHT,
    SIZE_MEMBER_LIST_WIDTH, SIZE_SIDEBAR_WIDTH,
};

fn bounds_of(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("`{selector}` was not drawn"))
}

/// A shell over the sample world, in a window of `width` × 800.
fn open_shell(
    cx: &mut TestAppContext,
    state: mellow_client_core::state::ClientState,
    width: f32,
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
    cx.simulate_resize(size(px(width), px(800.0)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (shell, cx)
}

/// `secondary` in a binding: Cmd on macOS, Ctrl elsewhere (CI tests on Linux).
fn secondary(key: &str) -> &'static str {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    // Leaked: a few bytes per test, for the keystroke API's static strings.
    Box::leak(format!("{modifier}-{key}").into_boxed_str())
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

fn place(shell: &Entity<DuskShell>, cx: &mut VisualTestContext) -> Option<Place> {
    shell.read_with(cx, |shell, _| shell.navigation().place())
}

fn selected_channel(shell: &Entity<DuskShell>, cx: &mut VisualTestContext) -> Option<Snowflake> {
    shell.read_with(cx, |shell, _| shell.navigation().selected_channel_id())
}

#[test]
fn the_layout_follows_the_generated_breakpoints() {
    let below = |breakpoint: f32| breakpoint - 1.0;
    assert_eq!(
        ShellLayout::for_window_width(below(BREAKPOINT_NAVIGATOR), true),
        ShellLayout::SheetAlone
    );
    assert_eq!(
        ShellLayout::for_window_width(BREAKPOINT_NAVIGATOR, true),
        ShellLayout::NavigatorAndSheet
    );
    assert_eq!(
        ShellLayout::for_window_width(below(BREAKPOINT_SIDE_SHEET), true),
        ShellLayout::NavigatorAndSheet
    );
    assert_eq!(
        ShellLayout::for_window_width(BREAKPOINT_SIDE_SHEET, true),
        ShellLayout::NavigatorSheetAndSideSheet
    );
    assert_eq!(
        ShellLayout::for_window_width(BREAKPOINT_SIDE_SHEET, false),
        ShellLayout::NavigatorAndSheet
    );
}

#[gpui::test]
fn the_shell_draws_its_cards_with_the_token_layout(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    // The first server opens at its first channel; its member sheet is closed until asked for.
    assert_eq!(
        place(&shell, cx),
        Some(Place::Server {
            guild_id: GUILD_RUST,
            channel_id: Some(RUST_ANNOUNCEMENTS)
        })
    );
    assert!(cx.debug_bounds("side-sheet").is_none());
    press(cx, secondary("u"));

    let navigator = bounds_of(cx, "navigator");
    let sheet = bounds_of(cx, "sheet");
    let side_sheet = bounds_of(cx, "side-sheet");
    let inset = px(SIZE_SHEET_INSET);
    assert_eq!(navigator.origin.x, inset);
    assert_eq!(navigator.origin.y, px(TOP_BAND_HEIGHT));
    assert_eq!(
        navigator.size.width,
        px(COMPONENT_NAVIGATOR_STRIP_WIDTH + SIZE_SIDEBAR_WIDTH)
    );
    assert_eq!(sheet.origin.x, navigator.right() + inset);
    assert_eq!(side_sheet.origin.x, sheet.right() + inset);
    assert_eq!(side_sheet.size.width, px(SIZE_MEMBER_LIST_WIDTH));
    assert_eq!(side_sheet.right(), px(1280.0) - inset);
    assert_eq!(
        bounds_of(cx, "server-rail").size.width,
        px(COMPONENT_NAVIGATOR_STRIP_WIDTH)
    );

    // ADR-0022 §4: the user card and the composer's base row share their height and bottom.
    let user_card = bounds_of(cx, "user-card");
    let composer = bounds_of(cx, "composer");
    assert_eq!(user_card.size.height, px(SIZE_DOCK_HEIGHT));
    assert_eq!(composer.size.height, user_card.size.height);
    assert_eq!(composer.bottom(), user_card.bottom());
    assert_eq!(navigator.bottom(), px(800.0) - inset);

    // The sheet stays open for this server, remembered in the preferences.
    let remembered = cx.update(|_, cx| {
        preferences(cx).map(|preferences| preferences.member_sheet_place_ids.clone())
    });
    assert_eq!(remembered, Some(vec![GUILD_RUST]));
}

#[gpui::test]
fn the_reader_s_column_widths_are_laid_out_and_fitted_to_the_window(cx: &mut TestAppContext) {
    let (_shell, cx) = open_shell(cx, sample_state(), 1280.0);
    cx.update(|_, cx| {
        change_preferences(cx, |preferences| {
            preferences.channels_column_width = Some(300.0);
        });
    });
    cx.run_until_parked();
    assert_eq!(
        bounds_of(cx, "navigator").size.width,
        px(COMPONENT_NAVIGATOR_STRIP_WIDTH + 300.0)
    );

    // Narrower: the conversation keeps its minimum, the channel list gives way to its minimum.
    cx.simulate_resize(size(px(BREAKPOINT_NAVIGATOR), px(800.0)));
    cx.run_until_parked();
    assert_eq!(
        bounds_of(cx, "navigator").size.width,
        px(COMPONENT_NAVIGATOR_STRIP_WIDTH + SIZE_SIDEBAR_WIDTH)
    );
}

#[gpui::test]
fn a_narrow_window_shows_the_sheet_alone_and_the_navigator_as_a_drawer(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), BREAKPOINT_NAVIGATOR - 1.0);
    assert!(cx.debug_bounds("navigator").is_none());
    assert!(cx.debug_bounds("side-sheet").is_none());
    let sheet = bounds_of(cx, "sheet");
    assert_eq!(sheet.origin.x, px(0.0));
    assert_eq!(sheet.origin.y, px(TITLE_BAR_BAND_HEIGHT));
    assert_eq!(sheet.size.width, px(BREAKPOINT_NAVIGATOR - 1.0));

    // "Open navigation" brings the navigator in from the leading edge; Esc puts it away.
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| shell.open_navigation_drawer(window, cx));
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("navigator").is_some());
    press(cx, "escape");
    assert!(!shell.read_with(cx, |shell, _| shell.is_navigation_drawer_open()));
    assert!(cx.debug_bounds("navigator").is_none());

    // Below the side sheet breakpoint the members come over the conversation.
    press(cx, secondary("u"));
    assert!(shell.read_with(cx, |shell, _| shell.is_member_overlay_open()));
    assert!(cx.debug_bounds("side-sheet").is_some());
    press(cx, "escape");
    assert!(!shell.read_with(cx, |shell, _| shell.is_member_overlay_open()));
}

#[gpui::test]
fn keyboard_shortcuts_walk_channels_unread_channels_and_servers(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state_with_unreads(&[(RUST_HELP, 0)]), 1280.0);
    press(cx, "alt-down");
    assert_eq!(selected_channel(&shell, cx), Some(RUST_GENERAL));
    press(cx, "alt-shift-down");
    assert_eq!(selected_channel(&shell, cx), Some(RUST_HELP));
    press(cx, secondary("alt-down"));
    assert_eq!(
        place(&shell, cx),
        Some(Place::Server {
            guild_id: GUILD_BOOKS,
            channel_id: Some(BOOKS_READING)
        })
    );
    // Back where it was in that server.
    press(cx, secondary("alt-up"));
    assert_eq!(selected_channel(&shell, cx), Some(RUST_HELP));
}

#[gpui::test]
fn the_quick_switcher_opens_what_is_typed_and_closes_with_its_shortcut(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    let is_open = |shell: &Entity<DuskShell>, cx: &mut VisualTestContext| {
        shell.read_with(cx, |shell, _| shell.is_quick_switcher_open())
    };
    press(cx, secondary("k"));
    assert!(is_open(&shell, cx));
    press(cx, secondary("k"));
    assert!(!is_open(&shell, cx));

    press(cx, secondary("k"));
    cx.simulate_input("#help");
    cx.run_until_parked();
    press(cx, "enter");
    assert!(!is_open(&shell, cx));
    assert_eq!(selected_channel(&shell, cx), Some(RUST_HELP));

    press(cx, secondary("k"));
    cx.simulate_input("*books");
    cx.run_until_parked();
    press(cx, "enter");
    assert_eq!(
        shell.read_with(cx, |shell, _| shell.navigation().selected_guild_id()),
        Some(GUILD_BOOKS)
    );

    press(cx, secondary("k"));
    press(cx, "escape");
    assert!(!is_open(&shell, cx));
}

#[gpui::test]
fn a_conversation_opened_from_the_switcher_keeps_the_shortcuts_working(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    press(cx, secondary("k"));
    let switcher = shell.read_with(cx, |shell, _| {
        shell
            .quick_switcher
            .as_ref()
            .map(|(switcher, _)| switcher.downgrade())
            .expect("the switcher is open")
    });
    // The instance's answer: the conversation now exists in the state.
    cx.update(|_, cx| open_conversation_chosen_in_switcher(&switcher, CONVERSATION_ANA, cx));
    cx.run_until_parked();
    assert!(!shell.read_with(cx, |shell, _| shell.is_quick_switcher_open()));
    assert_eq!(selected_channel(&shell, cx), Some(CONVERSATION_ANA));
    press(cx, secondary("k"));
    assert!(shell.read_with(cx, |shell, _| shell.is_quick_switcher_open()));
}

#[gpui::test]
fn closing_the_drawer_for_the_switcher_gives_focus_back_to_the_shell(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), BREAKPOINT_NAVIGATOR - 1.0);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| shell.open_navigation_drawer(window, cx));
    });
    cx.run_until_parked();
    press(cx, secondary("k"));
    assert!(!shell.read_with(cx, |shell, _| shell.is_navigation_drawer_open()));
    press(cx, "escape");
    press(cx, secondary("k"));
    assert!(shell.read_with(cx, |shell, _| shell.is_quick_switcher_open()));
}

#[gpui::test]
fn a_drawer_put_away_by_a_wider_window_leaves_focus_in_the_shell(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), BREAKPOINT_NAVIGATOR - 1.0);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| shell.open_navigation_drawer(window, cx));
    });
    cx.run_until_parked();
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    cx.run_until_parked();
    assert!(!shell.read_with(cx, |shell, _| shell.is_navigation_drawer_open()));
    let focus_is_in_the_shell = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| shell.read(cx).root_focus.contains_focused(window, cx))
    };
    assert!(focus_is_in_the_shell(cx));
    press(cx, "tab");
    assert!(focus_is_in_the_shell(cx));
    press(cx, secondary("k"));
    assert!(shell.read_with(cx, |shell, _| shell.is_quick_switcher_open()));
}

#[gpui::test]
fn dragging_the_channel_edge_beside_docked_members_saves_the_width_drawn(cx: &mut TestAppContext) {
    let window_width = 1100.0;
    let (shell, cx) = open_shell(cx, sample_state(), window_width);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            if !shell.is_member_sheet_docked_open(cx) {
                shell.toggle_member_sheet(window, cx);
            }
        });
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("side-sheet").is_some());
    let start_x = 300.0;
    shell.update(cx, |shell, _| {
        shell.column_drag = Some(ColumnDrag {
            column: ResizableColumn::Channels,
            start_x,
            start_width: SIZE_SIDEBAR_WIDTH,
            width: SIZE_SIDEBAR_WIDTH,
        });
    });
    cx.simulate_mouse_move(
        gpui::point(px(start_x + 600.0), px(400.0)),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::none(),
    );
    cx.run_until_parked();
    let (dragged, drawn) = cx.update(|window, cx| {
        let shell = shell.read(cx);
        let drawn = shell
            .column_layout(window_width_at_default_zoom(window), true, cx)
            .channels;
        (shell.column_drag.map(|drag| drag.width), drawn)
    });
    assert_eq!(dragged, Some(drawn));
}

fn focused_region(shell: &Entity<DuskShell>, cx: &mut VisualTestContext) -> Option<Region> {
    cx.update(|window, cx| shell.read(cx).focused_region(window, cx))
}

#[gpui::test]
fn f6_goes_round_the_regions_and_shift_f6_back(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| shell.open_channel(RUST_GENERAL, window, cx));
    });
    cx.run_until_parked();
    // Without the side sheet, F6 skips it.
    let mut regions = Vec::new();
    for _ in 0..5 {
        press(cx, "f6");
        regions.push(focused_region(&shell, cx));
    }
    assert_eq!(
        regions,
        [
            Some(Region::ServerRail),
            Some(Region::ChannelList),
            Some(Region::MessageList),
            Some(Region::Composer),
            Some(Region::ServerRail),
        ]
    );
    press(cx, secondary("u"));
    press(cx, "shift-f6");
    assert_eq!(focused_region(&shell, cx), Some(Region::SideSheet));
    press(cx, "shift-f6");
    assert_eq!(focused_region(&shell, cx), Some(Region::Composer));
}

#[gpui::test]
fn the_rail_is_one_tab_stop_whose_arrows_move_between_servers(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    // F6 lands on the open server; Up reaches Home, End the last server.
    press(cx, "f6");
    press(cx, "up");
    press(cx, "enter");
    assert_eq!(
        shell.read_with(cx, |shell, _| shell.navigation().selected_guild_id()),
        None
    );
    press(cx, "end");
    press(cx, "enter");
    assert_eq!(
        shell.read_with(cx, |shell, _| shell.navigation().selected_guild_id()),
        Some(GUILD_BOOKS)
    );
    // Tab leaves the rail at once, for the channel list's single stop: the open channel.
    press(cx, "tab");
    assert_eq!(focused_region(&shell, cx), Some(Region::ChannelList));
    press(cx, "enter");
    assert_eq!(selected_channel(&shell, cx), Some(BOOKS_READING));
    press(cx, "shift-tab");
    assert_eq!(focused_region(&shell, cx), Some(Region::ServerRail));
}

#[gpui::test]
fn in_a_narrow_window_f6_opens_the_drawer_for_the_navigator_and_closes_it_after(
    cx: &mut TestAppContext,
) {
    let (shell, cx) = open_shell(cx, sample_state(), BREAKPOINT_NAVIGATOR - 1.0);
    let is_drawer_open = |cx: &mut VisualTestContext| {
        shell.read_with(cx, |shell, _| shell.is_navigation_drawer_open())
    };
    press(cx, "f6");
    assert!(is_drawer_open(cx));
    assert_eq!(focused_region(&shell, cx), Some(Region::ServerRail));
    press(cx, "f6");
    assert_eq!(focused_region(&shell, cx), Some(Region::ChannelList));
    press(cx, "f6");
    assert!(!is_drawer_open(cx));
    assert_eq!(focused_region(&shell, cx), Some(Region::MessageList));
}

#[gpui::test]
fn choosing_a_member_shows_their_profile_and_back_returns_to_the_list(cx: &mut TestAppContext) {
    use crate::shell_fixtures::ANA;
    use mellow_client_core::shared_logic::profiles::UserProfileLoad;

    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    press(cx, secondary("u"));
    assert!(cx.debug_bounds("member-profile").is_none());
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.show_member_profile(GUILD_RUST, ANA, window, cx);
        });
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("member-profile").is_some());
    let is_back_focused = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            shell
                .read(cx)
                .member_profile
                .back_focus()
                .is_focused(window)
        })
    };
    assert!(is_back_focused(cx), "focus lands on Back");
    // Without an instance the details cannot be read: the card says so instead of waiting.
    let load = shell.read_with(cx, |shell, _| shell.member_profile.load().clone());
    assert_eq!(load, UserProfileLoad::Failed { user_id: ANA });

    // Another server's sheet shows its own list, not this profile.
    let shown_elsewhere = shell.read_with(cx, |shell, _| {
        shell.member_profile.shown_user_id(GUILD_BOOKS)
    });
    assert_eq!(shown_elsewhere, None);

    press(cx, "enter");
    assert!(cx.debug_bounds("member-profile").is_none());
    let focus_is_in_the_sheet = cx.update(|window, cx| {
        shell
            .read(cx)
            .member_sheet_focus
            .contains_focused(window, cx)
    });
    assert!(focus_is_in_the_sheet, "Back returns to the member's row");
}

#[gpui::test]
fn the_custom_status_dialog_saves_only_a_changed_status_within_the_limit(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| shell.open_custom_status_dialog(window, cx));
    });
    cx.run_until_parked();
    let dialog = shell
        .read_with(cx, |shell, _| {
            shell
                .custom_status_dialog
                .as_ref()
                .map(|(dialog, _)| dialog.clone())
        })
        .expect("the dialog opened");
    let can_save =
        |cx: &mut VisualTestContext| dialog.read_with(cx, |dialog, _| dialog.is_submit_enabled());
    assert!(!can_save(cx), "nothing changed yet");

    cx.simulate_input("En réunion");
    assert!(can_save(cx));
    cx.simulate_input(&"é".repeat(mellow_protocol::MAXIMUM_CUSTOM_STATUS_LENGTH));
    assert!(!can_save(cx), "past the limit");

    press(cx, "escape");
    assert!(shell.read_with(cx, |shell, _| shell.custom_status_dialog.is_none()));
    let is_status_button_focused =
        cx.update(|window, cx| shell.read(cx).status_button_focus.is_focused(window));
    assert!(is_status_button_focused, "focus goes back to the user card");
}
