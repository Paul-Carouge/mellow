//! Port of `web/src/components/push-to-talk-keys.ts`: when a key opens or closes push-to-talk,
//! and the key format both listeners share. The key is stored as a W3C `KeyboardEvent.code`
//! (`KeyV`, `Space`, `F13`, `ControlRight`): the web records it, the desktop's in-window handler
//! reads it through [`code_of_toolkit_key`], and a global-shortcut listener maps it to its own
//! key type (`global-hotkey` parses these names; the XDG portal takes an accelerator built from
//! them).

use super::voice_settings::{VoiceInputMode, VoiceSettings};

/// What a key press or release does to push-to-talk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushToTalkKeyEffect {
    /// Open the microphone; `should_swallow` when the key types a character, so it does not
    /// also type into the composer.
    Press {
        should_swallow: bool,
    },
    Release,
    Nothing,
}

/// A modifier key, which the desktop toolkit reports as a state change, not a key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierKey {
    Control,
    Shift,
    Alt,
    Meta,
}

/// The modifier a code names, either side.
#[must_use]
pub fn modifier_of_code(code: &str) -> Option<ModifierKey> {
    match code {
        "ControlLeft" | "ControlRight" => Some(ModifierKey::Control),
        "ShiftLeft" | "ShiftRight" => Some(ModifierKey::Shift),
        "AltLeft" | "AltRight" => Some(ModifierKey::Alt),
        "MetaLeft" | "MetaRight" => Some(ModifierKey::Meta),
        _ => None,
    }
}

/// The code a modifier records as.
// shortcut: the toolkit does not tell a left modifier from a right one, so the left one is
// recorded and either side matches; record sides once it reports them.
#[must_use]
pub fn code_of_modifier(modifier: ModifierKey) -> &'static str {
    match modifier {
        ModifierKey::Control => "ControlLeft",
        ModifierKey::Shift => "ShiftLeft",
        ModifierKey::Alt => "AltLeft",
        ModifierKey::Meta => "MetaLeft",
    }
}

/// The W3C code of a key as GPUI names it (`a`, `5`, `space`, `f13`, `pageup`, `-`).
// shortcut: GPUI names the key of the active layout, not the physical key, so on AZERTY the key
// typing "a" records as `KeyA` (QWERTY's A position for a global listener); use scan codes once
// the toolkit reports them.
#[must_use]
pub fn code_of_toolkit_key(key: &str) -> Option<String> {
    let mut characters = key.chars();
    if let (Some(character), None) = (characters.next(), characters.next()) {
        if character.is_ascii_alphabetic() {
            return Some(format!("Key{}", character.to_ascii_uppercase()));
        }
        if character.is_ascii_digit() {
            return Some(format!("Digit{character}"));
        }
        let punctuation = match character {
            '-' => "Minus",
            '=' => "Equal",
            '[' => "BracketLeft",
            ']' => "BracketRight",
            '\\' => "Backslash",
            ';' => "Semicolon",
            '\'' => "Quote",
            ',' => "Comma",
            '.' => "Period",
            '/' => "Slash",
            '`' => "Backquote",
            _ => return None,
        };
        return Some(punctuation.to_owned());
    }
    if let Some(number) = key.strip_prefix('f')
        && let Ok(number) = number.parse::<u8>()
        && (1..=24).contains(&number)
    {
        return Some(format!("F{number}"));
    }
    let named = match key {
        "space" => "Space",
        "enter" => "Enter",
        "backspace" => "Backspace",
        "tab" => "Tab",
        "escape" => "Escape",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "insert" => "Insert",
        "delete" => "Delete",
        _ => return None,
    };
    Some(named.to_owned())
}

/// Whether `code` is the push-to-talk key now: push-to-talk chosen, a key recorded, in a call.
#[must_use]
pub fn is_push_to_talk_code(settings: &VoiceSettings, is_in_call: bool, code: &str) -> bool {
    settings.input_mode == VoiceInputMode::PushToTalk
        && is_in_call
        && settings
            .push_to_talk_key
            .as_ref()
            .is_some_and(|key| key.code == code || both_name(&key.code, code))
}

