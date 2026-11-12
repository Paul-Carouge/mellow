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
