//! The calls' sounds and ringing on a test clock: no audio device (no app voice), so what plays is
//! read from the state the sound player would be handed.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{Entity, TestAppContext};
use mellow_protocol::{
    CALL_RING_TIMEOUT_MILLISECONDS, Call, CallParticipant, ChosenStatus, GatewayEvent,
    MELLOW_EPOCH_UNIX_MILLISECONDS, Snowflake,
};

use super::*;
use crate::session::Session;
use crate::shell_fixtures::{ANA, CONVERSATION_ANA, ME, sample_state_with_calls};

/// When the test's call starts, by every clock.
pub(crate) const STARTED_AT: u64 = MELLOW_EPOCH_UNIX_MILLISECONDS + 1_000_000;

pub(crate) fn participant(user_id: Snowflake) -> CallParticipant {
    CallParticipant {
        user_id,
        self_mute: false,
        self_deaf: false,
        self_video: false,
        self_stream: false,
        video_codec: None,
    }
}

/// Ana's call in her conversation, started at `STARTED_AT`, ringing `ringing`.
pub(crate) fn anas_call(ringing: &[Snowflake], participants: &[Snowflake]) -> Call {
    Call {
        channel_id: CONVERSATION_ANA,
        message_id: Snowflake::from_parts(STARTED_AT - MELLOW_EPOCH_UNIX_MILLISECONDS, 0)
            .expect("a test id"),
        caller_id: ANA,
        ringing_user_ids: ringing.to_vec(),
        participants: participants.iter().copied().map(participant).collect(),
    }
}

/// Applies a gateway event as the session does, telling its followers.
pub(crate) fn apply(session: &Entity<Session>, event: GatewayEvent, cx: &mut TestAppContext) {
    session.update(cx, |session, cx| {
        let change = session.state_mut().apply_event(event, STARTED_AT);
        cx.emit(StateChanged(vec![change]));
    });
    cx.run_until_parked();
}

pub(crate) fn set_do_not_disturb(session: &Entity<Session>, cx: &mut TestAppContext) {
    session.update(cx, |session, cx| {
        let mut user = session.state().current_user().cloned().expect("signed in");
        user.status = ChosenStatus::DoNotDisturb;
        let change = session.state_mut().apply_current_user(user);
        cx.emit(StateChanged(vec![change]));
    });
    cx.run_until_parked();
}

/// The app's calls over the sample world with calls, on a clock the test moves.
fn calls_on_a_test_clock(
    cx: &mut TestAppContext,
) -> (Entity<Session>, Entity<Calls>, Rc<Cell<u64>>) {
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| {
        *session.state_mut() = sample_state_with_calls();
        session.pretend_signed_in();
    });
    let now = Rc::new(Cell::new(STARTED_AT));
    let clock = Rc::clone(&now);
    let calls =
        cx.update(|cx| install_calls_with_clock(&session, Rc::new(move || clock.get()), cx));
    (session, calls, now)
}

#[gpui::test]
fn the_ringtone_plays_until_ringing_ends_even_without_an_event(cx: &mut TestAppContext) {
    let (session, calls, now) = calls_on_a_test_clock(cx);
    apply(
        &session,
        GatewayEvent::CallCreate(anas_call(&[ME], &[ANA])),
        cx,
    );
    assert_eq!(
        calls.read_with(cx, |calls, _| calls.incoming_sound()),
        Some(CallSound::Ringtone)
    );
    assert!(calls.read_with(cx, |calls, cx| calls.ringing_call(cx).is_some()));

    let almost = CALL_RING_TIMEOUT_MILLISECONDS - 1;
    now.set(STARTED_AT + almost);
    cx.executor().advance_clock(Duration::from_millis(almost));
    cx.run_until_parked();
    assert_eq!(
        calls.read_with(cx, |calls, _| calls.incoming_sound()),
        Some(CallSound::Ringtone),
        "still within its 30 seconds"
    );

    now.set(STARTED_AT + CALL_RING_TIMEOUT_MILLISECONDS);
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    calls.read_with(cx, |calls, cx| {
        assert_eq!(
            calls.incoming_sound(),
            None,
            "no event came: the deadline stops it"
        );
        assert!(calls.ringing_call(cx).is_none(), "and the card goes");
    });
}

