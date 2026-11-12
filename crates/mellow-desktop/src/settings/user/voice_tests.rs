//! The Voice page headless, with synthetic devices: the lists, the key recorder and what leaving
//! the page frees. No microphone is opened.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{TestAppContext, VisualTestContext, WindowAppearance};
use mellow_media::{StreamFormat, SyntheticAudioDevices, SyntheticSound};

use super::*;
use crate::session::Session;
use crate::voice::install_voice;

/// Generous: the synthetic microphone runs in real time. Never a measure.
const STEP_DEADLINE: Duration = Duration::from_secs(30);

fn open_page(
    cx: &mut TestAppContext,
) -> (Entity<VoicePage>, Entity<Voice>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(crate::strings_pending_i18n::ActiveStrings(
            &crate::strings_pending_i18n::ENGLISH,
        ));
        cx.set_global(crate::theme::ActiveTheme::for_appearance(
            WindowAppearance::Dark,
        ));
        cx.set_global(PreferencesStore::default());
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    let voice = cx.update(|cx| {
        install_voice(
            &session,
            Arc::new(|| {
                let format = StreamFormat {
                    sample_rate: 48_000,
                    channels: 1,
                };
                Box::new(SyntheticAudioDevices::new(
                    SyntheticSound::Silence,
                    format,
                    format,
                ))
            }),
            cx,
        )
    });
    let (page, cx) = cx.add_window_view(VoicePage::new);
    cx.run_until_parked();
    (page, voice, cx)
}

#[gpui::test]
fn the_page_lists_the_devices_as_it_opens(cx: &mut TestAppContext) {
    let (page, _voice, cx) = open_page(cx);
    page.read_with(cx, |page, _| {
        let inputs = page.inputs.as_deref().unwrap_or_default();
        let outputs = page.outputs.as_deref().unwrap_or_default();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].name, "Synthetic microphone");
        assert_eq!(outputs[0].name, "Synthetic output");
    });
}

#[gpui::test]
fn the_recorder_takes_the_next_key_and_esc_cancels(cx: &mut TestAppContext) {
    let (page, voice, cx) = open_page(cx);
    // The recorder shows with push-to-talk.
    cx.update(|_, cx| {
        change_voice_settings(cx, |settings| {
            settings.input_mode = VoiceInputMode::PushToTalk;
        });
    });
    cx.run_until_parked();
    let key = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| current_settings(cx).push_to_talk_key.map(|key| key.code))
    };
    cx.update(|window, cx| {
        page.update(cx, |page, cx| {
            window.focus(&page.key_recorder_focus, cx);
            page.start_recording_key(cx);
        });
    });
    assert!(voice.read_with(cx, |voice, _| voice.is_recording_push_to_talk_key()));
    cx.simulate_keystrokes("escape");
    assert_eq!(key(cx), None, "Esc records nothing");
    assert!(!voice.read_with(cx, |voice, _| voice.is_recording_push_to_talk_key()));

    page.update(cx, VoicePage::start_recording_key);
    cx.simulate_keystrokes("f13");
    assert_eq!(key(cx).as_deref(), Some("F13"));
    page.read_with(cx, |page, _| {
        assert!(!page.is_recording_key);
        assert_eq!(page.key_announcement.as_ref(), "Push to talk: F13.");
    });
    // Not recording: a key press is no longer taken.
    cx.simulate_keystrokes("f14");
    assert_eq!(key(cx).as_deref(), Some("F13"));

    // A modifier pressed alone is a key too.
    page.update(cx, VoicePage::start_recording_key);
    cx.simulate_modifiers_change(Modifiers::control());
    assert_eq!(key(cx).as_deref(), Some("ControlLeft"));
    assert!(!voice.read_with(cx, |voice, _| voice.is_recording_push_to_talk_key()));
    cx.simulate_modifiers_change(Modifiers::default());
}

#[gpui::test]
fn leaving_the_page_frees_the_test_s_microphone(cx: &mut TestAppContext) {
    let (page, voice, cx) = open_page(cx);
    cx.executor().allow_parking();
    cx.update(|_, cx| voice.update(cx, |voice, cx| voice.set_monitoring(true, cx)));
    assert!(voice.read_with(cx, |voice, _| voice.is_microphone_test_running()));
    let deadline = Instant::now() + STEP_DEADLINE;
    while voice
        .read_with(cx, |voice, _| voice.microphone_decibels())
        .is_none()
    {
        assert!(Instant::now() < deadline, "the test's meter never moved");
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(page);
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
    voice.read_with(cx, |voice, _| {
        assert!(!voice.is_microphone_test_running());
        assert!(!voice.is_monitoring());
    });
}
