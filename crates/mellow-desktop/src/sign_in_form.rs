//! The rules of the instance address and sign-in screens, without UI: checks before sending
//! (the web's `sign-in-form.ts` and the browser's own email check), the instance probe's verdict
//! and errors in words (the web's `describeError`).

use mellow_client_core::ClientError;
use mellow_protocol::{HealthResponse, HealthStatus};
use semver::Version;

use crate::strings_pending_i18n::Strings;

/// The oldest instance this app talks to: client-core speaks the protocol of 0.48 (unique names,
/// two-factor sign-in, ADR-0048 and ADR-0049).
pub const MINIMUM_SUPPORTED_INSTANCE_VERSION: Version = Version::new(0, 48, 0);
/// The web client's `MINIMUM_PASSWORD_LENGTH`, shown as a helper; the server decides.
pub const MINIMUM_PASSWORD_LENGTH: usize = 10;
/// The web client's `MAXIMUM_ACCOUNT_NAME_LENGTH` (the server's `account_names.rs`).
pub const MAXIMUM_ACCOUNT_NAME_LENGTH: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInField {
    InstanceAddress,
    Login,
    Password,
    Email,
    Username,
    NewPassword,
    Code,
}

/// One field as the form checks it before sending.
#[derive(Debug)]
pub struct FieldToCheck<'a> {
    pub field: SignInField,
    /// Already trimmed where spaces do not count; a password is as typed.
    pub value: &'a str,
    pub empty_error: &'static str,
    /// Set when the value is malformed (an email address), checked once the field is filled.
    pub malformed_error: Option<&'static str>,
}

/// Field to error, in the form's order, so the first one is the field to focus.
#[must_use]
pub fn field_errors_before_sending(
    fields: &[FieldToCheck<'_>],
) -> Vec<(SignInField, &'static str)> {
    fields
        .iter()
        .filter_map(|field| {
            if field.value.is_empty() {
                Some((field.field, field.empty_error))
            } else {
                field.malformed_error.map(|error| (field.field, error))
            }
        })
        .collect()
}

/// The HTML standard's "valid email address", which the web's `type="email"` field checks.
#[must_use]
pub fn is_valid_email_address(text: &str) -> bool {
    const LOCAL_PART_SYMBOLS: &str = ".!#$%&'*+/=?^_`{|}~-";
    const MAXIMUM_LABEL_LENGTH: usize = 63;
    let Some((local_part, domain)) = text.split_once('@') else {
        return false;
    };
    let is_local_part_valid = !local_part.is_empty()
        && local_part.chars().all(|character| {
            character.is_ascii_alphanumeric() || LOCAL_PART_SYMBOLS.contains(character)
        });
    let is_label_valid = |label: &str| {
        !label.is_empty()
            && label.len() <= MAXIMUM_LABEL_LENGTH
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
    };
    is_local_part_valid && domain.split('.').all(is_label_valid)
}

/// Why the instance address cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceRefusal {
    Unreachable,
    NotMellow,
    /// The instance answered but its database did not.
    DatabaseUnavailable,
    TooOld {
        version: String,
    },
}

impl InstanceRefusal {
    #[must_use]
    pub fn describe(&self, host: &str, strings: &Strings) -> String {
        match self {
            Self::Unreachable => (strings.instance_unreachable)(host),
            Self::NotMellow => (strings.instance_not_mellow)(host),
            Self::DatabaseUnavailable => format!(
                "{} {}",
                strings.app_unreachable_title, strings.app_unreachable_body
            ),
            Self::TooOld { version } => (strings.instance_too_old)(
                host,
                version,
                &MINIMUM_SUPPORTED_INSTANCE_VERSION.to_string(),
            ),
        }
    }
}

/// The verdict on `GET /api/v1/health`.
///
/// # Errors
/// Why the instance cannot be used.
pub fn judge_instance_health(
    answer: Result<HealthResponse, ClientError>,
) -> Result<(), InstanceRefusal> {
    let health = match answer {
        Ok(health) => health,
        Err(ClientError::UnexpectedResponse(_) | ClientError::Api { .. }) => {
            return Err(InstanceRefusal::NotMellow);
        }
        Err(_) => return Err(InstanceRefusal::Unreachable),
    };
    let Ok(version) = Version::parse(health.version.trim()) else {
        return Err(InstanceRefusal::NotMellow);
    };
    // A development build of a release (`0.48.0-dev`) speaks that release's protocol.
    let release = Version::new(version.major, version.minor, version.patch);
    if release < MINIMUM_SUPPORTED_INSTANCE_VERSION {
        return Err(InstanceRefusal::TooOld {
            version: health.version,
        });
    }
    match health.status {
        HealthStatus::Ok => Ok(()),
        HealthStatus::DatabaseUnavailable => Err(InstanceRefusal::DatabaseUnavailable),
    }
}

