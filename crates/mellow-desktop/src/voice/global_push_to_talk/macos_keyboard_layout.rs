//! macOS: Carbon hot keys name a key by its position on an ANSI (US) keyboard, while the stored
//! code names the character the layout typed when it was recorded (GPUI reads keys through the
//! layout). On AZERTY, `KeyA` must grab the key typing "a", which sits at ANSI's Q. The position
//! is found by asking `UCKeyTranslate` what each position types on the current layout.

use std::ffi::c_void;

use global_hotkey::hotkey::{Code, HotKey};
use mellow_client_core::shared_logic::push_to_talk_keys::character_of_code;

/// The keys whose character depends on the layout, with their ANSI virtual key code (Carbon's
/// `Events.h`, the values `global-hotkey` registers).
const LAYOUT_DEPENDENT_KEYS: [(Code, u16); 47] = [
    (Code::KeyA, 0x00),
    (Code::KeyS, 0x01),
    (Code::KeyD, 0x02),
    (Code::KeyF, 0x03),
    (Code::KeyH, 0x04),
    (Code::KeyG, 0x05),
    (Code::KeyZ, 0x06),
    (Code::KeyX, 0x07),
    (Code::KeyC, 0x08),
    (Code::KeyV, 0x09),
    (Code::KeyB, 0x0b),
    (Code::KeyQ, 0x0c),
    (Code::KeyW, 0x0d),
    (Code::KeyE, 0x0e),
    (Code::KeyR, 0x0f),
    (Code::KeyY, 0x10),
    (Code::KeyT, 0x11),
    (Code::Digit1, 0x12),
    (Code::Digit2, 0x13),
    (Code::Digit3, 0x14),
    (Code::Digit4, 0x15),
    (Code::Digit6, 0x16),
    (Code::Digit5, 0x17),
    (Code::Equal, 0x18),
    (Code::Digit9, 0x19),
    (Code::Digit7, 0x1a),
    (Code::Minus, 0x1b),
    (Code::Digit8, 0x1c),
    (Code::Digit0, 0x1d),
    (Code::BracketRight, 0x1e),
    (Code::KeyO, 0x1f),
    (Code::KeyU, 0x20),
    (Code::BracketLeft, 0x21),
    (Code::KeyI, 0x22),
    (Code::KeyP, 0x23),
    (Code::KeyL, 0x25),
    (Code::KeyJ, 0x26),
    (Code::Quote, 0x27),
    (Code::KeyK, 0x28),
    (Code::Semicolon, 0x29),
    (Code::Backslash, 0x2a),
    (Code::Comma, 0x2b),
    (Code::Slash, 0x2c),
    (Code::KeyN, 0x2d),
    (Code::KeyM, 0x2e),
    (Code::Period, 0x2f),
    (Code::Backquote, 0x32),
];

/// `UCKeyTranslate`'s modifier states, shifted right by 8 as it wants them.
const NO_MODIFIER: u32 = 0;
const COMMAND_MODIFIER: u32 = 1;
/// The position of A on an ANSI keyboard.
const ANSI_A: u16 = 0x00;

/// The hot key grabbing the key that types `hotkey`'s character on the current layout; the same
/// hot key for a key that types none (F keys, arrows); `None` when no key types it.
pub(super) fn hotkey_on_current_layout(hotkey: HotKey) -> Option<HotKey> {
    let Some(character) = character_of_code(&hotkey.key.to_string()) else {
        return Some(hotkey);
    };
    let layout = CurrentLayout::copy()?;
    // As GPUI does: a layout typing no ASCII (Russian) records keys through its Command layer.
    let modifiers = match layout.character_of(ANSI_A, NO_MODIFIER) {
        Some(character) if !character.is_ascii() => COMMAND_MODIFIER,
        _ => NO_MODIFIER,
    };
    position_typing(character, |virtual_key| {
        layout.character_of(virtual_key, modifiers)
    })
    .map(|code| HotKey::new(None, code))
}

/// The key whose position types `character`, given what each position types.
fn position_typing(character: char, typed_at: impl Fn(u16) -> Option<char>) -> Option<Code> {
    LAYOUT_DEPENDENT_KEYS
        .iter()
        .find(|(_, virtual_key)| {
            typed_at(*virtual_key).map(|typed| typed.to_ascii_lowercase()) == Some(character)
        })
        .map(|(code, _)| *code)
}

/// The current keyboard layout's key table, owned until dropped.
#[allow(unsafe_code)]
struct CurrentLayout {
    /// The input source, released on drop; it owns `layout_data`.
    source: *mut c_void,
    layout_data: *const u8,
    keyboard_type: u32,
}

