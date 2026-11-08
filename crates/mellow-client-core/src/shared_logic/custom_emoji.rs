//! How emoji show in messages and reactions (`web/src/components/custom-emoji.ts`, the jumbo and
//! reaction parts; `customEmojiOfReactionKey` of `protocol.ts` and `displayEmoji` of
//! `reactions.ts`; DESIGN.md §6.58, ADR-0028).

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::LazyLock;

use icu_properties::CodePointSetData;
use icu_properties::props::{EmojiPresentation, RegionalIndicator};
use mellow_protocol::{CustomEmoji, Snowflake};
use unicode_segmentation::UnicodeSegmentation as _;

use super::emoji_catalogue::EMOJI_CATALOGUE;
use super::markdown::{BlockNode, InlineNode};
use crate::state::reaction_key;

/// As on Discord: a message of up to 30 emoji and nothing else shows them large.
pub const MAXIMUM_JUMBO_EMOJI: usize = 30;
const EMOJI_PRESENTATION_SELECTOR: char = '\u{FE0F}';
const KEYCAP: char = '\u{20E3}';
/// The longest id a custom reaction key holds, as the server's `u64`.
const MAXIMUM_ID_DIGITS: usize = 20;

/// Each built-in emoji by its stored key (`CATALOGUE_EMOJI_BY_KEY`).
static CATALOGUE_EMOJI_BY_KEY: LazyLock<HashMap<String, &'static str>> = LazyLock::new(|| {
    EMOJI_CATALOGUE
        .iter()
        .map(|entry| (reaction_key(entry.emoji), entry.emoji))
        .collect()
});

/// Whether the picker knows this emoji, whatever its selectors (`readRecentEmoji`'s filter).
#[must_use]
pub fn is_catalogue_emoji(emoji: &str) -> bool {
    CATALOGUE_EMOJI_BY_KEY.contains_key(&reaction_key(emoji))
}

/// A grapheme drawn as an emoji: emoji presentation, a selector asking for it, a keycap or a flag.
fn is_emoji_grapheme(grapheme: &str) -> bool {
    grapheme.chars().any(|character| {
        character == EMOJI_PRESENTATION_SELECTOR
            || character == KEYCAP
            || CodePointSetData::new::<EmojiPresentation>().contains(character)
            || CodePointSetData::new::<RegionalIndicator>().contains(character)
    })
}

/// How many emoji `text` holds; `None` if it holds anything but emoji and spaces.
fn count_emoji_only(text: &str) -> Option<usize> {
    let mut count = 0;
    for grapheme in text.graphemes(true) {
        if grapheme.chars().all(char::is_whitespace) {
            continue;
        }
        if !is_emoji_grapheme(grapheme) {
            return None;
        }
        count += 1;
    }
    Some(count)
}

/// Whether a parsed message is one paragraph of 1 to 30 emoji, shown pictured, and spaces.
#[must_use]
pub fn is_jumbo_emoji_message(blocks: &[BlockNode]) -> bool {
    let [BlockNode::Paragraph(children)] = blocks else {
        return false;
    };
    let mut count = 0;
    for node in children {
        let added = match node {
            InlineNode::CustomEmoji(_) => Some(1),
            InlineNode::Text(text) => count_emoji_only(text),
            _ => None,
        };
        let Some(added) = added else {
            return false;
        };
        count += added;
        if count > MAXIMUM_JUMBO_EMOJI {
            return false;
        }
    }
    count > 0
}

/// The emoji id of a custom reaction key (`":501"`), `None` for a Unicode emoji.
#[must_use]
pub fn custom_emoji_of_reaction_key(key: &str) -> Option<Snowflake> {
    let digits = key.strip_prefix(':')?;
    let is_canonical = (1..=MAXIMUM_ID_DIGITS).contains(&digits.len())
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    is_canonical
        .then(|| digits.parse().ok().map(Snowflake::from_raw))
        .flatten()
}

/// How a stored key shows: as the picker writes it when the catalogue knows it (the selectors the
/// instance strips put back), else a lone symbol or keycap asks for its emoji look, which some
/// fonts only give with the selector.
#[must_use]
pub fn display_emoji(key: &str) -> String {
    if let Some(known) = CATALOGUE_EMOJI_BY_KEY.get(key) {
        return (*known).to_owned();
    }
    let mut characters = key.chars();
    match (characters.next(), characters.next(), characters.next()) {
        (Some(base), Some(KEYCAP), None) => format!("{base}{EMOJI_PRESENTATION_SELECTOR}{KEYCAP}"),
        (Some(single), None, None) => format!("{single}{EMOJI_PRESENTATION_SELECTOR}"),
        _ => key.to_owned(),
    }
}