fn both_name(stored: &str, pressed: &str) -> bool {
    modifier_of_code(stored).is_some() && modifier_of_code(stored) == modifier_of_code(pressed)
}

/// The key a global-shortcut listener holds now: push-to-talk chosen, a key recorded, in a voice
/// channel (connecting included, so the key works from the first second).
#[must_use]
pub fn global_push_to_talk_code(
    settings: &VoiceSettings,
    is_in_voice_channel: bool,
) -> Option<&str> {
    settings
        .push_to_talk_key
        .as_ref()
        .filter(|_| settings.input_mode == VoiceInputMode::PushToTalk && is_in_voice_channel)
        .map(|key| key.code.as_str())
}

/// The trigger offered to the XDG `GlobalShortcuts` portal for `code`: an xkbcommon keysym name, as
/// the XDG "shortcuts" specification wants. `None` when no keysym fits (a modifier alone, a media
/// key): the compositor then lets the person pick one.
#[must_use]
pub fn portal_trigger_of_code(code: &str) -> Option<String> {
    if let Some(letter) = code.strip_prefix("Key")
        && letter.len() == 1
        && letter
            .chars()
            .all(|character| character.is_ascii_uppercase())
    {
        return Some(letter.to_ascii_lowercase());
    }
    if let Some(digit) = code.strip_prefix("Digit")
        && digit.len() == 1
        && digit.chars().all(|character| character.is_ascii_digit())
    {
        return Some(digit.to_owned());
    }
    if let Some(digit) = code.strip_prefix("Numpad")
        && digit.len() == 1
        && digit.chars().all(|character| character.is_ascii_digit())
    {
        return Some(format!("KP_{digit}"));
    }
    if let Some(number) = code.strip_prefix('F')
        && let Ok(number) = number.parse::<u8>()
        && (1..=24).contains(&number)
    {
        return Some(format!("F{number}"));
    }
    let keysym = match code {
        "Space" => "space",
        "Enter" => "Return",
        "Backspace" => "BackSpace",
        "Tab" => "Tab",
        "Escape" => "Escape",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        "Home" => "Home",
        "End" => "End",
        "PageUp" => "Page_Up",
        "PageDown" => "Page_Down",
        "Insert" => "Insert",
        "Delete" => "Delete",
        "Minus" => "minus",
        "Equal" => "equal",
        "BracketLeft" => "bracketleft",
        "BracketRight" => "bracketright",
        "Backslash" => "backslash",
        "Semicolon" => "semicolon",
        "Quote" => "apostrophe",
        "Comma" => "comma",
        "Period" => "period",
        "Slash" => "slash",
        "Backquote" => "grave",
        "CapsLock" => "Caps_Lock",
        "ScrollLock" => "Scroll_Lock",
        "Pause" => "Pause",
        "PrintScreen" => "Print",
        "ContextMenu" => "Menu",
        _ => return None,
    };
    Some(keysym.to_owned())
}

/// Whether a global grab of `code` takes it from other apps for the call: every key but the ones
/// no app uses (F13 to F24, Pause, Scroll Lock). Enter, the arrows or a letter stop working there.
#[must_use]
pub fn does_grab_take_key_from_other_apps(code: &str) -> bool {
    let is_unused_function_key = code
        .strip_prefix('F')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (13..=24).contains(&number));
    !(is_unused_function_key || matches!(code, "Pause" | "ScrollLock"))
}

