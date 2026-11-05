//! Two-factor sign-in with time-based one-time passwords (ADR-0049).

use serde::{Deserialize, Serialize};

const REDACTED: &str = "<redacted>";

/// Answer of `POST /api/v1/auth/login` (status `202 Accepted`) when the account has two-factor
/// sign-in on: no session yet. The token is sent with a code to `POST /api/v1/auth/login/two-factor`
/// and works for one sign-in only.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TwoFactorChallenge {
    pub two_factor_token: String,
    pub expires_in_seconds: u32,
}

/// Body of `POST /api/v1/auth/login/two-factor`, answered like a login with `SessionCreated`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompleteTwoFactorSignInRequest {
    pub two_factor_token: String,
    /// The six digits of the authenticator app, or one of the recovery codes.
    pub code: String,
}

/// Body of `GET /api/v1/users/@me/two-factor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TwoFactorStatus {
    pub is_enabled: bool,
    pub recovery_codes_remaining: u32,
    /// When a wrong code was last typed for this account (sign-in or settings): someone who is not
    /// the owner may know the password. `null` when none was, or when two-factor sign-in is off.
    #[serde(default)]
    pub last_failed_code_at_unix_milliseconds: Option<u64>,
}

/// Body of `POST /api/v1/users/@me/two-factor/enrolment`: starts (or restarts) an enrolment.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartTwoFactorEnrolmentRequest {
    pub current_password: String,
}

/// The secret of a new enrolment, in unpadded base32. Authenticator apps take it as
/// `otpauth://totp/<label>?secret=<secret>&issuer=<issuer>&algorithm=SHA1&digits=6&period=30`.
/// Two-factor sign-in is on only once a code made from it is confirmed.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TwoFactorEnrolment {
    pub secret: String,
}

/// Body of `POST /api/v1/users/@me/two-factor/enable`, answered with `TwoFactorRecoveryCodes`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnableTwoFactorRequest {
    pub code: String,
}

/// Body of `POST /api/v1/users/@me/two-factor/disable` and
/// `POST /api/v1/users/@me/two-factor/recovery-codes`: the password and a current code (or a
/// recovery code).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProveTwoFactorRequest {
    pub current_password: String,
    pub code: String,
}

/// Ten single-use recovery codes, shown once: the server keeps only their hashes.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TwoFactorRecoveryCodes {
    pub recovery_codes: Vec<String>,
}

/// Secrets (tokens, codes, passwords) are never printed, so a debug log cannot leak them.
macro_rules! redacted_debug {
    ($type:ident { $($field:ident),* }) => {
        impl std::fmt::Debug for $type {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter
                    .debug_struct(stringify!($type))
                    $(.field(stringify!($field), &REDACTED))*
                    .finish()
            }
        }
    };
}

redacted_debug!(TwoFactorChallenge { two_factor_token });
redacted_debug!(CompleteTwoFactorSignInRequest {
    two_factor_token,
    code
});
redacted_debug!(StartTwoFactorEnrolmentRequest { current_password });
redacted_debug!(TwoFactorEnrolment { secret });
redacted_debug!(EnableTwoFactorRequest { code });
redacted_debug!(ProveTwoFactorRequest {
    current_password,
    code
});
redacted_debug!(TwoFactorRecoveryCodes { recovery_codes });

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_never_printed() {
        let printed = format!(
            "{:?} {:?} {:?} {:?}",
            CompleteTwoFactorSignInRequest {
                two_factor_token: "mlw_challenge-secret".into(),
                code: "123456".into(),
            },
            TwoFactorEnrolment {
                secret: "JBSWY3DPEHPK3PXP".into(),
            },
            ProveTwoFactorRequest {
                current_password: "hunter2-secret".into(),
                code: "654321".into(),
            },
            TwoFactorRecoveryCodes {
                recovery_codes: vec!["abcd-efgh-ijkl-mnop".into()],
            },
        );
        for secret in [
            "challenge-secret",
            "123456",
            "JBSWY3DP",
            "hunter2",
            "654321",
            "abcd",
        ] {
            assert!(!printed.contains(secret), "{printed}");
        }
    }

    #[test]
    fn unknown_request_fields_are_rejected() {
        let json = r#"{"two_factor_token":"a","code":"b","remember":true}"#;
        assert!(serde_json::from_str::<CompleteTwoFactorSignInRequest>(json).is_err());
    }
}
