//! Input rules for messages.

use crate::domain::account_rules::is_invisible_format_character;

pub const MAXIMUM_MESSAGE_LENGTH: usize = 4_000;
/// Also bounded in bytes: 4 000 four-byte emoji would otherwise store 16 KB per message.
pub const MAXIMUM_MESSAGE_BYTES: usize = 8_000;
pub const MAXIMUM_NONCE_LENGTH: usize = 64;
pub const DEFAULT_HISTORY_PAGE_SIZE: u32 = 50;
pub const MAXIMUM_HISTORY_PAGE_SIZE: u32 = 100;
/// Long enough for the longest standard sequences (a family with skin tones, a subdivision flag).
pub const MAXIMUM_REACTION_EMOJI_BYTES: usize = 64;
/// Distinct emoji on one message, as on Discord.
pub const MAXIMUM_DISTINCT_EMOJI_PER_MESSAGE: u32 = 20;
/// Pinned messages per channel, as on Discord.
pub const MAXIMUM_PINS_PER_CHANNEL: u32 = 50;
pub const DEFAULT_REACTING_USERS_PAGE_SIZE: u32 = 25;
pub const MAXIMUM_REACTING_USERS_PAGE_SIZE: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MessageRuleViolation {
    #[error(
        "messages are 1 to {MAXIMUM_MESSAGE_LENGTH} characters ({MAXIMUM_MESSAGE_BYTES} bytes), visible, without control or bidirectional-override characters"
    )]
    Content,
    #[error("nonces are 1 to {MAXIMUM_NONCE_LENGTH} printable ASCII characters")]
    Nonce,
    #[error("a reply must answer a message of the same channel")]
    ReplyTarget,
    #[error(
        "history pages hold 1 to {MAXIMUM_HISTORY_PAGE_SIZE} messages and use at most one of `before` and `after`"
    )]
    HistoryQuery,
    #[error("messages can only be sent in text channels")]
    NotATextChannel,
    #[error(
        "a message carries at most {MAXIMUM_ATTACHMENTS_PER_MESSAGE} attachments, each uploaded to its channel by its author and not sent yet"
    )]
    Attachments,
    #[error(
        "file names are 1 to {MAXIMUM_FILE_NAME_BYTES} bytes, without path separators or control characters"
    )]
    FileName,
    #[error(
        "pictures are PNG, JPEG, GIF or WebP, at most 8192 pixels a side, 24 megapixels in all frames together and 600 frames"
    )]
    Picture,
    #[error(
        "a reaction is exactly one Unicode emoji (at most {MAXIMUM_REACTION_EMOJI_BYTES} bytes), without text or spaces"
    )]
    ReactionEmoji,
    #[error("pages of reacting users hold 1 to {MAXIMUM_REACTING_USERS_PAGE_SIZE} users")]
    ReactingUsersQuery,
}

pub const MAXIMUM_ATTACHMENTS_PER_MESSAGE: usize = 10;
pub const MAXIMUM_FILE_NAME_BYTES: usize = 255;
/// Shown as `inline` previews: only raster images whose bytes say so (ADR-0020).
pub const PREVIEWABLE_IMAGE_TYPES: [&str; 4] =
    ["image/png", "image/jpeg", "image/gif", "image/webp"];
pub const UNKNOWN_CONTENT_TYPE: &str = "application/octet-stream";

/// The file name kept for an upload: its last path segment, trimmed. Refused when empty, too long,
/// a dot name, or holding control or bidirectional characters (a name is shown and downloaded).
///
/// # Errors
/// `FileName` for anything else.
pub fn clean_file_name(raw: &str) -> Result<String, MessageRuleViolation> {
    let name = raw.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > MAXIMUM_FILE_NAME_BYTES
        || name
            .chars()
            .any(|character| character.is_control() || is_forbidden_in_messages(character))
    {
        return Err(MessageRuleViolation::FileName);
    }
    Ok(name.to_owned())
}

/// The content type an upload is served with, from its first bytes only: what a client claims is
/// never trusted. Anything but the four raster formats is `application/octet-stream`.
#[must_use]
pub fn sniff_content_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        UNKNOWN_CONTENT_TYPE
    }
}

/// Windows clients send `\r\n`; messages store `\n` only.
#[must_use]
pub fn normalize_line_endings(content: &str) -> String {
    content.replace("\r\n", "\n")
}

