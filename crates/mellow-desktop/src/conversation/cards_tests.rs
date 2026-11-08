//! Polls, checklists and progress bars in the conversation, headless: the poll clock, the menu's
//! "End poll" and the composer's "/" commands opening their dialogs.

use std::time::Duration;

use gpui::{TestAppContext, VisualTestContext};
use mellow_protocol::{Message, Poll, PollAnswer, Snowflake};

use super::tests::{composer_text, focus_composer, me, message, open_general, press};
use super::*;
use crate::shell_fixtures::RUST_GENERAL;

const POLL_ID: u64 = 10_001;
const DAY_IN_MILLISECONDS: u64 = 24 * 60 * 60 * 1_000;

fn lunch_poll() -> Message {
    let mut poll_message = message(POLL_ID, &me(), "");
    let answer = |id, text: &str| PollAnswer {
        id,
        text: text.to_owned(),
        vote_count: 0,
        me_voted: false,
    };
    poll_message.poll = Some(Poll {
        question: "Lunch?".into(),
        answers: vec![answer(1, "Pizza"), answer(2, "Soup")],
        allows_multiple_answers: false,
        expires_at_unix_milliseconds: now_unix_milliseconds() + DAY_IN_MILLISECONDS,
        ended_at_unix_milliseconds: None,
    });
    poll_message
}

fn type_in_composer(view: &Entity<ConversationView>, text: &str, cx: &mut VisualTestContext) {
    focus_composer(view, cx);
    cx.update(|window, cx| {
        let composer = view.read(cx).composer().clone();
        composer.update(cx, |composer, cx| composer.type_text(text, window, cx));
    });
    cx.run_until_parked();
}

fn is_clock_running(view: &Entity<ConversationView>, cx: &mut VisualTestContext) -> bool {
    view.read_with(cx, |view, _| view.is_poll_clock_running())
}

/// One tick of the poll clock, on the test executor's time.
fn advance_a_minute(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
}

#[gpui::test]
fn an_open_poll_on_screen_runs_the_clock_until_it_ends(cx: &mut TestAppContext) {
    let (view, session, cx) = open_general(cx, vec![lunch_poll()], None);
    assert!(is_clock_running(&view, cx), "an open poll is drawn");
    advance_a_minute(cx);
    assert!(is_clock_running(&view, cx), "still on screen");

    session.update(cx, |session, cx| {
        let change = session
            .state_mut()
            .end_poll(RUST_GENERAL, Snowflake::from_raw(POLL_ID), 1);
        cx.emit(StateChanged(vec![change]));
    });
    cx.run_until_parked();
    advance_a_minute(cx);
    advance_a_minute(cx);
    assert!(!is_clock_running(&view, cx), "an ended poll stops watching");
}

#[gpui::test]
fn the_author_ends_an_open_poll_after_confirming(cx: &mut TestAppContext) {
    let (view, _session, cx) = open_general(cx, vec![lunch_poll()], None);
    let actions = cx.update(|_, cx| {
        let view = view.read(cx);
        let poll = view
            .cached_message(Snowflake::from_raw(POLL_ID), cx)
            .expect("the poll is cached");
        message_menu_actions(view.message_facts(&poll, cx), &[])
    });
    assert!(actions.contains(&MenuLine::Action(MessageMenuAction::EndPoll)));
    assert!(!actions.contains(&MenuLine::Action(MessageMenuAction::Edit)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.run_menu_action(
                Snowflake::from_raw(POLL_ID),
                MessageMenuAction::EndPoll,
                window,
                cx,
            );
        });
    });
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| view.confirmation.is_some()));
    press(cx, "escape");
    assert!(view.read_with(cx, |view, _| view.confirmation.is_none()));
}

#[gpui::test]
fn slash_poll_opens_its_dialog_only_where_polls_may_be_sent(cx: &mut TestAppContext) {
    let (view, _session, cx) = open_general(cx, vec![message(10_001, &me(), "hi")], None);
    // The fixture's server channel grants no "Send polls": "/poll" names no command there.
    type_in_composer(&view, "/poll", cx);
    press(cx, "enter");
    assert!(view.read_with(cx, |view, _| view.confirmation.is_none()));

    // A conversation lets whoever writes there send polls (ADR-0043).
    let conversation_with_ana = OpenConversation {
        channel_id: crate::shell_fixtures::CONVERSATION_ANA,
        guild_id: None,
        label: "Ana".to_owned(),
        composer_label: "Message @Ana".to_owned(),
        hero_icon: IconName::ChatCircle,
        hero_title: "Ana".to_owned(),
        hero_sentence: String::new(),
        topic: String::new(),
    };
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.set_place(Some(conversation_with_ana), cx);
        });
    });
    cx.run_until_parked();
    type_in_composer(&view, "/poll", cx);
    press(cx, "enter");
    assert!(view.read_with(cx, |view, _| view.confirmation.is_some()));
    assert_eq!(
        composer_text(&view, cx),
        "",
        "the /query went, nothing else"
    );
}