#[allow(unsafe_code)]
impl CurrentLayout {
    /// `None` when the layout has no Unicode key table (some input methods).
    fn copy() -> Option<Self> {
        // SAFETY: takes no argument; the returned source follows Core Foundation's Copy rule,
        // so it is ours to release, which `Drop` does.
        let source = unsafe { TISCopyCurrentKeyboardLayoutInputSource() };
        if source.is_null() {
            return None;
        }
        let mut layout = Self {
            source,
            layout_data: std::ptr::null(),
            // SAFETY: takes no argument and reads a global.
            keyboard_type: u32::from(unsafe { LMGetKbdType() }),
        };
        // SAFETY: `source` is a live input source and the key is the framework's constant; the
        // returned CFData follows the Get rule, owned by `source`, which outlives it here.
        let data = unsafe { TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData) };
        if data.is_null() {
            return None;
        }
        // SAFETY: `data` is a live CFData; its bytes live as long as it does.
        layout.layout_data = unsafe { CFDataGetBytePtr(data) };
        (!layout.layout_data.is_null()).then_some(layout)
    }

    /// The single character the key at `virtual_key` types with `modifiers`; dead keys give
    /// their own accent, never a letter.
    fn character_of(&self, virtual_key: u16, modifiers: u32) -> Option<char> {
        const KEY_ACTION_DOWN: u16 = 0;
        const NO_DEAD_KEYS_OPTION: u32 = 1;
        let mut dead_key_state = 0_u32;
        let mut buffer = [0_u16; 4];
        let mut length = 0_usize;
        // SAFETY: `layout_data` is the live `UCKeyboardLayout` of `self.source`; the buffer and
        // its length match, and every out pointer is a live local.
        let status = unsafe {
            UCKeyTranslate(
                self.layout_data.cast(),
                virtual_key,
                KEY_ACTION_DOWN,
                modifiers,
                self.keyboard_type,
                NO_DEAD_KEYS_OPTION,
                &raw mut dead_key_state,
                buffer.len(),
                &raw mut length,
                buffer.as_mut_ptr(),
            )
        };
        if status != 0 {
            return None;
        }
        let mut characters = char::decode_utf16(buffer.get(..length)?.iter().copied());
        match (characters.next(), characters.next()) {
            (Some(Ok(character)), None) => Some(character),
            _ => None,
        }
    }
}

#[allow(unsafe_code)]
impl Drop for CurrentLayout {
    fn drop(&mut self) {
        // SAFETY: `source` came from a Copy function and is released once, here.
        unsafe { CFRelease(self.source) };
    }
}

#[allow(unsafe_code)]
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardLayoutInputSource() -> *mut c_void;
    fn TISGetInputSourceProperty(input_source: *mut c_void, key: *const c_void) -> *const c_void;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const c_void,
        virtual_key_code: u16,
        key_action: u16,
        modifier_key_state: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_length: usize,
        actual_length: *mut usize,
        unicode_string: *mut u16,
    ) -> i32;
    static kTISPropertyUnicodeKeyLayoutData: *const c_void;
}

#[allow(unsafe_code)]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    fn CFRelease(object: *const c_void);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AZERTY's letters that move: A and Q, Z and W swap; M sits at ANSI's semicolon.
    fn azerty(virtual_key: u16) -> Option<char> {
        let ansi = LAYOUT_DEPENDENT_KEYS
            .iter()
            .find(|(_, key)| *key == virtual_key)
            .and_then(|(code, _)| character_of_code(&code.to_string()))?;
        Some(match ansi {
            'a' => 'q',
            'q' => 'a',
            'z' => 'w',
            'w' => 'z',
            ';' => 'm',
            'm' => ',',
            other => other,
        })
    }

    #[test]
    fn grabs_the_position_typing_the_recorded_character() {
        assert_eq!(position_typing('a', azerty), Some(Code::KeyQ));
        assert_eq!(position_typing('w', azerty), Some(Code::KeyZ));
        assert_eq!(position_typing('m', azerty), Some(Code::Semicolon));
        assert_eq!(position_typing('v', azerty), Some(Code::KeyV));
        assert_eq!(position_typing('é', azerty), None);
    }

    #[test]
    fn keeps_keys_that_type_no_character_and_lists_each_position_once() {
        let f13 = HotKey::new(None, Code::F13);
        assert_eq!(hotkey_on_current_layout(f13), Some(f13));
        let mut virtual_keys: Vec<u16> =
            LAYOUT_DEPENDENT_KEYS.iter().map(|(_, key)| *key).collect();
        virtual_keys.sort_unstable();
        virtual_keys.dedup();
        assert_eq!(virtual_keys.len(), LAYOUT_DEPENDENT_KEYS.len());
        for (code, _) in LAYOUT_DEPENDENT_KEYS {
            assert!(character_of_code(&code.to_string()).is_some(), "{code}");
        }
    }

    /// Reads the machine's real layout table: some position types a character.
    #[test]
    fn reads_the_current_layout() {
        let layout = CurrentLayout::copy();
        if let Some(layout) = layout {
            assert!(
                LAYOUT_DEPENDENT_KEYS
                    .iter()
                    .any(|(_, key)| layout.character_of(*key, NO_MODIFIER).is_some())
            );
        }
    }
}
