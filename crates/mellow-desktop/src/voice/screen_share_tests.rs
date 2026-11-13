//! Your screen share against a real in-process `mellow-server`, with the engine's synthetic
//! screen and sources that refuse or are cancelled: the picker's list, the share turning on for
//! the instance, its quality, stopping it, and the problems.

// The server refuses Windows until its master key file can be protected there (ADR-0006).
#![cfg(not(windows))]

use std::sync::Arc;

use gpui::TestAppContext;
use mellow_media::{
    RunningScreenCapture, ScreenCaptureError, ScreenChoice, ScreenPicker, ScreenShareContext,
    ScreenSourceDescription, ScreenSources, ShareSound, SyntheticScreens,
};
use tokio::runtime::Runtime;

use super::screen_share::{ScreenSharePhase, ScreenShareProblem, ScreenSourceListing, SharedSound};
use super::tests::{RunningInstance, owner_with_two_voice_channels, wait_until};
use super::*;
use crate::tokio_bridge::build_network_runtime;

/// A system picker (as Wayland's portal) that answers `error`.
struct FailingScreens(ScreenCaptureError);

impl ScreenSources for FailingScreens {
    fn picker(&self) -> ScreenPicker {
        ScreenPicker::System
    }

    fn sources(
        &self,
        _with_thumbnails: bool,
    ) -> Result<Vec<ScreenSourceDescription>, ScreenCaptureError> {
        Ok(Vec::new())
    }

    fn start(
        &self,
        _choice: &ScreenChoice,
        _sound: &ShareSound,
        _share: ScreenShareContext,
    ) -> Result<Box<dyn RunningScreenCapture>, ScreenCaptureError> {
        Err(self.0.clone())
    }
}

fn is_streaming(session: &Entity<Session>, cx: &mut TestAppContext) -> bool {
    session.read_with(cx, |session, _| {
        let state = session.state();
        state
            .current_user()
            .and_then(|user| state.voice_state_of(user.id))
            .is_some_and(|voice_state| voice_state.self_stream)
    })
}

#[gpui::test]
fn a_screen_picked_in_the_app_is_shared_changed_and_stopped(cx: &mut TestAppContext) {
    let runtime: Runtime = build_network_runtime().unwrap();
    let instance = runtime.block_on(RunningInstance::start());
    let (session, voice, channel_id, _) = owner_with_two_voice_channels(&runtime, &instance, cx);
    voice.update(cx, |voice, _| {
        voice.set_screen_sources_for_tests(Arc::new(SyntheticScreens {
            width: 640,
            height: 360,
        }));
    });
    assert!(!voice.read_with(cx, |voice, _| voice.can_share_screen()));
    voice.update(cx, |voice, cx| voice.join(channel_id, cx));
    wait_until(cx, "the call", |cx| {
        voice.read_with(cx, |voice, _| voice.phase() == Some(CallPhase::Connected))
    });
    assert!(voice.read_with(cx, |voice, _| voice.can_share_screen()));

    // The picker lists the synthetic screen, with its thumbnail.
    voice.update(cx, Voice::start_screen_share);
    wait_until(cx, "the picker's list", |cx| {
        voice.read_with(cx, |voice, _| {
            matches!(voice.screen_sources(), Some(ScreenSourceListing::Listed(_)))
        })
    });
    let source_id = voice.read_with(cx, |voice, _| {
        let Some(ScreenSourceListing::Listed(cards)) = voice.screen_sources() else {
            unreachable!("listed above");
        };
        assert_eq!(cards.len(), 1);
        assert!(cards[0].thumbnail.is_some());
        cards[0].id.clone()
    });
    // Closing the picker shares nothing.
    voice.update(cx, Voice::close_screen_picker);
    assert!(voice.read_with(cx, |voice, _| voice.screen_sources().is_none()));
    assert_eq!(
        voice.read_with(cx, |voice, _| voice.screen_share_phase()),
        ScreenSharePhase::Off
    );

    voice.update(cx, |voice, cx| voice.share_screen_source(source_id, cx));
    assert_eq!(
        voice.read_with(cx, |voice, _| voice.screen_share_phase()),
        ScreenSharePhase::Starting
    );
    wait_until(cx, "the share, seen by the instance", |cx| {
        voice.read_with(cx, |voice, _| voice.is_sharing_screen()) && is_streaming(&session, cx)
    });
    voice.read_with(cx, |voice, _| {
        assert_eq!(voice.shared_sound(), &SharedSound::FromSource);
        assert_eq!(
            voice.screen_share_announcement(),
            "You're sharing your screen"
        );
    });

    // The quality is remembered for the next share and reaches the running one.
    voice.update(cx, |voice, cx| {
        voice.set_screen_share_quality(mellow_media::ScreenShareQuality::Standard, cx);
    });
    assert_eq!(
        cx.update(|cx| Voice::screen_share_quality(cx)),
        mellow_media::ScreenShareQuality::Standard
    );
    assert_eq!(
        voice.read_with(cx, |voice, _| voice
            .engine()
            .and_then(VoiceSession::video_source)),
        Some(mellow_media::VideoSourceKind::Screen(
            mellow_media::ScreenShareQuality::Standard
        ))
    );

    voice.update(cx, Voice::stop_screen_share);
    assert_eq!(
        voice.read_with(cx, |voice, _| voice.screen_share_phase()),
        ScreenSharePhase::Off
    );
    wait_until(cx, "the share's end, seen by the instance", |cx| {
        !is_streaming(&session, cx)
    });

    voice.update(cx, Voice::leave);
    drop((voice, session));
    cx.run_until_parked();
    runtime.block_on(instance.stop());
}

