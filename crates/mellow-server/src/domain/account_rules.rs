//! Input rules for accounts, checked at the server boundary even though clients check too.

use serde::{Deserialize, Serialize};

/// RFC 5321 limit for a forward path.
pub const MAXIMUM_EMAIL_LENGTH: usize = 254;
/// NIST SP 800-63B: at least 8 characters; we ask for 10 since there is no breach-list check yet.
pub const MINIMUM_PASSWORD_LENGTH: usize = 10;
/// Bounds Argon2 work per request; long enough for any passphrase.
pub const MAXIMUM_PASSWORD_LENGTH: usize = 256;
pub const MAXIMUM_DEVICE_NAME_LENGTH: usize = 64;
pub use mellow_protocol::MAXIMUM_CUSTOM_STATUS_LENGTH;
/// A login is a name or an email address, so the longer of the two bounds it.
pub const MAXIMUM_LOGIN_LENGTH: usize = MAXIMUM_EMAIL_LENGTH;

/// Who may create an account on this instance (ADR-0005).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationMode {
    Open,
    /// Only the instance administrator creates accounts (`mellow-server admin create-user`).
    #[default]
    Closed,
}

/// Who may create guilds on this instance (`guild_creation` in the configuration). Instance
/// administrators are the accounts made with `admin create-user --administrator`: neither guild
/// owners nor members holding a guild's `ADMINISTRATOR` permission.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuildCreationPolicy {
    /// Others join guilds through invites; conversations and groups stay open to everyone.
    #[default]
    Administrators,
    Everyone,
}

impl GuildCreationPolicy {
    #[must_use]
    pub const fn allows(self, is_instance_administrator: bool) -> bool {
        match self {
            Self::Administrators => is_instance_administrator,
            Self::Everyone => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountRuleViolation {
    #[error("names are 2 to 32 characters: letters, digits, spaces, `_`, `.` and `-` (ADR-0048)")]
    InvalidAccountName,
    #[error("this does not look like an email address")]
    InvalidEmail,
    #[error("passwords are {MINIMUM_PASSWORD_LENGTH} to {MAXIMUM_PASSWORD_LENGTH} characters")]
    InvalidPasswordLength,
    #[error(
        "device names are 1 to {MAXIMUM_DEVICE_NAME_LENGTH} characters, without control characters"
    )]
    InvalidDeviceName,
    #[error("logins are 1 to {MAXIMUM_LOGIN_LENGTH} characters")]
    InvalidLogin,
    #[error(
        "custom statuses are up to {MAXIMUM_CUSTOM_STATUS_LENGTH} characters, without control characters"
    )]
    InvalidCustomStatus,
}

/// # Errors
/// `InvalidDeviceName` when empty, too long or containing control characters.
pub fn normalize_device_name(device_name: &str) -> Result<String, AccountRuleViolation> {
    normalize_short_text(device_name, MAXIMUM_DEVICE_NAME_LENGTH)
        .ok_or(AccountRuleViolation::InvalidDeviceName)
}

/// Trims surrounding whitespace and returns the custom status to store; blank clears it. Emoji
/// joiners and presentation selectors stay allowed, so emoji such as "❤️" or "🧑‍💻" pass; tag
/// characters do not, so the flags of England, Scotland and Wales are refused on purpose (tags can
/// hide ASCII text).
///
/// # Errors
/// `InvalidCustomStatus` when too long or containing control or other invisible characters.
pub fn normalize_custom_status(
    custom_status: &str,
) -> Result<Option<String>, AccountRuleViolation> {
    let trimmed = custom_status.trim();
    let length = trimmed.chars().count();
    let has_hidden_characters = trimmed.chars().any(|character| {
        character.is_control()
            || matches!(character, '\u{2028}' | '\u{2029}')
            || (is_invisible_format_character(character)
                && !matches!(character, '\u{200D}' | '\u{FE0E}' | '\u{FE0F}'))
    });
    if length > MAXIMUM_CUSTOM_STATUS_LENGTH || has_hidden_characters {
        Err(AccountRuleViolation::InvalidCustomStatus)
    } else {
        Ok((length > 0).then(|| trimmed.to_owned()))
    }
}

fn normalize_short_text(text: &str, maximum_length: usize) -> Option<String> {
    let trimmed = text.trim();
    let length = trimmed.chars().count();
    let has_hidden_characters = trimmed
        .chars()
        .any(|character| character.is_control() || is_invisible_format_character(character));
    ((1..=maximum_length).contains(&length) && !has_hidden_characters).then(|| trimmed.to_owned())
}