/// The character the key of `code` types without a modifier, as [`code_of_toolkit_key`] read it
/// from the active layout; `None` for a key that types none (F keys, arrows, Space).
#[must_use]
pub fn character_of_code(code: &str) -> Option<char> {
    if let Some(rest) = code
        .strip_prefix("Key")
        .or_else(|| code.strip_prefix("Digit"))
    {
        let mut characters = rest.chars();
        return match (characters.next(), characters.next()) {
            (Some(character), None) if character.is_ascii_alphanumeric() => {
                Some(character.to_ascii_lowercase())
            }
            _ => None,
        };
    }
    match code {
        "Minus" => Some('-'),
        "Equal" => Some('='),
        "BracketLeft" => Some('['),
        "BracketRight" => Some(']'),
        "Backslash" => Some('\\'),
        "Semicolon" => Some(';'),
        "Quote" => Some('\''),
        "Comma" => Some(','),
        "Period" => Some('.'),
        "Slash" => Some('/'),
        "Backquote" => Some('`'),
        _ => None,
    }
}

/// A key went down. Keys pressed while recording a new one belong to the recorder.
#[must_use]
pub fn push_to_talk_key_down(
    settings: &VoiceSettings,
    is_in_call: bool,
    code: &str,
    is_character_key: bool,
    is_in_key_recorder: bool,
) -> PushToTalkKeyEffect {
    if is_in_key_recorder || !is_push_to_talk_code(settings, is_in_call, code) {
        return PushToTalkKeyEffect::Nothing;
    }
    PushToTalkKeyEffect::Press {
        should_swallow: is_character_key,
    }
}

