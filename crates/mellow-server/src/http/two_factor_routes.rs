//! Two-factor sign-in (ADR-0049): the second sign-in step and `/api/v1/users/@me/two-factor/*`.

use std::net::SocketAddr;
use std::time::Instant;

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use mellow_protocol::{
    CompleteTwoFactorSignInRequest, EnableTwoFactorRequest, ProveTwoFactorRequest,
    StartTwoFactorEnrolmentRequest, TwoFactorStatus,
};

use crate::domain::clock::now_unix_milliseconds;
use crate::http::AppState;
use crate::http::account_routes::{
    is_password_check_allowed, record_authentication_attempt_per_client, session_response,
};
use crate::http::api_error::{ApiError, ApiJson};
use crate::http::authentication::{AuthenticatedUser, session_token};
use crate::storage::accounts::StoredUser;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/login/two-factor", post(complete_sign_in))
        .route("/api/v1/users/@me/two-factor", get(status))
        .route(
            "/api/v1/users/@me/two-factor/enrolment",
            post(start_enrolment),
        )
        .route("/api/v1/users/@me/two-factor/enable", post(enable))
        .route("/api/v1/users/@me/two-factor/disable", post(disable))
        .route(
            "/api/v1/users/@me/two-factor/recovery-codes",
            post(regenerate_recovery_codes),
        )
}

/// Secrets and recovery codes must not be kept by any cache.
fn uncached<T: serde::Serialize>(body: T) -> Response {
    (
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        Json(body),
    )
        .into_response()
}

async fn complete_sign_in(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<CompleteTwoFactorSignInRequest>,
) -> Result<Response, ApiError> {
    let now = Instant::now();
    if !record_authentication_attempt_per_client(&state, peer, &headers, now) {
        return Err(ApiError::rate_limited());
    }
    // Per account, wrong codes lock app codes in the database (`TwoFactor::check_code`); recovery
    // codes stay usable, so someone who has the password cannot lock the owner out.
    let session = state
        .two_factor
        .complete_sign_in(
            &state.accounts,
            &request.two_factor_token,
            &request.code,
            now,
            now_unix_milliseconds(),
        )
        .await?;
    session_response(StatusCode::OK, session, &headers)
}

async fn status(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<TwoFactorStatus>, ApiError> {
    Ok(Json(state.two_factor.status(user.id).await?))
}

/// The signed-in user with their password hash, once a code or password check is within the
/// session's budget.
async fn checked_user(
    state: &AppState,
    user: &mellow_protocol::CurrentUser,
    headers: &HeaderMap,
) -> Result<StoredUser, ApiError> {
    let token = session_token(headers)?;
    if !is_password_check_allowed(state, user, token) {
        return Err(ApiError::rate_limited());
    }
    Ok(state
        .accounts
        .verified_session_owner(token, now_unix_milliseconds())
        .await?)
}

async fn start_enrolment(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    headers: HeaderMap,
    ApiJson(request): ApiJson<StartTwoFactorEnrolmentRequest>,
) -> Result<Response, ApiError> {
    let user = checked_user(&state, &user, &headers).await?;
    let enrolment = state
        .two_factor
        .start_enrolment(&user, request.current_password, now_unix_milliseconds())
        .await?;
    Ok(uncached(enrolment))
}

async fn enable(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    headers: HeaderMap,
    ApiJson(request): ApiJson<EnableTwoFactorRequest>,
) -> Result<Response, ApiError> {
    let user = checked_user(&state, &user, &headers).await?;
    let codes = state
        .two_factor
        .enable(
            &state.accounts,
            session_token(&headers)?,
            user.id,
            &request.code,
            now_unix_milliseconds(),
        )
        .await?;
    Ok(uncached(codes))
}

async fn disable(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    headers: HeaderMap,
    ApiJson(request): ApiJson<ProveTwoFactorRequest>,
) -> Result<StatusCode, ApiError> {
    let user = checked_user(&state, &user, &headers).await?;
    state
        .two_factor
        .disable(
            &user,
            request.current_password,
            &request.code,
            now_unix_milliseconds(),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn regenerate_recovery_codes(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    headers: HeaderMap,
    ApiJson(request): ApiJson<ProveTwoFactorRequest>,
) -> Result<Response, ApiError> {
    let user = checked_user(&state, &user, &headers).await?;
    let codes = state
        .two_factor
        .regenerate_recovery_codes(
            &user,
            request.current_password,
            &request.code,
            now_unix_milliseconds(),
        )
        .await?;
    Ok(uncached(codes))
}

#[cfg(test)]
mod tests;
