//! `/api/v1/auth/*` and `/api/v1/users/@me`.

use std::net::SocketAddr;
use std::time::Instant;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use mellow_protocol::{
    ChangePasswordRequest, CurrentUser, LoginRequest, NameAvailability, RegisterRequest,
    SessionCreated, UpdateCurrentUserRequest, UpdatePresenceRequest,
};
use sha2::{Digest as _, Sha256};

use crate::domain::account_names::account_name_key;
use crate::domain::account_rules::normalize_login;
use crate::domain::accounts::AccountError;
use crate::domain::clock::now_unix_milliseconds;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiJson, ApiQuery};
use crate::http::authentication::{
    AuthenticatedUser, expired_session_cookie_header, session_cookie_header, session_token,
    wants_cookie_session,
};
use crate::http::client_address::{client_address_key, client_ipv6_site_key};
use crate::http::rate_limit::{
    AUTHENTICATING_ADDRESSES_PER_IPV6_SITE, AUTHENTICATION_ATTEMPTS_PER_ADDRESS,
    LOGIN_ATTEMPTS_PER_ACCOUNT, LOGIN_ATTEMPTS_PER_ACCOUNT_AND_ADDRESS,
    NAME_AVAILABILITY_CHECKS_PER_USER, PASSWORD_CHECKS_PER_SESSION, PASSWORD_CHECKS_PER_USER,
    PROFILE_WRITES_PER_SESSION, PROFILE_WRITES_PER_USER,
};

/// Enough of SHA-256 to make rate-limit keys unique per login or session while keeping them small.
const RATE_LIMIT_KEY_HASH_BYTES: usize = 16;

/// Fixed-size key for a login or a session token, so attacker-chosen values cannot grow the
/// limiter's memory.
/// shortcut: a username and its email give two keys for one account; acceptable at these limits.
fn fixed_size_key(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .take(RATE_LIMIT_KEY_HASH_BYTES)
        .flat_map(|byte| [byte >> 4, byte & 0x0F])
        // A nibble is always below 16, so `from_digit` never drops one.
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}

/// A response carrying a session token must not be stored by any cache (RFC 6749 §5.1). The web
/// client gets the token in an `HttpOnly` cookie instead, out of reach of page scripts.
fn session_response(
    status: StatusCode,
    mut session: SessionCreated,
    request_headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let cookie = if wants_cookie_session(request_headers) {
        let cookie = session_cookie_header(&session.token)
            .ok_or_else(|| ApiError::internal("the session cookie", &"invalid header value"))?;
        session.token = String::new();
        Some(cookie)
    } else {
        None
    };
    let mut response = (status, Json(session)).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(cookie) = cookie {
        headers.insert(header::SET_COOKIE, cookie);
    }
    Ok(response)
}

/// Counts a registration or login attempt against the client's address. An address without a live
/// window first takes one unit of its IPv6 /48's new-address budget: one /48 cycling through its
/// 65 536 /64s adds at most a site's worth of address keys per window, while a single /64 over its
/// own limit cannot use that budget up. Returns whether the limits still allow the attempt.
fn record_authentication_attempt_per_client(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
    now: Instant,
) -> bool {
    let trusted_proxy_networks = &state.trusted_proxy_networks;
    let site_key = client_ipv6_site_key(peer, headers, trusted_proxy_networks)
        .map(|site| format!("authentication-site:{site}"));
    state
        .unauthenticated_rate_limiter
        .record_address_event_within_site(
            &format!(
                "authentication:{}",
                client_address_key(peer, headers, trusted_proxy_networks)
            ),
            AUTHENTICATION_ATTEMPTS_PER_ADDRESS,
            site_key.as_deref(),
            AUTHENTICATING_ADDRESSES_PER_IPV6_SITE,
            now,
        )
}

pub(crate) async fn register(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<RegisterRequest>,
) -> Result<Response, ApiError> {
    if !record_authentication_attempt_per_client(&state, peer, &headers, Instant::now()) {
        return Err(ApiError::rate_limited());
    }
    let session = state
        .accounts
        .register(request, now_unix_milliseconds())
        .await?;
    session_response(StatusCode::CREATED, session, &headers)
}

