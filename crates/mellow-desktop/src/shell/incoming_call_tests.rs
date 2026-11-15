//! The incoming call card and the header's call key in the shell, with the app's calls on a test
//! clock and no voice engine: focus, Ctrl+Enter, Esc, Do not disturb, and when the key hides.

use std::rc::Rc;

use gpui::{Entity, FocusHandle, TestAppContext, VisualTestContext, px, size};
use mellow_protocol::{GatewayEvent, Relationship, RelationshipKind};

use super::*;
use crate::calls::tests::{STARTED_AT, anas_call, set_do_not_disturb};
use crate::calls::{Calls, install_calls_with_clock};
use crate::navigation::Place;
use crate::session::{Session, StateChanged};
use crate::shell_fixtures::{
    ANA, CONVERSATION_ANA, ME, person, sample_state, sample_state_with_calls, session_for_shell,
};

fn shell_with_calls(
    cx: &mut TestAppContext,
    state: mellow_client_core::state::ClientState,
) -> (
    Entity<DuskShell>,
    Entity<Session>,
    Entity<Calls>,
    &mut VisualTestContext,
) {
    let session = session_for_shell(cx, state);
    session.update(cx, |session, _| session.pretend_signed_in());
    let calls = cx.update(|cx| install_calls_with_clock(&session, Rc::new(|| STARTED_AT), cx));
    let shell_session = session.clone();
    let (shell, cx) = cx.add_window_view(|window, cx| DuskShell::new(shell_session, window, cx));
    cx.simulate_resize(size(px(1_280.0), px(800.0)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (shell, session, calls, cx)
}

/// Rings the reader with Ana's call, as the gateway would.
fn ring(session: &Entity<Session>, cx: &mut VisualTestContext) {
    session.update(cx, |session, cx| {
        let change = session.state_mut().apply_event(
            GatewayEvent::CallCreate(anas_call(&[ME], &[ANA])),
            STARTED_AT,
        );
        cx.emit(StateChanged(vec![change]));
    });
    cx.run_until_parked();
}

fn is_focused(handle: &FocusHandle, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, _| handle.is_focused(window))
}

fn answer_focus(shell: &Entity<DuskShell>, cx: &mut VisualTestContext) -> FocusHandle {
    shell.read_with(cx, |shell, _| shell.incoming_call_focus.answer.clone())
}

fn is_ringing(calls: &Entity<Calls>, cx: &mut VisualTestContext) -> bool {
    calls.read_with(cx, |calls, cx| calls.ringing_call(cx).is_some())
}

#[gpui::test]
fn the_card_takes_focus_on_answer_and_esc_in_it_declines(cx: &mut TestAppContext) {
    let (shell, session, calls, cx) = shell_with_calls(cx, sample_state_with_calls());
    ring(&session, cx);
    assert!(cx.debug_bounds("incoming-call").is_some(), "the card shows");
    let answer = answer_focus(&shell, cx);
    assert!(is_focused(&answer, cx), "focus goes to Answer");
    let announced = shell.read_with(cx, |shell, _| {
        shell
            .incoming_call
            .as_ref()
            .map(|shown| shown.announcement.clone())
    });
    assert_eq!(
        announced.as_deref(),
        Some("Ana is calling you. Ctrl+Enter to answer.")
    );

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!is_ringing(&calls, cx), "Esc in the card declines");
    assert!(cx.debug_bounds("incoming-call").is_none());
    assert!(!is_focused(&answer, cx), "focus went back where it was");
}

#[gpui::test]
fn the_card_leaves_focus_where_the_reader_types_and_esc_there_does_not_decline(
    cx: &mut TestAppContext,
) {
    let (shell, session, calls, cx) = shell_with_calls(cx, sample_state_with_calls());
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_channel(CONVERSATION_ANA, window, cx);
            shell.conversation().update(cx, |conversation, cx| {
                conversation.focus_composer(window, cx);
            });
        });
    });
    cx.run_until_parked();
    let typing_focus = cx.update(|window, cx| window.focused(cx));
    ring(&session, cx);
    assert!(cx.debug_bounds("incoming-call").is_some());
    assert!(!is_focused(&answer_focus(&shell, cx), cx));
    assert_eq!(cx.update(|window, cx| window.focused(cx)), typing_focus);

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(
        is_ringing(&calls, cx),
        "Esc outside the card leaves the call ringing"
    );
}

#[gpui::test]
fn an_update_of_the_same_call_neither_reopens_the_card_nor_takes_focus_again(
    cx: &mut TestAppContext,
) {
    let (shell, session, _, cx) = shell_with_calls(cx, sample_state_with_calls());
    ring(&session, cx);
    let root = shell.read_with(cx, |shell, _| shell.root_focus.clone());
    cx.update(|window, cx| window.focus(&root, cx));
    // Someone else of the conversation joins: the same call, changed.
    session.update(cx, |session, cx| {
        let mut call = anas_call(&[ME], &[ANA]);
        call.participants
            .push(crate::calls::tests::participant(crate::shell_fixtures::ZOE));
        let change = session
            .state_mut()
            .apply_event(GatewayEvent::CallUpdate(call), STARTED_AT);
        cx.emit(StateChanged(vec![change]));
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("incoming-call").is_some());
    assert!(is_focused(&root, cx), "focus stays where the reader put it");
}

#[gpui::test]
fn under_do_not_disturb_the_card_shows_without_taking_focus(cx: &mut TestAppContext) {
    let (shell, session, calls, cx) = shell_with_calls(cx, sample_state_with_calls());
    set_do_not_disturb(&session, cx);
    ring(&session, cx);
    assert!(cx.debug_bounds("incoming-call").is_some());
    assert!(!is_focused(&answer_focus(&shell, cx), cx));
    assert_eq!(calls.read_with(cx, |calls, _| calls.incoming_sound()), None);
}

#[gpui::test]
fn ctrl_enter_answers_from_anywhere_by_opening_the_conversation(cx: &mut TestAppContext) {
    let (shell, session, _, cx) = shell_with_calls(cx, sample_state_with_calls());
    ring(&session, cx);
    let root = shell.read_with(cx, |shell, _| shell.root_focus.clone());
    cx.update(|window, cx| window.focus(&root, cx));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    });
    cx.run_until_parked();
    assert_eq!(
        shell.read_with(cx, |shell, _| shell.navigation().place()),
        Some(Place::Home {
            conversation_id: Some(CONVERSATION_ANA)
        })
    );
}

#[gpui::test]
fn the_header_s_call_key_shows_only_where_calling_is_possible(cx: &mut TestAppContext) {
    let (shell, session, _, cx) = shell_with_calls(cx, sample_state_with_calls());
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_channel(CONVERSATION_ANA, window, cx);
        });
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("call-conversation").is_some(), "Call Ana");

    session.update(cx, |session, cx| {
        let change = session.state_mut().apply_event(
            GatewayEvent::RelationshipAdd(Relationship {
                user: person(ANA, "ana", "Ana"),
                kind: RelationshipKind::Blocked,
                since_unix_milliseconds: STARTED_AT,
            }),
            STARTED_AT,
        );
        cx.emit(StateChanged(vec![change]));
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("call-conversation").is_none(),
        "not with someone the reader blocked"
    );
}

#[gpui::test]
fn the_call_key_hides_on_an_instance_without_calls(cx: &mut TestAppContext) {
    let (shell, _, _, cx) = shell_with_calls(cx, sample_state());
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_channel(CONVERSATION_ANA, window, cx);
        });
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("chat-header").is_some());
    assert!(cx.debug_bounds("call-conversation").is_none());
}
