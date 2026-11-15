//! The profile modal and hover card in a headless shell, on GPUI's test clock: their delays,
//! a trigger gone, Esc, focus given back, the shell's shortcuts under the modal, and the modal
//! dying with its shell.

use std::time::Duration;

use gpui::{
    AppContext as _, Bounds, Entity, InputEvent as _, KeyUpEvent, Keystroke, Modifiers, Pixels,
    TestAppContext, VisualTestContext, point, px, size,
};
use mellow_client_core::shared_logic::profiles::UserProfileLoad;

use super::*;
use crate::shell::DuskShell;
use crate::shell_fixtures::{ANA, ZOE, open_shell, person, sample_state};
use crate::theme::tokens::{
    COMPONENT_PROFILE_HOVER_CARD_DETAILS_DELAY, MOTION_DELAY_HOVER_CARD,
    MOTION_DELAY_HOVER_CARD_CLOSE,
};

/// What the window shows once the session ended.
struct EmptyRoot;

impl gpui::Render for EmptyRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        gpui::div()
    }
}

fn milliseconds(token_milliseconds: u64) -> Duration {
    Duration::from_millis(token_milliseconds)
}

/// Moves the test clock on and lets what came due run.
fn advance(cx: &mut VisualTestContext, by: Duration) {
    cx.executor().advance_clock(by);
    cx.run_until_parked();
}

fn press(cx: &mut VisualTestContext, key: &'static str) {
    cx.simulate_keystrokes(key);
    let keystroke = Keystroke::parse(key).expect("a valid keystroke in a test");
    cx.update(|window, cx| {
        window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
    });
    cx.run_until_parked();
}

fn secondary(key: &str) -> &'static str {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    Box::leak(format!("{modifier}-{key}").into_boxed_str())
}

fn surfaces_of(shell: &Entity<DuskShell>, cx: &mut VisualTestContext) -> Entity<ProfileSurfaces> {
    shell.read_with(cx, |shell, _| shell.profile_surfaces().clone())
}

/// A name drawn at a fixed place; dropping the returned anchor is the name leaving the window.
fn trigger_for(
    user_id: Snowflake,
    cx: &mut VisualTestContext,
) -> (ProfileHoverTrigger, Entity<Option<Bounds<Pixels>>>) {
    let anchor = cx.new(|_| {
        Some(Bounds::new(
            point(px(400.0), px(200.0)),
            size(px(80.0), px(24.0)),
        ))
    });
    let trigger = ProfileHoverTrigger {
        user: person(user_id, "ana", "Ana"),
        placement: HoverCardPlacement::Below,
        anchor: anchor.downgrade(),
        column: None,
        opener: None,
    };
    (trigger, anchor)
}

fn card_user(surfaces: &Entity<ProfileSurfaces>, cx: &mut VisualTestContext) -> Option<Snowflake> {
    surfaces.read_with(cx, |surfaces, _| surfaces.hover_card_user_id())
}

fn modal_user(surfaces: &Entity<ProfileSurfaces>, cx: &mut VisualTestContext) -> Option<Snowflake> {
    surfaces.read_with(cx, |surfaces, _| surfaces.modal_user_id())
}

#[gpui::test]
fn the_hover_card_opens_after_its_delay_and_closes_after_the_grace(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    let surfaces = surfaces_of(&shell, cx);
    let (trigger, _anchor) = trigger_for(ANA, cx);
    let open_delay = milliseconds(MOTION_DELAY_HOVER_CARD.0);
    let close_delay = milliseconds(MOTION_DELAY_HOVER_CARD_CLOSE.0);
    let just_before = Duration::from_millis(1);

    cx.update(|_, cx| enter_profile_trigger(trigger.clone(), cx));
    advance(cx, open_delay.saturating_sub(just_before));
    assert_eq!(
        card_user(&surfaces, cx),
        None,
        "crossing a name opens nothing"
    );
    advance(cx, just_before);
    assert_eq!(card_user(&surfaces, cx), Some(ANA));
    assert!(cx.debug_bounds("profile-hover-card").is_some());

    // Leaving, then coming back within the grace, keeps it.
    cx.update(|_, cx| leave_profile_trigger(&trigger, cx));
    advance(cx, close_delay.saturating_sub(just_before));
    cx.update(|_, cx| enter_profile_trigger(trigger.clone(), cx));
    advance(cx, close_delay);
    assert_eq!(card_user(&surfaces, cx), Some(ANA));

    // The pointer travelling to the card keeps it too.
    cx.update(|_, cx| leave_profile_trigger(&trigger, cx));
    surfaces.update(cx, |surfaces, _| surfaces.enter_card());
    advance(cx, close_delay);
    assert_eq!(card_user(&surfaces, cx), Some(ANA));

    surfaces.update(cx, ProfileSurfaces::leave_card);
    advance(cx, close_delay.saturating_sub(just_before));
    assert_eq!(card_user(&surfaces, cx), Some(ANA), "still in its grace");
    advance(cx, just_before);
    assert_eq!(card_user(&surfaces, cx), None);
}