/// Bidirectional overrides and isolates, tags and interlinear annotations: they can make a link
/// or text read differently from what it is (Trojan Source, CWE-451). Zero-width joiners and
/// variation selectors stay allowed: emoji sequences need them.
fn is_forbidden_in_messages(character: char) -> bool {
    (character.is_control() && character != '\n' && character != '\t')
        || matches!(
            character,
            '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{E0000}'..='\u{E007F}' | '\u{FFF9}'..='\u{FFFB}'
        )
}

/// Keeps the text as typed (leading spaces and blank lines are meaningful in code), but refuses
/// messages that show nothing (only whitespace or invisible characters), oversized ones, and
/// forbidden characters.
///
/// # Errors
/// `Content` when invisible, too long or containing forbidden characters.
pub fn validate_message_content(content: &str) -> Result<(), MessageRuleViolation> {
    let length = content.chars().count();
    let shows_something = content
        .chars()
        .any(|character| !character.is_whitespace() && !is_invisible_format_character(character));
    if !shows_something
        || length > MAXIMUM_MESSAGE_LENGTH
        || content.len() > MAXIMUM_MESSAGE_BYTES
        || content.chars().any(is_forbidden_in_messages)
    {
        Err(MessageRuleViolation::Content)
    } else {
        Ok(())
    }
}

/// # Errors
/// `Nonce` when empty, too long or not printable ASCII.
pub fn validate_nonce(nonce: &str) -> Result<(), MessageRuleViolation> {
    let is_printable_ascii = nonce.bytes().all(|byte| byte.is_ascii_graphic());
    if (1..=MAXIMUM_NONCE_LENGTH).contains(&nonce.len()) && is_printable_ascii {
        Ok(())
    } else {
        Err(MessageRuleViolation::Nonce)
    }
}

/// # Errors
/// `HistoryQuery` for a limit outside 1..=100.
pub fn history_page_size(requested: Option<u32>) -> Result<u32, MessageRuleViolation> {
    let size = requested.unwrap_or(DEFAULT_HISTORY_PAGE_SIZE);
    if (1..=MAXIMUM_HISTORY_PAGE_SIZE).contains(&size) {
        Ok(size)
    } else {
        Err(MessageRuleViolation::HistoryQuery)
    }
}

/// A code point that starts an emoji: Unicode's `Extended_Pictographic`, without the enclosed
/// letters and digits of U+1F100 to U+1F1FF that are not emoji (🅵 is a letter, 🅰 is an emoji)
/// nor the regional indicators, which only count as a pair (a flag).
/// shortcut: hand-written superset of `Extended_Pictographic` elsewhere (whole symbol blocks); use
/// a Unicode property table if a non-emoji symbol from those blocks must be refused.
fn is_emoji_code_point(character: char) -> bool {
    matches!(
        character,
        '\u{A9}'
            | '\u{AE}'
            | '\u{203C}'
            | '\u{2049}'
            | '\u{2122}'
            | '\u{2139}'
            | '\u{2194}'..='\u{2199}'
            | '\u{21A9}'..='\u{21AA}'
            | '\u{231A}'..='\u{231B}'
            | '\u{2328}'
            | '\u{2388}'
            | '\u{23CF}'
            | '\u{23E9}'..='\u{23F3}'
            | '\u{23F8}'..='\u{23FA}'
            | '\u{24C2}'
            | '\u{25AA}'..='\u{25AB}'
            | '\u{25B6}'
            | '\u{25C0}'
            | '\u{25FB}'..='\u{25FE}'
            | '\u{2600}'..='\u{27BF}'
            | '\u{2934}'..='\u{2935}'
            | '\u{2B05}'..='\u{2B07}'
            | '\u{2B1B}'..='\u{2B1C}'
            | '\u{2B50}'
            | '\u{2B55}'
            | '\u{3030}'
            | '\u{303D}'
            | '\u{3297}'
            | '\u{3299}'
            | '\u{1F000}'..='\u{1F0FF}'
            // 🅰 🅱 🅾 🅿 🆎 and 🆑 to 🆚.
            | '\u{1F170}'..='\u{1F171}'
            | '\u{1F17E}'..='\u{1F17F}'
            | '\u{1F18E}'
            | '\u{1F191}'..='\u{1F19A}'
            | '\u{1F200}'..='\u{1FAFF}'
            | '\u{1FC00}'..='\u{1FFFD}'
    )
}

const ZERO_WIDTH_JOINER: char = '\u{200D}';
const TEXT_PRESENTATION_SELECTOR: char = '\u{FE0E}';
const EMOJI_PRESENTATION_SELECTOR: char = '\u{FE0F}';
const KEYCAP: char = '\u{20E3}';
const BLACK_FLAG: char = '\u{1F3F4}';
const CANCEL_TAG: char = '\u{E007F}';

