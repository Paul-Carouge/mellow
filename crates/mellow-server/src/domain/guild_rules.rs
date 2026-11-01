//! Input rules for guilds, channels and invites.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use mellow_protocol::{
    ChannelKind, ChannelPlacement, InitialCategory, InitialTextChannel,
    MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH, MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS,
    MAXIMUM_VOICE_CHANNEL_PARTICIPANTS, Permissions, Snowflake,
};

use crate::domain::account_rules::is_invisible_format_character;
use crate::domain::sound_rules::{
    MAXIMUM_SOUND_DURATION_MILLISECONDS, MAXIMUM_SOUND_NAME_LENGTH, MINIMUM_SOUND_NAME_LENGTH,
};

pub const MINIMUM_GUILD_NAME_LENGTH: usize = 2;
pub const MAXIMUM_GUILD_NAME_LENGTH: usize = 100;
pub const MAXIMUM_CHANNEL_NAME_LENGTH: usize = 100;
pub const MAXIMUM_CHANNEL_TOPIC_LENGTH: usize = 1024;
pub const MAXIMUM_INVITE_USES: u32 = 10_000;
pub const MAXIMUM_INVITE_LIFETIME: Duration = Duration::from_hours(30 * 24);
/// Symbols of an invite code, without its dashes: three groups of four, as in `AE8C-78A9-K2QF`.
pub const INVITE_CODE_SYMBOLS: usize = 12;
const INVITE_CODE_GROUP_LENGTH: usize = 4;
/// Quotas, as on Discord, so one account cannot grow the database or a response without bound.
pub const MAXIMUM_CHANNELS_PER_GUILD: u32 = 500;
pub const MAXIMUM_OWNED_GUILDS_PER_USER: u32 = 100;
/// What `CreateGuildRequest::initial_categories` may hold.
pub const MAXIMUM_INITIAL_CATEGORIES: usize = 10;
pub const MAXIMUM_INITIAL_TEXT_CHANNELS: usize = 50;
pub const MAXIMUM_ACTIVE_INVITES_PER_GUILD: u32 = 1_000;
/// `@everyone` included.
pub const MAXIMUM_ROLES_PER_GUILD: u32 = 250;
pub const MAXIMUM_ROLE_NAME_LENGTH: usize = 100;
pub const MAXIMUM_ROLE_COLOR: u32 = 0xFF_FFFF;
pub const MAXIMUM_OVERWRITES_PER_CHANNEL: u32 = 100;
pub const MAXIMUM_BAN_REASON_LENGTH: usize = 512;
pub const DEFAULT_AUDIT_LOG_PAGE_SIZE: u32 = 50;
pub const MAXIMUM_AUDIT_LOG_PAGE_SIZE: u32 = 100;
/// shortcut: the ban list is capped, not paginated; paginate with the bans screen.
pub const MAXIMUM_BANS_LISTED: u32 = 1_000;
/// shortcut: the member list is capped, not paginated; paginate with the member list UI.
pub const MAXIMUM_MEMBERS_LISTED: u32 = 1_000;
/// Custom emoji (ADR-0028), as on Discord: each guild holds this many still and as many animated.
pub const MAXIMUM_EMOJIS_PER_GUILD_AND_KIND: u32 = 50;
pub const MINIMUM_EMOJI_NAME_LENGTH: usize = 2;
pub const MAXIMUM_EMOJI_NAME_LENGTH: usize = 32;
/// Uploaded emoji pictures larger than this are refused before processing.
pub const MAXIMUM_EMOJI_UPLOAD_BYTES: usize = 256 * 1024;
/// A processed emoji larger than this (its still frame included) is refused.
pub const MAXIMUM_EMOJI_STORED_BYTES: usize = 512 * 1024;
/// Crockford's base 32: digits and capitals without I, L, O and U, so a code read aloud or
/// copied by hand is not mistaken. 256 is a multiple of 32, so `byte % 32` has no bias.
const INVITE_CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GuildRuleViolation {
    #[error(
        "guild names are {MINIMUM_GUILD_NAME_LENGTH} to {MAXIMUM_GUILD_NAME_LENGTH} characters, without control characters"
    )]
    GuildName,
    #[error(
        "channel and category names are 1 to {MAXIMUM_CHANNEL_NAME_LENGTH} characters, without hidden characters"
    )]
    ChannelName,
    #[error(
        "channel topics are at most {MAXIMUM_CHANNEL_TOPIC_LENGTH} characters, without control characters"
    )]
    Topic,
    #[error(
        "a channel's parent must be a category of the same guild, and categories cannot be nested"
    )]
    Parent,
    #[error("invites allow 1 to {MAXIMUM_INVITE_USES} uses and last at most 30 days")]
    InviteLimits,
    #[error(
        "role names are 1 to {MAXIMUM_ROLE_NAME_LENGTH} characters, without control characters"
    )]
    RoleName,
    #[error("role colours are 0xRRGGBB values, 0 for none")]
    RoleColor,
    #[error("unknown permission bits")]
    UnknownPermissions,
    #[error("a conversation needs another user")]
    DirectMessageRecipient,
    #[error(
        "a group starts with 2 to {} other people, each named once",
        MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS - 1
    )]
    GroupConversationRecipients,
    #[error(
        "group names are at most {MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH} characters, without hidden characters"
    )]
    GroupConversationName,
    #[error(
        "ban reasons are at most {MAXIMUM_BAN_REASON_LENGTH} characters, without control characters"
    )]
    BanReason,
    #[error("audit log pages hold 1 to {MAXIMUM_AUDIT_LOG_PAGE_SIZE} entries")]
    AuditLogLimit,
    #[error("you cannot kick or ban yourself; leave the guild instead")]
    ModerationTarget,
    #[error("an overwrite cannot allow and deny the same permission")]
    OverwriteAllowsAndDeniesOneBit,
    #[error("this permission applies to the whole guild and cannot be set for one channel")]
    OverwriteOutsideChannelScope,
    #[error("role_ids must list every role of the guild except @everyone, once each")]
    RoleOrder,
    #[error(
        "channels must list every channel you can view, once each, categories at the top level and other channels in one of the listed categories or none"
    )]
    ChannelOrder,
    #[error(
        "a new server starts with at most {MAXIMUM_INITIAL_CATEGORIES} categories and {MAXIMUM_INITIAL_TEXT_CHANNELS} channels"
    )]
    InitialChannels,
    #[error("@everyone keeps its name and colour, and cannot be deleted, given or taken away")]
    EveryoneRole,
    #[error("categories have no icon")]
    CategoryIcon,
    #[error(
        "emoji names are {MINIMUM_EMOJI_NAME_LENGTH} to {MAXIMUM_EMOJI_NAME_LENGTH} ASCII letters, digits and underscores"
    )]
    EmojiName,
    #[error("another emoji of this server already has this name")]
    EmojiNameTaken,
    #[error(
        "only voice channels have a user limit, from 1 to {MAXIMUM_VOICE_CHANNEL_PARTICIPANTS}"
    )]
    UserLimit,
    #[error(
        "sound names are {MINIMUM_SOUND_NAME_LENGTH} to {MAXIMUM_SOUND_NAME_LENGTH} characters, without hidden ones"
    )]
    SoundName,
    #[error("a sound's emoji is one emoji, or one of this server's custom emoji")]
    SoundEmoji,
    #[error(
        "a sound is a PCM WAV, mono, 48 kHz, 16-bit, with a 44-byte header and at most {MAXIMUM_SOUND_DURATION_MILLISECONDS} ms of samples"
    )]
    SoundFile,
}

