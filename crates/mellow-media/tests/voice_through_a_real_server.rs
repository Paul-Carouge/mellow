//! Two voice engines join a voice channel of a real in-process `mellow-server` through its
//! gateway and UDP port: one speaks a synthetic tone, the other must decode and play it, and the
//! server must hear the speaker's audio level. No real audio device is touched.

// The server refuses Windows until its master key file can be protected there (ADR-0006).
#![cfg(not(windows))]
// Test helpers outside `#[test]` functions: failing loudly is the point.
#![allow(clippy::unwrap_used, clippy::panic)]

mod support;

use std::time::{Duration, Instant};

use mellow_media::{SyntheticSound, VoiceSessionEvent};
use mellow_protocol::{ChannelKind, CreateChannelRequest, CreateInviteRequest, GatewayEvent};
use support::{HEARING_TIMEOUT, Participant, RunningInstance, identified_gateway};

/// Half a second of a 44.1 kHz stereo output playing the tone.
const AUDIBLE_SAMPLES_EXPECTED: u64 = 44_100;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_speaker_is_heard_decoded_by_another_engine_and_seen_speaking_by_the_server() {
    let instance = RunningInstance::start().await;
    let (owner, speaker_id) = instance.registered("speaker").await;
    let (listener, listener_id) = instance.registered("listener").await;
    let guild = owner.create_guild("Club").await.unwrap();
    let lounge = owner
        .create_channel(
            guild.id,
            &CreateChannelRequest {
                kind: ChannelKind::Voice,
                name: "Lounge".into(),
                topic: String::new(),
                parent_id: None,
                user_limit: None,
            },
        )
        .await
        .unwrap();
    let invite = owner
        .create_invite(guild.id, &CreateInviteRequest::default())
        .await
        .unwrap();
    listener.join_with_invite(&invite.code).await.unwrap();

    let speaker_gateway = identified_gateway(instance.listen_address, owner.token().unwrap()).await;
    let listener_gateway =
        identified_gateway(instance.listen_address, listener.token().unwrap()).await;
    let mut speaker = Participant::join(
        speaker_gateway,
        lounge.id,
        SyntheticSound::Sine {
            frequency: 440.0,
            amplitude: 0.3,
        },
    );
    let mut listener = Participant::join(listener_gateway, lounge.id, SyntheticSound::Silence);

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