#[gpui::test]
fn a_cancelled_dialog_is_no_problem_and_a_refused_permission_is_one(cx: &mut TestAppContext) {
    let runtime: Runtime = build_network_runtime().unwrap();
    let instance = runtime.block_on(RunningInstance::start());
    let (session, voice, channel_id, _) = owner_with_two_voice_channels(&runtime, &instance, cx);
    voice.update(cx, |voice, cx| voice.join(channel_id, cx));
    wait_until(cx, "the call", |cx| {
        voice.read_with(cx, |voice, _| voice.phase() == Some(CallPhase::Connected))
    });

    // The system's dialog (Wayland) opens at once: no in-app list.
    voice.update(cx, |voice, cx| {
        voice.set_screen_sources_for_tests(Arc::new(FailingScreens(ScreenCaptureError::Cancelled)));
        voice.start_screen_share(cx);
    });
    assert!(voice.read_with(cx, |voice, _| voice.screen_sources().is_none()));
    wait_until(cx, "the cancelled dialog", |cx| {
        voice.read_with(cx, |voice, _| {
            voice.screen_share_phase() == ScreenSharePhase::Off
        })
    });
    voice.read_with(cx, |voice, _| {
        assert_eq!(voice.screen_share_problem(), None);
        assert_eq!(
            voice.screen_share_announcement(),
            "Screen sharing cancelled."
        );
    });

    voice.update(cx, |voice, cx| {
        voice.set_screen_sources_for_tests(Arc::new(FailingScreens(
            ScreenCaptureError::PermissionDenied,
        )));
        voice.start_screen_share(cx);
    });
    wait_until(cx, "the refusal", |cx| {
        voice.read_with(cx, |voice, _| voice.screen_share_problem().is_some())
    });
    assert_eq!(
        voice.read_with(cx, |voice, _| voice.screen_share_problem()),
        Some(ScreenShareProblem::PermissionDenied)
    );
    assert!(!is_streaming(&session, cx));
    // Leaving forgets it.
    voice.update(cx, Voice::leave);
    assert_eq!(
        voice.read_with(cx, |voice, _| voice.screen_share_problem()),
        None
    );
    drop((voice, session));
    cx.run_until_parked();
    runtime.block_on(instance.stop());
}