#[gpui::test]
fn the_ringtone_stops_when_the_call_ends_or_is_answered_elsewhere(cx: &mut TestAppContext) {
    let (session, calls, _) = calls_on_a_test_clock(cx);
    apply(
        &session,
        GatewayEvent::CallCreate(anas_call(&[ME], &[ANA])),
        cx,
    );
    // Answered on another device: the user is no longer rung, and joined there.
    apply(
        &session,
        GatewayEvent::CallUpdate(anas_call(&[], &[ANA, ME])),
        cx,
    );
    assert_eq!(calls.read_with(cx, |calls, _| calls.incoming_sound()), None);

    apply(
        &session,
        GatewayEvent::CallUpdate(anas_call(&[ME], &[ANA])),
        cx,
    );
    assert_eq!(
        calls.read_with(cx, |calls, _| calls.incoming_sound()),
        Some(CallSound::Ringtone)
    );
    apply(
        &session,
        GatewayEvent::CallDelete {
            channel_id: CONVERSATION_ANA,
        },
        cx,
    );
    assert_eq!(calls.read_with(cx, |calls, _| calls.incoming_sound()), None);
}

#[gpui::test]
fn do_not_disturb_rings_without_sound_but_still_shows_the_call(cx: &mut TestAppContext) {
    let (session, calls, _) = calls_on_a_test_clock(cx);
    set_do_not_disturb(&session, cx);
    apply(
        &session,
        GatewayEvent::CallCreate(anas_call(&[ME], &[ANA])),
        cx,
    );
    calls.read_with(cx, |calls, cx| {
        assert_eq!(calls.incoming_sound(), None);
        assert!(calls.ringing_call(cx).is_some());
    });
}

#[gpui::test]
fn a_declined_call_stays_silent_when_a_late_update_still_lists_the_user(cx: &mut TestAppContext) {
    let (session, calls, _) = calls_on_a_test_clock(cx);
    apply(
        &session,
        GatewayEvent::CallCreate(anas_call(&[ME], &[ANA])),
        cx,
    );
    calls.update(cx, |calls, cx| calls.decline_call(CONVERSATION_ANA, cx));
    cx.run_until_parked();
    assert_eq!(calls.read_with(cx, |calls, _| calls.incoming_sound()), None);
    // Sent before the instance took the decline in.
    apply(
        &session,
        GatewayEvent::CallUpdate(anas_call(&[ME], &[ANA])),
        cx,
    );
    calls.read_with(cx, |calls, cx| {
        assert_eq!(calls.incoming_sound(), None);
        assert!(calls.ringing_call(cx).is_none());
    });
}

#[gpui::test]
fn ringing_lasts_at_most_thirty_local_seconds_whatever_the_instance_s_clock_says(
    cx: &mut TestAppContext,
) {
    let (session, calls, now) = calls_on_a_test_clock(cx);
    // This device's clock is a minute ahead: by the instance's, the call already stopped ringing.
    now.set(STARTED_AT + 60_000);
    apply(
        &session,
        GatewayEvent::CallCreate(anas_call(&[ME], &[ANA])),
        cx,
    );
    assert_eq!(
        calls.read_with(cx, |calls, _| calls.incoming_sound()),
        Some(CallSound::Ringtone),
        "it rings for its full time from now"
    );
    now.set(STARTED_AT + 60_000 + CALL_RING_TIMEOUT_MILLISECONDS);
    cx.executor()
        .advance_clock(Duration::from_millis(CALL_RING_TIMEOUT_MILLISECONDS));
    cx.run_until_parked();
    assert_eq!(calls.read_with(cx, |calls, _| calls.incoming_sound()), None);
}

#[gpui::test]
fn a_failed_start_says_why_and_whether_calling_again_may_work(cx: &mut TestAppContext) {
    let (_, calls, _) = calls_on_a_test_clock(cx);
    // No instance behind the session: the start fails as a network failure would.
    calls.update(cx, |calls, cx| {
        calls.start_call(CONVERSATION_ANA, cx, |_, _| {});
        assert!(calls.is_starting(CONVERSATION_ANA));
        // A second press while it starts does nothing.
        calls.start_call(CONVERSATION_ANA, cx, |_, _| {});
    });
    cx.run_until_parked();
    let problem = calls.read_with(cx, |calls, _| calls.start_problem());
    assert_eq!(
        problem,
        Some(CallStartProblem {
            channel_id: CONVERSATION_ANA,
            failure: CallStartFailure::Other
        })
    );
    assert!(CallStartFailure::Other.may_work_again());
    let refused = |code| mellow_client_core::ClientError::Api {
        status: 409,
        code,
        message: String::new(),
    };
    assert_eq!(
        CallStartFailure::of(&refused(ApiErrorCode::Conflict)),
        CallStartFailure::AlreadyCalling
    );
    assert_eq!(
        CallStartFailure::of(&refused(ApiErrorCode::RateLimited)),
        CallStartFailure::RateLimited
    );
    assert!(!CallStartFailure::AlreadyCalling.may_work_again());
}
