//! Input rules for accounts, checked at the server boundary even though clients check too.

use serde::{Deserialize, Serialize};

pub const MINIMUM_USERNAME_LENGTH: usize = 2;
pub const MAXIMUM_USERNAME_LENGTH: usize = 32;
pub const MAXIMUM_DISPLAY_NAME_LENGTH: usize = 32;
/// RFC 5321 limit for a forward path.
pub const MAXIMUM_EMAIL_LENGTH: usize = 254;
/// NIST SP 800-63B: at least 8 characters; we ask for 10 since there is no breach-list check yet.
pub const MINIMUM_PASSWORD_LENGTH: usize = 10;
/// Bounds Argon2 work per request; long enough for any passphrase.
pub const MAXIMUM_PASSWORD_LENGTH: usize = 256;
pub const MAXIMUM_DEVICE_NAME_LENGTH: usize = 64;
/// A login is a username or an email address, so the longer of the two bounds it.
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountRuleViolation {
    #[error(
        "usernames are {MINIMUM_USERNAME_LENGTH} to {MAXIMUM_USERNAME_LENGTH} characters: lowercase letters, digits, `_` and `.`, without `..`"
    )]
    InvalidUsername,
    #[error(
        "display names are 1 to {MAXIMUM_DISPLAY_NAME_LENGTH} characters, without control characters"
    )]
    InvalidDisplayName,
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
}

/// Discord-style usernames: lowercase so they are unambiguous when typed or mentioned.
///
/// # Errors
/// `InvalidUsername` when the rules above are not met.
pub fn validate_username(username: &str) -> Result<(), AccountRuleViolation> {
    let length = username.chars().count();
    let has_allowed_characters = username.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'.'
    });
    if (MINIMUM_USERNAME_LENGTH..=MAXIMUM_USERNAME_LENGTH).contains(&length)
        && has_allowed_characters
        && !username.contains("..")
    {
        Ok(())
    } else {
        Err(AccountRuleViolation::InvalidUsername)
    }
}

/// Trims surrounding whitespace and returns the display name to store.
///
/// # Errors
/// `InvalidDisplayName` when empty, too long or containing control characters.
pub fn normalize_display_name(display_name: &str) -> Result<String, AccountRuleViolation> {
    normalize_short_text(display_name, MAXIMUM_DISPLAY_NAME_LENGTH)
        .ok_or(AccountRuleViolation::InvalidDisplayName)
}

/// # Errors
/// `InvalidDeviceName` when empty, too long or containing control characters.
pub fn normalize_device_name(device_name: &str) -> Result<String, AccountRuleViolation> {
    normalize_short_text(device_name, MAXIMUM_DEVICE_NAME_LENGTH)
        .ok_or(AccountRuleViolation::InvalidDeviceName)
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
/// blank Braille pattern. They let a name look empty, impersonate another one, or flip the text
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
            | '\u{202A}'..='\u{202E}'
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
    fn usernames_follow_discord_style_rules() {
        for valid in ["ab", "paul.carouge", "user_42", &"a".repeat(32)] {
            assert_eq!(validate_username(valid), Ok(()), "{valid}");
        }
        for invalid in [
            "a",
            "Paul",
            "paul carouge",
            "élise",
            "a..b",
            "a-b",
            &"a".repeat(33),
            "",
        ] {
            assert_eq!(
                validate_username(invalid),
                Err(AccountRuleViolation::InvalidUsername),
                "{invalid}"
            );
        }
    }

    #[test]
    fn display_and_device_names_are_trimmed_and_bounded() {
        assert_eq!(normalize_display_name("  Élise  ").unwrap(), "Élise");
        assert!(normalize_display_name("   ").is_err());
        assert!(normalize_display_name("a\u{0007}b").is_err());
        assert!(normalize_display_name(&"é".repeat(33)).is_err());
        for hidden in [
            "\u{202E}nimda",
            "\u{3164}",
            "ab\u{E0041}",
            "x\u{FE0F}",
            "\u{2800}\u{2800}",
            "Aur\u{200B}ore",
            "\u{FEFF}Basile",
            "Chlo\u{2066}é",
        ] {
            assert!(normalize_display_name(hidden).is_err(), "{hidden:?}");
        }
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
}
