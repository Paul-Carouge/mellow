//! Naming and checking a server's emoji before upload, and why the instance refused one (the
//! "Uploading" part of `web/src/components/custom-emoji.ts`, `isValidEmojiName` of `protocol.ts`,
//! DESIGN.md §5.8, ADR-0028).

use mellow_protocol::{ApiErrorCode, CustomEmoji, Snowflake};
use unicode_normalization::UnicodeNormalization as _;
use unicode_normalization::char::is_combining_mark;

use crate::ClientError;

/// Each server holds this many still emoji, and as many animated ones (`guild_rules.rs`).
pub const MAXIMUM_EMOJIS_PER_GUILD_AND_KIND: usize = 50;
/// The largest picture the instance takes for an emoji (`guild_rules.rs`).
pub const MAXIMUM_EMOJI_UPLOAD_BYTES: u64 = 256 * 1024;
pub const MINIMUM_EMOJI_NAME_LENGTH: usize = 2;
pub const MAXIMUM_EMOJI_NAME_LENGTH: usize = 32;
const FALLBACK_EMOJI_NAME: &str = "emoji";

/// The picture types the instance takes (`PICTURE_UPLOAD_TYPES`), known here by extension as the
/// browser knows a file's type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureType {
    Png,
    Jpeg,
    Gif,
    WebP,
}

impl PictureType {
    #[must_use]
    pub fn from_file_name(file_name: &str) -> Option<Self> {
        let (_, extension) = file_name.rsplit_once('.')?;
        match extension.to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "gif" => Some(Self::Gif),
            "webp" => Some(Self::WebP),
            _ => None,
        }
    }

    /// PNG and JPEG are always still; a GIF or WebP may be either, which only the instance can tell.
    #[must_use]
    pub fn is_always_still(self) -> bool {
        matches!(self, Self::Png | Self::Jpeg)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EmojiCounts {
    pub still: usize,
    pub animated: usize,
}

impl EmojiCounts {
    #[must_use]
    pub fn is_still_full(self) -> bool {
        self.still >= MAXIMUM_EMOJIS_PER_GUILD_AND_KIND
    }

    #[must_use]
    pub fn is_animated_full(self) -> bool {
        self.animated >= MAXIMUM_EMOJIS_PER_GUILD_AND_KIND
    }
}

#[must_use]
pub fn emoji_counts(emojis: &[CustomEmoji]) -> EmojiCounts {
    let animated = emojis.iter().filter(|emoji| emoji.is_animated).count();
    EmojiCounts {
        still: emojis.len() - animated,
        animated,
    }
}

/// 2 to 32 ASCII letters, digits and `_`.
#[must_use]
pub fn is_valid_emoji_name(name: &str) -> bool {
    (MINIMUM_EMOJI_NAME_LENGTH..=MAXIMUM_EMOJI_NAME_LENGTH).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// A name from a file's name: accents dropped, every other run of characters outside letters,
/// digits and `_` made `_` ("emoji" when nothing is left), cut to 32 and padded to 2, then `_2`,
/// `_3`… while `taken_names` has it whatever the case (the instance would refuse it).
#[must_use]
pub fn emoji_name_from_file_name<'name>(
    file_name: &str,
    taken_names: impl IntoIterator<Item = &'name str>,
) -> String {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem);
    let mut readable = String::new();
    let mut is_in_gap = false;
    for character in stem
        .nfd()
        .filter(|character| !is_combining_mark(*character))
    {
        if character.is_ascii_alphanumeric() || character == '_' {
            readable.push(character);
            is_in_gap = false;
        } else if !is_in_gap {
            readable.push('_');
            is_in_gap = true;
        }
    }
    let readable = readable.trim_matches('_');
    let mut base: String = if readable.is_empty() {
        FALLBACK_EMOJI_NAME
    } else {
        readable
    }
    .chars()
    .take(MAXIMUM_EMOJI_NAME_LENGTH)
    .collect();
    while base.len() < MINIMUM_EMOJI_NAME_LENGTH {
        base.push('_');
    }
    let taken: Vec<String> = taken_names
        .into_iter()
        .map(str::to_ascii_lowercase)
        .collect();
    let is_taken = |candidate: &str| taken.contains(&candidate.to_ascii_lowercase());
    let mut candidate = base.clone();
    let mut suffix = 2_usize;
    while is_taken(&candidate) {
        let ending = format!("_{suffix}");
        // `base` is ASCII: any byte index is a character boundary.
        let kept = MAXIMUM_EMOJI_NAME_LENGTH
            .saturating_sub(ending.len())
            .min(base.len());
        candidate = format!("{}{ending}", &base[..kept]);
        suffix += 1;
    }
    candidate
}