/// The user limit a channel of `kind` may be given: none outside voice channels, 1 to the hard
/// cap in them (ADR-0030).
///
/// # Errors
/// `UserLimit` for a limit on another kind, or out of range.
pub fn validate_user_limit(
    kind: ChannelKind,
    user_limit: Option<u8>,
) -> Result<Option<u8>, GuildRuleViolation> {
    match (kind, user_limit) {
        (_, None) => Ok(None),
        (ChannelKind::Voice, Some(limit))
            if (1..=MAXIMUM_VOICE_CHANNEL_PARTICIPANTS).contains(&limit) =>
        {
            Ok(Some(limit))
        }
        _ => Err(GuildRuleViolation::UserLimit),
    }
}

fn has_hidden_characters(text: &str) -> bool {
    text.chars()
        .any(|character| character.is_control() || is_invisible_format_character(character))
}

pub(crate) fn trimmed_within(text: &str, minimum: usize, maximum: usize) -> Option<&str> {
    let trimmed = text.trim();
    let length = trimmed.chars().count();
    ((minimum..=maximum).contains(&length) && !has_hidden_characters(trimmed)).then_some(trimmed)
}

/// # Errors
/// `GuildName` when too short, too long or containing hidden characters.
pub fn normalize_guild_name(name: &str) -> Result<String, GuildRuleViolation> {
    trimmed_within(name, MINIMUM_GUILD_NAME_LENGTH, MAXIMUM_GUILD_NAME_LENGTH)
        .map(str::to_owned)
        .ok_or(GuildRuleViolation::GuildName)
}

