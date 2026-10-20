use serde::{Deserialize, Serialize};

use crate::Snowflake;

/// Body of `POST /api/v1/auth/register`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterRequest {
    pub username: String,
    /// Defaults to the username when absent.
    pub display_name: Option<String>,
    pub email: String,
    pub password: String,
    /// Shown in the user's session list, e.g. "Mellow desktop on a laptop".
    pub device_name: String,
}

/// Body of `POST /api/v1/auth/login`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    /// Username or email address.
    pub login: String,
    pub password: String,
    pub device_name: String,
}

/// Response to a successful registration or login. The token is shown once and sent back as
/// `Authorization: Bearer <token>`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCreated {
    pub token: String,
    pub user: CurrentUser,
}

/// Body of `GET /api/v1/users/@me`: the signed-in user, including private fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentUser {
    pub id: Snowflake,
    pub username: String,
    pub display_name: String,
    pub email: String,
    pub is_instance_administrator: bool,
}

/// Body of every error response. `code` is stable and machine-readable; `message` is for humans.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub code: ApiErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApiErrorCode {
    ValidationFailed,
    UsernameTaken,
    EmailTaken,
    RegistrationClosed,
    /// Deliberately the same for an unknown login and a wrong password.
    InvalidCredentials,
    Unauthorized,
    AccountDisabled,
    RateLimited,
    /// The server is overloaded (e.g. too many password checks at once); retry shortly.
    ServerBusy,
    NotFound,
    /// Signed in, but this member lacks the permission the action needs.
    MissingPermissions,
    /// The invite code is unknown, expired or used up.
    InvalidInvite,
    /// The guild owner cannot leave their own guild (transfer or delete it first).
    OwnerCannotLeave,
    /// The user is banned from this guild.
    Banned,
    /// A quota was reached (channels per guild, owned guilds, active invites).
    LimitReached,
    /// What the request was based on changed meanwhile (e.g. another reordering of the roles);
    /// refresh and try again.
    Conflict,
    Internal,
}

const REDACTED: &str = "<redacted>";

/// Secrets (passwords, tokens) are never printed, so a debug log cannot leak them.
impl std::fmt::Debug for RegisterRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegisterRequest")
            .field("username", &self.username)
            .field("display_name", &self.display_name)
            .field("email", &self.email)
            .field("password", &REDACTED)
            .field("device_name", &self.device_name)
            .finish()
    }
}

impl std::fmt::Debug for LoginRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoginRequest")
            .field("login", &self.login)
            .field("password", &REDACTED)
            .field("device_name", &self.device_name)
            .finish()
    }
}

impl std::fmt::Debug for SessionCreated {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionCreated")
            .field("token", &REDACTED)
            .field("user", &self.user)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_screaming_snake_case_on_the_wire() {
        let body = ApiErrorBody {
            code: ApiErrorCode::InvalidCredentials,
            message: "Wrong login or password.".into(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains(r#""code":"INVALID_CREDENTIALS""#), "{json}");
        for (code, wire) in [
            (ApiErrorCode::MissingPermissions, "MISSING_PERMISSIONS"),
            (ApiErrorCode::InvalidInvite, "INVALID_INVITE"),
            (ApiErrorCode::OwnerCannotLeave, "OWNER_CANNOT_LEAVE"),
            (ApiErrorCode::Banned, "BANNED"),
            (ApiErrorCode::LimitReached, "LIMIT_REACHED"),
        ] {
            assert_eq!(serde_json::to_value(code).unwrap(), wire);
        }
    }

    #[test]
    fn secrets_are_never_printed() {
        let login = LoginRequest {
            login: "aurore".into(),
            password: "hunter2-secret".into(),
            device_name: "d".into(),
        };
        let session = SessionCreated {
            token: "mlw_secret-token".into(),
            user: CurrentUser {
                id: Snowflake::from_raw(1),
                username: "aurore".into(),
                display_name: "Aurore".into(),
                email: "aurore@example.org".into(),
                is_instance_administrator: false,
            },
        };
        let printed = format!("{login:?} {session:?}");
        assert!(
            !printed.contains("hunter2-secret") && !printed.contains("secret-token"),
            "{printed}"
        );
    }

    #[test]
    fn unknown_request_fields_are_rejected() {
        let json = r#"{"login":"a","password":"b","device_name":"c","is_admin":true}"#;
        assert!(serde_json::from_str::<LoginRequest>(json).is_err());
    }
}