#[gpui::test]
fn about_me_is_read_only_once_the_card_stayed_open_a_moment(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    let surfaces = surfaces_of(&shell, cx);
    let (trigger, _anchor) = trigger_for(ANA, cx);
    cx.update(|_, cx| enter_profile_trigger(trigger, cx));
    advance(cx, milliseconds(MOTION_DELAY_HOVER_CARD.0));
    let details_delay = milliseconds(COMPONENT_PROFILE_HOVER_CARD_DETAILS_DELAY.0);
    advance(cx, details_delay.saturating_sub(Duration::from_millis(1)));
    let details = surfaces.read_with(cx, |surfaces, _| surfaces.hover_card_details().clone());
    assert_eq!(
        details,
        UserProfileLoad::Idle,
        "nothing read for a card only crossed"
    );
    advance(cx, Duration::from_millis(1));
    // Without an instance the read fails at once: it was asked.
    let details = surfaces.read_with(cx, |surfaces, _| surfaces.hover_card_details().clone());
    assert_eq!(details, UserProfileLoad::Failed { user_id: ANA });
}

#[gpui::test]
fn a_wait_ending_on_a_name_gone_opens_nothing(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    let surfaces = surfaces_of(&shell, cx);
    let (trigger, anchor) = trigger_for(ANA, cx);
    cx.update(|_, cx| enter_profile_trigger(trigger, cx));
    advance(cx, milliseconds(MOTION_DELAY_HOVER_CARD.0) / 2);
    // The member regrouped by a presence change: the row is no longer drawn.
    drop(anchor);
    advance(cx, milliseconds(MOTION_DELAY_HOVER_CARD.0));
    assert_eq!(card_user(&surfaces, cx), None);

    // A card shown whose name leaves goes with it.
    let (trigger, anchor) = trigger_for(ZOE, cx);
    cx.update(|_, cx| enter_profile_trigger(trigger, cx));
    advance(cx, milliseconds(MOTION_DELAY_HOVER_CARD.0));
    assert_eq!(card_user(&surfaces, cx), Some(ZOE));
    drop(anchor);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert_eq!(card_user(&surfaces, cx), None);
}

#[gpui::test]
fn esc_closes_the_hover_card_and_only_it(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1000.0);
    let surfaces = surfaces_of(&shell, cx);
    // The member sheet over the conversation, which Esc would close too.
    press(cx, secondary("u"));
    assert!(shell.read_with(cx, |shell, _| shell.is_member_overlay_open()));
    let (trigger, _anchor) = trigger_for(ANA, cx);
    cx.update(|_, cx| enter_profile_trigger(trigger, cx));
    advance(cx, milliseconds(MOTION_DELAY_HOVER_CARD.0));
    assert_eq!(card_user(&surfaces, cx), Some(ANA));

    press(cx, "escape");
    assert_eq!(card_user(&surfaces, cx), None);
    assert!(
        shell.read_with(cx, |shell, _| shell.is_member_overlay_open()),
        "the sheet never saw the key"
    );
}