/// Why a new name cannot be given to an emoji, checked before asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmojiRenameProblem {
    Invalid,
    Taken,
}

/// `None` lets the instance decide.
#[must_use]
pub fn emoji_rename_problem(
    name: &str,
    emoji_id: Snowflake,
    emojis: &[CustomEmoji],
) -> Option<EmojiRenameProblem> {
    if !is_valid_emoji_name(name) {
        return Some(EmojiRenameProblem::Invalid);
    }
    emojis
        .iter()
        .any(|emoji| emoji.id != emoji_id && emoji.name.eq_ignore_ascii_case(name))
        .then_some(EmojiRenameProblem::Taken)
}

/// Why a picture cannot be added, known before sending it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmojiFileProblem {
    NotAPicture,
    TooLarge,
    NoRoom,
}

/// `None` lets the instance decide.
#[must_use]
pub fn emoji_file_problem(
    file_name: &str,
    size_bytes: u64,
    counts: EmojiCounts,
) -> Option<EmojiFileProblem> {
    let Some(picture_type) = PictureType::from_file_name(file_name) else {
        return Some(EmojiFileProblem::NotAPicture);
    };
    if size_bytes > MAXIMUM_EMOJI_UPLOAD_BYTES {
        return Some(EmojiFileProblem::TooLarge);
    }
    (counts.is_still_full() && (counts.is_animated_full() || picture_type.is_always_still()))
        .then_some(EmojiFileProblem::NoRoom)
}

/// Why the instance refused a picture (DESIGN.md §8); `Other` is said as any other error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmojiUploadRefusal {
    TooLarge,
    NoRoom,
    /// Names are checked before sending: what is left is a picture it could not use.
    Unusable,
    /// The picture budget each person shares with avatars and icons (ADR-0028); the uploads
    /// waiting behind it stop too.
    TooMany,
    Other,
}

