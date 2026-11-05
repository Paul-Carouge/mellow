//! Input rules for messages.

use mellow_protocol::{
    MAXIMUM_CHECKLIST_ITEM_LENGTH, MAXIMUM_CHECKLIST_ITEMS, MAXIMUM_CHECKLIST_TITLE_LENGTH,
    MAXIMUM_POLL_ANSWER_LENGTH, MAXIMUM_POLL_ANSWERS, MAXIMUM_POLL_QUESTION_LENGTH,
    MAXIMUM_PROGRESS_BAR_LABEL_LENGTH, MAXIMUM_PROGRESS_BAR_MAXIMUM, MINIMUM_POLL_ANSWERS,
    POLL_DURATIONS_IN_HOURS, Snowflake,
};

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
        "MP4 and QuickTime files need well-formed boxes, nested at most 8 deep, with at most 4 MiB of movie header (`moov`)"
    )]
    Media,
    #[error(
        "a reaction is exactly one Unicode emoji (at most {MAXIMUM_REACTION_EMOJI_BYTES} bytes), without text or spaces, or a custom emoji as `:id`"
    )]
    ReactionEmoji,
    #[error("a custom emoji is added as a reaction only in its own server, while it exists")]
    CustomReactionEmoji,
    #[error("pages of reacting users hold 1 to {MAXIMUM_REACTING_USERS_PAGE_SIZE} users")]
    ReactingUsersQuery,
    #[error("a forward carries no content, attachments or reply of its own")]
    ForwardWithItsOwnContent,
    #[error("a forwarded message cannot be edited")]
    ForwardedMessageEdit,
    #[error(
        "a poll asks a question of 1 to {MAXIMUM_POLL_QUESTION_LENGTH} characters with {MINIMUM_POLL_ANSWERS} to {MAXIMUM_POLL_ANSWERS} different answers of 1 to {MAXIMUM_POLL_ANSWER_LENGTH} characters, for 1, 4, 8, 24, 72, 168 or 336 hours, and carries no text, files, reply or forward"
    )]
    Poll,
    #[error("a poll cannot be edited or forwarded")]
    PollChange,
    #[error("this poll has ended")]
    PollEnded,
    #[error(
        "a checklist has a title of 1 to {MAXIMUM_CHECKLIST_TITLE_LENGTH} characters and 1 to {MAXIMUM_CHECKLIST_ITEMS} items of 1 to {MAXIMUM_CHECKLIST_ITEM_LENGTH} characters, and carries no text, files, reply, forward or poll"
    )]
    Checklist,
    #[error(
        "a progress bar has a label of 1 to {MAXIMUM_PROGRESS_BAR_LABEL_LENGTH} characters and a value from 0 to its maximum, itself 1 to {MAXIMUM_PROGRESS_BAR_MAXIMUM}, and carries no text, files, reply, forward or poll"
    )]
    ProgressBar,
    #[error("a checklist or a progress bar changes through its own routes and cannot be forwarded")]
    TrackerChange,
    #[error(transparent)]
    SearchQuery(#[from] crate::domain::search_query::SearchQueryViolation),
}

pub const MAXIMUM_ATTACHMENTS_PER_MESSAGE: usize = 10;
pub const MAXIMUM_FILE_NAME_BYTES: usize = 255;
/// Shown as `inline` previews: only raster images whose bytes say so (ADR-0020); AVIF is what
/// compressed pictures are mostly stored as (ADR-0033).
pub const PREVIEWABLE_IMAGE_TYPES: [&str; 5] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
];
/// Played `inline` by the clients' media players (ADR-0038): only containers whose bytes say so.
pub const PLAYABLE_MEDIA_TYPES: [&str; 9] = [
    "video/mp4",
    "video/quicktime",
    "video/webm",
    "audio/mpeg",
    "audio/aac",
    "audio/mp4",
    "audio/ogg",
    "audio/wav",
    "audio/flac",
];
pub const UNKNOWN_CONTENT_TYPE: &str = "application/octet-stream";

