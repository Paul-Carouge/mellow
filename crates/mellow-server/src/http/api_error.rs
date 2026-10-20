//! Maps domain errors to `{ code, message }` JSON responses with stable codes (ARCHITECTURE.md §5).

use axum::Json;
use axum::extract::FromRequest;
use axum::extract::FromRequestParts;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use mellow_protocol::{ApiErrorBody, ApiErrorCode};

use crate::domain::accounts::AccountError;
use crate::domain::guilds::GuildError;
use crate::domain::messages::MessageError;

#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    code: ApiErrorCode,
    message: String,
}

impl ApiError {
    pub(crate) fn rate_limited() -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: ApiErrorCode::RateLimited,
            message: "Too many attempts. Wait a few minutes and try again.".into(),
        }
    }

    pub(crate) fn unauthorized() -> Self {
        Self::from(AccountError::Unauthorized)
    }

    /// Details go to the log only; clients get a generic message.
    fn internal(context: &str, source: &dyn std::fmt::Display) -> Self {
        tracing::error!(error = %source, "internal error while handling {context}");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: ApiErrorCode::Internal,
            message: "Something went wrong on the server.".into(),
        }
    }

    pub(crate) fn validation(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: ApiErrorCode::ValidationFailed,
            message,
        }
    }

    pub(crate) fn server_busy() -> Self {
        Self::from(AccountError::ServerBusy)
    }

    pub(crate) fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: ApiErrorCode::NotFound,
            message: "Not found.".into(),
        }
    }

    pub(crate) fn method_not_allowed() -> Self {
        Self {
            status: StatusCode::METHOD_NOT_ALLOWED,
            code: ApiErrorCode::NotFound,
            message: "This endpoint does not accept this method.".into(),
        }
    }
}

impl From<AccountError> for ApiError {
    fn from(error: AccountError) -> Self {
        let (status, code) = match &error {
            AccountError::InvalidInput(_) => {
                (StatusCode::BAD_REQUEST, ApiErrorCode::ValidationFailed)
            }
            AccountError::UsernameTaken => (StatusCode::CONFLICT, ApiErrorCode::UsernameTaken),
            AccountError::EmailTaken => (StatusCode::CONFLICT, ApiErrorCode::EmailTaken),
            AccountError::RegistrationClosed => {
                (StatusCode::FORBIDDEN, ApiErrorCode::RegistrationClosed)
            }
            AccountError::InvalidCredentials => {
                (StatusCode::UNAUTHORIZED, ApiErrorCode::InvalidCredentials)
            }
            AccountError::AccountDisabled => (StatusCode::FORBIDDEN, ApiErrorCode::AccountDisabled),
            AccountError::Unauthorized => (StatusCode::UNAUTHORIZED, ApiErrorCode::Unauthorized),
            AccountError::ServerBusy => (StatusCode::SERVICE_UNAVAILABLE, ApiErrorCode::ServerBusy),
            AccountError::Internal(source) => return Self::internal("an account request", source),
        };
        Self {
            status,
            code,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiErrorBody {
                code: self.code,
                message: self.message,
            }),
        )
            .into_response()
    }
}

impl From<GuildError> for ApiError {
    fn from(error: GuildError) -> Self {
        let (status, code) = match &error {
            GuildError::InvalidInput(_) => {
                (StatusCode::BAD_REQUEST, ApiErrorCode::ValidationFailed)
            }
            GuildError::NotFound => (StatusCode::NOT_FOUND, ApiErrorCode::NotFound),
            GuildError::MissingPermissions => {
                (StatusCode::FORBIDDEN, ApiErrorCode::MissingPermissions)
            }
            GuildError::InvalidInvite => (StatusCode::NOT_FOUND, ApiErrorCode::InvalidInvite),
            GuildError::OwnerCannotLeave => (StatusCode::CONFLICT, ApiErrorCode::OwnerCannotLeave),
            GuildError::Banned => (StatusCode::FORBIDDEN, ApiErrorCode::Banned),
            GuildError::LimitReached(_) => (StatusCode::CONFLICT, ApiErrorCode::LimitReached),
            GuildError::Conflict => (StatusCode::CONFLICT, ApiErrorCode::Conflict),
            GuildError::Internal(source) => return Self::internal("a guild request", source),
        };
        Self {
            status,
            code,
            message: error.to_string(),
        }
    }
}

impl From<MessageError> for ApiError {
    fn from(error: MessageError) -> Self {
        match error {
            MessageError::Guild(guild_error) => guild_error.into(),
            MessageError::InvalidInput(violation) => Self {
                status: StatusCode::BAD_REQUEST,
                code: ApiErrorCode::ValidationFailed,
                message: violation.to_string(),
            },
            MessageError::MessageNotFound => Self::not_found(),
            MessageError::Internal(source) => Self::internal("a message request", &source),
        }
    }
}

/// `Query` whose rejections (unknown or malformed parameters) use our error format.
#[derive(Debug, FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(ApiError))]
pub(crate) struct ApiQuery<T>(pub(crate) T);

impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> Self {
        Self {
            status: rejection.status(),
            code: ApiErrorCode::ValidationFailed,
            message: rejection.body_text(),
        }
    }
}

/// `Path` whose rejections (e.g. a malformed id) use our error format.
#[derive(Debug, FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(ApiError))]
pub(crate) struct ApiPath<T>(pub(crate) T);

impl From<PathRejection> for ApiError {
    fn from(rejection: PathRejection) -> Self {
        Self {
            status: rejection.status(),
            code: ApiErrorCode::ValidationFailed,
            message: rejection.body_text(),
        }
    }
}

/// `Json` whose rejections (bad content type, malformed or unknown fields) use our error format.
#[derive(Debug, FromRequest)]
#[from_request(via(Json), rejection(ApiError))]
pub(crate) struct ApiJson<T>(pub(crate) T);

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        Self {
            status: rejection.status(),
            code: ApiErrorCode::ValidationFailed,
            message: rejection.body_text(),
        }
    }
}
