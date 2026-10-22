//! Input rules for guilds, channels and invites.

use std::time::Duration;

use mellow_protocol::{Permissions, Snowflake};

use crate::domain::account_rules::is_invisible_format_character;

pub const MINIMUM_GUILD_NAME_LENGTH: usize = 2;
pub const MAXIMUM_GUILD_NAME_LENGTH: usize = 100;
pub const MAXIMUM_CHANNEL_NAME_LENGTH: usize = 100;
pub const MAXIMUM_CHANNEL_TOPIC_LENGTH: usize = 1024;
pub const MAXIMUM_INVITE_USES: u32 = 10_000;
pub const MAXIMUM_INVITE_LIFETIME: Duration = Duration::from_hours(30 * 24);
pub const INVITE_CODE_LENGTH: usize = 10;
/// Quotas, as on Discord, so one account cannot grow the database or a response without bound.
pub const MAXIMUM_CHANNELS_PER_GUILD: u32 = 500;
pub const MAXIMUM_OWNED_GUILDS_PER_USER: u32 = 100;
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
const INVITE_CODE_ALPHABET: &[u8; 62] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
/// Largest multiple of 62 below 256: bytes at or above it are redrawn, so every character is
/// equally likely (no modulo bias).
const UNBIASED_BYTE_LIMIT: u8 = 248;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GuildRuleViolation {
    #[error(
        "guild names are {MINIMUM_GUILD_NAME_LENGTH} to {MAXIMUM_GUILD_NAME_LENGTH} characters, without control characters"
    )]
    GuildName,
    #[error("channel names are 1 to {MAXIMUM_CHANNEL_NAME_LENGTH} letters, digits, `-` or `_`")]
    ChannelName,
    #[error(
        "category names are 1 to {MAXIMUM_CHANNEL_NAME_LENGTH} characters, without control characters"
    )]
    CategoryName,
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
    #[error("@everyone keeps its name and colour, and cannot be deleted, given or taken away")]
    EveryoneRole,
    #[error("categories have no icon")]
    CategoryIcon,
}

fn has_hidden_characters(text: &str) -> bool {
    text.chars()
        .any(|character| character.is_control() || is_invisible_format_character(character))
}

fn trimmed_within(text: &str, minimum: usize, maximum: usize) -> Option<&str> {
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

/// # Errors
/// `CategoryName` when empty, too long or containing hidden characters.
pub fn normalize_category_name(name: &str) -> Result<String, GuildRuleViolation> {
    trimmed_within(name, 1, MAXIMUM_CHANNEL_NAME_LENGTH)
        .map(str::to_owned)
        .ok_or(GuildRuleViolation::CategoryName)
}

/// Discord-style text channel names: lowercase, spaces become `-` ("Off Topic" → "off-topic").
///
/// # Errors
/// `ChannelName` when empty, too long, or containing anything but letters, digits, `-`, `_`.
pub fn normalize_text_channel_name(name: &str) -> Result<String, GuildRuleViolation> {
    // Hangul fillers and similar count as letters for `is_alphanumeric`, yet render as nothing.
    if has_hidden_characters(name) {
        return Err(GuildRuleViolation::ChannelName);
    }
    let normalized = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase();
    let length = normalized.chars().count();
    let has_allowed_characters = normalized
        .chars()
        .all(|character| character.is_alphanumeric() || character == '-' || character == '_');
    if (1..=MAXIMUM_CHANNEL_NAME_LENGTH).contains(&length) && has_allowed_characters {
        Ok(normalized)
    } else {
        Err(GuildRuleViolation::ChannelName)
    }
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

/// A 10-character code over 62 symbols (about 59 bits): unguessable at our rate limits.
///
/// # Errors
/// Fails only if the operating system cannot provide randomness.
pub fn generate_invite_code() -> Result<String, getrandom::Error> {
    let mut code = String::with_capacity(INVITE_CODE_LENGTH);
    let mut random_bytes = [0u8; INVITE_CODE_LENGTH * 2];
    while code.len() < INVITE_CODE_LENGTH {
        getrandom::fill(&mut random_bytes)?;
        code.extend(
            random_bytes
                .iter()
                .filter(|byte| **byte < UNBIASED_BYTE_LIMIT)
                .map(|byte| INVITE_CODE_ALPHABET[usize::from(byte % 62)] as char)
                .take(INVITE_CODE_LENGTH - code.len()),
        );
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_channel_names_are_lowercased_and_hyphenated() {
        assert_eq!(
            normalize_text_channel_name("  Off   Topic ").unwrap(),
            "off-topic"
        );
        assert_eq!(
            normalize_text_channel_name("général_2").unwrap(),
            "général_2"
        );
        for invalid in [
            "",
            "   ",
            "no#hash",
            "a\u{202E}b",
            "\u{3164}",
            "\u{115F}x",
            &"a".repeat(101),
        ] {
            assert!(normalize_text_channel_name(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn guild_category_and_topic_rules() {
        assert_eq!(normalize_guild_name(" Mellow Dev ").unwrap(), "Mellow Dev");
        assert!(normalize_guild_name("M").is_err());
        assert_eq!(
            normalize_category_name("Text Channels").unwrap(),
            "Text Channels"
        );
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

    #[test]
    fn invite_codes_are_ten_alphanumerics_and_distinct() {
        let first = generate_invite_code().unwrap();
        let second = generate_invite_code().unwrap();
        assert_eq!(first.len(), INVITE_CODE_LENGTH);
        assert!(first.bytes().all(|byte| byte.is_ascii_alphanumeric()));
        assert_ne!(first, second);
    }
}
