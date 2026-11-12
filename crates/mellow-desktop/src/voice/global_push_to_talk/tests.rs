use global_hotkey::hotkey::Code;
use mellow_client_core::shared_logic::push_to_talk_keys::{
    ModifierKey, code_of_modifier, code_of_toolkit_key,
};

use super::*;

#[test]
fn every_key_the_window_records_becomes_the_same_hot_key() {
    let toolkit_keys = [
        "v", "7", "space", "f13", "f24", "pageup", "`", "-", "enter", "left",
    ];
    for toolkit_key in toolkit_keys {
        let code = code_of_toolkit_key(toolkit_key).unwrap_or_default();
        let hotkey = hotkey_of_code(&code);
        assert!(hotkey.is_some(), "{code}");
        assert!(
            hotkey.is_some_and(|hotkey| hotkey.mods.is_empty()),
            "{code} needs no modifier"
        );
    }
    assert_eq!(
        hotkey_of_code("KeyV").map(|hotkey| hotkey.key),
        Some(Code::KeyV)
    );
}

#[test]
fn a_modifier_alone_stays_in_the_window_and_an_unknown_code_is_unavailable() {
    for modifier in [
        ModifierKey::Control,
        ModifierKey::Shift,
        ModifierKey::Alt,
        ModifierKey::Meta,
    ] {
        let code = code_of_modifier(modifier);
        assert!(hotkey_of_code(code).is_none());
        assert_eq!(
            status_without_hotkey(code),
            GlobalPushToTalkStatus::ModifierKeyWindowOnly
        );
    }
    assert!(hotkey_of_code("NotAKey").is_none());
    // macOS hears media keys only through an event tap, which needs Accessibility.
    assert!(hotkey_of_code("MediaPlayPause").is_none());
    assert_eq!(
        status_without_hotkey("NotAKey"),
        GlobalPushToTalkStatus::KeyUnavailable
    );
}

#[test]
fn bindings_are_never_reused() {
    let first = next_binding();
    let second = next_binding();
    assert_ne!(first, NO_BINDING);
    assert!(second > first);
}

#[test]
fn a_full_queue_never_loses_the_last_release() {
    let (queue, mut receiver) = mpsc::channel(1);
    let reports = ListenerReports::new(queue);
    let binding = next_binding();
    for _ in 0..LISTENER_EVENT_QUEUE_CAPACITY {
        reports.key(binding, true);
        reports.key(binding, false);
    }
    assert_eq!(receiver.try_recv(), Ok(ListenerEvent::KeyChanged));
    assert_eq!(reports.latest_key(), (binding, false));

    // A late report of an earlier binding never hides the current one's state.
    let later = next_binding();
    reports.key(later, true);
    reports.key(binding, false);
    assert_eq!(reports.latest_key(), (later, true));
}

/// Server tests: the server refuses Windows until its master key file can be protected there.
#[cfg(not(windows))]
mod binding {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::TestAppContext;
    use mellow_client_core::shared_logic::voice_settings::{PushToTalkKey, VoiceInputMode};
    use tokio::runtime::Runtime;

    use super::super::super::change_voice_settings;
    use super::super::super::tests::{RunningInstance, owner_with_two_voice_channels, wait_until};
    use super::*;
    use crate::tokio_bridge::build_network_runtime;
    use crate::voice::CallPhase;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ListenerCall {
        Bind { code: String, binding: u64 },
        Unbind,
    }

    /// Stands in for the system's listener, recording what it is asked.
    struct RecordingListener(Rc<RefCell<Vec<ListenerCall>>>);

    impl KeyListener for RecordingListener {
        fn bind(&mut self, code: &str, binding: u64, _: &str) -> GlobalPushToTalkStatus {
            self.0.borrow_mut().push(ListenerCall::Bind {
                code: code.to_owned(),
                binding,
            });
            GlobalPushToTalkStatus::Listening {
                trigger: None,
                is_key_taken_from_other_apps: true,
            }
        }

        fn unbind(&mut self) {
            self.0.borrow_mut().push(ListenerCall::Unbind);
        }
    }

    fn choose_push_to_talk_key(code: &str, cx: &mut TestAppContext) {
        let code = code.to_owned();
        cx.update(|cx| {
            change_voice_settings(cx, move |settings| {
                settings.input_mode = VoiceInputMode::PushToTalk;
                settings.push_to_talk_key = Some(PushToTalkKey {
                    label: code.clone(),
                    code,
                });
            });
        });
    }

    #[gpui::test]
    fn holds_the_key_for_the_call_and_drops_stale_events(cx: &mut TestAppContext) {
        let runtime: Runtime = build_network_runtime().unwrap();
        let instance = runtime.block_on(RunningInstance::start());
        let (session, voice, channel_id, _) =
            owner_with_two_voice_channels(&runtime, &instance, cx);
        choose_push_to_talk_key("Space", cx);
        let calls = Rc::new(RefCell::new(Vec::new()));
        let reports = Rc::new(RefCell::new(None));
        let global_push_to_talk = cx.new(|cx| {
            let (calls, reports) = (Rc::clone(&calls), Rc::clone(&reports));
            GlobalPushToTalk::new(
                voice.clone(),
                move |listener_reports| {
                    *reports.borrow_mut() = Some(listener_reports);
                    RecordingListener(calls)
                },
                cx,
            )
        });
        let reports = reports.borrow_mut().take().unwrap();
        let is_pressed =
            |cx: &mut TestAppContext| voice.read_with(cx, |voice, _| voice.is_push_to_talk_pressed);
        assert!(calls.borrow().is_empty(), "no key held outside a call");

        voice.update(cx, |voice, cx| voice.join(channel_id, cx));
        wait_until(cx, "the call", |cx| {
            voice.read_with(cx, |voice, _| voice.phase() == Some(CallPhase::Connected))
        });
        let ListenerCall::Bind {
            code,
            binding: first,
        } = calls.borrow()[0].clone()
        else {
            panic!("bound on join: {:?}", calls.borrow());
        };
        assert_eq!(code, "Space");
        reports.key(first, true);
        cx.run_until_parked();
        assert!(is_pressed(cx));

        // Another key: the first is let go, and a press it held is released.
        choose_push_to_talk_key("KeyV", cx);
        cx.run_until_parked();
        assert!(!is_pressed(cx));
        let second = match calls.borrow().as_slice() {
            [
                _,
                ListenerCall::Unbind,
                ListenerCall::Bind { code, binding },
            ] if code == "KeyV" => *binding,
            other => panic!("rebound on a key change: {other:?}"),
        };
        // A late press of the first binding does not open the microphone.
        reports.key(first, true);
        cx.run_until_parked();
        assert!(!is_pressed(cx));

        reports.key(second, true);
        cx.run_until_parked();
        assert!(is_pressed(cx));
        voice.update(cx, Voice::leave);
        cx.run_until_parked();
        assert!(!is_pressed(cx));
        assert_eq!(calls.borrow().last(), Some(&ListenerCall::Unbind));

        drop((global_push_to_talk, voice, session));
        cx.run_until_parked();
        runtime.block_on(instance.stop());
    }
}
