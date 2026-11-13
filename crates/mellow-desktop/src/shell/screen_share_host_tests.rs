//! The voice panel's share row, the in-app picker and the share's states in the shell, drawn
//! from the engine's synthetic screen: no server, no engine (a call still starting), so only the
//! interface's own rules run.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    AppContext as _, Entity, TestAppContext, VisualTestContext, WindowAppearance, px, size,
};
use mellow_media::{
    ScreenCaptureError, ScreenShareEnd, StreamFormat, SyntheticAudioDevices, SyntheticScreens,
    SyntheticSound, VoiceSessionEvent,
};
use mellow_protocol::VideoCodec;

use crate::shell::DuskShell;
use crate::shell_fixtures::{GUILD_RUST, RUST_VOICE, sample_state};
use crate::voice::screen_share::{ScreenSharePhase, ScreenShareProblem, SharedSound};
use crate::voice::{Voice, install_voice};

/// Pictures and lists come from real threads: generous, never a measure.
const STEP_DEADLINE: Duration = Duration::from_secs(10);

fn wait_until(
    cx: &mut VisualTestContext,
    what: &str,
    mut condition: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + STEP_DEADLINE;
    loop {
        cx.run_until_parked();
        if condition(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "{what} did not happen in time");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A shell in a call (no engine) in Rust's Lounge, sharing from `sources`.
fn shell_in_a_call(
    sources: Arc<dyn mellow_media::ScreenSources>,
    cx: &mut TestAppContext,
) -> (Entity<DuskShell>, Entity<Voice>, &mut VisualTestContext) {
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(crate::strings_pending_i18n::ActiveStrings(
            &crate::strings_pending_i18n::ENGLISH,
        ));
        cx.set_global(crate::theme::ActiveTheme::for_appearance(
            WindowAppearance::Dark,
        ));
        crate::components::bind_tab_navigation(cx);
        crate::shell::bind_shell_keys(cx);
    });
    let session = cx.new(|_| crate::session::Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| *session.state_mut() = sample_state());
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
    let (shell, cx) = cx.add_window_view(|window, cx| DuskShell::new(session, window, cx));
    cx.simulate_resize(size(px(1_400.0), px(800.0)));
    cx.update(|window, _| window.activate_window());
    voice.update(cx, |voice, _| {
        voice.set_screen_sources_for_tests(sources);
        voice.pretend_call_for_tests(GUILD_RUST, RUST_VOICE);
    });
    cx.run_until_parked();
    (shell, voice, cx)
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
fn the_share_button_opens_the_picker_whose_cards_show_the_screens(cx: &mut TestAppContext) {
    let (shell, voice, cx) = shell_in_a_call(Arc::new(SyntheticScreens::default()), cx);
    assert!(cx.debug_bounds("voice-screen-share-button").is_some());
    assert!(cx.debug_bounds("screen-picker").is_none());

    click(cx, "voice-screen-share-button");
    assert!(
        cx.debug_bounds("screen-picker").is_some(),
        "the picker opens"
    );
    wait_until(cx, "the synthetic screen's card", |cx| {
        cx.debug_bounds("screen-source-synthetic-screen").is_some()
    });

    // Esc closes it without sharing; the row stays as it was.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.debug_bounds("screen-picker").is_none());
    assert!(shell.read_with(cx, |shell, _| !shell.screen_share.is_open()));
    assert!(voice.read_with(cx, |voice, _| voice.screen_sources().is_none()));
    assert_eq!(
        voice.read_with(cx, |voice, _| voice.screen_share_phase()),
        ScreenSharePhase::Off
    );

    // The options: the quality chosen is remembered and shown on the button.
    click(cx, "voice-screen-share-options-button");
    assert!(shell.read_with(cx, |shell, _| shell.screen_share.is_open()));
    voice.update(cx, |voice, cx| {
        voice.set_screen_share_quality(mellow_media::ScreenShareQuality::Highest, cx);
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(shell.read_with(cx, |shell, _| !shell.screen_share.is_open()));
    assert_eq!(
        cx.update(|_, cx| Voice::screen_share_quality(cx)),
        mellow_media::ScreenShareQuality::Highest
    );
}

#[gpui::test]
fn a_refused_recording_shows_in_the_picker(cx: &mut TestAppContext) {
    struct RefusingScreens;
    impl mellow_media::ScreenSources for RefusingScreens {
        fn picker(&self) -> mellow_media::ScreenPicker {
            mellow_media::ScreenPicker::InApp
        }
        fn sources(
            &self,
            _with_thumbnails: bool,
        ) -> Result<Vec<mellow_media::ScreenSourceDescription>, ScreenCaptureError> {
            Err(ScreenCaptureError::PermissionDenied)
        }
        fn start(
            &self,
            _choice: &mellow_media::ScreenChoice,
            _sound: &mellow_media::ShareSound,
            _share: mellow_media::ScreenShareContext,
        ) -> Result<Box<dyn mellow_media::RunningScreenCapture>, ScreenCaptureError> {
            Err(ScreenCaptureError::PermissionDenied)
        }
    }
    let (_shell, voice, cx) = shell_in_a_call(Arc::new(RefusingScreens), cx);
    click(cx, "voice-screen-share-button");
    wait_until(cx, "the refusal in the picker", |cx| {
        voice.read_with(cx, |voice, _| {
            matches!(
                voice.screen_sources(),
                Some(crate::voice::screen_share::ScreenSourceListing::Failed(
                    ScreenShareProblem::PermissionDenied
                ))
            )
        })
    });
    assert!(cx.debug_bounds("screen-picker").is_some());
}

#[gpui::test]
fn the_row_waits_for_the_desktop_s_dialog_then_says_what_the_share_sends(cx: &mut TestAppContext) {
    struct PortalScreens;
    impl mellow_media::ScreenSources for PortalScreens {
        fn picker(&self) -> mellow_media::ScreenPicker {
            mellow_media::ScreenPicker::System
        }
        fn sources(
            &self,
            _with_thumbnails: bool,
        ) -> Result<Vec<mellow_media::ScreenSourceDescription>, ScreenCaptureError> {
            Ok(Vec::new())
        }
        fn start(
            &self,
            _choice: &mellow_media::ScreenChoice,
            _sound: &mellow_media::ShareSound,
            _share: mellow_media::ScreenShareContext,
        ) -> Result<Box<dyn mellow_media::RunningScreenCapture>, ScreenCaptureError> {
            Err(ScreenCaptureError::Cancelled)
        }
    }
    let (_shell, voice, cx) = shell_in_a_call(Arc::new(PortalScreens), cx);
    voice.update(cx, |voice, cx| {
        voice.pretend_screen_share_phase_for_tests(
            ScreenSharePhase::Starting,
            SharedSound::Off,
            VideoCodec::Vp8,
            cx,
        );
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("voice-screen-share-waiting").is_some());
    assert!(cx.debug_bounds("voice-screen-share-live").is_none());

    // On, in VP8 behind the portal: VA-API was not found, and the window's sound is unknown.
    voice.update(cx, |voice, cx| {
        voice.pretend_screen_share_phase_for_tests(
            ScreenSharePhase::On,
            SharedSound::NotAvailableOnWayland,
            VideoCodec::Vp8,
            cx,
        );
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("voice-screen-share-waiting").is_none());
    assert!(cx.debug_bounds("voice-screen-share-live").is_some());
    assert!(
        cx.debug_bounds("voice-screen-share-software-encoding")
            .is_some()
    );

    // Encoded on the GPU: nothing to say about the processor.
    voice.update(cx, |voice, cx| {
        voice.pretend_screen_share_phase_for_tests(
            ScreenSharePhase::On,
            SharedSound::FromSource,
            VideoCodec::H264,
            cx,
        );
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("voice-screen-share-software-encoding")
            .is_none()
    );

    // The capture failed: the share is off and says why, until dismissed.
    voice.update(cx, |voice, cx| {
        voice.follow_screen_share_event_for_tests(
            &VoiceSessionEvent::ScreenShareEnded(ScreenShareEnd::Failed("encoder".into())),
            cx,
        );
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("voice-screen-share-live").is_none());
    assert!(cx.debug_bounds("voice-screen-share-problem").is_some());
    assert_eq!(
        voice.read_with(cx, |voice, _| voice.screen_share_problem()),
        Some(ScreenShareProblem::CaptureFailed)
    );
    voice.update(cx, Voice::dismiss_screen_share_problem);
    cx.run_until_parked();
    assert!(cx.debug_bounds("voice-screen-share-problem").is_none());

    // A source closed by the person is no problem, only an announcement.
    voice.update(cx, |voice, cx| {
        voice.pretend_screen_share_phase_for_tests(
            ScreenSharePhase::On,
            SharedSound::FromSource,
            VideoCodec::H264,
            cx,
        );
        voice.follow_screen_share_event_for_tests(
            &VoiceSessionEvent::ScreenShareEnded(ScreenShareEnd::SourceClosed),
            cx,
        );
    });
    voice.read_with(cx, |voice, _| {
        assert_eq!(voice.screen_share_problem(), None);
        assert_eq!(
            voice.screen_share_announcement(),
            "Your screen share ended."
        );
    });
}
