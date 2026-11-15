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
    ANA, CONVERSATION_ANA, GUILD_RUST, ME, RUST_VOICE, person, sample_state,
    sample_state_with_calls, session_for_shell,
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

#[gpui::test]
fn a_call_that_stopped_ringing_here_shows_the_join_banner(cx: &mut TestAppContext) {
    let (shell, session, calls, cx) = shell_with_calls(cx, sample_state_with_calls());
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_channel(CONVERSATION_ANA, window, cx);
        });
    });
    ring(&session, cx);
    assert!(
        cx.debug_bounds("call-banner").is_none(),
        "the card shows instead"
    );
    // Declined here before the instance heard it: the call still lists the reader as rung.
    calls.update(cx, |calls, cx| calls.decline_call(CONVERSATION_ANA, cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("incoming-call").is_none());
    assert!(
        cx.debug_bounds("call-banner").is_some(),
        "joinable, as on the web"
    );
}

#[gpui::test]
fn esc_closes_a_hover_card_before_it_declines_a_call(cx: &mut TestAppContext) {
    use gpui::{AppContext as _, Bounds, point};

    let (shell, session, calls, cx) = shell_with_calls(cx, sample_state_with_calls());
    ring(&session, cx);
    assert!(is_focused(&answer_focus(&shell, cx), cx));
    let anchor = cx.new(|_| {
        Some(Bounds::new(
            point(px(400.0), px(200.0)),
            size(px(80.0), px(24.0)),
        ))
    });
    let trigger = crate::profile_surfaces::ProfileHoverTrigger {
        user: person(ANA, "ana", "Ana"),
        placement: crate::profile_surfaces::HoverCardPlacement::Below,
        anchor: anchor.downgrade(),
        column: None,
        opener: None,
    };
    cx.update(|_, cx| crate::profile_surfaces::enter_profile_trigger(trigger, cx));
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(
            crate::theme::tokens::MOTION_DELAY_HOVER_CARD.0,
        ));
    cx.run_until_parked();
    assert!(cx.update(|_, cx| crate::profile_surfaces::is_profile_hover_card_shown(cx)));

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!cx.update(|_, cx| crate::profile_surfaces::is_profile_hover_card_shown(cx)));
    assert!(is_ringing(&calls, cx), "the first Esc only closed the card");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!is_ringing(&calls, cx), "the next one declines");
    drop(anchor);
}

#[gpui::test]
fn the_conversation_redraws_only_when_what_the_banner_reads_of_the_voice_changes(
    cx: &mut TestAppContext,
) {
    use std::cell::Cell;

    let session = session_for_shell(cx, sample_state_with_calls());
    session.update(cx, |session, _| session.pretend_signed_in());
    cx.update(|cx| cx.set_global(crate::preferences::PreferencesStore::default()));
    let format = mellow_media::StreamFormat {
        sample_rate: 48_000,
        channels: 1,
    };
    let voice = cx.update(|cx| {
        crate::voice::install_voice(
            &session,
            std::sync::Arc::new(move || {
                Box::new(mellow_media::SyntheticAudioDevices::new(
                    mellow_media::SyntheticSound::Silence,
                    format,
                    format,
                ))
            }),
            cx,
        )
    });
    let shell_session = session.clone();
    let (shell, cx) = cx.add_window_view(|window, cx| DuskShell::new(shell_session, window, cx));
    cx.run_until_parked();
    let conversation = shell.read_with(cx, |shell, _| shell.conversation().clone());
    let redraws = Rc::new(Cell::new(0_u32));
    let counted = Rc::clone(&redraws);
    let _counting =
        cx.update(|_, cx| cx.observe(&conversation, move |_, _| counted.set(counted.get() + 1)));

    // Speaking edges, the round trip, the meter: the voice notifies, the banner reads nothing new.
    for _ in 0..5 {
        voice.update(cx, |_, cx| cx.notify());
    }
    cx.run_until_parked();
    assert_eq!(redraws.get(), 0);

    voice.update(cx, |voice, cx| {
        voice.pretend_call_for_tests(GUILD_RUST, RUST_VOICE);
        voice.open_call_view(cx);
    });
    cx.run_until_parked();
    assert!(
        redraws.get() >= 1,
        "the call view opening is news to the banner"
    );
}