#[gpui::test]
fn a_member_row_opens_the_modal_whose_esc_gives_focus_back_to_the_row(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    let surfaces = surfaces_of(&shell, cx);
    press(cx, secondary("u"));
    let selector: &'static str = Box::leak(format!("member-row-{}", ANA.raw()).into_boxed_str());
    let row = cx
        .debug_bounds(selector)
        .expect("Ana's row is drawn in the member sheet");
    cx.simulate_click(row.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(modal_user(&surfaces, cx), Some(ANA));
    assert!(cx.debug_bounds("profile-modal").is_some());
    assert!(
        cx.debug_bounds("member-profile").is_none(),
        "the sheet stays the list"
    );
    let is_primary_focused = cx.update(|window, cx| {
        surfaces
            .read(cx)
            .modal_primary_focus()
            .is_some_and(|focus| focus.is_focused(window))
    });
    assert!(is_primary_focused, "focus starts on Send message");
    // The card was waiting under the click: the modal took its place.
    advance(cx, milliseconds(MOTION_DELAY_HOVER_CARD.0));
    assert_eq!(card_user(&surfaces, cx), None);

    press(cx, "escape");
    assert_eq!(modal_user(&surfaces, cx), None);
    assert!(cx.debug_bounds("profile-modal").is_none());
    let is_in_sheet = cx.update(|window, cx| {
        shell
            .read(cx)
            .member_sheet_focus_for_tests()
            .contains_focused(window, cx)
    });
    assert!(is_in_sheet, "focus is back on Ana's row");
    assert!(
        cx.debug_bounds("side-sheet").is_some(),
        "Esc closed the modal only"
    );
    // Focus given back does not open the row's card again.
    advance(cx, milliseconds(MOTION_DELAY_HOVER_CARD.0));
    assert_eq!(card_user(&surfaces, cx), None);
}

#[gpui::test]
fn the_shell_s_shortcuts_wait_under_the_modal(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    let surfaces = surfaces_of(&shell, cx);
    cx.update(|window, cx| open_profile_modal(person(ANA, "ana", "Ana"), None, window, cx));
    cx.run_until_parked();
    assert_eq!(modal_user(&surfaces, cx), Some(ANA));
    let channel = shell.read_with(cx, |shell, _| shell.navigation().selected_channel_id());

    press(cx, secondary("k"));
    press(cx, "alt-down");
    press(cx, secondary("u"));
    press(cx, "f6");
    shell.read_with(cx, |shell, _| {
        assert!(!shell.is_quick_switcher_open());
        assert_eq!(shell.navigation().selected_channel_id(), channel);
    });
    assert!(cx.debug_bounds("side-sheet").is_none());
    let is_focus_inside = cx.update(|window, cx| {
        surfaces
            .read(cx)
            .modal_primary_focus()
            .is_some_and(|focus| focus.is_focused(window))
    });
    assert!(is_focus_inside, "F6 left focus in the modal");
    // Tab cycles inside.
    for _ in 0..6 {
        press(cx, "tab");
    }
    let is_in_modal = cx.update(|window, cx| {
        let modal = surfaces
            .read(cx)
            .modal
            .as_ref()
            .map(modal::ProfileModal::container_focus_for_tests);
        modal.is_some_and(|container| container.contains_focused(window, cx))
    });
    assert!(is_in_modal, "Tab stays inside the modal");
}

#[gpui::test]
fn the_modal_says_why_add_friend_failed(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    let surfaces = surfaces_of(&shell, cx);
    cx.update(|window, cx| open_profile_modal(person(ANA, "ana", "Ana"), None, window, cx));
    cx.run_until_parked();
    // Signed out: the instance cannot be asked, as when it is unreachable.
    surfaces.update_in(cx, |surfaces, window, cx| {
        surfaces.change_relationship(modal::RelationshipChange::AddFriend, window, cx);
    });
    cx.run_until_parked();
    let error = surfaces.read_with(cx, |surfaces, _| surfaces.modal_error().map(str::to_owned));
    assert!(
        error.is_some_and(|error| error.starts_with("Couldn't send the friend request")),
        "the refusal shows under the actions"
    );
}

#[gpui::test]
fn the_modal_dies_with_its_shell(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    cx.update(|window, cx| open_profile_modal(person(ANA, "ana", "Ana"), None, window, cx));
    cx.run_until_parked();
    assert!(cx.update(|_, cx| is_profile_modal_open(cx)));
    let surfaces = surfaces_of(&shell, cx).downgrade();
    // The session ended: the window's root moves on, the shell goes.
    cx.update(|window, cx| {
        window.replace_root(cx, |_, _| EmptyRoot);
    });
    let weak_shell = shell.downgrade();
    drop(shell);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(weak_shell.upgrade().is_none(), "the shell went");
    assert!(surfaces.upgrade().is_none());
    assert!(!cx.update(|_, cx| is_profile_modal_open(cx)));
}

#[gpui::test]
fn the_profile_entry_of_a_person_menu_opens_the_modal(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    let surfaces = surfaces_of(&shell, cx);
    press(cx, secondary("u"));
    let selector: &'static str = Box::leak(format!("member-row-{}", ANA.raw()).into_boxed_str());
    let row = cx.debug_bounds(selector).expect("Ana's row is drawn");
    cx.simulate_event(gpui::MouseDownEvent {
        position: row.center(),
        modifiers: Modifiers::none(),
        button: gpui::MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();
    assert!(shell.read_with(cx, |shell, cx| shell.home().read(cx).is_menu_open()));
    // Profile comes first.
    press(cx, "down");
    press(cx, "enter");
    assert_eq!(modal_user(&surfaces, cx), Some(ANA));
}

fn row_center(member: Snowflake, cx: &mut VisualTestContext) -> gpui::Point<Pixels> {
    let selector: &'static str = Box::leak(format!("member-row-{}", member.raw()).into_boxed_str());
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .center()
}

#[gpui::test]
fn esc_in_the_composer_closes_the_card_and_keeps_the_edit(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    cx.update(|_, cx| crate::composer::bind_composer_keys(cx));
    let surfaces = surfaces_of(&shell, cx);
    let composer = shell.update_in(cx, |shell, window, cx| {
        shell.open_channel(crate::shell_fixtures::RUST_GENERAL, window, cx);
        shell.conversation().read(cx).composer().clone()
    });
    composer.update_in(cx, |composer, window, cx| {
        composer.start_editing(Snowflake::from_raw(4242), "Bonjour", window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        composer.read_with(cx, |composer, _| composer.editing_message_id()),
        Some(Snowflake::from_raw(4242))
    );
    let (trigger, _anchor) = trigger_for(ANA, cx);
    cx.update(|_, cx| enter_profile_trigger(trigger, cx));
    advance(cx, milliseconds(MOTION_DELAY_HOVER_CARD.0));
    assert_eq!(card_user(&surfaces, cx), Some(ANA));

    press(cx, "escape");
    assert_eq!(card_user(&surfaces, cx), None, "Esc closed the card");
    assert_eq!(
        composer.read_with(cx, |composer, _| composer.editing_message_id()),
        Some(Snowflake::from_raw(4242)),
        "and only the card: the edit goes on"
    );
}

#[gpui::test]
fn the_pointer_moving_from_one_member_to_the_next_switches_the_card(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    let surfaces = surfaces_of(&shell, cx);
    press(cx, secondary("u"));
    let ana = row_center(ANA, cx);
    let zoe = row_center(ZOE, cx);
    cx.simulate_mouse_move(ana, None, Modifiers::none());
    advance(cx, milliseconds(MOTION_DELAY_HOVER_CARD.0));
    assert_eq!(card_user(&surfaces, cx), Some(ANA));
    cx.simulate_mouse_move(zoe, None, Modifiers::none());
    advance(cx, milliseconds(MOTION_DELAY_HOVER_CARD_CLOSE.0));
    assert_eq!(card_user(&surfaces, cx), Some(ZOE));
}

#[gpui::test]
fn keyboard_focus_moving_between_members_switches_the_card(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    let surfaces = surfaces_of(&shell, cx);
    press(cx, secondary("u"));
    let zoe = row_center(ZOE, cx);
    // A click focuses Zoé's row without a card; the keyboard then moves on.
    cx.simulate_click(zoe, Modifiers::none());
    cx.run_until_parked();
    press(cx, "escape");
    cx.simulate_mouse_move(gpui::point(px(5.0), px(790.0)), None, Modifiers::none());
    press(cx, "shift-tab");
    advance(cx, milliseconds(MOTION_DELAY_HOVER_CARD.0));
    let first = card_user(&surfaces, cx);
    assert!(first.is_some(), "the focused row's card opened");
    press(cx, "shift-tab");
    advance(cx, milliseconds(MOTION_DELAY_HOVER_CARD.0));
    let second = card_user(&surfaces, cx);
    assert!(
        second.is_some() && second != first,
        "the next row's card took its place: {first:?} then {second:?}"
    );
}

#[gpui::test]
fn a_shown_card_is_drawn_again_once_measured(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    let surfaces = surfaces_of(&shell, cx);
    let (trigger, _anchor) = trigger_for(ANA, cx);
    cx.update(|_, cx| enter_profile_trigger(trigger, cx));
    advance(cx, milliseconds(MOTION_DELAY_HOVER_CARD.0));
    // Nothing else moves: only the frame the card asked for draws it again.
    cx.update(gpui::Window::simulate_next_frame);
    cx.run_until_parked();
    let is_drawn_measured = surfaces.read_with(cx, |surfaces, _| surfaces.hover.is_drawn_measured);
    assert!(
        is_drawn_measured,
        "not left invisible after its first frame"
    );
}

#[gpui::test]
fn a_late_answer_for_the_last_profile_never_reaches_the_new_one(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    let surfaces = surfaces_of(&shell, cx);
    // Ana's profile comes from the last minute's reads, Zoé's from the instance (which fails at
    // once here): Ana's answer lands after Zoé's modal opened.
    surfaces.update(cx, |surfaces, cx| {
        let now = cx.background_executor().now();
        let reader_id = surfaces
            .session
            .read(cx)
            .state()
            .current_user()
            .map(|user| user.id);
        surfaces.recent_profiles.get(ANA, reader_id, now);
        surfaces.recent_profiles.remember(
            ANA,
            mellow_protocol::UserProfile {
                user_id: ANA,
                about_me: "Ana".to_owned(),
                links: Vec::new(),
            },
            now,
        );
    });
    cx.update(|window, cx| {
        open_profile_modal(person(ANA, "ana", "Ana"), None, window, cx);
        open_profile_modal(person(ZOE, "zoe", "Zoé"), None, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(modal_user(&surfaces, cx), Some(ZOE));
    let details = surfaces.read_with(cx, |surfaces, _| surfaces.modal_details_for_tests().clone());
    assert_eq!(details, UserProfileLoad::Failed { user_id: ZOE });
}
