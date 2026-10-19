//! Maps domain errors to `{ code, message }` JSON responses with stable codes (ARCHITECTURE.md §5).

use axum::Json;
use axum::extract::FromRequest;
use axum::extract::FromRequestParts;
use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use mellow_protocol::{ApiErrorBody, ApiErrorCode};

use crate::domain::accounts::AccountError;
use crate::domain::guilds::GuildError;

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

    pub(crate) fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: ApiErrorCode::NotFound,
            message: "No such endpoint.".into(),
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
            AccountError::Internal(source) => {
                // Details go to the log only; clients get a generic message.
                tracing::error!(error = %source, "internal error while handling an account request");
                return Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    code: ApiErrorCode::Internal,
                    message: "Something went wrong on the server.".into(),
                };
            }
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
            GuildError::LimitReached(_) => (StatusCode::CONFLICT, ApiErrorCode::LimitReached),
            GuildError::Internal(source) => {
                tracing::error!(error = %source, "internal error while handling a guild request");
                return Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    code: ApiErrorCode::Internal,
                    message: "Something went wrong on the server.".into(),
                };
            }
        };
        Self {
            status,
            code,
            message: error.to_string(),
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
