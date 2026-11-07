//! The composer's ":na" suggestions (`web/src/components/custom-emoji.ts`, "Suggestions while
//! typing", DESIGN.md §5.5): where the query is, which built-in emoji it finds, and what choosing
//! one writes.
//!
//! shortcut: a server's own emoji (ADR-0028) are not suggested yet, nor turned into their markup
//! on send; add `CustomEmoji` suggestions with `withEmojiNamesAsMarkup` when a native client
//! shows them.

use super::emoji_catalogue::search_emoji;
use super::unicode_classes::{character_before, is_js_whitespace};

/// Emoji names hold 2 to 32 of `[A-Za-z0-9_]` (the server's `is_valid_emoji_name`).
const MINIMUM_EMOJI_NAME_LENGTH: usize = 2;
const MAXIMUM_EMOJI_NAME_LENGTH: usize = 32;
pub const MAXIMUM_EMOJI_SUGGESTIONS: usize = 10;

fn is_emoji_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// What is being typed after a ":": replaced from `start` to `end` (bytes) when a suggestion is
/// chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmojiSuggestionQuery {
    pub start: usize,
    pub end: usize,
    pub query: String,
}

/// The ":na" being typed before `caret` (a byte offset), as on Discord: a colon at the start or
/// after a space, then at least two name characters up to the caret. `None` otherwise, so
/// "12:30" or a URL's "https:" never opens the list.
#[must_use]
pub fn emoji_suggestion_query(text: &str, caret: usize) -> Option<EmojiSuggestionQuery> {
    let bytes = text.as_bytes();
    text.get(..caret)?;
    let mut start = caret;
    while start > 0
        && caret - start <= MAXIMUM_EMOJI_NAME_LENGTH
        && bytes
            .get(start - 1)
            .copied()
            .is_some_and(is_emoji_name_byte)
    {
        start -= 1;
    }
    let colon = start.checked_sub(1)?;
    let length = caret - start;
    if bytes.get(colon) != Some(&b':')
        || !(MINIMUM_EMOJI_NAME_LENGTH..=MAXIMUM_EMOJI_NAME_LENGTH).contains(&length)
    {
        return None;
    }
    if colon > 0 && !character_before(text, colon).is_some_and(is_js_whitespace) {
        return None;
    }
    // To the end of the name, so choosing with the caret inside ":smi|le" leaves no "le" behind.
    let end = caret
        + bytes
            .get(caret..)?
            .iter()
            .take_while(|byte| is_emoji_name_byte(**byte))
            .count();
    Some(EmojiSuggestionQuery {
        start: colon,
        end,
        query: text.get(start..caret)?.to_owned(),
    })
}

/// A built-in emoji offered for the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmojiSuggestion {
    pub emoji: &'static str,
    /// The emoji's first English search word, shown beside it.
    pub name: &'static str,
}

/// Built-in emoji whose words start with the query, at most [`MAXIMUM_EMOJI_SUGGESTIONS`].
#[must_use]
pub fn suggest_emoji(query: &str) -> Vec<EmojiSuggestion> {
    // Built-in emoji are searched by words: "thumbs_up" reads as "thumbs up".
    search_emoji(&query.replace('_', " "))
        .into_iter()
        .take(MAXIMUM_EMOJI_SUGGESTIONS)
        .map(|entry| EmojiSuggestion {
            emoji: entry.emoji,
            name: entry.keywords.first().copied().unwrap_or_default(),
        })
        .collect()
}

/// The draft with `replacement` and a space in place of `start`–`end` (bytes), and where the
/// caret goes: after the space, which is not doubled when one already follows.
#[must_use]
pub fn replace_in_draft(
    draft: &str,
    start: usize,
    end: usize,
    replacement: &str,
) -> (String, usize) {
    let before = draft.get(..start).unwrap_or(draft);
    let after = draft.get(end..).unwrap_or_default();
    let caret = start + replacement.len() + 1;
    let text = if after.starts_with(' ') {
        format!("{before}{replacement}{after}")
    } else {
        format!("{before}{replacement} {after}")
    };
    (text, caret)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(start: usize, end: usize, query: &str) -> EmojiSuggestionQuery {
        EmojiSuggestionQuery {
            start,
            end,
            query: query.to_owned(),
        }
    }

    #[test]
    fn opens_after_a_colon_and_two_name_characters_at_the_start_or_after_a_space() {
        assert_eq!(emoji_suggestion_query(":pa", 3), Some(found(0, 3, "pa")));
        assert_eq!(
            emoji_suggestion_query("hi :party_p", 11),
            Some(found(3, 11, "party_p"))
        );
        assert_eq!(
            emoji_suggestion_query("hi\n:Wa", 6),
            Some(found(3, 6, "Wa"))
        );
        // Only what is before the caret is searched; the whole name is replaced, so no "ty" stays.
        assert_eq!(
            emoji_suggestion_query(":party rest", 4),
            Some(found(0, 6, "par"))
        );
        // After any whitespace, even past multi-byte text.
        assert_eq!(emoji_suggestion_query("é :sm", 6), Some(found(3, 6, "sm")));
    }

    #[test]
    fn stays_closed_for_one_character_times_urls_and_finished_names() {
        let too_long = format!(":{}", "a".repeat(33));
        for (text, caret) in [
            (":p", 2),
            ("at 12:30", 8),
            ("https://example.org", 6),
            ("a:bc", 4),
            (":party: ", 8),
            (too_long.as_str(), 34),
            // A caret inside a character is no caret.
            ("é:sm", 1),
        ] {
            assert_eq!(emoji_suggestion_query(text, caret), None, "{text}");
        }
    }

    #[test]
    fn suggests_built_in_emoji_by_their_words_and_stops_at_ten() {
        assert!(suggest_emoji("thumbs_up").contains(&EmojiSuggestion {
            emoji: "👍",
            name: "thumbs"
        }));
        assert!(suggest_emoji("sm").len() <= MAXIMUM_EMOJI_SUGGESTIONS);
        assert_eq!(suggest_emoji("zzzzzz"), Vec::new());
    }

    #[test]
    fn writes_the_choice_then_a_space_that_is_not_doubled() {
        assert_eq!(
            replace_in_draft("hi :par", 3, 7, "👍"),
            ("hi 👍 ".to_owned(), 3 + "👍".len() + 1)
        );
        assert_eq!(
            replace_in_draft("hi :par there", 3, 7, ":party:"),
            ("hi :party: there".to_owned(), 11)
        );
        assert_eq!(
            replace_in_draft("hi :par", 3, 7, ":party:"),
            ("hi :party: ".to_owned(), 11)
        );
    }
}
