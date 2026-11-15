//! The voice in conversations' calls (ADR-0062), without a server or an engine (a call still
//! starting): joining and moving, the default mute of blocked people, and who joins and leaves.

use std::sync::Arc;

use gpui::{Entity, TestAppContext, WindowAppearance};
use mellow_media::{StreamFormat, SyntheticAudioDevices, SyntheticSound};
use mellow_protocol::{GatewayEvent, Relationship, RelationshipKind};

use super::*;
use crate::calls::tests::{STARTED_AT, anas_call};
use crate::preferences::PreferencesStore;
use crate::session::{Session, StateChanged};
use crate::shell_fixtures::{
    ANA, CONVERSATION_ANA, GUILD_RUST, ME, RUST_VOICE, person, sample_state_with_calls,
};

fn apply(session: &Entity<Session>, event: GatewayEvent, cx: &mut TestAppContext) {
    session.update(cx, |session, cx| {
        let change = session.state_mut().apply_event(event, STARTED_AT);
        cx.emit(StateChanged(vec![change]));
    });
    cx.run_until_parked();
}

fn block_ana(session: &Entity<Session>, cx: &mut TestAppContext) {
    apply(
        session,
        GatewayEvent::RelationshipAdd(Relationship {
            user: person(ANA, "ana", "Ana"),
            kind: RelationshipKind::Blocked,
            since_unix_milliseconds: STARTED_AT,
        }),
        cx,
    );
}

