//! The character classes of the web client's regular expressions, as JavaScript reads them, and
//! small helpers to look at the characters around a byte position.

use icu_properties::props::{Diacritic, GeneralCategory, GeneralCategoryGroup};
use icu_properties::{CodePointMapData, CodePointSetData};

/// `\p{L}`.
pub(crate) fn is_letter(character: char) -> bool {
    GeneralCategoryGroup::Letter.contains(CodePointMapData::<GeneralCategory>::new().get(character))
}

/// `\p{N}`: `char::is_numeric` is exactly the Nd, Nl and No categories.
pub(crate) fn is_number(character: char) -> bool {
    character.is_numeric()
}

/// `[\p{L}\p{N}]`.
pub(crate) fn is_letter_or_number(character: char) -> bool {
    is_letter(character) || is_number(character)
}

/// `\p{Cf}`.
pub(crate) fn is_format_character(character: char) -> bool {
    CodePointMapData::<GeneralCategory>::new().get(character) == GeneralCategory::Format
}

/// `\p{Z}`.
pub(crate) fn is_separator(character: char) -> bool {
    GeneralCategoryGroup::Separator
        .contains(CodePointMapData::<GeneralCategory>::new().get(character))
}

/// `\p{Diacritic}`.
pub(crate) fn is_diacritic(character: char) -> bool {
    CodePointSetData::new::<Diacritic>().contains(character)
}

/// JavaScript's `\s`: Unicode `White_Space` without U+0085, plus U+FEFF.
pub(crate) fn is_js_whitespace(character: char) -> bool {
    character == '\u{feff}' || (character.is_whitespace() && character != '\u{85}')
}

/// JavaScript's `String.prototype.trim`.
pub(crate) fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_whitespace)
}

pub(crate) fn js_trim_start(text: &str) -> &str {
    text.trim_start_matches(is_js_whitespace)
}

pub(crate) fn js_trim_end(text: &str) -> &str {
    text.trim_end_matches(is_js_whitespace)
}

/// The character starting at byte `position`, if `position` starts one.
pub(crate) fn character_at(text: &str, position: usize) -> Option<char> {
    text.get(position..)?.chars().next()
}

/// The character ending right before byte `position`, if `position` is a character boundary.
pub(crate) fn character_before(text: &str, position: usize) -> Option<char> {
    text.get(..position)?.chars().next_back()
}

/// The byte length of the character at `position`, 1 when `position` starts none (a loop always moves on).
pub(crate) fn character_length_at(text: &str, position: usize) -> usize {
    character_at(text, position).map_or(1, char::len_utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_classes_as_javascript_does() {
        assert!(is_letter('é') && is_letter('名') && !is_letter('1') && !is_letter('_'));
        assert!(is_number('٣') && is_number('½'));
        assert!(
            is_js_whitespace('\u{feff}')
                && !is_js_whitespace('\u{85}')
                && is_js_whitespace('\u{3000}')
        );
        assert!(is_format_character('\u{200b}') && is_separator('\u{2028}'));
        assert!(is_diacritic('^') && is_diacritic('\u{301}') && !is_diacritic('a'));
        assert_eq!(js_trim("\u{feff} a \n"), "a");
    }
}
