//! The reactions dialog headless: who gets a Remove button, and where focus goes after one.

use gpui::{AppContext as _, TestAppContext, VisualTestContext, WindowAppearance};
use mellow_protocol::{PublicUser, ReactionSummary};

use super::*;
use crate::conversation::tests::{ana, message};
use crate::shell_fixtures::{RUST_GENERAL, person, sample_state};
use crate::strings_pending_i18n::{ActiveStrings, ENGLISH};
use crate::theme::ActiveTheme;

const MESSAGE: u64 = 10_001;
const THUMBS_UP: &str = "👍";

fn bob() -> PublicUser {
    person(Snowflake::from_raw(1_002), "bob", "Bob")
}

fn cleo() -> PublicUser {
    person(Snowflake::from_raw(1_003), "cleo", "Cleo")
}

fn thumbs_up(count: u32) -> ReactionSummary {
    ReactionSummary {
        emoji: THUMBS_UP.to_owned(),
        count,
        me: false,
    }
}

/// The dialog over a message with `count` 👍, already showing `users` as their answer (no
/// instance: the session is signed out, so nothing is asked again while the count holds).
fn open_dialog(
    cx: &mut TestAppContext,
    users: Vec<PublicUser>,
    can_remove_others: bool,
) -> (
    Entity<ReactionsDialogBody>,
    Entity<Session>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(ActiveStrings(&ENGLISH));
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
    });
    let count = u32::try_from(users.len()).expect("a handful of test users");
    let mut liked = message(MESSAGE, &ana(), "hi");
    liked.reactions = vec![thumbs_up(count)];
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| {
        let state = session.state_mut();
        *state = sample_state();
        state.insert_history_page(RUST_GENERAL, vec![liked]);
    });
    let dialog_session = session.clone();
    let (body, cx) = cx.add_window_view(|_, cx| {
        let mut body = ReactionsDialogBody::new(
            dialog_session,
            RUST_GENERAL,
            Snowflake::from_raw(MESSAGE),
            HashMap::new(),
            can_remove_others,
            cx,
        );
        body.asked = Some((THUMBS_UP.to_owned(), count));
        body.users = users;
        body.load_state = LoadState::Loaded;
        body
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (body, session, cx)
}

/// `user`'s 👍 goes, as the gateway's event says it.
fn remove_reaction_of(session: &Entity<Session>, user: &PublicUser, cx: &mut VisualTestContext) {
    session.update(cx, |session, cx| {
        let event = mellow_protocol::GatewayEvent::MessageReactionRemove(
            mellow_protocol::MessageReaction {
                guild_id: Some(crate::shell_fixtures::GUILD_RUST),
                channel_id: RUST_GENERAL,
                message_id: Snowflake::from_raw(MESSAGE),
                user_id: user.id,
                emoji: THUMBS_UP.to_owned(),
            },
        );
        session.state_mut().apply_event(event, 0);
        cx.notify();
    });
}

fn is_remove_focused(
    body: &Entity<ReactionsDialogBody>,
    user: &PublicUser,
    cx: &mut VisualTestContext,
) -> bool {
    cx.update(|window, cx| {
        body.read(cx)
            .remove_focus
            .get(&user.id)
            .is_some_and(|focus| focus.is_focused(window))
    })
}

#[gpui::test]
fn a_reader_gets_no_remove_button(cx: &mut TestAppContext) {
    let (body, _session, cx) = open_dialog(cx, vec![ana(), bob()], false);
    assert!(body.read_with(cx, |body, _| body.remove_focus.is_empty()));
}

#[gpui::test]
fn a_moderator_gets_a_remove_button_per_person(cx: &mut TestAppContext) {
    let (body, _session, cx) = open_dialog(cx, vec![ana(), bob()], true);
    assert_eq!(body.read_with(cx, |body, _| body.remove_focus.len()), 2);
}

#[gpui::test]
fn focus_goes_to_the_next_remove_button_or_the_previous_one_at_the_end(cx: &mut TestAppContext) {
    let (body, session, cx) = open_dialog(cx, vec![ana(), bob(), cleo()], true);
    // Bob's reaction goes: the reload lists Ana and Cleo, and Cleo's button takes Bob's place.
    body.update(cx, |body, _| {
        body.removed_row_index = Some(1);
        // Already asked for the new count: a signed-out session's refusal would race the answer.
        body.asked = Some((THUMBS_UP.to_owned(), 2));
    });
    remove_reaction_of(&session, &bob(), cx);
    body.update(cx, |body, cx| {
        body.show_who_reacted(Ok(vec![ana(), cleo()]), cx);
    });
    cx.run_until_parked();
    assert!(is_remove_focused(&body, &cleo(), cx));

    // Cleo's goes from the end: Ana's button is the one left.
    body.update(cx, |body, _| {
        body.removed_row_index = Some(1);
        // Already asked for the new count: a signed-out session's refusal would race the answer.
        body.asked = Some((THUMBS_UP.to_owned(), 1));
    });
    remove_reaction_of(&session, &cleo(), cx);
    body.update(cx, |body, cx| body.show_who_reacted(Ok(vec![ana()]), cx));
    cx.run_until_parked();
    assert!(is_remove_focused(&body, &ana(), cx));
}

#[gpui::test]
fn focus_stays_in_the_dialog_when_the_last_reaction_goes(cx: &mut TestAppContext) {
    let (body, session, cx) = open_dialog(cx, vec![ana()], true);
    body.update(cx, |body, _| body.removed_row_index = Some(0));
    remove_reaction_of(&session, &ana(), cx);
    cx.run_until_parked();
    let is_dialog_focused = cx.update(|window, cx| body.read(cx).body_focus.is_focused(window));
    assert!(
        is_dialog_focused,
        "no Remove button is left, so the dialog keeps focus"
    );
    assert_eq!(body.read_with(cx, |body, _| body.removed_row_index), None);
}