/// A group's name; empty clears it (ADR-0026).
///
/// # Errors
/// `GroupConversationName` when too long or containing hidden characters.
pub fn normalize_group_conversation_name(name: &str) -> Result<String, GuildRuleViolation> {
    trimmed_within(name, 0, MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH)
        .map(str::to_owned)
        .ok_or(GuildRuleViolation::GroupConversationName)
}

/// Channel and category names are kept as typed, capitals and spaces included (owner's
/// decision, 2026-10-28: unlike Discord, text channels are not lowercased and hyphenated).
///
/// # Errors
/// `ChannelName` when empty, too long or containing hidden characters.
pub fn normalize_channel_name(name: &str) -> Result<String, GuildRuleViolation> {
    trimmed_within(name, 1, MAXIMUM_CHANNEL_NAME_LENGTH)
        .map(str::to_owned)
        .ok_or(GuildRuleViolation::ChannelName)
}

/// # Errors
/// `Topic` when too long or containing hidden characters.
pub fn normalize_topic(topic: &str) -> Result<String, GuildRuleViolation> {
    trimmed_within(topic, 0, MAXIMUM_CHANNEL_TOPIC_LENGTH)
        .map(str::to_owned)
        .ok_or(GuildRuleViolation::Topic)
}

/// # Errors
/// `InviteLimits` for zero or too many uses, or a lifetime of zero or over 30 days.
pub fn validate_invite_limits(
    max_uses: Option<u32>,
    expires_in_seconds: Option<u32>,
) -> Result<(), GuildRuleViolation> {
    let are_uses_valid = max_uses.is_none_or(|uses| (1..=MAXIMUM_INVITE_USES).contains(&uses));
    let is_lifetime_valid = expires_in_seconds.is_none_or(|seconds| {
        seconds > 0 && u64::from(seconds) <= MAXIMUM_INVITE_LIFETIME.as_secs()
    });
    if are_uses_valid && is_lifetime_valid {
        Ok(())
    } else {
        Err(GuildRuleViolation::InviteLimits)
    }
}

/// # Errors
/// `AuditLogLimit` for a limit outside 1..=100.
pub fn audit_log_page_size(requested: Option<u32>) -> Result<u32, GuildRuleViolation> {
    let size = requested.unwrap_or(DEFAULT_AUDIT_LOG_PAGE_SIZE);
    (1..=MAXIMUM_AUDIT_LOG_PAGE_SIZE)
        .contains(&size)
        .then_some(size)
        .ok_or(GuildRuleViolation::AuditLogLimit)
}

/// # Errors
/// `BanReason` when too long or containing hidden characters.
pub fn normalize_ban_reason(reason: &str) -> Result<String, GuildRuleViolation> {
    trimmed_within(reason, 0, MAXIMUM_BAN_REASON_LENGTH)
        .map(str::to_owned)
        .ok_or(GuildRuleViolation::BanReason)
}

