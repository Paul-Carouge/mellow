//! How much a draft may still hold (`web/src/components/message-length.ts`).

use super::unicode_classes::js_trim;

/// The instance's bounds (`domain::message_rules` on the server).
pub const MAXIMUM_MESSAGE_LENGTH: usize = 4_000;
pub const MAXIMUM_MESSAGE_BYTES: usize = 8_000;
/// Below this many characters left, the composer shows the count.
pub const MESSAGE_LENGTH_COUNTER_THRESHOLD: i64 = 500;

/// How many characters the message may still take, negative when it is too long. Counted as the
/// instance does: Unicode scalar values of the trimmed text, and at most
/// [`MAXIMUM_MESSAGE_BYTES`] of UTF-8, whichever runs out first (only emoji-heavy text reaches
/// the byte bound).
#[must_use]
pub fn message_characters_left(draft: &str) -> i64 {
    let text = js_trim(draft);
    let as_count = |value: usize| i64::try_from(value).unwrap_or(i64::MAX);
    let characters_left = as_count(MAXIMUM_MESSAGE_LENGTH) - as_count(text.chars().count());
    let bytes_left = as_count(MAXIMUM_MESSAGE_BYTES) - as_count(text.len());
    characters_left.min(bytes_left)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: i64 = 4_000;

    #[test]
    fn counts_trimmed_characters_an_emoji_as_one() {
        assert_eq!(message_characters_left("  hi  "), LIMIT - 2);
        assert_eq!(message_characters_left("🙂"), LIMIT - 1);
    }

    #[test]
    fn goes_negative_past_the_instances_limit() {
        assert_eq!(
            message_characters_left(&"x".repeat(MAXIMUM_MESSAGE_LENGTH)),
            0
        );
        assert_eq!(message_characters_left(&"x".repeat(4_419)), -419);
    }

    #[test]
    fn runs_out_on_bytes_first_for_four_byte_emoji() {
        // 2 001 emoji are 8 004 bytes: 4 over the byte bound while far under the character one.
        assert_eq!(message_characters_left(&"🙂".repeat(2_001)), -4);
    }
}