pub(crate) async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<LoginRequest>,
) -> Result<Response, ApiError> {
    let address = client_address_key(peer, &headers, &state.trusted_proxy_networks);
    // Keyed on the folded name key sign-in looks accounts up by, so respelling one name (accents,
    // full-width letters, spaces) cannot reach one account under fresh limits. An email folds too,
    // which only merges keys.
    let account = fixed_size_key(&account_name_key(
        &normalize_login(&request.login).map_err(AccountError::from)?,
    ));
    let now = Instant::now();
    // Every attempt is counted before the password check, so concurrent requests cannot all
    // pass under the limit while Argon2 runs. The per-account limits are recorded only for
    // attempts the site and address limits let through: otherwise one address cycling login names would
    // add two long-lived keys per request until the limiter's table is full.
    if !record_authentication_attempt_per_client(&state, peer, &headers, now) {
        return Err(ApiError::rate_limited());
    }
    let limiter = &state.unauthenticated_rate_limiter;
    let within_account_and_address_limit = limiter.record_event(
        &format!("login:{account}:{address}"),
        LOGIN_ATTEMPTS_PER_ACCOUNT_AND_ADDRESS,
        now,
    );
    let within_account_limit =
        limiter.record_event(&format!("login:{account}"), LOGIN_ATTEMPTS_PER_ACCOUNT, now);
    if !(within_account_and_address_limit && within_account_limit) {
        return Err(ApiError::rate_limited());
    }
    let session = state
        .accounts
        .login(request, now_unix_milliseconds())
        .await?;
    session_response(StatusCode::OK, session, &headers)
}

pub(crate) async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    state.accounts.logout(session_token(&headers)?).await?;
    Ok((
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, expired_session_cookie_header())],
    )
        .into_response())
}

pub(crate) async fn current_user(AuthenticatedUser(user): AuthenticatedUser) -> Json<CurrentUser> {
    Json(user)
}

/// Counts a current-password check, before Argon2 runs. The user's budget is counted only for checks
/// the session's budget allows, so one session adds at most its own budget to it and cannot fill it.
fn is_password_check_allowed(state: &AppState, user: &CurrentUser, token: &str) -> bool {
    let now = Instant::now();
    state.rate_limiter.record_event(
        &format!("password-check:{}", fixed_size_key(token)),
        PASSWORD_CHECKS_PER_SESSION,
        now,
    ) && state.rate_limiter.record_event(
        &format!("password-check:{}", user.id),
        PASSWORD_CHECKS_PER_USER,
        now,
    )
}

/// Counts a profile write. The user's budget counts only what the session's allowed, as for
/// password checks.
fn is_profile_write_allowed(state: &AppState, user: &CurrentUser, token: &str) -> bool {
    let now = Instant::now();
    state.rate_limiter.record_event(
        &format!("profile:{}", fixed_size_key(token)),
        PROFILE_WRITES_PER_SESSION,
        now,
    ) && state.rate_limiter.record_event(
        &format!("profile:{}", user.id),
        PROFILE_WRITES_PER_USER,
        now,
    )
}

pub(crate) async fn update_current_user(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    headers: HeaderMap,
    ApiJson(request): ApiJson<UpdateCurrentUserRequest>,
) -> Result<Json<CurrentUser>, ApiError> {
    let token = session_token(&headers)?;
    if !is_profile_write_allowed(&state, &user, token) {
        return Err(ApiError::rate_limited());
    }
    Ok(Json(
        state
            .accounts
            .update_current_user(token, request, now_unix_milliseconds())
            .await?,
    ))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NameAvailabilityQuery {
    name: String,
}

/// Answers whether the name is free, never who holds it; rate-limited so names cannot be listed.
pub(crate) async fn name_availability(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiQuery(query): ApiQuery<NameAvailabilityQuery>,
) -> Result<Json<NameAvailability>, ApiError> {
    if !state.rate_limiter.record_event(
        &format!("name-availability:{}", user.id),
        NAME_AVAILABILITY_CHECKS_PER_USER,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    let is_available = state
        .accounts
        .is_name_available(user.id, &query.name)
        .await?;
    Ok(Json(NameAvailability { is_available }))
}

pub(crate) async fn update_presence(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    headers: HeaderMap,
    ApiJson(request): ApiJson<UpdatePresenceRequest>,
) -> Result<Json<CurrentUser>, ApiError> {
    let token = session_token(&headers)?;
    // Shares the profile budget: each change is a write and a fan-out to every guild of the user.
    if !is_profile_write_allowed(&state, &user, token) {
        return Err(ApiError::rate_limited());
    }
    Ok(Json(
        state
            .accounts
            .update_presence(token, request, now_unix_milliseconds())
            .await?,
    ))
}

pub(crate) async fn change_password(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    headers: HeaderMap,
    ApiJson(request): ApiJson<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    let token = session_token(&headers)?;
    if !is_password_check_allowed(&state, &user, token) {
        return Err(ApiError::rate_limited());
    }
    state
        .accounts
        .change_password(token, request, now_unix_milliseconds())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests;