/// `summary: reason.`, never the server's own text (development notes: "Server text in the UI").
#[must_use]
pub fn describe_error(error: &ClientError, summary: &str, strings: &Strings) -> String {
    let reasons = &strings.error_reasons;
    let reason = match error {
        ClientError::Network(_) | ClientError::TransferStalled => reasons.network,
        ClientError::Api { code, .. } => reasons
            .for_code(*code)
            .unwrap_or(strings.error_unknown_reason),
        ClientError::FileTooLarge => reasons.file_too_large,
        ClientError::InvalidInstanceAddress(_)
        | ClientError::UnexpectedResponse(_)
        | ClientError::NotSignedIn
        | ClientError::RandomGeneratorFailed(_) => strings.error_unknown_reason,
    };
    (strings.error_with_reason)(summary, reason)
}

#[cfg(test)]
mod tests {
    use mellow_protocol::ApiErrorCode;

    use super::*;
    use crate::strings_pending_i18n::{ENGLISH, FRENCH};

    fn field(field: SignInField, value: &str) -> FieldToCheck<'_> {
        FieldToCheck {
            field,
            value,
            empty_error: "empty",
            malformed_error: None,
        }
    }

    #[test]
    fn empty_fields_are_reported_in_order_then_malformed_ones() {
        let errors = field_errors_before_sending(&[
            FieldToCheck {
                malformed_error: Some("malformed"),
                ..field(SignInField::Email, "ana")
            },
            field(SignInField::Username, ""),
            field(SignInField::NewPassword, " "),
        ]);
        assert_eq!(
            errors,
            [
                (SignInField::Email, "malformed"),
                (SignInField::Username, "empty")
            ]
        );
        assert_eq!(
            field_errors_before_sending(&[field(SignInField::Login, "ana")]),
            []
        );
    }

    #[test]
    fn email_addresses_follow_the_html_standard() {
        for valid in [
            "ana@example.org",
            "a.b+tag@sub.example.org",
            "ana@localhost",
            "x@a-b.c",
        ] {
            assert!(is_valid_email_address(valid), "{valid}");
        }
        for invalid in [
            "ana",
            "@example.org",
            "ana@",
            "ana@example..org",
            "ana@-example.org",
            "ana@example-.org",
            "an a@example.org",
            "ana@exa_mple.org",
            "ana@b@example.org",
            "anä@example.org",
        ] {
            assert!(!is_valid_email_address(invalid), "{invalid}");
        }
    }

    #[expect(clippy::unnecessary_wraps, reason = "the probe's answer is a `Result`")]
    fn health(version: &str, status: HealthStatus) -> Result<HealthResponse, ClientError> {
        Ok(HealthResponse {
            status,
            version: version.into(),
        })
    }

    #[test]
    fn instances_older_than_the_minimum_are_refused_with_their_version() {
        assert_eq!(
            judge_instance_health(health("0.48.0", HealthStatus::Ok)),
            Ok(())
        );
        assert_eq!(
            judge_instance_health(health("1.2.3", HealthStatus::Ok)),
            Ok(())
        );
        assert_eq!(
            judge_instance_health(health("0.48.0-dev", HealthStatus::Ok)),
            Ok(())
        );
        assert_eq!(
            judge_instance_health(health("0.47.9", HealthStatus::Ok)),
            Err(InstanceRefusal::TooOld {
                version: "0.47.9".into()
            })
        );
        assert_eq!(
            judge_instance_health(health("not a version", HealthStatus::Ok)),
            Err(InstanceRefusal::NotMellow)
        );
        assert_eq!(
            judge_instance_health(health("0.48.0", HealthStatus::DatabaseUnavailable)),
            Err(InstanceRefusal::DatabaseUnavailable)
        );
        assert_eq!(
            judge_instance_health(Err(ClientError::UnexpectedResponse("html".into()))),
            Err(InstanceRefusal::NotMellow)
        );
        assert_eq!(
            judge_instance_health(Err(ClientError::TransferStalled)),
            Err(InstanceRefusal::Unreachable)
        );
        assert_eq!(
            InstanceRefusal::TooOld {
                version: "0.3.0".into()
            }
            .describe("chat.example.org", &ENGLISH),
            "chat.example.org runs Mellow 0.3.0. This app needs 0.48.0 or later. Ask the instance admin to update."
        );
    }

    #[test]
    fn errors_read_as_a_summary_and_a_reason_never_the_servers_text() {
        let refused = ClientError::Api {
            status: 401,
            code: ApiErrorCode::InvalidCredentials,
            message: "server text".into(),
        };
        assert_eq!(
            describe_error(&refused, ENGLISH.sign_in_failed, &ENGLISH),
            "Couldn't sign in: the login or the password is wrong."
        );
        assert_eq!(
            describe_error(&refused, FRENCH.sign_in_failed, &FRENCH),
            "Impossible de te connecter\u{a0}: l’identifiant ou le mot de passe est incorrect."
        );
        let internal = ClientError::Api {
            status: 500,
            code: ApiErrorCode::Internal,
            message: "server text".into(),
        };
        assert_eq!(
            describe_error(&internal, ENGLISH.sign_in_failed, &ENGLISH),
            "Couldn't sign in: something went wrong. Try again."
        );
    }
}