/// ISO-BMFF major brands of ordinary video files (MP4, M4V, phone recordings).
const MP4_VIDEO_BRANDS: [&[u8; 4]; 11] = [
    b"isom", b"iso2", b"iso4", b"iso5", b"iso6", b"mp41", b"mp42", b"avc1", b"M4V ", b"dash",
    b"mmp4",
];
/// Compatible brands that make a file of any other major brand (cameras write their own, such as
/// Sony's `XAVC` or Apple's `MSNV`) an ordinary video.
const MP4_COMPATIBLE_VIDEO_BRANDS: [&[u8; 4]; 3] = [b"isom", b"mp41", b"mp42"];
/// Brands of ISO-BMFF pictures that may list `isom` too: HEIF and AVIF pictures and image
/// sequences, and Canon CR3 raw photos.
const ISO_BMFF_PICTURE_BRANDS: [&[u8; 4]; 7] = [
    b"mif1", b"msf1", b"heic", b"heix", b"avif", b"avis", b"crx ",
];
/// Compatible brands read from an `ftyp` box; real files list a handful.
const MAXIMUM_COMPATIBLE_BRANDS_READ: usize = 16;
/// ISO-BMFF major brands of audio-only files (iTunes AAC and audiobooks).
const MP4_AUDIO_BRANDS: [&[u8; 4]; 2] = [b"M4A ", b"M4B "];
/// The EBML magic number Matroska and `WebM` files start with.
const EBML_MAGIC: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];
/// How far into an EBML header its `DocType` is looked for: real headers hold it in the first
/// few dozen bytes.
const EBML_DOCTYPE_SEARCH_BYTES: usize = 64;

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
/// never trusted. Anything but the five raster formats and the playable media containers is
/// `application/octet-stream`.
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
    } else if bytes.get(4..12) == Some(b"ftypavif") || bytes.get(4..12) == Some(b"ftypavis") {
        // An ISO-BMFF file whose major brand is AVIF, still or sequence.
        "image/avif"
    } else {
        sniff_media_type(bytes).unwrap_or(UNKNOWN_CONTENT_TYPE)
    }
}

/// Whether an `ftyp` box whose major brand is unknown lists `isom`, `mp41` or `mp42` among its
/// first compatible brands, and no picture brand (HEIF, AVIF, CR3).
fn is_compatible_with_ordinary_video(bytes: &[u8]) -> bool {
    const FIRST_COMPATIBLE_BRAND_OFFSET: usize = 16;
    let Some(box_size) = bytes
        .first_chunk::<4>()
        .and_then(|size| usize::try_from(u32::from_be_bytes(*size)).ok())
    else {
        return false;
    };
    let end = box_size
        .min(bytes.len())
        .min(FIRST_COMPATIBLE_BRAND_OFFSET + 4 * MAXIMUM_COMPATIBLE_BRANDS_READ);
    let (brands, _) = bytes
        .get(FIRST_COMPATIBLE_BRAND_OFFSET..end)
        .unwrap_or_default()
        .as_chunks::<4>();
    let lists = |known: &[&[u8; 4]]| brands.iter().any(|brand| known.contains(&brand));
    let major_brand = bytes.get(8..12).unwrap_or_default();
    let is_picture = lists(&ISO_BMFF_PICTURE_BRANDS)
        || ISO_BMFF_PICTURE_BRANDS
            .iter()
            .any(|known| known.as_slice() == major_brand);
    lists(&MP4_COMPATIBLE_VIDEO_BRANDS) && !is_picture
}

/// The playable audio or video container these bytes start with (ADR-0038), if any. Checked
/// after the pictures: a JPEG also starts with `0xFF`, as MPEG audio frames do.
fn sniff_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.get(4..8) == Some(b"ftyp") {
        let brand = bytes.get(8..12)?;
        return if MP4_VIDEO_BRANDS
            .iter()
            .any(|known| known.as_slice() == brand)
        {
            Some("video/mp4")
        } else if MP4_AUDIO_BRANDS
            .iter()
            .any(|known| known.as_slice() == brand)
        {
            Some("audio/mp4")
        } else if brand == b"qt  " {
            Some("video/quicktime")
        } else if is_compatible_with_ordinary_video(bytes) {
            Some("video/mp4")
        } else {
            None
        };
    }
    if bytes.starts_with(&EBML_MAGIC) {
        let header = &bytes[..bytes.len().min(EBML_DOCTYPE_SEARCH_BYTES)];
        // The DocType element (0x4282), one-byte size 0x84, then "webm"; Matroska stays a download.
        let is_webm = header
            .windows(7)
            .any(|window| window == b"\x42\x82\x84webm");
        return is_webm.then_some("video/webm");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        Some("audio/wav")
    } else if bytes.starts_with(b"fLaC") {
        Some("audio/flac")
    } else if bytes.starts_with(b"OggS") {
        Some("audio/ogg")
    } else if bytes.starts_with(b"ID3") || is_mpeg_layer_three_frame(bytes) {
        Some("audio/mpeg")
    } else if is_adts_frame(bytes) {
        Some("audio/aac")
    } else {
        None
    }
}

