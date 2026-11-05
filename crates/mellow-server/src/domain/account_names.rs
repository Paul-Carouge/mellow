//! Account names and friend codes (ADR-0048): one name per account, unique by its folded key, and
//! a permanent code that finds the account whatever its name.

use unicode_normalization::UnicodeNormalization as _;
use unicode_normalization::char::is_combining_mark;

use crate::domain::account_rules::{AccountRuleViolation, is_invisible_format_character};

pub const MINIMUM_ACCOUNT_NAME_LENGTH: usize = 2;
pub const MAXIMUM_ACCOUNT_NAME_LENGTH: usize = 32;
/// The word that mentions everyone in a channel (ADR-0040), so no account may be named so.
const RESERVED_NAME_KEY: &str = "everyone";

/// Crockford's base 32: no I, L, O or U, so a code read aloud or copied by hand stays unambiguous.
const CROCKFORD_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const FRIEND_CODE_SYMBOLS: usize = 8;
const BITS_PER_SYMBOL: u32 = 5;
/// 40 bits: a trillion codes, so random ones rarely collide and cannot be guessed in bulk.
pub(crate) const FRIEND_CODE_BITS: u32 = 40;
const FRIEND_CODE_MASK: u64 = (1 << FRIEND_CODE_BITS) - 1;
const SYMBOL_MASK: u64 = (1 << BITS_PER_SYMBOL) - 1;

fn is_allowed_name_character(character: char, previous: Option<char>) -> bool {
    if is_invisible_format_character(character) {
        return false;
    }
    if is_combining_mark(character) {
        // Accents and vowel signs only on a letter, so a name cannot start with a stack of marks.
        return previous
            .is_some_and(|previous| previous.is_alphanumeric() || is_combining_mark(previous));
    }
    character.is_alphanumeric() || matches!(character, ' ' | '_' | '.' | '-')
}

/// Composed (NFC), trimmed, every run of white space made one space.
fn tidied_name(name: &str) -> String {
    let composed: String = name.nfc().collect();
    composed.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The name to store: letters of any script, digits, spaces, `_`, `.` and `-`, 2 to 32 characters
/// once trimmed and with inner spaces collapsed; capitals kept as typed.
///
/// # Errors
/// `InvalidAccountName` when the rules are not met or the name is the reserved "everyone".
pub fn normalize_account_name(name: &str) -> Result<String, AccountRuleViolation> {
    let tidied = tidied_name(name);
    let length = tidied.chars().count();
    let mut previous = None;
    let has_allowed_characters = tidied.chars().all(|character| {
        let is_allowed = is_allowed_name_character(character, previous);
        previous = Some(character);
        is_allowed
    });
    if (MINIMUM_ACCOUNT_NAME_LENGTH..=MAXIMUM_ACCOUNT_NAME_LENGTH).contains(&length)
        && has_allowed_characters
        && account_name_key(&tidied) != RESERVED_NAME_KEY
    {
        Ok(tidied)
    } else {
        Err(AccountRuleViolation::InvalidAccountName)
    }
}

/// What uniqueness compares: NFKC, case folded (each character upper- then lowercased, so "ß" and
/// "ss" meet), then accents and other combining marks removed, so "Élodie", "elodie" and "ÉLODIE"
/// are one name. Also what sign-in and friend requests look names up by.
#[must_use]
pub fn account_name_key(name: &str) -> String {
    let folded: String = tidied_name(name)
        .nfkc()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect();
    folded
        .nfkd()
        .filter(|character| !is_combining_mark(*character))
        .collect()
}

/// For accounts made before ADR-0048: their display name without the characters names may no
/// longer hold, or `fallback` (their old username, always valid) when too little is left.
#[must_use]
pub(crate) fn account_name_from_old_names(display_name: &str, fallback: &str) -> String {
    let mut previous = None;
    let kept: String = tidied_name(display_name)
        .chars()
        .filter(|character| {
            let is_allowed = is_allowed_name_character(*character, previous);
            if is_allowed {
                previous = Some(*character);
            }
            is_allowed
        })
        .collect();
    let kept: String = tidied_name(&kept)
        .chars()
        .take(MAXIMUM_ACCOUNT_NAME_LENGTH)
        .collect();
    normalize_account_name(&kept)
        .or_else(|_| normalize_account_name(fallback))
        // Only an old username that is the reserved word fails both: "everyone 1" is valid.
        .unwrap_or_else(|_| numbered_account_name(fallback, 1))
}

/// `name` followed by " {number}", shortened so the whole stays within the maximum length.
#[must_use]
pub(crate) fn numbered_account_name(name: &str, number: u32) -> String {
    let suffix = format!(" {number}");
    let room = MAXIMUM_ACCOUNT_NAME_LENGTH.saturating_sub(suffix.chars().count());
    let base: String = name.chars().take(room).collect();
    format!("{}{suffix}", base.trim_end())
}

/// A new friend code from the operating system's random generator.
///
/// # Errors
/// Fails when the operating system has no randomness to give.
pub(crate) fn generate_friend_code() -> Result<u64, getrandom::Error> {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes) & FRIEND_CODE_MASK)
}

