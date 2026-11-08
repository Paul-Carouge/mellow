//! Home headless: the Friends space, the people picker and a group's side sheet.

use gpui::{
    AppContext as _, Entity, InputEvent as _, KeyUpEvent, Keystroke, TestAppContext,
    VisualTestContext, WindowAppearance, px, size,
};
use mellow_client_core::shared_logic::friends::FriendsTab;
use mellow_client_core::state::ClientState;
use mellow_protocol::{DirectMessageChannel, Relationship, RelationshipKind, Snowflake};

use super::FriendsPane;
use crate::components::bind_tab_navigation;
use crate::navigation::Place;
use crate::session::Session;
use crate::shell::{DuskShell, bind_shell_keys};
use crate::shell_fixtures::{ANA, ME, ZOE, person, sample_state};
use crate::strings_pending_i18n::{ActiveStrings, ENGLISH};
use crate::theme::ActiveTheme;
use crate::theme::tokens::BREAKPOINT_NAVIGATOR;

const LEA: Snowflake = Snowflake::from_raw(1003);
const GROUP: Snowflake = Snowflake::from_raw(60);

/// The sample world with two friends, a request waiting and a group the reader owns.
fn home_state() -> ClientState {
    let mut state = sample_state();
    for (user, kind) in [
        (person(ANA, "ana", "Ana"), RelationshipKind::Friend),
        (person(ZOE, "zoe", "Zoé"), RelationshipKind::Friend),
        (person(LEA, "lea", "Léa"), RelationshipKind::IncomingRequest),
    ] {
        state.apply_relationship(Relationship {
            user,
            kind,
            since_unix_milliseconds: 0,
        });
    }
    state.upsert_direct_message_channel(DirectMessageChannel {
        id: GROUP,
        recipients: vec![person(ANA, "ana", "Ana"), person(ZOE, "zoe", "Zoé")],
        last_message_id: None,
        owner_id: Some(ME),
        name: "Book club".to_owned(),
    });
    state
}

/// A shell at Home over `home_state`, in a window of `width` × 800.
fn open_home(cx: &mut TestAppContext, width: f32) -> (Entity<DuskShell>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(ActiveStrings(&ENGLISH));
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
        bind_tab_navigation(cx);
        bind_shell_keys(cx);
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| *session.state_mut() = home_state());
    let (shell, cx) = cx.add_window_view(|window, cx| DuskShell::new(session, window, cx));
    cx.simulate_resize(size(px(width), px(800.0)));
    cx.update(|window, _| window.activate_window());
    shell.update_in(cx, |shell, window, cx| {
        shell.open_place(None, None, window, cx);
    });
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

fn friends_tab(shell: &Entity<DuskShell>, cx: &mut VisualTestContext) -> FriendsTab {
    shell.read_with(cx, |shell, cx| {
        shell.home().read(cx).friends_pane().read(cx).tab()
    })
}

#[gpui::test]
fn home_opens_on_the_friends_space_in_both_layouts(cx: &mut TestAppContext) {
    let (shell, cx) = open_home(cx, 1280.0);
    assert_eq!(
        shell.read_with(cx, |shell, _| shell.navigation().place()),
        Some(Place::Home {
            conversation_id: None
        })
    );
    assert!(cx.debug_bounds("friends-pane").is_some());
    assert_eq!(friends_tab(&shell, cx), FriendsTab::Online);

    // In the drawer layout the tabs move under the header; the pane stays.
    cx.simulate_resize(size(px(BREAKPOINT_NAVIGATOR - 1.0), px(800.0)));
    cx.run_until_parked();
    assert!(cx.debug_bounds("friends-pane").is_some());
}

#[gpui::test]
fn the_friends_tabs_move_with_arrows_home_and_end(cx: &mut TestAppContext) {
    let (shell, cx) = open_home(cx, 1280.0);
    let pane = shell.read_with(cx, |shell, cx| shell.home().read(cx).friends_pane().clone());
    pane.update_in(cx, FriendsPane::focus_start);
    press(cx, "right");
    assert_eq!(friends_tab(&shell, cx), FriendsTab::All);
    press(cx, "end");
    assert_eq!(friends_tab(&shell, cx), FriendsTab::Add);
    press(cx, "right");
    assert_eq!(friends_tab(&shell, cx), FriendsTab::Online);
    press(cx, "left");
    press(cx, "home");
    assert_eq!(friends_tab(&shell, cx), FriendsTab::Online);
}

#[gpui::test]
fn the_people_picker_picks_by_search_and_takes_back_with_backspace(cx: &mut TestAppContext) {
    let (shell, cx) = open_home(cx, 1280.0);
    shell.update_in(cx, DuskShell::open_new_message);
    cx.run_until_parked();
    let picker = shell
        .read_with(cx, |shell, cx| {
            shell
                .home()
                .read(cx)
                .picker
                .as_ref()
                .map(|(picker, _)| picker.clone())
        })
        .expect("the picker opens");
    assert!(cx.debug_bounds("people-picker").is_some());
    let picked_ids =
        |cx: &mut VisualTestContext| picker.read_with(cx, |picker, _| picker.picked_ids().to_vec());

    // The search has focus: Enter picks the first friend found and clears it.
    cx.simulate_input("zo");
    press(cx, "enter");
    assert_eq!(picked_ids(cx), vec![ZOE]);
    cx.simulate_input("an");
    press(cx, "enter");
    assert_eq!(picked_ids(cx), vec![ZOE, ANA]);
    // Backspace in the empty search takes back the last friend picked.
    press(cx, "backspace");
    assert_eq!(picked_ids(cx), vec![ZOE]);
}

#[gpui::test]
fn a_group_shows_its_members_in_the_side_sheet(cx: &mut TestAppContext) {
    let (shell, cx) = open_home(cx, 1280.0);
    shell.update_in(cx, |shell, window, cx| {
        shell.open_channel(GROUP, window, cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("side-sheet").is_none());
    let modifier = if cfg!(target_os = "macos") {
        "cmd-u"
    } else {
        "ctrl-u"
    };
    press(cx, modifier);
    assert!(cx.debug_bounds("side-sheet").is_some());

    // A one-to-one conversation has no member sheet.
    shell.update_in(cx, |shell, window, cx| {
        shell.open_channel(crate::shell_fixtures::CONVERSATION_ANA, window, cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("side-sheet").is_none());
}