/// The app's voice in Rust's Lounge (no engine), Ana's call going on in her conversation.
fn voice_in_the_lounge(cx: &mut TestAppContext) -> (Entity<Session>, Entity<Voice>) {
    cx.update(|cx| {
        cx.set_global(crate::strings_pending_i18n::ActiveStrings(
            &crate::strings_pending_i18n::ENGLISH,
        ));
        cx.set_global(crate::theme::ActiveTheme::for_appearance(
            WindowAppearance::Dark,
        ));
        cx.set_global(PreferencesStore::default());
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| {
        *session.state_mut() = sample_state_with_calls();
        session.pretend_signed_in();
    });
    let format = StreamFormat {
        sample_rate: 48_000,
        channels: 1,
    };
    let voice = cx.update(|cx| {
        install_voice(
            &session,
            Arc::new(move || {
                Box::new(SyntheticAudioDevices::new(
                    SyntheticSound::Silence,
                    format,
                    format,
                ))
            }),
            cx,
        )
    });
    apply(
        &session,
        GatewayEvent::CallCreate(anas_call(&[], &[ANA])),
        cx,
    );
    voice.update(cx, |voice, _| {
        voice.pretend_call_for_tests(GUILD_RUST, RUST_VOICE);
    });
    (session, voice)
}

fn is_muted_for_me(voice: &Entity<Voice>, cx: &mut TestAppContext) -> (bool, bool) {
    voice.read_with(cx, |voice, cx| {
        (
            voice.effective_person_setting(ANA, cx).is_locally_muted,
            voice
                .engine_person_setting_for_tests(ANA)
                .is_some_and(|setting| setting.is_locally_muted),
        )
    })
}

fn stored_people(cx: &mut TestAppContext) -> Vec<(Snowflake, PersonVoiceSetting)> {
    cx.update(|cx| {
        preferences(cx)
            .map(|preferences| preferences.voice_people.clone())
            .unwrap_or_default()
    })
}

#[gpui::test]
fn a_blocked_person_is_off_in_a_call_and_heard_again_back_in_a_channel(cx: &mut TestAppContext) {
    let (session, voice) = voice_in_the_lounge(cx);
    block_ana(&session, cx);
    assert_eq!(
        is_muted_for_me(&voice, cx),
        (false, false),
        "not in a channel"
    );

    voice.update(cx, |voice, cx| voice.join_call(CONVERSATION_ANA, cx));
    assert_eq!(
        voice.read_with(cx, |voice, _| voice.call_conversation_id()),
        Some(CONVERSATION_ANA)
    );
    assert_eq!(
        is_muted_for_me(&voice, cx),
        (true, true),
        "moving into the call mutes her"
    );

    voice.update(cx, |voice, cx| voice.join(RUST_VOICE, cx));
    assert_eq!(
        is_muted_for_me(&voice, cx),
        (false, false),
        "moving back to the channel lifts it"
    );
    assert_eq!(stored_people(cx), Vec::new(), "never stored");
}

#[gpui::test]
fn a_block_placed_or_lifted_during_the_call_follows_at_once(cx: &mut TestAppContext) {
    let (session, voice) = voice_in_the_lounge(cx);
    voice.update(cx, |voice, cx| voice.join_call(CONVERSATION_ANA, cx));
    assert_eq!(is_muted_for_me(&voice, cx), (false, false));
    block_ana(&session, cx);
    assert_eq!(is_muted_for_me(&voice, cx), (true, true));
    apply(
        &session,
        GatewayEvent::RelationshipRemove { user_id: ANA },
        cx,
    );
    assert_eq!(is_muted_for_me(&voice, cx), (false, false));
}

#[gpui::test]
fn turning_a_blocked_person_up_lasts_for_the_call_and_is_never_stored(cx: &mut TestAppContext) {
    let (session, voice) = voice_in_the_lounge(cx);
    block_ana(&session, cx);
    voice.update(cx, |voice, cx| voice.join_call(CONVERSATION_ANA, cx));

    voice.update(cx, |voice, cx| voice.toggle_person_muted(ANA, cx));
    assert_eq!(is_muted_for_me(&voice, cx), (false, false), "turned up");
    assert_eq!(stored_people(cx), Vec::new());
    voice.update(cx, |voice, cx| voice.toggle_person_muted(ANA, cx));
    assert_eq!(is_muted_for_me(&voice, cx), (true, true), "off again");
    assert_eq!(stored_people(cx), Vec::new());

    // A volume is the reader's choice, kept, without the call's mute.
    voice.update(cx, |voice, cx| voice.set_person_volume(ANA, 50, cx));
    let stored = stored_people(cx);
    assert_eq!(stored.len(), 1);
    assert!(!stored[0].1.is_locally_muted);
    assert_eq!(stored[0].1.volume_percent, 50);
    assert_eq!(
        is_muted_for_me(&voice, cx),
        (true, true),
        "still off in the call"
    );

    // Turned up, then a new call: the default mute comes back.
    voice.update(cx, |voice, cx| voice.toggle_person_muted(ANA, cx));
    assert_eq!(is_muted_for_me(&voice, cx), (false, false));
    voice.update(cx, |voice, cx| {
        voice.join(RUST_VOICE, cx);
        voice.join_call(CONVERSATION_ANA, cx);
    });
    assert_eq!(is_muted_for_me(&voice, cx), (true, true));
}

#[gpui::test]
fn others_joining_and_leaving_the_call_are_read_out(cx: &mut TestAppContext) {
    let (session, voice) = voice_in_the_lounge(cx);
    voice.update(cx, |voice, cx| {
        voice.join_call(CONVERSATION_ANA, cx);
        voice.pretend_connected_for_tests();
    });
    apply(
        &session,
        GatewayEvent::CallUpdate(anas_call(&[], &[ANA, ME])),
        cx,
    );
    apply(
        &session,
        GatewayEvent::CallUpdate(anas_call(&[], &[ME])),
        cx,
    );
    assert_eq!(
        voice
            .read_with(cx, |voice, _| voice.announcement())
            .as_ref(),
        "Ana left the call."
    );
    apply(
        &session,
        GatewayEvent::CallUpdate(anas_call(&[], &[ME, ANA])),
        cx,
    );
    assert_eq!(
        voice
            .read_with(cx, |voice, _| voice.announcement())
            .as_ref(),
        "Ana joined the call."
    );
}

#[test]
fn the_own_entry_dropped_from_the_call_is_told_to_the_engine_once() {
    assert!(is_own_call_entry_dropped(&[ANA, ME], &[ANA], ME));
    assert!(
        !is_own_call_entry_dropped(&[ANA], &[ANA], ME),
        "never in it"
    );
    assert!(
        !is_own_call_entry_dropped(&[ANA, ME], &[ME], ME),
        "still in it"
    );
}

#[gpui::test]
fn a_call_gone_from_the_state_is_not_joined(cx: &mut TestAppContext) {
    let (session, voice) = voice_in_the_lounge(cx);
    let calls = cx.update(|cx| crate::calls::install_calls(&session, cx));
    apply(
        &session,
        GatewayEvent::CallDelete {
            channel_id: CONVERSATION_ANA,
        },
        cx,
    );
    calls.update(cx, |calls, cx| calls.join_call(CONVERSATION_ANA, cx));
    assert_eq!(
        voice.read_with(cx, |voice, _| voice.channel_id()),
        Some(RUST_VOICE),
        "the voice stays where it was"
    );
}