/// `XXXX-XXXX`.
#[must_use]
pub fn format_friend_code(code: u64) -> String {
    let symbols: String = (0..FRIEND_CODE_SYMBOLS)
        .rev()
        .map(|index| {
            let shift = u32::try_from(index).unwrap_or(0) * BITS_PER_SYMBOL;
            let value = usize::try_from((code >> shift) & SYMBOL_MASK).unwrap_or(0);
            CROCKFORD_ALPHABET
                .get(value)
                .map_or('0', |symbol| char::from(*symbol))
        })
        .collect();
    let (first, second) = symbols.split_at(FRIEND_CODE_SYMBOLS / 2);
    format!("{first}-{second}")
}

/// A code as people type it: any case, dashes and spaces anywhere, and I, L and O read as 1, 1
/// and 0 as Crockford's decoding does. `None` for anything else.
#[must_use]
pub fn parse_friend_code(text: &str) -> Option<u64> {
    let mut code = 0_u64;
    let mut symbols = 0;
    for character in text
        .chars()
        .filter(|character| !matches!(character, '-' | ' '))
    {
        let upper = match character.to_ascii_uppercase() {
            'I' | 'L' => '1',
            'O' => '0',
            other => other,
        };
        let value = CROCKFORD_ALPHABET
            .iter()
            .position(|symbol| char::from(*symbol) == upper)?;
        symbols += 1;
        if symbols > FRIEND_CODE_SYMBOLS {
            return None;
        }
        code = (code << BITS_PER_SYMBOL) | u64::try_from(value).ok()?;
    }
    (symbols == FRIEND_CODE_SYMBOLS).then_some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_keep_their_capitals_and_lose_extra_spaces() {
        assert_eq!(
            normalize_account_name("  Paul   Carouge ").unwrap(),
            "Paul Carouge"
        );
        assert_eq!(normalize_account_name("Élodie").unwrap(), "Élodie");
        assert_eq!(
            normalize_account_name("E\u{301}lodie").unwrap(),
            "Élodie",
            "composed"
        );
        for valid in ["Jean-Paul", "ana_b.", "李小龍", "Ωmega 2", "नमस्ते", "ab"] {
            assert!(normalize_account_name(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "a",
            "",
            "   ",
            "paul@home",
            "😀 smile",
            "a\u{200B}b",
            "\u{301}ab",
            "\u{3164}\u{3164}",
            "Everyone",
            "\u{202E}nimda",
            "ab\u{E0041}",
            "x\u{FE0F}",
            "\u{2800}\u{2800}",
            "\u{FEFF}Basile",
            "Chlo\u{2066}é",
            "Aur\u{0007}ore",
            &"x".repeat(33),
        ] {
            assert!(normalize_account_name(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn keys_ignore_case_accents_and_compatibility_forms() {
        let key = account_name_key("Élodie");
        for same in [
            "elodie",
            "ELODIE",
            "élodie",
            "E\u{301}LODIE",
            "ｅｌｏｄｉｅ",
        ] {
            assert_eq!(account_name_key(same), key, "{same}");
        }
        assert_eq!(account_name_key("Straße"), account_name_key("STRASSE"));
        assert_eq!(account_name_key(" Paul  C "), "paul c");
        assert_ne!(account_name_key("Paul C"), account_name_key("Paul_C"));
    }

    #[test]
    fn old_display_names_are_cleaned_or_replaced_by_the_old_username() {
        assert_eq!(account_name_from_old_names("Aurore 🌸", "aurore"), "Aurore");
        assert_eq!(account_name_from_old_names("🌸", "aurore"), "aurore");
        assert_eq!(account_name_from_old_names("everyone", "eve"), "eve");
        assert_eq!(
            account_name_from_old_names("Everyone", "everyone"),
            "everyone 1"
        );
        assert_eq!(
            account_name_from_old_names("Élodie (admin)", "elo"),
            "Élodie admin"
        );
    }

    #[test]
    fn numbered_names_stay_within_the_maximum() {
        assert_eq!(numbered_account_name("Paul", 2), "Paul 2");
        let long = "x".repeat(MAXIMUM_ACCOUNT_NAME_LENGTH);
        let numbered = numbered_account_name(&long, 12);
        assert_eq!(numbered.chars().count(), MAXIMUM_ACCOUNT_NAME_LENGTH);
        assert!(numbered.ends_with(" 12"));
        assert!(normalize_account_name(&numbered).is_ok());
    }

    #[test]
    fn friend_codes_round_trip_and_are_typed_loosely() {
        for code in [0, 1, FRIEND_CODE_MASK, 0x12_3456_789A] {
            let shown = format_friend_code(code);
            assert_eq!(shown.len(), 9);
            assert_eq!(parse_friend_code(&shown), Some(code), "{shown}");
            assert_eq!(
                parse_friend_code(&shown.to_lowercase().replace('-', "")),
                Some(code)
            );
        }
        assert_eq!(format_friend_code(0), "0000-0000");
        assert_eq!(
            parse_friend_code("oooo-llll"),
            parse_friend_code("0000-1111")
        );
        for invalid in ["", "ABCD-EFG", "ABCD-EFGHJ", "ABCD-EFGU", "ABCD_EFGH"] {
            assert_eq!(parse_friend_code(invalid), None, "{invalid}");
        }
        let generated = generate_friend_code().unwrap();
        assert!(generated <= FRIEND_CODE_MASK);
    }
}