/// Whether `name` may name a custom emoji: ASCII letters, digits and `_` only, so it reads the
/// same in `:name:` and `<:name:id>` (ADR-0028).
#[must_use]
pub fn is_valid_emoji_name(name: &str) -> bool {
    (MINIMUM_EMOJI_NAME_LENGTH..=MAXIMUM_EMOJI_NAME_LENGTH).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// # Errors
/// `EmojiName` unless `is_valid_emoji_name`, once trimmed.
pub fn normalize_emoji_name(name: &str) -> Result<String, GuildRuleViolation> {
    let trimmed = name.trim();
    if is_valid_emoji_name(trimmed) {
        Ok(trimmed.to_owned())
    } else {
        Err(GuildRuleViolation::EmojiName)
    }
}

/// # Errors
/// `RoleName` when empty, too long or containing hidden characters.
pub fn normalize_role_name(name: &str) -> Result<String, GuildRuleViolation> {
    trimmed_within(name, 1, MAXIMUM_ROLE_NAME_LENGTH)
        .map(str::to_owned)
        .ok_or(GuildRuleViolation::RoleName)
}

/// # Errors
/// `RoleColor` above `0xFFFFFF`.
pub fn validate_role_color(color: u32) -> Result<u32, GuildRuleViolation> {
    (color <= MAXIMUM_ROLE_COLOR)
        .then_some(color)
        .ok_or(GuildRuleViolation::RoleColor)
}

/// Clients keep unknown bits (`Permissions` deserialization); the server never stores them, or a
/// client could pre-grant permissions a future version defines.
///
/// # Errors
/// `UnknownPermissions` when a bit is not a permission this server knows.
pub fn validate_known_permissions(
    permissions: Permissions,
) -> Result<Permissions, GuildRuleViolation> {
    Permissions::from_bits(permissions.bits()).ok_or(GuildRuleViolation::UnknownPermissions)
}

/// # Errors
/// `RoleOrder` unless `new_order` holds exactly the ids of `current_order`, each once.
pub fn validate_role_order(
    current_order: &[Snowflake],
    new_order: &[Snowflake],
) -> Result<(), GuildRuleViolation> {
    let mut current_sorted = current_order.to_vec();
    let mut new_sorted = new_order.to_vec();
    current_sorted.sort_unstable();
    new_sorted.sort_unstable();
    (current_sorted == new_sorted && new_sorted.windows(2).all(|pair| pair[0] != pair[1]))
        .then_some(())
        .ok_or(GuildRuleViolation::RoleOrder)
}

/// A guild channel's place in the guild's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelPlace {
    pub id: Snowflake,
    pub kind: ChannelKind,
    pub parent_id: Option<Snowflake>,
}