fn is_regional_indicator(character: char) -> bool {
    matches!(character, '\u{1F1E6}'..='\u{1F1FF}')
}

fn is_skin_tone(character: char) -> bool {
    matches!(character, '\u{1F3FB}'..='\u{1F3FF}')
}

fn is_variation_selector(character: char) -> bool {
    matches!(
        character,
        TEXT_PRESENTATION_SELECTOR | EMOJI_PRESENTATION_SELECTOR
    )
}

/// One emoji of a ZWJ sequence: an emoji code point, then at most a skin tone and one variation
/// selector.
fn is_emoji_element(element: &[char]) -> bool {
    match element {
        [base, modifiers @ ..] if is_emoji_code_point(*base) => match modifiers {
            [] => true,
            [modifier] => is_skin_tone(*modifier) || is_variation_selector(*modifier),
            [skin_tone, EMOJI_PRESENTATION_SELECTOR] => is_skin_tone(*skin_tone),
            _ => false,
        },
        _ => false,
    }
}

/// Exactly one emoji: a keycap (`1️⃣`), a flag (two regional indicators), a subdivision flag (🏴
/// with tags), or emoji joined by zero-width joiners, each with at most a skin tone and one
/// variation selector.
fn is_one_emoji(characters: &[char]) -> bool {
    match characters {
        [base, KEYCAP] | [base, EMOJI_PRESENTATION_SELECTOR, KEYCAP] => {
            matches!(base, '0'..='9' | '#' | '*')
        }
        [first, second] if is_regional_indicator(*first) => is_regional_indicator(*second),
        [BLACK_FLAG, tags @ .., CANCEL_TAG] if !tags.is_empty() => tags
            .iter()
            .all(|tag| matches!(tag, '\u{E0020}'..='\u{E007E}')),
        _ => characters
            .split(|character| *character == ZERO_WIDTH_JOINER)
            .all(is_emoji_element),
    }
}

/// The key a reaction is stored and counted under: exactly one Unicode emoji, at most
/// `MAXIMUM_REACTION_EMOJI_BYTES` as sent, without its variation selectors, so `❤️` and `❤` are
/// one reaction.
///
/// # Errors
/// `ReactionEmoji` for anything else (text, spaces, several emoji, stray joiners or selectors).
pub fn reaction_emoji_key(emoji: &str) -> Result<String, MessageRuleViolation> {
    let characters: Vec<char> = emoji.chars().collect();
    if emoji.len() <= MAXIMUM_REACTION_EMOJI_BYTES && is_one_emoji(&characters) {
        Ok(characters
            .into_iter()
            .filter(|character| !is_variation_selector(*character))
            .collect())
    } else {
        Err(MessageRuleViolation::ReactionEmoji)
    }
}

