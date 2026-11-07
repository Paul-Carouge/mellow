//! Two voice engines join a voice channel of a real in-process `mellow-server` through its
//! gateway and UDP port: one speaks a synthetic tone, the other must decode and play it, and the
//! server must hear the speaker's audio level; then the speaker loses its network path and must
//! reconnect on a new one by itself. No real audio device is touched.

// The server refuses Windows until its master key file can be protected there (ADR-0006).
#![cfg(not(windows))]
// Test helpers outside `#[test]` functions: failing loudly is the point.
#![allow(clippy::unwrap_used, clippy::panic)]

mod support;

use std::time::{Duration, Instant};

use mellow_media::{SyntheticSound, VoicePhase, VoiceSessionEvent};
use mellow_protocol::{GatewayEvent, Snowflake};
use support::{
    HEARING_TIMEOUT, Participant, RunningInstance, identified_gateway, shared_voice_channel,
};

/// ICE must notice the silence, wait out its 3 s grace, then connect on a new path.
const RECONNECTION_TIMEOUT: Duration = Duration::from_secs(60);
/// Half a second of a 44.1 kHz stereo output playing the tone.
const AUDIBLE_SAMPLES_EXPECTED: u64 = 44_100;

/// The speaker and the listener, both joining a new voice channel of `instance`.
async fn speaker_and_listener_in_a_channel(
    instance: &RunningInstance,
) -> (Participant, Participant, Snowflake, Snowflake) {
    let (owner, speaker_id) = instance.registered("speaker").await;
    let (listener, listener_id) = instance.registered("listener").await;
    let channel_id = shared_voice_channel(&owner, &listener).await;
    let speaker_gateway = identified_gateway(instance.listen_address, owner.token().unwrap()).await;
    let listener_gateway =
        identified_gateway(instance.listen_address, listener.token().unwrap()).await;
    let speaker = Participant::join(
        speaker_gateway,
        channel_id,
        speaker_id,
        SyntheticSound::Sine {
            frequency: 440.0,
            amplitude: 0.3,
        },
    );
    let listener = Participant::join(
        listener_gateway,
        channel_id,
        listener_id,
        SyntheticSound::Silence,
    );
    (speaker, listener, speaker_id, listener_id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_speaker_is_heard_decoded_by_another_engine_and_seen_speaking_by_the_server() {
    let instance = RunningInstance::start().await;
    let (mut speaker, mut listener, speaker_id, listener_id) =
        speaker_and_listener_in_a_channel(&instance).await;
    let deadline = Instant::now() + HEARING_TIMEOUT;
    let mut is_speaker_seen_speaking = false;
    while !(is_speaker_seen_speaking
        && listener.probe.audible_samples() >= AUDIBLE_SAMPLES_EXPECTED)
    {
        assert!(
            Instant::now() < deadline,
            "speaking seen: {is_speaker_seen_speaking}, audible samples: {}, listener events: \
             {:?}, speaker events: {:?}",
            listener.probe.audible_samples(),
            listener.seen_session_events,
            speaker.seen_session_events,
        );
        speaker.pump();
        is_speaker_seen_speaking |= listener.pump().iter().any(|event| {
            matches!(
                event,
                GatewayEvent::Speaking { user_id, speaking: true, .. } if *user_id == speaker_id
            )
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    speaker.pump();
    listener.pump();

    for participant in [&speaker, &listener] {
        assert!(
            participant
                .seen_session_events
                .contains(&VoiceSessionEvent::Connected),
            "{:?}",
            participant.seen_session_events
        );
    }
    assert!(
        speaker
            .seen_session_events
            .contains(&VoiceSessionEvent::SelfSpeaking(true)),
        "the speaker sees their own ring"
    );
    assert!(
        !listener
            .seen_session_events
            .contains(&VoiceSessionEvent::SelfSpeaking(true)),
        "a silent microphone never speaks"
    );
    assert_eq!(
        speaker.probe.audible_samples(),
        0,
        "the listener's silence sends nothing, and nobody hears themselves"
    );
    assert!(speaker.probe.played_samples() > 0, "the output still runs");
    assert_ne!(speaker_id, listener_id);

    speaker.session.leave();
    listener.session.leave();
    instance.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_network_path_is_noticed_and_voice_comes_back_on_a_new_one() {
    let instance = RunningInstance::start().await;
    let (mut speaker, mut listener, _, _) = speaker_and_listener_in_a_channel(&instance).await;

    let deadline = Instant::now() + HEARING_TIMEOUT;
    while listener.probe.audible_samples() < AUDIBLE_SAMPLES_EXPECTED {
        assert!(
            Instant::now() < deadline,
            "never heard: {:?}",
            speaker.seen_session_events
        );
        speaker.pump();
        listener.pump();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    speaker.session.drop_network_path_for_tests();
    let deadline = Instant::now() + RECONNECTION_TIMEOUT;
    let is_reconnected = |events: &[VoiceSessionEvent]| {
        events
            .iter()
            .skip_while(|event| {
                **event != VoiceSessionEvent::PhaseChanged(VoicePhase::Reconnecting)
            })
            .any(|event| *event == VoiceSessionEvent::PhaseChanged(VoicePhase::Connected))
    };
    while !is_reconnected(&speaker.seen_session_events) {
        assert!(
            Instant::now() < deadline,
            "no reconnection: {:?}",
            speaker.seen_session_events
        );
        speaker.pump();
        listener.pump();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Voice flows again over the new path.
    let heard_before = listener.probe.audible_samples();
    let deadline = Instant::now() + HEARING_TIMEOUT;
    while listener.probe.audible_samples() < heard_before + AUDIBLE_SAMPLES_EXPECTED {
        assert!(Instant::now() < deadline, "not heard after reconnecting");
        speaker.pump();
        listener.pump();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    speaker.pump();
    assert_eq!(
        speaker
            .seen_session_events
            .iter()
            .filter(|event| **event == VoiceSessionEvent::Connected)
            .count(),
        1,
        "the join cue plays once: {:?}",
        speaker.seen_session_events
    );
    assert!(
        !speaker
            .seen_session_events
            .iter()
            .any(|event| matches!(event, VoiceSessionEvent::ConnectionFailed(_))),
        "{:?}",
        speaker.seen_session_events
    );

    speaker.session.leave();
    listener.session.leave();
    instance.stop().await;
}
