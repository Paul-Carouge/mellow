//! "About me" texts, profile links and their site icons (ADR-0050).

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::response::Response;
use mellow_protocol::{Snowflake, UpdateUserProfileRequest, UserProfile};

use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiJson, ApiPath};
use crate::http::authentication::AuthenticatedUser;
use crate::http::profile_image_routes::{picture_response, record_download};
use crate::http::rate_limit::{PROFILE_DETAILS_WRITES_PER_USER, PROFILE_READS_PER_USER};

fn record_profile_read(state: &AppState, user_id: Snowflake) -> Result<(), ApiError> {
    if state.rate_limiter.record_event(
        &format!("profile-read:{user_id}"),
        PROFILE_READS_PER_USER,
        Instant::now(),
    ) {
        Ok(())
    } else {
        Err(ApiError::rate_limited())
    }
}

/// `GET /api/v1/users/@me/profile`.
pub(crate) async fn get_my_profile(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<UserProfile>, ApiError> {
    record_profile_read(&state, user.id)?;
    Ok(Json(state.profiles.profile(user.id, user.id).await?))
}

/// `GET /api/v1/users/{user_id}/profile`: `NOT_FOUND` unless the reader shares a server, a
/// conversation or a relationship with the user.
pub(crate) async fn get_user_profile(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(user_id): ApiPath<Snowflake>,
) -> Result<Json<UserProfile>, ApiError> {
    record_profile_read(&state, user.id)?;
    Ok(Json(state.profiles.profile(user.id, user_id).await?))
}

/// `PUT /api/v1/users/@me/profile`: replaces the text and the links.
pub(crate) async fn put_my_profile(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiJson(request): ApiJson<UpdateUserProfileRequest>,
) -> Result<Json<UserProfile>, ApiError> {
    if !state.rate_limiter.record_event(
        &format!("profile-details:{}", user.id),
        PROFILE_DETAILS_WRITES_PER_USER,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    Ok(Json(state.profiles.update(user.id, &request).await?))
}

/// `GET /api/v1/link-icons/{id}`: a site's icon, cached for good (each version has its own id).
pub(crate) async fn get_link_icon(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(id): ApiPath<Snowflake>,
) -> Result<Response, ApiError> {
    record_download(&state, user.id)?;
    let (bytes, permit) = state.profiles.link_icon(id).await?;
    Ok(picture_response(bytes, permit))
}

#[cfg(test)]
mod tests;
