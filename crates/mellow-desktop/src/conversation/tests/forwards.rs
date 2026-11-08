//! Forwards headless: the dialog's places, search and picking, a refusal kept on its row, and
//! a forward's footer reaching its original.

use std::collections::HashMap;

use gpui::{Entity, TestAppContext, VisualTestContext};
use mellow_protocol::Snowflake;

use super::super::forward_dialog::{ForwardDialog, ForwardDialogEvent};
use super::{ana, message, open_general, press};
use crate::conversation::ConversationView;
use crate::shell_fixtures::{CONVERSATION_ANA, RUST_GENERAL, RUST_HELP};

const FORWARDED: u64 = 10_001;

/// The forward dialog on Ana's message, drawn in the conversation's dialog slot.
fn open_dialog(cx: &mut TestAppContext) -> (Entity<ForwardDialog>, &mut VisualTestContext) {
    let (view, session, cx) = open_general(cx, vec![message(FORWARDED, &ana(), "Look")], None);
    let body = cx.update(|window, cx| {
        let original = session
            .read(cx)
            .state()
            .cached_message(RUST_GENERAL, Snowflake::from_raw(FORWARDED))
            .cloned()
            .expect("the message is cached");
        let (dialog, body) = ForwardDialog::open(
            session.clone(),
            original,
            Some(RUST_GENERAL),
            HashMap::new(),
            window,
            cx,
        );
        view.update(cx, |view: &mut ConversationView, cx| {
            let subscription = cx.subscribe(&body, |_, _, _: &ForwardDialogEvent, _| {});
            view.confirmation = Some((dialog, subscription));
            cx.notify();
        });
        body
    });
    cx.run_until_parked();
    (body, cx)
}

#[gpui::test]
fn enter_in_the_search_picks_the_first_matching_place_and_clears_it(cx: &mut TestAppContext) {
    let (body, cx) = open_dialog(cx);
    cx.simulate_input("help");
    press(cx, "enter");
    body.read_with(cx, |body, _| assert_eq!(body.picked_ids(), [RUST_HELP]));
    cx.simulate_input("@ana");
    press(cx, "enter");
    body.read_with(cx, |body, _| {
        assert_eq!(body.picked_ids(), [RUST_HELP, CONVERSATION_ANA]);
    });
}

#[gpui::test]
fn a_place_that_refuses_stays_picked_with_its_reason_on_its_row(cx: &mut TestAppContext) {
    let (body, cx) = open_dialog(cx);
    cx.simulate_input("help");
    press(cx, "enter");
    // Enter with nothing typed forwards; signed out, the place refuses at once.
    press(cx, "enter");
    body.read_with(cx, |body, _| {
        assert_eq!(body.picked_ids(), [RUST_HELP]);
        assert!(body.refusal_of(RUST_HELP).is_some());
        assert!(!body.is_busy());
    });
}

#[gpui::test]
fn a_forwards_footer_reaches_its_original_once_its_channel_is_open(cx: &mut TestAppContext) {
    let (view, _session, cx) = open_general(
        cx,
        (1..=3)
            .map(|index| message(FORWARDED + index, &ana(), "hi"))
            .collect(),
        None,
    );
    let original = Snowflake::from_raw(FORWARDED + 1);
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.reach_after_opening(RUST_GENERAL, original, cx);
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(view.highlight.map(|(id, _)| id), Some(original));
        assert_eq!(view.pending_origin_jump, None);
    });
}
