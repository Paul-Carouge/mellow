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
    #[error(
        "a message carries at most {MAXIMUM_ATTACHMENTS_PER_MESSAGE} attachments, each uploaded to its channel by its author and not sent yet"
    )]
    Attachments,
    #[error(
        "file names are 1 to {MAXIMUM_FILE_NAME_BYTES} bytes, without path separators or control characters"
    )]
    FileName,
    #[error("pictures are PNG, JPEG, GIF or WebP, at most 8192 pixels a side and 600 frames")]
    Picture,
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