#[must_use]
pub fn emoji_upload_refusal(error: &ClientError) -> EmojiUploadRefusal {
    match error {
        ClientError::FileTooLarge => EmojiUploadRefusal::TooLarge,
        ClientError::Api { code, .. } => match code {
            ApiErrorCode::LimitReached => EmojiUploadRefusal::NoRoom,
            ApiErrorCode::ValidationFailed => EmojiUploadRefusal::Unusable,
            ApiErrorCode::RateLimited => EmojiUploadRefusal::TooMany,
            _ => EmojiUploadRefusal::Other,
        },
        _ => EmojiUploadRefusal::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from `custom-emoji.test.ts` ("uploads").

    fn emoji(id: u64, name: &str, is_animated: bool) -> CustomEmoji {
        CustomEmoji {
            id: Snowflake::from_raw(id),
            name: name.to_owned(),
            is_animated,
        }
    }

    #[test]
    fn names_a_file_after_its_name_unique_whatever_the_case() {
        let none: [&str; 0] = [];
        assert_eq!(
            emoji_name_from_file_name("party parrot.gif", none),
            "party_parrot"
        );
        assert_eq!(
            emoji_name_from_file_name("Café-crème!.png", none),
            "Cafe_creme"
        );
        assert_eq!(emoji_name_from_file_name("a.png", none), "a_");
        assert_eq!(emoji_name_from_file_name("😀.png", none), "emoji");
        let long = format!("{}.png", "x".repeat(40));
        assert_eq!(emoji_name_from_file_name(&long, none), "x".repeat(32));
        assert_eq!(
            emoji_name_from_file_name("party.png", ["PARTY", "party_2"]),
            "party_3"
        );
        let taken = "x".repeat(32);
        assert_eq!(
            emoji_name_from_file_name(&long, [taken.as_str()]),
            format!("{}_2", "x".repeat(30))
        );
    }

    #[test]
    fn counts_still_and_animated_emoji_apart() {
        let emojis = [
            emoji(1, "party", false),
            emoji(2, "parrot", true),
            emoji(3, "wave", false),
        ];
        assert_eq!(
            emoji_counts(&emojis),
            EmojiCounts {
                still: 2,
                animated: 1
            }
        );
    }

    #[test]
    fn refuses_other_files_files_over_256_kb_and_a_kind_with_no_room_left() {
        let empty = EmojiCounts::default();
        assert_eq!(emoji_file_problem("a.png", 1000, empty), None);
        assert_eq!(emoji_file_problem("a.PNG", 1000, empty), None);
        assert_eq!(
            emoji_file_problem("a.svg", 1000, empty),
            Some(EmojiFileProblem::NotAPicture)
        );
        assert_eq!(
            emoji_file_problem("picture", 1000, empty),
            Some(EmojiFileProblem::NotAPicture)
        );
        assert_eq!(
            emoji_file_problem("a.gif", 256 * 1024 + 1, empty),
            Some(EmojiFileProblem::TooLarge)
        );
        let still_full = EmojiCounts {
            still: 50,
            animated: 3,
        };
        assert_eq!(
            emoji_file_problem("a.jpg", 1000, still_full),
            Some(EmojiFileProblem::NoRoom)
        );
        // A GIF may be animated: the instance decides.
        assert_eq!(emoji_file_problem("a.gif", 1000, still_full), None);
        assert_eq!(
            emoji_file_problem(
                "a.gif",
                1000,
                EmojiCounts {
                    still: 50,
                    animated: 50
                }
            ),
            Some(EmojiFileProblem::NoRoom)
        );
    }

    #[test]
    fn says_why_the_instance_refused_a_picture() {
        let api = |code| ClientError::Api {
            status: 400,
            code,
            message: String::new(),
        };
        assert_eq!(
            emoji_upload_refusal(&ClientError::FileTooLarge),
            EmojiUploadRefusal::TooLarge
        );
        assert_eq!(
            emoji_upload_refusal(&api(ApiErrorCode::LimitReached)),
            EmojiUploadRefusal::NoRoom
        );
        assert_eq!(
            emoji_upload_refusal(&api(ApiErrorCode::ValidationFailed)),
            EmojiUploadRefusal::Unusable
        );
        assert_eq!(
            emoji_upload_refusal(&api(ApiErrorCode::RateLimited)),
            EmojiUploadRefusal::TooMany
        );
        assert_eq!(
            emoji_upload_refusal(&api(ApiErrorCode::MissingPermissions)),
            EmojiUploadRefusal::Other
        );
    }

    #[test]
    fn checks_a_new_name_before_asking() {
        let emojis = [emoji(1, "party", false), emoji(2, "party_parrot", true)];
        let party = Snowflake::from_raw(1);
        assert_eq!(emoji_rename_problem("fiesta", party, &emojis), None);
        assert_eq!(emoji_rename_problem("Party", party, &emojis), None);
        assert_eq!(
            emoji_rename_problem("PARTY_parrot", party, &emojis),
            Some(EmojiRenameProblem::Taken)
        );
        assert_eq!(
            emoji_rename_problem("a", party, &emojis),
            Some(EmojiRenameProblem::Invalid)
        );
        assert_eq!(
            emoji_rename_problem("two words", party, &emojis),
            Some(EmojiRenameProblem::Invalid)
        );
    }
}
