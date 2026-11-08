//! Home headless: the Friends space, the people picker and a group's side sheet.

use gpui::{
    AppContext as _, Entity, InputEvent as _, KeyUpEvent, Keystroke, TestAppContext,
    VisualTestContext, WindowAppearance, px, size,
};
use mellow_client_core::shared_logic::context_menus_people::{GroupMenuAction, PersonMenuAction};
use mellow_client_core::shared_logic::friends::FriendsTab;
use mellow_client_core::state::ClientState;
use mellow_protocol::{DirectMessageChannel, Relationship, RelationshipKind, Snowflake};

use super::{FriendsPane, PersonMenuRequest};
use crate::components::{MenuOrigin, bind_tab_navigation};
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

fn is_group_dialog_open(shell: &Entity<DuskShell>, cx: &mut VisualTestContext) -> bool {
    shell.read_with(cx, |shell, cx| shell.home().read(cx).group_dialog.is_some())
}

#[gpui::test]
fn renaming_a_group_to_its_own_name_closes_without_asking_the_instance(cx: &mut TestAppContext) {
    let (shell, cx) = open_home(cx, 1280.0);
    let home = shell.read_with(cx, |shell, _| shell.home().clone());
    home.update_in(cx, |home, window, cx| {
        home.ask_to_rename_group(GROUP, window, cx);
    });
    cx.run_until_parked();
    assert!(is_group_dialog_open(&shell, cx));
    // The field starts with "Book club"; the session is signed out, so a request would hang.
    press(cx, "enter");
    assert!(!is_group_dialog_open(&shell, cx));
}

#[gpui::test]
fn a_remove_dialog_closes_when_its_person_left_the_group_meanwhile(cx: &mut TestAppContext) {
    let (shell, cx) = open_home(cx, 1280.0);
    let home = shell.read_with(cx, |shell, _| shell.home().clone());
    home.update_in(cx, |home, window, cx| {
        home.ask_to_remove_from_group(GROUP, person(ZOE, "zoe", "Zoé"), window, cx);
    });
    cx.run_until_parked();
    assert!(is_group_dialog_open(&shell, cx));

    // Someone else's change: Ana leaves, the dialog about Zoé stays.
    let session = home.read_with(cx, |home, _| home.session.clone());
    let keep_only = |recipients: Vec<_>, cx: &mut VisualTestContext| {
        session.update(cx, |session, cx| {
            session
                .state_mut()
                .upsert_direct_message_channel(DirectMessageChannel {
                    id: GROUP,
                    recipients,
                    last_message_id: None,
                    owner_id: Some(ME),
                    name: "Book club".to_owned(),
                });
            cx.notify();
        });
        cx.run_until_parked();
    };
    keep_only(vec![person(ZOE, "zoe", "Zoé")], cx);
    assert!(is_group_dialog_open(&shell, cx));
    // Zoé leaves too: there is no one to remove any more.
    keep_only(vec![person(LEA, "lea", "Léa")], cx);
    assert!(!is_group_dialog_open(&shell, cx));
}

// ---- Context menus ----

fn is_menu_open(shell: &Entity<DuskShell>, cx: &mut VisualTestContext) -> bool {
    shell.read_with(cx, |shell, cx| shell.home().read(cx).menu.is_some())
}

fn clipboard_text(cx: &mut VisualTestContext) -> String {
    cx.read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default()
}

const SOMEWHERE: MenuOrigin = MenuOrigin::Point { x: 40.0, y: 40.0 };

#[gpui::test]
fn shift_f10_or_the_menu_key_on_a_friend_row_opens_their_menu_and_esc_closes_it(
    cx: &mut TestAppContext,
) {
    let (shell, cx) = open_home(cx, 1280.0);
    let pane = shell.read_with(cx, |shell, cx| shell.home().read(cx).friends_pane().clone());
    for key in ["shift-f10", "menu"] {
        pane.update_in(cx, |pane, window, cx| pane.focus_row_at(0, window, cx));
        cx.run_until_parked();
        assert!(!is_menu_open(&shell, cx));
        press(cx, key);
        assert!(is_menu_open(&shell, cx), "{key} opens the person's menu");
        press(cx, "escape");
        assert!(!is_menu_open(&shell, cx));
    }
}

#[gpui::test]
fn a_persons_menu_copies_their_id_and_says_so(cx: &mut TestAppContext) {
    let (shell, cx) = open_home(cx, 1280.0);
    let home = shell.read_with(cx, |shell, _| shell.home().clone());
    let request = PersonMenuRequest::new(person(ANA, "ana", "Ana"), SOMEWHERE);
    home.update_in(cx, |home, window, cx| {
        home.open_person_menu(request.clone(), window, cx);
    });
    assert!(is_menu_open(&shell, cx));
    home.update_in(cx, |home, window, cx| {
        home.run_person_action(PersonMenuAction::CopyUserId, &request, window, cx);
    });
    assert_eq!(clipboard_text(cx), ANA.raw().to_string());
    assert_eq!(
        home.read_with(cx, |home, _| home.announcement.clone()),
        "User ID copied."
    );
}

#[gpui::test]
fn block_from_a_persons_menu_asks_first_in_a_dialog_drawn_over_the_window(cx: &mut TestAppContext) {
    let (shell, cx) = open_home(cx, 1280.0);
    let home = shell.read_with(cx, |shell, _| shell.home().clone());
    let request = PersonMenuRequest::new(person(ZOE, "zoe", "Zoé"), SOMEWHERE);
    home.update_in(cx, |home, window, cx| {
        home.run_person_action(PersonMenuAction::Block, &request, window, cx);
    });
    cx.run_until_parked();
    let is_asking = home.read_with(cx, |home, cx| {
        home.friends_pane.read(cx).confirmation_dialog().is_some()
    });
    assert!(is_asking);
    press(cx, "escape");
    let is_asking = home.read_with(cx, |home, cx| {
        home.friends_pane.read(cx).confirmation_dialog().is_some()
    });
    assert!(!is_asking, "Esc cancels: nothing was blocked");
}

#[gpui::test]
fn a_groups_menu_renames_it_and_copies_its_id(cx: &mut TestAppContext) {
    let (shell, cx) = open_home(cx, 1280.0);
    let home = shell.read_with(cx, |shell, _| shell.home().clone());
    home.update_in(cx, |home, window, cx| {
        home.open_group_menu(GROUP, SOMEWHERE, None, window, cx);
    });
    assert!(is_menu_open(&shell, cx));
    home.update_in(cx, |home, window, cx| {
        home.run_group_action(GroupMenuAction::CopyChannelId, GROUP, window, cx);
    });
    assert_eq!(clipboard_text(cx), GROUP.raw().to_string());
    home.update_in(cx, |home, window, cx| {
        home.run_group_action(GroupMenuAction::RenameGroup, GROUP, window, cx);
    });
    cx.run_until_parked();
    assert!(is_group_dialog_open(&shell, cx));
}