/// Invisible or text-reordering characters: Unicode `Default_Ignorable_Code_Point` (zero-width,
/// bidi controls, fillers, variation selectors, tags), the interlinear annotation marks and the
/// blank Braille pattern, and the line and paragraph separators (U+2028, U+2029), which break a
/// name onto a second line. They let a name look empty, impersonate another one, or flip the text
/// after it (CWE-451).
pub(crate) fn is_invisible_format_character(character: char) -> bool {
    matches!(
        character,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{2800}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFFB}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// Trims and lowercases a username-or-email login, bounding its size before any lookup.
///
/// # Errors
/// `InvalidLogin` when empty or longer than any username or email can be.
pub fn normalize_login(login: &str) -> Result<String, AccountRuleViolation> {
    let trimmed = login.trim();
    if (1..=MAXIMUM_LOGIN_LENGTH).contains(&trimmed.chars().count()) {
        Ok(trimmed.to_lowercase())
    } else {
        Err(AccountRuleViolation::InvalidLogin)
    }
}

/// At login only the maximum applies: a password set under older, looser rules must still work.
///
/// # Errors
/// `InvalidPasswordLength` above the maximum.
pub fn validate_password_maximum_length(password: &str) -> Result<(), AccountRuleViolation> {
    if password.chars().count() <= MAXIMUM_PASSWORD_LENGTH {
        Ok(())
    } else {
        Err(AccountRuleViolation::InvalidPasswordLength)
    }
}

/// Shape check only (one `@`, non-empty parts, a dot in the domain); deliverability is the
/// mail server's job. Returns the address trimmed, with its domain lowercased.
///
/// # Errors
/// `InvalidEmail` when the shape check fails.
pub fn normalize_email(email: &str) -> Result<String, AccountRuleViolation> {
    let trimmed = email.trim();
    let (local_part, domain) = trimmed
        .split_once('@')
        .ok_or(AccountRuleViolation::InvalidEmail)?;
    let is_plausible = trimmed.len() <= MAXIMUM_EMAIL_LENGTH
        && !local_part.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !trimmed
            .chars()
            .any(|character| character.is_whitespace() || character.is_control());
    if is_plausible {
        Ok(format!("{local_part}@{}", domain.to_lowercase()))
    } else {
        Err(AccountRuleViolation::InvalidEmail)
    }
}

/// # Errors
/// `InvalidPasswordLength` outside the allowed range.
pub fn validate_password_length(password: &str) -> Result<(), AccountRuleViolation> {
    if (MINIMUM_PASSWORD_LENGTH..=MAXIMUM_PASSWORD_LENGTH).contains(&password.chars().count()) {
        Ok(())
    } else {
        Err(AccountRuleViolation::InvalidPasswordLength)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_names_are_trimmed_and_bounded() {
        assert!(normalize_device_name("   ").is_err());
        assert!(normalize_device_name("a\u{0007}b").is_err());
        assert!(normalize_device_name("Aur\u{200B}ore").is_err());
        assert_eq!(
            normalize_device_name("Paul's MacBook").unwrap(),
            "Paul's MacBook"
        );
    }

    #[test]
    fn emails_get_a_shape_check_and_a_lowercased_domain() {
        assert_eq!(
            normalize_email(" Paul@Example.ORG ").unwrap(),
            "Paul@example.org"
        );
        for invalid in [
            "paul",
            "@example.org",
            "paul@",
            "paul@localhost",
            "a@b@c.org",
            "pa ul@example.org",
            "paul@.org",
        ] {
            assert!(normalize_email(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn logins_are_trimmed_lowercased_and_bounded() {
        assert_eq!(
            normalize_login("  Aurore@Example.ORG ").unwrap(),
            "aurore@example.org"
        );
        assert!(normalize_login("   ").is_err());
        assert!(normalize_login(&"a".repeat(MAXIMUM_LOGIN_LENGTH + 1)).is_err());
        assert!(validate_password_maximum_length("short").is_ok());
        assert!(
            validate_password_maximum_length(&"a".repeat(MAXIMUM_PASSWORD_LENGTH + 1)).is_err()
        );
    }

    #[test]
    fn password_length_is_counted_in_characters() {
        assert!(validate_password_length("123456789").is_err());
        assert!(validate_password_length("éééééééééé").is_ok());
        assert!(validate_password_length(&"a".repeat(257)).is_err());
    }

    #[test]
    fn custom_statuses_are_trimmed_bounded_and_keep_emoji() {
        assert_eq!(
            normalize_custom_status("  En réunion ❤️ 🧑‍💻 ").unwrap(),
            Some("En réunion ❤️ 🧑‍💻".to_owned())
        );
        assert_eq!(normalize_custom_status("   ").unwrap(), None);
        assert!(normalize_custom_status(&"é".repeat(MAXIMUM_CUSTOM_STATUS_LENGTH)).is_ok());
        assert!(normalize_custom_status(&"é".repeat(MAXIMUM_CUSTOM_STATUS_LENGTH + 1)).is_err());
        for hidden in [
            "line\nbreak",
            "line\u{2028}separator",
            "paragraph\u{2029}separator",
            "bidi \u{202E}flip",
            "zero\u{200B}width",
            "\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}",
        ] {
            assert!(normalize_custom_status(hidden).is_err(), "{hidden:?}");
        }
    }
}
