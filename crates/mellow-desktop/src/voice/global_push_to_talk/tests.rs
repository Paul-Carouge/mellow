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