/// How a reaction's emoji shows; the client's catalogue labels a deleted one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReactionAppearance {
    /// A built-in emoji, ready to draw; it is its own label.
    Unicode(String),
    /// A server emoji's picture, labelled `:name:`.
    Custom(CustomEmoji),
    /// Its server deleted it: the reaction stays, joined and taken back, but never started anew.
    Deleted,
}

/// `custom_emojis` are those of the user's servers: a custom key missing there is a deleted emoji.
#[must_use]
pub fn reaction_appearance<S: BuildHasher>(
    key: &str,
    custom_emojis: &HashMap<Snowflake, CustomEmoji, S>,
) -> ReactionAppearance {
    match custom_emoji_of_reaction_key(key) {
        None => ReactionAppearance::Unicode(display_emoji(key)),
        Some(id) => custom_emojis
            .get(&id)
            .map_or(ReactionAppearance::Deleted, |emoji| {
                ReactionAppearance::Custom(emoji.clone())
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::super::markdown::{MarkdownSources, RoleMentionContext, parse_message_markdown};
    use super::super::mentions::MentionablePeople;
    use super::*;

    fn known() -> HashMap<Snowflake, CustomEmoji> {
        [
            (500, "party", false),
            (501, "party_parrot", true),
            (502, "WaveHello", false),
        ]
        .into_iter()
        .map(|(id, name, is_animated)| {
            let id = Snowflake::from_raw(id);
            (
                id,
                CustomEmoji {
                    id,
                    name: name.to_owned(),
                    is_animated,
                },
            )
        })
        .collect()
    }

    fn is_jumbo(text: &str) -> bool {
        let people = MentionablePeople::new();
        let custom_emojis = known();
        let roles = HashMap::new();
        let blocks = parse_message_markdown(
            text,
            MarkdownSources {
                people: &people,
                custom_emojis: &custom_emojis,
                role_mentions: RoleMentionContext {
                    roles: &roles,
                    is_everyone_mentioned: false,
                },
            },
        );
        is_jumbo_emoji_message(&blocks)
    }

    #[test]
    fn jumbo_messages_are_made_only_of_emoji_and_spaces() {
        assert!(is_jumbo("<:party:500>"));
        assert!(is_jumbo("<:party:500> <a:party_parrot:501>"));
        assert!(is_jumbo("👍 ❤️ <:party:500>"));
        assert!(is_jumbo("👨‍👩‍👧 🇫🇷 1️⃣"));
        assert!(is_jumbo(&"<:party:500>".repeat(30)));
    }

    #[test]
    fn jumbo_messages_never_hold_text_formatting_an_unknown_emoji_more_than_30_or_blocks() {
        for text in [
            "hi <:party:500>",
            "**<:party:500>**",
            "<:gone:999>",
            &"<:party:500>".repeat(31),
            "<:party:500>\n\n> <:party:500>",
            "123",
            "©",
            "",
        ] {
            assert!(!is_jumbo(text), "{text}");
        }
    }

    #[test]
    fn reactions_show_a_server_emoji_by_id_and_a_missing_one_as_deleted() {
        let known = known();
        assert_eq!(
            reaction_appearance("👍", &known),
            ReactionAppearance::Unicode("👍".to_owned())
        );
        assert_eq!(
            reaction_appearance(":502", &known),
            ReactionAppearance::Custom(known[&Snowflake::from_raw(502)].clone())
        );
        assert_eq!(
            reaction_appearance(":999", &known),
            ReactionAppearance::Deleted
        );
    }

    #[test]
    fn only_canonical_ids_are_custom_reaction_keys() {
        assert_eq!(
            custom_emoji_of_reaction_key(":501"),
            Some(Snowflake::from_raw(501))
        );
        for key in [":", ":01", "501", ":5a", ":123456789012345678901", "👍"] {
            assert_eq!(custom_emoji_of_reaction_key(key), None, "{key}");
        }
    }

    #[test]
    fn a_lone_symbol_or_keycap_asks_for_its_emoji_look() {
        assert_eq!(display_emoji("❤"), "❤\u{FE0F}");
        assert_eq!(display_emoji("1\u{20E3}"), "1\u{FE0F}\u{20E3}");
        assert_eq!(display_emoji("👨‍👩‍👧"), "👨‍👩‍👧");
    }

    // Ported from `reactions.test.ts` ("show with the picker's spelling").
    #[test]
    fn a_known_key_shows_as_the_picker_writes_it() {
        assert_eq!(display_emoji("☺"), "☺\u{FE0F}");
        assert_eq!(display_emoji("👨‍💻"), "👨‍💻");
        for entry in EMOJI_CATALOGUE.iter() {
            assert_eq!(display_emoji(&reaction_key(entry.emoji)), entry.emoji);
        }
        assert!(is_catalogue_emoji("❤") && is_catalogue_emoji("❤\u{FE0F}"));
        assert!(!is_catalogue_emoji("lol"));
    }
}