/// A key came up, wherever focus went meanwhile (into the recorder too): a lost release would
/// leave the microphone open.
#[must_use]
pub fn push_to_talk_key_up(
    settings: &VoiceSettings,
    is_in_call: bool,
    code: &str,
) -> PushToTalkKeyEffect {
    if is_push_to_talk_code(settings, is_in_call, code) {
        PushToTalkKeyEffect::Release
    } else {
        PushToTalkKeyEffect::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_logic::voice_settings::PushToTalkKey;

    fn settings_with(code: &str) -> VoiceSettings {
        VoiceSettings {
            input_mode: VoiceInputMode::PushToTalk,
            push_to_talk_key: Some(PushToTalkKey {
                code: code.into(),
                label: code.into(),
            }),
            ..VoiceSettings::DEFAULT
        }
    }

    #[test]
    fn opens_on_the_key_and_closes_on_its_release_without_typing_its_letter() {
        let settings = settings_with("KeyV");
        assert_eq!(
            push_to_talk_key_down(&settings, true, "KeyV", true, false),
            PushToTalkKeyEffect::Press {
                should_swallow: true
            }
        );
        assert_eq!(
            push_to_talk_key_up(&settings, true, "KeyV"),
            PushToTalkKeyEffect::Release
        );
    }

    #[test]
    fn releases_the_key_even_when_it_comes_up_inside_the_key_recorder() {
        let settings = settings_with("KeyV");
        assert_eq!(
            push_to_talk_key_down(&settings, true, "KeyV", true, true),
            PushToTalkKeyEffect::Nothing
        );
        assert_eq!(
            push_to_talk_key_up(&settings, true, "KeyV"),
            PushToTalkKeyEffect::Release
        );
    }

    #[test]
    fn leaves_a_modifier_keys_default_alone_and_matches_either_side() {
        let settings = settings_with("ControlRight");
        assert_eq!(
            push_to_talk_key_down(&settings, true, "ControlLeft", false, false),
            PushToTalkKeyEffect::Press {
                should_swallow: false
            }
        );
    }

    #[test]
    fn ignores_other_keys_voice_activity_no_key_and_being_out_of_a_call() {
        let settings = settings_with("KeyV");
        assert_eq!(
            push_to_talk_key_down(&settings, true, "KeyB", true, false),
            PushToTalkKeyEffect::Nothing
        );
        let voice_activity = VoiceSettings {
            input_mode: VoiceInputMode::VoiceActivity,
            ..settings.clone()
        };
        assert_eq!(
            push_to_talk_key_down(&voice_activity, true, "KeyV", true, false),
            PushToTalkKeyEffect::Nothing
        );
        let no_key = VoiceSettings {
            push_to_talk_key: None,
            ..settings.clone()
        };
        assert_eq!(
            push_to_talk_key_down(&no_key, true, "KeyV", true, false),
            PushToTalkKeyEffect::Nothing
        );
        assert_eq!(
            push_to_talk_key_down(&settings, false, "KeyV", true, false),
            PushToTalkKeyEffect::Nothing
        );
    }

    #[test]
    fn a_global_listener_holds_the_key_only_in_a_voice_channel_with_push_to_talk_on() {
        let settings = settings_with("KeyV");
        assert_eq!(global_push_to_talk_code(&settings, true), Some("KeyV"));
        assert_eq!(global_push_to_talk_code(&settings, false), None);
        let voice_activity = VoiceSettings {
            input_mode: VoiceInputMode::VoiceActivity,
            ..settings.clone()
        };
        assert_eq!(global_push_to_talk_code(&voice_activity, true), None);
        let no_key = VoiceSettings {
            push_to_talk_key: None,
            ..settings
        };
        assert_eq!(global_push_to_talk_code(&no_key, true), None);
    }

    #[test]
    fn offers_the_portal_xkb_keysym_names() {
        for (code, trigger) in [
            ("KeyV", "v"),
            ("Digit7", "7"),
            ("Numpad3", "KP_3"),
            ("F13", "F13"),
            ("Space", "space"),
            ("Enter", "Return"),
            ("PageDown", "Page_Down"),
            ("Quote", "apostrophe"),
            ("Backquote", "grave"),
            ("CapsLock", "Caps_Lock"),
        ] {
            assert_eq!(portal_trigger_of_code(code).as_deref(), Some(trigger));
        }
        for code in ["ControlLeft", "MediaPlayPause", "F25", "Keyv", "KeyAB", ""] {
            assert_eq!(portal_trigger_of_code(code), None, "{code}");
        }
    }

    #[test]
    fn warns_that_a_grab_takes_every_key_but_the_unused_ones() {
        for code in [
            "KeyV",
            "Digit1",
            "Space",
            "Enter",
            "ArrowLeft",
            "PageUp",
            "F1",
            "F12",
        ] {
            assert!(does_grab_take_key_from_other_apps(code), "{code}");
        }
        for code in ["F13", "F24", "Pause", "ScrollLock"] {
            assert!(!does_grab_take_key_from_other_apps(code), "{code}");
        }
    }

    #[test]
    fn reads_back_the_character_a_recorded_key_typed() {
        for character in ('a'..='z').chain('0'..='9').chain("-=[]\\;',./`".chars()) {
            let code = code_of_toolkit_key(&character.to_string()).unwrap_or_default();
            assert_eq!(character_of_code(&code), Some(character), "{code}");
        }
        for code in ["Space", "F13", "ArrowLeft", "KeyAB", "Keyé", ""] {
            assert_eq!(character_of_code(code), None, "{code}");
        }
    }

    #[test]
    fn names_toolkit_keys_as_w3c_codes() {
        assert_eq!(code_of_toolkit_key("v").as_deref(), Some("KeyV"));
        assert_eq!(code_of_toolkit_key("7").as_deref(), Some("Digit7"));
        assert_eq!(code_of_toolkit_key("space").as_deref(), Some("Space"));
        assert_eq!(code_of_toolkit_key("f13").as_deref(), Some("F13"));
        assert_eq!(code_of_toolkit_key("pageup").as_deref(), Some("PageUp"));
        assert_eq!(code_of_toolkit_key("`").as_deref(), Some("Backquote"));
        assert_eq!(code_of_toolkit_key("f99"), None);
        assert_eq!(code_of_toolkit_key("é"), None);
        assert_eq!(code_of_modifier(ModifierKey::Shift), "ShiftLeft");
        assert_eq!(modifier_of_code("AltRight"), Some(ModifierKey::Alt));
    }
}
