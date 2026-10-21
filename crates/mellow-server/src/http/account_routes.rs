//! `/api/v1/auth/*` and `/api/v1/users/@me`.

use std::net::SocketAddr;
use std::time::Instant;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use mellow_protocol::{
    ChangePasswordRequest, CurrentUser, LoginRequest, RegisterRequest, SessionCreated,
    UpdateCurrentUserRequest,
};
use sha2::{Digest as _, Sha256};

use crate::domain::account_rules::normalize_login;
use crate::domain::accounts::AccountError;
use crate::domain::clock::now_unix_milliseconds;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiJson};
use crate::http::authentication::{AuthenticatedUser, bearer_token};
use crate::http::client_address::client_address_key;
use crate::http::rate_limit::{
    AUTHENTICATION_ATTEMPTS_PER_ADDRESS, LOGIN_ATTEMPTS_PER_ACCOUNT,
    LOGIN_ATTEMPTS_PER_ACCOUNT_AND_ADDRESS, PASSWORD_CHANGE_ATTEMPTS_PER_USER,
    PROFILE_WRITES_PER_USER,
};

/// Enough of SHA-256 to make rate-limit keys unique per login while keeping them small.
const ACCOUNT_KEY_HASH_BYTES: usize = 16;

/// Fixed-size key for a login, so attacker-chosen logins cannot grow the limiter's memory.
/// shortcut: a username and its email give two keys for one account; acceptable at these limits.
fn account_key(normalized_login: &str) -> String {
    Sha256::digest(normalized_login.as_bytes())
        .iter()
        .take(ACCOUNT_KEY_HASH_BYTES)
        .flat_map(|byte| [byte >> 4, byte & 0x0F])
        // A nibble is always below 16, so `from_digit` never drops one.
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}

/// A response carrying a session token must not be stored by any cache (RFC 6749 §5.1).
fn session_response(status: StatusCode, session: SessionCreated) -> Response {
    let mut response = (status, Json(session)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(crate) async fn register(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<RegisterRequest>,
) -> Result<Response, ApiError> {
    let address = client_address_key(peer, &headers, &state.trusted_proxy_networks);
    if !state.rate_limiter.record_event(
        &format!("authentication:{address}"),
        AUTHENTICATION_ATTEMPTS_PER_ADDRESS,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    let session = state
        .accounts
        .register(request, now_unix_milliseconds())
        .await?;
    Ok(session_response(StatusCode::CREATED, session))
}

pub(crate) async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<LoginRequest>,
) -> Result<Response, ApiError> {
    let address = client_address_key(peer, &headers, &state.trusted_proxy_networks);
    let account = account_key(&normalize_login(&request.login).map_err(AccountError::from)?);
    let now = Instant::now();
    // Every attempt is counted before the password check, so concurrent requests cannot all
    // pass under the limit while Argon2 runs. All three limits are always recorded.
    let within_address_limit = state.rate_limiter.record_event(
        &format!("authentication:{address}"),
        AUTHENTICATION_ATTEMPTS_PER_ADDRESS,
        now,
    );
    let within_account_and_address_limit = state.rate_limiter.record_event(
        &format!("login:{account}:{address}"),
        LOGIN_ATTEMPTS_PER_ACCOUNT_AND_ADDRESS,
        now,
    );
    let within_account_limit = state.rate_limiter.record_event(
        &format!("login:{account}"),
        LOGIN_ATTEMPTS_PER_ACCOUNT,
        now,
    );
    if !(within_address_limit && within_account_and_address_limit && within_account_limit) {
        return Err(ApiError::rate_limited());
    }
    let session = state
        .accounts
        .login(request, now_unix_milliseconds())
        .await?;
    Ok(session_response(StatusCode::OK, session))
}

pub(crate) async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    state.accounts.logout(bearer_token(&headers)?).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn current_user(AuthenticatedUser(user): AuthenticatedUser) -> Json<CurrentUser> {
    Json(user)
}

pub(crate) async fn update_current_user(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiJson(request): ApiJson<UpdateCurrentUserRequest>,
) -> Result<Json<CurrentUser>, ApiError> {
    if !state.rate_limiter.record_event(
        &format!("profile:{}", user.id),
        PROFILE_WRITES_PER_USER,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    Ok(Json(
        state.accounts.update_current_user(user, request).await?,
    ))
}

pub(crate) async fn change_password(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    headers: HeaderMap,
    ApiJson(request): ApiJson<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    // Counted before the password check, as for logins.
    if !state.rate_limiter.record_event(
        &format!("password-change:{}", user.id),
        PASSWORD_CHANGE_ATTEMPTS_PER_USER,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    state
        .accounts
        .change_password(bearer_token(&headers)?, request, now_unix_milliseconds())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