#[gpui::test]
fn slash_bold_writes_an_empty_pair_and_never_sends(cx: &mut TestAppContext) {
    let (view, session, cx) = open_general(cx, vec![message(10_001, &me(), "hi")], None);
    type_in_composer(&view, "/bold", cx);
    press(cx, "enter");
    assert_eq!(composer_text(&view, cx), "****");
    let pending = session.read_with(cx, |session, _| {
        session.state().pending_messages(RUST_GENERAL).count()
    });
    assert_eq!(pending, 0);
    assert!(view.read_with(cx, |view, _| view.confirmation.is_none()));
}

fn release_checklist_and_sync_bar() -> Vec<Message> {
    let mut checklist = message(10_002, &me(), "");
    checklist.checklist = Some(mellow_protocol::Checklist {
        title: "Release".into(),
        items: vec![
            mellow_protocol::ChecklistItem {
                id: 1,
                text: "Tag".into(),
                checked: Some(mellow_protocol::ChecklistTick {
                    user_id: crate::shell_fixtures::ME,
                    at_unix_milliseconds: 1_000,
                }),
            },
            mellow_protocol::ChecklistItem {
                id: 2,
                text: "Deploy".into(),
                checked: None,
            },
        ],
        revision: 3,
    });
    let mut bar = message(10_003, &me(), "");
    bar.progress_bar = Some(mellow_protocol::ProgressBar {
        label: "Sync".into(),
        value: 2,
        maximum: 5,
        revision: 1,
    });
    vec![checklist, bar]
}

#[gpui::test]
fn the_author_edits_a_checklist_in_place_and_updates_a_progress_bar_in_a_dialog(
    cx: &mut TestAppContext,
) {
    let (view, _session, cx) = open_general(cx, release_checklist_and_sync_bar(), None);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.toggle_checklist_editing(Snowflake::from_raw(10_002), window, cx);
        });
    });
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| view.is_editing_checklist()));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.toggle_checklist_editing(Snowflake::from_raw(10_002), window, cx);
            view.open_card_dialog(
                CardDialogKind::ProgressBar(Some(Snowflake::from_raw(10_003))),
                window,
                cx,
            );
        });
    });
    cx.run_until_parked();
    assert!(!view.read_with(cx, |view, _| view.is_editing_checklist()));
    assert!(view.read_with(cx, |view, _| view.confirmation.is_some()));
}

#[gpui::test]
fn a_checklist_without_its_title_stays_in_its_dialog_and_a_valid_one_is_sent(
    cx: &mut TestAppContext,
) {
    let (view, session, cx) = open_general(cx, Vec::new(), None);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_card_dialog(CardDialogKind::Checklist, window, cx);
        });
    });
    cx.run_until_parked();
    let submit = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let dialog = view
                .read(cx)
                .confirmation
                .as_ref()
                .map(|(dialog, _)| dialog.clone())
                .expect("the dialog is open");
            dialog.update(cx, crate::components::ModalDialog::submit_now);
        });
        cx.run_until_parked();
    };
    submit(cx);
    assert!(
        view.read_with(cx, |view, _| view.confirmation.is_some()),
        "a missing title is said in the dialog"
    );
    cx.simulate_input("Release");
    press(cx, "enter");
    cx.simulate_input("Tag");
    submit(cx);
    assert!(view.read_with(cx, |view, _| view.confirmation.is_none()));
    // Signed out in tests: the valid checklist reached the send, which says it could not go.
    let error = view.read_with(cx, |view, _| view.action_error.clone());
    assert_eq!(
        error.as_deref(),
        Some(crate::i18n::Strings::default().chat().send_failed())
    );
    let queued = session.read_with(cx, |session, _| {
        session.state().pending_messages(RUST_GENERAL).count()
    });
    assert_eq!(queued, 0);
}
