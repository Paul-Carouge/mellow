//! Bearer-token authentication for handlers.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
use mellow_protocol::CurrentUser;

use crate::domain::clock::now_unix_milliseconds;
use crate::http::AppState;
use crate::http::api_error::ApiError;

const BEARER_PREFIX: &str = "Bearer ";

pub(crate) fn bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix(BEARER_PREFIX))
        .ok_or_else(ApiError::unauthorized)
}

/// The signed-in user; extracting it rejects the request with `UNAUTHORIZED` otherwise.
#[derive(Debug, Clone)]
pub(crate) struct AuthenticatedUser(pub(crate) CurrentUser);

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers)?;
        let user = state
            .accounts
            .authenticate(token, now_unix_milliseconds())
            .await?;
        Ok(Self(user))
    }
}