/// # Errors
/// `ReactingUsersQuery` for a limit outside 1..=100.
pub fn reacting_users_page_size(requested: Option<u32>) -> Result<u32, MessageRuleViolation> {
    let size = requested.unwrap_or(DEFAULT_REACTING_USERS_PAGE_SIZE);
    if (1..=MAXIMUM_REACTING_USERS_PAGE_SIZE).contains(&size) {
        Ok(size)
    } else {
        Err(MessageRuleViolation::ReactingUsersQuery)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_keeps_formatting_and_emoji_but_rejects_blank_and_control_characters() {
        for valid in [
            "hi",
            "  indented\n\tcode",
            "👩‍💻",
            "❤️",
            &"a".repeat(MAXIMUM_MESSAGE_LENGTH),
        ] {
            assert_eq!(validate_message_content(valid), Ok(()), "{valid:?}");
        }
        for invalid in [
            "",
            "  \n ",
            "bell\u{0007}",
            "esc\u{001B}[31m",
            "\u{200B}\u{3164}",
            "https://bank.example/\u{202E}moc.live",
            "carriage\rreturn",
            &"a".repeat(MAXIMUM_MESSAGE_LENGTH + 1),
            &"😀".repeat(MAXIMUM_MESSAGE_BYTES / 4 + 1),
        ] {
            assert!(validate_message_content(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn a_reaction_is_exactly_one_emoji() {
        for (valid, key) in [
            ("👍", "👍"),
            ("❤️", "❤"),
            ("❤", "❤"),
            ("👍🏽", "👍🏽"),
            ("👨\u{200D}👩\u{200D}👧", "👨\u{200D}👩\u{200D}👧"),
            ("🏳️\u{200D}🌈", "🏳\u{200D}🌈"),
            ("🇫🇷", "🇫🇷"),
            (
                "🏴\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}",
                "🏴\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}",
            ),
            ("1️⃣", "1\u{20E3}"),
            ("#\u{20E3}", "#\u{20E3}"),
            ("🅰️", "🅰"),
            ("©️", "©"),
        ] {
            assert_eq!(reaction_emoji_key(valid), Ok(key.to_owned()), "{valid:?}");
        }
        for invalid in [
            "",
            "a",
            "lol",
            "lol😀",
            "😀 ",
            " 😀",
            "😀\n",
            "\u{FE0F}",
            "\u{200D}",
            "1",
            "##",
            "😀69",
            "👍##",
            "😀0612345678",
            "🅵🆄🅲🅺",
            "🅵",
            "👍\u{FE0F}\u{FE0F}",
            "\u{FE0F}👍",
            "👍\u{E0041}",
            "👍👍",
            "🇫",
            "🇫🇷🇫🇷",
            "🏴\u{E007F}",
            "🏴\u{E0067}",
            "👍\u{200D}",
            "\u{200D}👍",
            "👍\u{200D}\u{200D}👍",
            "1\u{FE0F}\u{FE0F}\u{20E3}",
            "😀\u{202E}",
            "<script>",
        ] {
            assert_eq!(
                reaction_emoji_key(invalid),
                Err(MessageRuleViolation::ReactionEmoji),
                "{invalid:?}"
            );
        }
        let long_family = ["👨"; 11].join("\u{200D}");
        assert!(long_family.len() > MAXIMUM_REACTION_EMOJI_BYTES);
        assert!(reaction_emoji_key(&long_family).is_err());
    }

    #[test]
    fn reacting_user_pages_are_bounded() {
        assert_eq!(
            reacting_users_page_size(None),
            Ok(DEFAULT_REACTING_USERS_PAGE_SIZE)
        );
        assert!(reacting_users_page_size(Some(0)).is_err());
        assert!(reacting_users_page_size(Some(MAXIMUM_REACTING_USERS_PAGE_SIZE + 1)).is_err());
    }

    #[test]
    fn windows_line_endings_are_normalized() {
        assert_eq!(normalize_line_endings("a\r\nb"), "a\nb");
    }

    #[test]
    fn nonces_and_page_sizes_are_bounded() {
        assert!(validate_nonce("client-123").is_ok());
        assert!(validate_nonce("").is_err());
        assert!(validate_nonce("has space").is_err());
        assert!(validate_nonce(&"n".repeat(MAXIMUM_NONCE_LENGTH + 1)).is_err());
        assert_eq!(history_page_size(None), Ok(DEFAULT_HISTORY_PAGE_SIZE));
        assert!(history_page_size(Some(0)).is_err());
        assert!(history_page_size(Some(MAXIMUM_HISTORY_PAGE_SIZE + 1)).is_err());
    }

    #[test]
    fn file_names_keep_their_last_segment_and_refuse_tricks() {
        assert_eq!(clean_file_name("  photo.png ").unwrap(), "photo.png");
        assert_eq!(clean_file_name("../../etc/passwd").unwrap(), "passwd");
        assert_eq!(
            clean_file_name("C:\\Users\\me\\notes.txt").unwrap(),
            "notes.txt"
        );
        for refused in [
            "",
            "..",
            "dir/",
            "a\u{202E}gnp.exe",
            "line\nbreak",
            &"a".repeat(256),
        ] {
            assert!(clean_file_name(refused).is_err(), "{refused:?}");
        }
    }

    #[test]
    fn only_raster_images_get_a_previewable_type() {
        assert_eq!(sniff_content_type(b"\x89PNG\r\n\x1a\nrest"), "image/png");
        assert_eq!(sniff_content_type(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(sniff_content_type(b"GIF89a.."), "image/gif");
        assert_eq!(sniff_content_type(b"RIFF\0\0\0\0WEBPVP8 "), "image/webp");
        assert_eq!(sniff_content_type(b"<svg xmlns="), UNKNOWN_CONTENT_TYPE);
        assert_eq!(sniff_content_type(b"<html><script>"), UNKNOWN_CONTENT_TYPE);
        assert_eq!(sniff_content_type(b""), UNKNOWN_CONTENT_TYPE);
    }
}