/// An MPEG audio frame header: 11 sync bits, any version, layer III (`01`).
fn is_mpeg_layer_three_frame(bytes: &[u8]) -> bool {
    matches!(bytes, [0xFF, second, ..] if second & 0xE0 == 0xE0 && second & 0x06 == 0x02)
}

/// An AAC ADTS frame header: 12 sync bits, then layer `00`.
fn is_adts_frame(bytes: &[u8]) -> bool {
    matches!(bytes, [0xFF, second, ..] if second & 0xF6 == 0xF0)
}

/// Windows clients send `\r\n`; messages store `\n` only.
#[must_use]
pub fn normalize_line_endings(content: &str) -> String {
    content.replace("\r\n", "\n")
}

/// Bidirectional overrides and isolates, tags and interlinear annotations: they can make a link
/// or text read differently from what it is (Trojan Source, CWE-451). Zero-width joiners and
/// variation selectors stay allowed: emoji sequences need them.
pub(crate) fn is_forbidden_in_messages(character: char) -> bool {
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

/// A poll's question and answers, trimmed, once they are within the limits of ADR-0043: visible,
/// without control or bidirectional characters, answers different from each other (case ignored).
///
/// # Errors
/// `Poll` otherwise.
pub fn validated_poll(
    poll: &mellow_protocol::NewPoll,
) -> Result<(String, Vec<String>), MessageRuleViolation> {
    let is_valid_text = |text: &str, maximum: usize| {
        let length = text.chars().count();
        (1..=maximum).contains(&length)
            && validate_message_content(text).is_ok()
            && !text.contains('\n')
    };
    // Counted before any copy: a body may hold thousands of empty answers.
    if !(MINIMUM_POLL_ANSWERS..=MAXIMUM_POLL_ANSWERS).contains(&poll.answers.len()) {
        return Err(MessageRuleViolation::Poll);
    }
    let question = poll.question.trim().to_owned();
    let answers: Vec<String> = poll
        .answers
        .iter()
        .map(|answer| answer.trim().to_owned())
        .collect();
    let mut folded: Vec<String> = answers.iter().map(|answer| answer.to_lowercase()).collect();
    folded.sort_unstable();
    folded.dedup();
    let is_valid = is_valid_text(&question, MAXIMUM_POLL_QUESTION_LENGTH)
        && (MINIMUM_POLL_ANSWERS..=MAXIMUM_POLL_ANSWERS).contains(&answers.len())
        && folded.len() == answers.len()
        && answers
            .iter()
            .all(|answer| is_valid_text(answer, MAXIMUM_POLL_ANSWER_LENGTH))
        && POLL_DURATIONS_IN_HOURS.contains(&poll.duration_hours);
    if is_valid {
        Ok((question, answers))
    } else {
        Err(MessageRuleViolation::Poll)
    }
}

/// One line of visible text of 1 to `maximum` characters, trimmed, as poll answers are.
fn trimmed_single_line(text: &str, maximum: usize) -> Option<String> {
    let text = text.trim();
    let is_valid = (1..=maximum).contains(&text.chars().count())
        && validate_message_content(text).is_ok()
        && !text.contains('\n');
    is_valid.then(|| text.to_owned())
}

/// A checklist's title and items, trimmed, once within the limits of ADR-0047. Items may repeat.
///
/// # Errors
/// `Checklist` otherwise.
pub fn validated_checklist(
    checklist: &mellow_protocol::NewChecklist,
) -> Result<mellow_protocol::NewChecklist, MessageRuleViolation> {
    // Counted before any copy: a body may hold thousands of empty items.
    if !(1..=MAXIMUM_CHECKLIST_ITEMS).contains(&checklist.items.len()) {
        return Err(MessageRuleViolation::Checklist);
    }
    Ok(mellow_protocol::NewChecklist {
        title: trimmed_single_line(&checklist.title, MAXIMUM_CHECKLIST_TITLE_LENGTH)
            .ok_or(MessageRuleViolation::Checklist)?,
        items: checklist
            .items
            .iter()
            .map(|item| validated_checklist_item(item))
            .collect::<Result<_, _>>()?,
    })
}

/// # Errors
/// `Checklist` unless one line of 1 to `MAXIMUM_CHECKLIST_ITEM_LENGTH` visible characters.
pub fn validated_checklist_item(text: &str) -> Result<String, MessageRuleViolation> {
    trimmed_single_line(text, MAXIMUM_CHECKLIST_ITEM_LENGTH).ok_or(MessageRuleViolation::Checklist)
}

/// A progress bar with its label trimmed, once within the limits of ADR-0047.
///
/// # Errors
/// `ProgressBar` otherwise.
pub fn validated_progress_bar(
    bar: &mellow_protocol::NewProgressBar,
) -> Result<mellow_protocol::NewProgressBar, MessageRuleViolation> {
    let label = trimmed_single_line(&bar.label, MAXIMUM_PROGRESS_BAR_LABEL_LENGTH)
        .ok_or(MessageRuleViolation::ProgressBar)?;
    if !(1..=MAXIMUM_PROGRESS_BAR_MAXIMUM).contains(&bar.maximum) || bar.value > bar.maximum {
        return Err(MessageRuleViolation::ProgressBar);
    }
    Ok(mellow_protocol::NewProgressBar {
        label,
        value: bar.value,
        maximum: bar.maximum,
    })
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

/// The id of a custom emoji's reaction key, `:id` (ADR-0028), the id in its one decimal
/// spelling; `None` for anything else, Unicode emoji included (none starts with a colon).
#[must_use]
pub fn custom_emoji_of_reaction_key(key: &str) -> Option<Snowflake> {
    key.strip_prefix(':')?.parse().ok()
}

/// The key a reaction is stored and counted under: a custom emoji's `:id` as sent (at most 21
/// bytes), or exactly one Unicode emoji, at most `MAXIMUM_REACTION_EMOJI_BYTES` as sent,
/// without its variation selectors, so `❤️` and `❤` are one reaction.
///
/// # Errors
/// `ReactionEmoji` for anything else (text, spaces, several emoji, stray joiners or selectors).
pub fn reaction_emoji_key(emoji: &str) -> Result<String, MessageRuleViolation> {
    if custom_emoji_of_reaction_key(emoji).is_some() {
        return Ok(emoji.to_owned());
    }
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

    fn poll(question: &str, answers: &[&str], duration_hours: u32) -> mellow_protocol::NewPoll {
        mellow_protocol::NewPoll {
            question: question.into(),
            answers: answers.iter().map(|answer| (*answer).into()).collect(),
            allows_multiple_answers: false,
            duration_hours,
        }
    }

    #[test]
    fn checklists_have_a_visible_title_and_one_to_twenty_five_single_line_items() {
        let checklist = |title: &str, items: &[&str]| mellow_protocol::NewChecklist {
            title: title.into(),
            items: items.iter().map(|item| (*item).to_owned()).collect(),
        };
        assert_eq!(
            validated_checklist(&checklist(" Release ", &[" Tag", "Tag"])),
            Ok(checklist("Release", &["Tag", "Tag"])),
            "trimmed; items may repeat"
        );
        let too_many: Vec<&str> = vec!["x"; MAXIMUM_CHECKLIST_ITEMS + 1];
        let too_long = "x".repeat(MAXIMUM_CHECKLIST_ITEM_LENGTH + 1);
        for refused in [
            checklist("", &["a"]),
            checklist("t", &[]),
            checklist("t", &too_many),
            checklist("t", &["a", " "]),
            checklist("t", &[&too_long]),
            checklist("t", &["two\nlines"]),
            checklist("t", &["a\u{202E}"]),
            checklist(&"t".repeat(MAXIMUM_CHECKLIST_TITLE_LENGTH + 1), &["a"]),
        ] {
            assert_eq!(
                validated_checklist(&refused),
                Err(MessageRuleViolation::Checklist),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn progress_bars_hold_a_value_from_zero_to_their_maximum() {
        let bar = |label: &str, value, maximum| mellow_protocol::NewProgressBar {
            label: label.into(),
            value,
            maximum,
        };
        assert_eq!(
            validated_progress_bar(&bar(" Sync ", 0, 100)),
            Ok(bar("Sync", 0, 100))
        );
        assert_eq!(
            validated_progress_bar(&bar("Sync", 5, 5)),
            Ok(bar("Sync", 5, 5))
        );
        for refused in [
            bar("", 1, 2),
            bar("Sync", 6, 5),
            bar("Sync", 0, 0),
            bar("Sync", 0, MAXIMUM_PROGRESS_BAR_MAXIMUM + 1),
        ] {
            assert_eq!(
                validated_progress_bar(&refused),
                Err(MessageRuleViolation::ProgressBar),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn polls_ask_a_visible_question_with_two_to_ten_different_answers_for_a_set_time() {
        assert_eq!(
            validated_poll(&poll("  Lunch? ", &[" Pizza", "Soup "], 24)).unwrap(),
            (
                "Lunch?".to_owned(),
                vec!["Pizza".to_owned(), "Soup".to_owned()]
            )
        );
        let refused = [
            poll("", &["a", "b"], 24),
            poll("q", &["only one"], 24),
            poll("q", &["same", "SAME"], 24),
            poll("q", &["a", ""], 24),
            poll("q", &["a", "b"], 5),
            poll("q", &["a", &"x".repeat(56)], 24),
            poll(&"q".repeat(301), &["a", "b"], 24),
            poll("q", &["a", "b\u{202E}"], 24),
            poll("two\nlines", &["a", "b"], 24),
            poll(
                "q",
                &["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"],
                24,
            ),
        ];
        for poll in refused {
            assert_eq!(
                validated_poll(&poll),
                Err(MessageRuleViolation::Poll),
                "{poll:?}"
            );
        }
    }

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
    fn custom_emoji_react_as_a_colon_and_their_canonical_id() {
        assert_eq!(reaction_emoji_key(":123"), Ok(":123".to_owned()));
        assert_eq!(
            custom_emoji_of_reaction_key(":18446744073709551615"),
            Some(Snowflake::from_raw(u64::MAX))
        );
        for invalid in [
            ":",
            "123",
            "party:123",
            ":0123",
            ":-1",
            ":18446744073709551616",
            "::1",
            ":1 ",
            "<:ok:1>",
        ] {
            assert_eq!(custom_emoji_of_reaction_key(invalid), None, "{invalid:?}");
            assert!(reaction_emoji_key(invalid).is_err(), "{invalid:?}");
        }
        assert_eq!(custom_emoji_of_reaction_key("👍"), None);
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
        assert_eq!(sniff_content_type(b"\0\0\0\x1cftypavif\0\0"), "image/avif");
        assert_eq!(
            sniff_content_type(b"\0\0\0\x1cftypheic\0\0"),
            UNKNOWN_CONTENT_TYPE
        );
        assert_eq!(sniff_content_type(b"<svg xmlns="), UNKNOWN_CONTENT_TYPE);
        assert_eq!(sniff_content_type(b"<html><script>"), UNKNOWN_CONTENT_TYPE);
        assert_eq!(sniff_content_type(b""), UNKNOWN_CONTENT_TYPE);
    }

    #[test]
    fn common_audio_and_video_containers_get_a_playable_type() {
        let cases: [(&[u8], &str); 12] = [
            (b"\0\0\0\x20ftypisom\0\0\x02\0", "video/mp4"),
            (b"\0\0\0\x18ftypmp42\0\0\0\0", "video/mp4"),
            (b"\0\0\0\x14ftypqt  \0\0\0\0", "video/quicktime"),
            (b"\0\0\0\x20ftypM4A \0\0\0\0", "audio/mp4"),
            (
                b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\x42\x82\x84webm\x42\x87",
                "video/webm",
            ),
            (b"RIFF\0\0\0\0WAVEfmt ", "audio/wav"),
            (b"fLaC\0\0\0\x22", "audio/flac"),
            (b"OggS\0\x02\0\0", "audio/ogg"),
            (b"ID3\x04\0\0\0\0", "audio/mpeg"),
            (&[0xFF, 0xFB, 0x90, 0x64], "audio/mpeg"),
            (&[0xFF, 0xF3, 0x90, 0x64], "audio/mpeg"),
            (&[0xFF, 0xF1, 0x50, 0x80], "audio/aac"),
        ];
        for (bytes, expected) in cases {
            assert_eq!(sniff_content_type(bytes), expected, "{bytes:?}");
            assert!(PLAYABLE_MEDIA_TYPES.contains(&expected), "{expected}");
        }
    }

    #[test]
    fn lookalike_media_headers_stay_downloads() {
        let refused: [&[u8]; 5] = [
            b"\x1a\x45\xdf\xa3\x9f\x42\x82\x88matroska",
            b"\0\0\0\x08ftyp",
            &[0xFF, 0xFD, 0x90, 0x64],
            b"RIFF\0\0\0\0AVI LIST",
            b"Ogg",
        ];
        for bytes in refused {
            assert_eq!(sniff_content_type(bytes), UNKNOWN_CONTENT_TYPE, "{bytes:?}");
        }
    }

    #[test]
    fn a_camera_brand_is_a_video_when_it_lists_an_ordinary_compatible_brand() {
        // Sony's XAVC and Apple's MSNV, compatible with isom, mp42 or mp41.
        let videos: [&[u8]; 3] = [
            b"\0\0\0\x18ftypXAVC\0\0\0\0isom",
            b"\0\0\0\x1cftypMSNV\0\0\x02\0MSNVmp42",
            b"\0\0\0\x20ftypxxxx\0\0\0\0aaaabbbbmp41",
        ];
        for bytes in videos {
            assert_eq!(sniff_content_type(bytes), "video/mp4", "{bytes:?}");
        }
    }

    #[test]
    fn heif_and_avif_pictures_listing_isom_are_not_videos() {
        let pictures: [&[u8]; 4] = [
            b"\0\0\0\x1cftypheic\0\0\0\0mif1isom",
            b"\0\0\0\x1cftypmif1\0\0\0\0heicisom",
            b"\0\0\0\x20ftypmif1\0\0\0\0avifmiafisom",
            b"\0\0\0\x1cftypmsf1\0\0\0\0avisisom",
        ];
        for bytes in pictures {
            assert_eq!(sniff_content_type(bytes), UNKNOWN_CONTENT_TYPE, "{bytes:?}");
        }
    }

    #[test]
    fn canon_raw_pictures_listing_isom_stay_downloads() {
        // CR3: major brand `crx `, compatible `crx ` and `isom`; its GPS sits in Canon `uuid` boxes.
        let pictures: [&[u8]; 2] = [
            b"\0\0\0\x18ftypcrx \0\0\0\x01crx isom",
            b"\0\0\0\x18ftypXAVC\0\0\0\x01isomcrx ",
        ];
        for bytes in pictures {
            assert_eq!(sniff_content_type(bytes), UNKNOWN_CONTENT_TYPE, "{bytes:?}");
        }
    }

    #[test]
    fn compatible_brands_are_read_within_the_box_and_only_the_first_sixteen() {
        // The box ends before `isom`, which belongs to the next box.
        assert_eq!(
            sniff_content_type(b"\0\0\0\x10ftypXAVC\0\0\0\0isom"),
            UNKNOWN_CONTENT_TYPE
        );
        let mut seventeenth = b"\0\0\0\x54ftypXAVC\0\0\0\0".to_vec();
        seventeenth.extend_from_slice(&b"XAVC".repeat(16));
        seventeenth.extend_from_slice(b"isom");
        assert_eq!(sniff_content_type(&seventeenth), UNKNOWN_CONTENT_TYPE);
    }
}
