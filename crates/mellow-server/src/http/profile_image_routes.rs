//! Avatars and profile banners (ADR-0021).

use std::time::Instant;

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use mellow_protocol::{CurrentUser, Snowflake};

use crate::domain::clock::now_unix_milliseconds;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiPath};
use crate::http::authentication::AuthenticatedUser;
use crate::http::rate_limit::{ATTACHMENT_DOWNLOADS_PER_USER, PROFILE_IMAGE_WRITES_PER_USER};
use crate::storage::profile_images::ProfileImageSlot;

/// Pictures larger than this are refused before processing (ADR-0021).
pub(crate) const MAXIMUM_PROFILE_IMAGE_BYTES: usize = 8 * 1024 * 1024;
/// Each version has its own id and never changes.
const CACHE_IMMUTABLE: &str = "private, max-age=31536000, immutable";

async fn replace(
    state: &AppState,
    user: CurrentUser,
    slot: ProfileImageSlot,
    body: Option<Body>,
) -> Result<Json<CurrentUser>, ApiError> {
    if !state.rate_limiter.record_event(
        &format!("profile-image:{}", user.id),
        PROFILE_IMAGE_WRITES_PER_USER,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    let bytes = match body {
        Some(body) => {
            let bytes = axum::body::to_bytes(body, MAXIMUM_PROFILE_IMAGE_BYTES)
                .await
                .map_err(|_| ApiError::payload_too_large())?;
            if bytes.is_empty() {
                return Err(ApiError::validation("a picture cannot be empty".into()));
            }
            Some(Vec::from(bytes))
        }
        None => None,
    };
    let updated = state
        .profile_images
        .replace(user, slot, bytes, now_unix_milliseconds())
        .await?;
    Ok(Json(updated))
}

/// `PUT /api/v1/users/@me/avatar`, the picture as the raw body.
pub(crate) async fn put_avatar(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    body: Body,
) -> Result<Json<CurrentUser>, ApiError> {
    replace(&state, user, ProfileImageSlot::Avatar, Some(body)).await
}

pub(crate) async fn delete_avatar(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<CurrentUser>, ApiError> {
    replace(&state, user, ProfileImageSlot::Avatar, None).await
}

/// `PUT /api/v1/users/@me/banner`, the picture as the raw body.
pub(crate) async fn put_banner(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    body: Body,
) -> Result<Json<CurrentUser>, ApiError> {
    replace(&state, user, ProfileImageSlot::Banner, Some(body)).await
}

pub(crate) async fn delete_banner(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<CurrentUser>, ApiError> {
    replace(&state, user, ProfileImageSlot::Banner, None).await
}

async fn serve(
    state: &AppState,
    user_id: Snowflake,
    id: Snowflake,
    is_still_wanted: bool,
) -> Result<Response, ApiError> {
    if !state.rate_limiter.record_event(
        &format!("attachment-download:{user_id}"),
        ATTACHMENT_DOWNLOADS_PER_USER,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    let bytes = state.profile_images.read(id, is_still_wanted).await?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("image/webp")),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("inline"),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("default-src 'none'; sandbox"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static(CACHE_IMMUTABLE),
            ),
        ],
        bytes,
    )
        .into_response())
}

/// `GET /api/v1/profile-images/{id}`: animated when the version is.
pub(crate) async fn get_profile_image(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(id): ApiPath<Snowflake>,
) -> Result<Response, ApiError> {
    serve(&state, user.id, id, false).await
}

/// `GET /api/v1/profile-images/{id}/still`: the first frame of an animated version.
pub(crate) async fn get_profile_image_still(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(id): ApiPath<Snowflake>,
) -> Result<Response, ApiError> {
    serve(&state, user.id, id, true).await
}