/// The guild's whole new order. `current` is the guild's channels in order; `requested` lists
/// the channels `is_viewable` accepts, in their new order, and fills the slots those channels
/// hold in `current`. Hidden channels keep their slots and categories, so the requester needs
/// to know nothing about them.
///
/// # Errors
/// `ChannelOrder` unless `requested` lists every viewable channel once, categories at the top
/// level, and other channels in a viewable category, none, or the hidden category they are
/// already in (channels do not inherit their category's overwrites).
pub fn merge_channel_order(
    current: &[ChannelPlace],
    is_viewable: impl Fn(Snowflake) -> bool,
    requested: &[ChannelPlacement],
) -> Result<Vec<ChannelPlace>, GuildRuleViolation> {
    let viewable: HashMap<Snowflake, &ChannelPlace> = current
        .iter()
        .filter(|place| is_viewable(place.id))
        .map(|place| (place.id, place))
        .collect();
    if requested.len() != viewable.len() {
        return Err(GuildRuleViolation::ChannelOrder);
    }
    let mut placed_ids = HashSet::with_capacity(requested.len());
    let mut placed = Vec::with_capacity(requested.len());
    for placement in requested {
        let before = *viewable
            .get(&placement.id)
            .ok_or(GuildRuleViolation::ChannelOrder)?;
        let kind = before.kind;
        let is_parent_valid = match (kind, placement.parent_id) {
            (_, None) => true,
            (ChannelKind::Category, Some(_)) => false,
            (ChannelKind::Text | ChannelKind::Voice, Some(parent_id)) => {
                before.parent_id == Some(parent_id)
                    || viewable
                        .get(&parent_id)
                        .is_some_and(|parent| parent.kind == ChannelKind::Category)
            }
        };
        if !placed_ids.insert(placement.id) || !is_parent_valid {
            return Err(GuildRuleViolation::ChannelOrder);
        }
        placed.push(ChannelPlace {
            id: placement.id,
            kind,
            parent_id: placement.parent_id,
        });
    }
    let mut placed = placed.into_iter();
    current
        .iter()
        .map(|place| {
            if is_viewable(place.id) {
                placed.next()
            } else {
                Some(*place)
            }
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(GuildRuleViolation::ChannelOrder)
}

/// # Errors
/// `InitialChannels` beyond the quotas, `ChannelName` or `Topic` for an invalid entry. Returns
/// the categories with normalized names and topics.
pub fn normalize_initial_categories(
    categories: &[InitialCategory],
) -> Result<Vec<InitialCategory>, GuildRuleViolation> {
    let text_channel_count: usize = categories
        .iter()
        .map(|category| category.channels.len())
        .sum();
    if categories.len() > MAXIMUM_INITIAL_CATEGORIES
        || text_channel_count > MAXIMUM_INITIAL_TEXT_CHANNELS
    {
        return Err(GuildRuleViolation::InitialChannels);
    }
    categories
        .iter()
        .map(|category| {
            Ok(InitialCategory {
                name: normalize_channel_name(&category.name)?,
                channels: category
                    .channels
                    .iter()
                    .map(|channel| {
                        Ok(InitialTextChannel {
                            name: normalize_channel_name(&channel.name)?,
                            topic: normalize_topic(&channel.topic)?,
                        })
                    })
                    .collect::<Result<_, GuildRuleViolation>>()?,
            })
        })
        .collect()
}

/// A readable code such as `AE8C-78A9-K2QF`: 12 symbols of Crockford's base 32 (60 bits), as
/// strong as the older 10-character codes. Two groups (40 bits) were guessable from the many
/// addresses of an IPv6 /48, and the audit log keeps a code's first group.
///
/// # Errors
/// Fails only if the operating system cannot provide randomness.
pub fn generate_invite_code() -> Result<String, getrandom::Error> {
    let mut random_bytes = [0u8; INVITE_CODE_SYMBOLS];
    getrandom::fill(&mut random_bytes)?;
    let symbols: Vec<u8> = random_bytes
        .iter()
        .map(|byte| INVITE_CODE_ALPHABET[usize::from(byte % 32)])
        .collect();
    Ok(grouped_invite_code(&symbols))
}

/// `AE8C78A9K2QF` as `AE8C-78A9-K2QF`; only called with alphabet symbols, which are ASCII.
fn grouped_invite_code(symbols: &[u8]) -> String {
    symbols
        .chunks(INVITE_CODE_GROUP_LENGTH)
        .map(|group| {
            group
                .iter()
                .map(|byte| char::from(*byte))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// The stored form of a code someone typed: `ae8c 78a9 k2qf`, `AE8C78A9K2QF` or `ae8c-78ao-k2qf`
/// all find `AE8C-78A9-K2QF` (case, spaces and dashes ignored; O read as 0, I and L as 1). Anything else, such
/// as a 10-character code from before `v0.18.0`, is only trimmed: those are case-sensitive.
#[must_use]
pub fn normalize_invite_code(typed: &str) -> String {
    let symbols: String = typed
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '-')
        .map(|character| match character.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            other => other,
        })
        .collect();
    let is_readable_code = symbols.len() == INVITE_CODE_SYMBOLS
        && symbols
            .bytes()
            .all(|byte| INVITE_CODE_ALPHABET.contains(&byte));
    if is_readable_code {
        grouped_invite_code(symbols.as_bytes())
    } else {
        typed.trim().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_names_are_2_to_32_ascii_letters_digits_and_underscores() {
        assert_eq!(
            normalize_emoji_name(" party_parrot2 ").unwrap(),
            "party_parrot2"
        );
        assert_eq!(normalize_emoji_name("ok").unwrap(), "ok");
        assert!(normalize_emoji_name(&"a".repeat(32)).is_ok());
        for invalid in [
            "",
            "a",
            "two words",
            "café",
            "a-b",
            "a:b",
            "<a>",
            &"a".repeat(33),
        ] {
            assert_eq!(
                normalize_emoji_name(invalid),
                Err(GuildRuleViolation::EmojiName),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn channel_names_keep_capitals_and_spaces() {
        assert_eq!(
            normalize_channel_name("  Off   Topic ").unwrap(),
            "Off   Topic"
        );
        assert_eq!(normalize_channel_name("général_2").unwrap(), "général_2");
        assert_eq!(normalize_channel_name("Q&A #1").unwrap(), "Q&A #1");
        for invalid in [
            "",
            "   ",
            "two\nlines",
            "a\u{202E}b",
            "\u{3164}",
            "\u{115F}x",
            &"a".repeat(101),
        ] {
            assert!(normalize_channel_name(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn guild_name_and_topic_rules() {
        assert_eq!(normalize_guild_name(" Mellow Dev ").unwrap(), "Mellow Dev");
        assert!(normalize_guild_name("M").is_err());
        assert_eq!(normalize_topic("").unwrap(), "");
        assert!(normalize_topic("a\u{0007}").is_err());
        assert!(normalize_topic(&"a".repeat(1025)).is_err());
    }

    #[test]
    fn invite_limits_are_bounded() {
        assert!(validate_invite_limits(None, None).is_ok());
        assert!(validate_invite_limits(Some(1), Some(60)).is_ok());
        assert!(validate_invite_limits(Some(0), None).is_err());
        assert!(validate_invite_limits(Some(MAXIMUM_INVITE_USES + 1), None).is_err());
        assert!(validate_invite_limits(None, Some(0)).is_err());
        assert!(validate_invite_limits(None, Some(31 * 24 * 3600)).is_err());
    }

    #[test]
    fn role_names_colors_and_permissions_are_validated() {
        assert_eq!(normalize_role_name("  Moderators ").unwrap(), "Moderators");
        assert!(normalize_role_name(" ").is_err());
        assert!(normalize_role_name("a\u{202E}b").is_err());
        assert!(normalize_role_name(&"a".repeat(MAXIMUM_ROLE_NAME_LENGTH + 1)).is_err());
        assert_eq!(validate_role_color(0xFF_FFFF), Ok(0xFF_FFFF));
        assert!(validate_role_color(0x100_0000).is_err());
        assert!(validate_known_permissions(Permissions::all()).is_ok());
        assert_eq!(
            validate_known_permissions(Permissions::from_bits_retain(1 << 63)),
            Err(GuildRuleViolation::UnknownPermissions)
        );
    }

    #[test]
    fn a_role_order_is_a_permutation_of_the_current_one() {
        let [a, b, c] = [1, 2, 3].map(Snowflake::from_raw);
        assert!(validate_role_order(&[a, b, c], &[c, a, b]).is_ok());
        assert!(validate_role_order(&[], &[]).is_ok());
        for invalid in [
            vec![a, b],
            vec![a, b, c, c],
            vec![a, a, b],
            vec![a, b, Snowflake::from_raw(4)],
        ] {
            assert_eq!(
                validate_role_order(&[a, b, c], &invalid),
                Err(GuildRuleViolation::RoleOrder),
                "{invalid:?}"
            );
        }
    }

    fn place(raw: u64, kind: ChannelKind, parent: Option<u64>) -> ChannelPlace {
        ChannelPlace {
            id: Snowflake::from_raw(raw),
            kind,
            parent_id: parent.map(Snowflake::from_raw),
        }
    }

    fn placement(raw: u64, parent: Option<u64>) -> ChannelPlacement {
        ChannelPlacement {
            id: Snowflake::from_raw(raw),
            parent_id: parent.map(Snowflake::from_raw),
        }
    }

    #[test]
    fn a_channel_order_fills_the_viewable_slots_and_keeps_hidden_channels_in_place() {
        use ChannelKind::{Category, Text};
        // 1 and 4 are categories; 3 is hidden from the requester.
        let current = [
            place(1, Category, None),
            place(2, Text, Some(1)),
            place(3, Text, Some(1)),
            place(4, Category, None),
            place(5, Text, Some(4)),
        ];
        let is_viewable = |id: Snowflake| id.raw() != 3;
        // A visible channel stays in its category hidden from the requester.
        let in_hidden_category = [place(6, Category, None), place(7, Text, Some(6))];
        assert_eq!(
            merge_channel_order(
                &in_hidden_category,
                |id| id.raw() == 7,
                &[placement(7, Some(6))]
            ),
            Ok(in_hidden_category.to_vec())
        );
        let merged = merge_channel_order(
            &current,
            is_viewable,
            &[
                placement(4, None),
                placement(5, Some(1)),
                placement(1, None),
                placement(2, None),
            ],
        )
        .unwrap();
        assert_eq!(
            merged,
            [
                place(4, Category, None),
                place(5, Text, Some(1)),
                place(3, Text, Some(1)),
                place(1, Category, None),
                place(2, Text, None),
            ]
        );
        for invalid in [
            // Missing, repeated, hidden or unknown channels.
            vec![placement(1, None), placement(2, None), placement(4, None)],
            vec![
                placement(1, None),
                placement(2, None),
                placement(2, None),
                placement(4, None),
            ],
            vec![
                placement(1, None),
                placement(2, None),
                placement(3, None),
                placement(4, None),
            ],
            vec![
                placement(1, None),
                placement(2, None),
                placement(9, None),
                placement(4, None),
            ],
            // A nested category, a channel under a channel, under a missing category.
            vec![
                placement(1, Some(4)),
                placement(2, None),
                placement(4, None),
                placement(5, None),
            ],
            vec![
                placement(1, None),
                placement(2, Some(5)),
                placement(4, None),
                placement(5, None),
            ],
            vec![
                placement(1, None),
                placement(2, Some(9)),
                placement(4, None),
                placement(5, None),
            ],
        ] {
            assert_eq!(
                merge_channel_order(&current, is_viewable, &invalid),
                Err(GuildRuleViolation::ChannelOrder),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn initial_categories_are_normalized_and_bounded() {
        let category = |name: &str, channel_names: &[&str]| InitialCategory {
            name: name.into(),
            channels: channel_names
                .iter()
                .map(|name| InitialTextChannel {
                    name: (*name).into(),
                    topic: " Say hi ".into(),
                })
                .collect(),
        };
        let normalized =
            normalize_initial_categories(&[category(" Salons textuels ", &[" Général "])]).unwrap();
        assert_eq!(normalized[0].name, "Salons textuels");
        assert_eq!(normalized[0].channels[0].name, "Général");
        assert_eq!(normalized[0].channels[0].topic, "Say hi");
        assert_eq!(normalize_initial_categories(&[]), Ok(Vec::new()));
        assert_eq!(
            normalize_initial_categories(&[category("", &[])]),
            Err(GuildRuleViolation::ChannelName)
        );
        let too_many_categories = vec![category("a", &[]); MAXIMUM_INITIAL_CATEGORIES + 1];
        let too_many_channels = vec![category("a", &["b"; MAXIMUM_INITIAL_TEXT_CHANNELS + 1])];
        for invalid in [too_many_categories, too_many_channels] {
            assert_eq!(
                normalize_initial_categories(&invalid),
                Err(GuildRuleViolation::InitialChannels)
            );
        }
    }

    #[test]
    fn invite_codes_are_three_readable_groups_and_distinct() {
        let first = generate_invite_code().unwrap();
        let second = generate_invite_code().unwrap();
        let groups: Vec<&str> = first.split('-').collect();
        assert_eq!(
            groups.iter().map(|group| group.len()).collect::<Vec<_>>(),
            [4, 4, 4]
        );
        assert!(
            groups
                .concat()
                .bytes()
                .all(|byte| INVITE_CODE_ALPHABET.contains(&byte))
        );
        assert_eq!(normalize_invite_code(&first), first);
        assert_ne!(first, second);
    }

    #[test]
    fn typed_invite_codes_find_their_readable_form() {
        for typed in [
            "AE8C-78A9-K2QF",
            "ae8c78a9k2qf",
            " ae8c 78a9 k2qf ",
            "AE8C-78A9-K2QF\n",
        ] {
            assert_eq!(normalize_invite_code(typed), "AE8C-78A9-K2QF", "{typed:?}");
        }
        assert_eq!(normalize_invite_code("10ol-iI00-0000"), "1001-1100-0000");
        // Codes from before v0.18.0 stay as typed, case included; so does anything else.
        assert_eq!(normalize_invite_code(" aB3dE5gH7k "), "aB3dE5gH7k");
        assert_eq!(normalize_invite_code("AE8C-78U9-K2QF"), "AE8C-78U9-K2QF");
        assert_eq!(normalize_invite_code("AE8C-78A9"), "AE8C-78A9");
    }

    #[test]
    fn only_voice_channels_take_a_user_limit_within_the_hard_cap() {
        assert_eq!(validate_user_limit(ChannelKind::Voice, None), Ok(None));
        assert_eq!(
            validate_user_limit(ChannelKind::Voice, Some(1)),
            Ok(Some(1))
        );
        assert_eq!(
            validate_user_limit(ChannelKind::Voice, Some(25)),
            Ok(Some(25))
        );
        for (kind, limit) in [
            (ChannelKind::Voice, 0),
            (ChannelKind::Voice, 26),
            (ChannelKind::Text, 5),
            (ChannelKind::Category, 5),
        ] {
            assert_eq!(
                validate_user_limit(kind, Some(limit)),
                Err(GuildRuleViolation::UserLimit),
                "{kind:?} {limit}"
            );
        }
        assert_eq!(validate_user_limit(ChannelKind::Text, None), Ok(None));
    }
}
