//! Input rules for messages.

use crate::domain::account_rules::is_invisible_format_character;

pub const MAXIMUM_MESSAGE_LENGTH: usize = 4_000;
/// Also bounded in bytes: 4 000 four-byte emoji would otherwise store 16 KB per message.
pub const MAXIMUM_MESSAGE_BYTES: usize = 8_000;
pub const MAXIMUM_NONCE_LENGTH: usize = 64;
pub const DEFAULT_HISTORY_PAGE_SIZE: u32 = 50;
pub const MAXIMUM_HISTORY_PAGE_SIZE: u32 = 100;

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
}
