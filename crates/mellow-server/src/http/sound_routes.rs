//! A guild's soundboard sounds and members' join sounds (ADR-0032).

use std::time::Instant;

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use mellow_protocol::{
    CreateSoundQuery, SetJoinSoundRequest, Snowflake, SoundboardSound, UpdateSoundRequest,
};

use crate::domain::clock::now_unix_milliseconds;
use crate::domain::sound_rules::MAXIMUM_SOUND_FILE_BYTES;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiJson, ApiPath, ApiQuery};
use crate::http::attachment_routes::read_upload_body;
use crate::http::authentication::AuthenticatedUser;
use crate::http::guild_routes::limit_guild_writes;
use crate::http::rate_limit::{RateLimit, SOUND_DOWNLOADS_PER_USER, SOUND_UPLOADS_PER_USER};

/// A sound's file never changes: an edit changes its name or emoji only.
const CACHE_IMMUTABLE: &str = "private, max-age=31536000, immutable";

fn limit_per_user(
    state: &AppState,
    kind: &str,
    user_id: Snowflake,
    limit: RateLimit,
) -> Result<(), ApiError> {
    if state
        .rate_limiter
        .record_event(&format!("{kind}:{user_id}"), limit, Instant::now())
    {
        Ok(())
    } else {
        Err(ApiError::rate_limited())
    }
}

fn limit_sound_uploads(state: &AppState, user_id: Snowflake) -> Result<(), ApiError> {
    limit_per_user(state, "sound-upload", user_id, SOUND_UPLOADS_PER_USER)
}

fn limit_sound_downloads(state: &AppState, user_id: Snowflake) -> Result<(), ApiError> {
    limit_per_user(state, "sound-download", user_id, SOUND_DOWNLOADS_PER_USER)
}

/// `GET /api/v1/guilds/{guild_id}/sounds`: members only.
pub(crate) async fn list_sounds(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
) -> Result<Json<Vec<SoundboardSound>>, ApiError> {
    Ok(Json(state.guilds.list_sounds(user.id, guild_id).await?))
}

/// `POST /api/v1/guilds/{guild_id}/sounds?name=…&emoji=…`, the canonical WAV as the raw body.
pub(crate) async fn create_sound(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
    ApiQuery(query): ApiQuery<CreateSoundQuery>,
    body: Body,
) -> Result<(StatusCode, Json<SoundboardSound>), ApiError> {
    // Counted before the body is read, so refused uploads count too.
    limit_sound_uploads(&state, user.id)?;
    let bytes = read_upload_body(body, MAXIMUM_SOUND_FILE_BYTES).await?;
    let sound = state
        .guilds
        .create_sound(
            &state.profile_images,
            user.id,
            guild_id,
            &query.name,
            query.emoji.as_deref(),
            Vec::from(bytes),
            now_unix_milliseconds(),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(sound)))
}

/// `PATCH /api/v1/guilds/{guild_id}/sounds/{sound_id}`: replaces its name and emoji.
pub(crate) async fn update_sound(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((guild_id, sound_id)): ApiPath<(Snowflake, Snowflake)>,
    ApiJson(request): ApiJson<UpdateSoundRequest>,
) -> Result<Json<SoundboardSound>, ApiError> {
    limit_guild_writes(&state, user.id)?;
    Ok(Json(
        state
            .guilds
            .update_sound(
                user.id,
                guild_id,
                sound_id,
                &request.name,
                request.emoji.as_deref(),
            )
            .await?,
    ))
}

/// `DELETE /api/v1/guilds/{guild_id}/sounds/{sound_id}`.
pub(crate) async fn delete_sound(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((guild_id, sound_id)): ApiPath<(Snowflake, Snowflake)>,
) -> Result<StatusCode, ApiError> {
    limit_guild_writes(&state, user.id)?;
    state
        .guilds
        .delete_sound(&state.profile_images, user.id, guild_id, sound_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `PATCH /api/v1/guilds/{guild_id}/members/@me/join-sound`.
pub(crate) async fn set_join_sound(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
    ApiJson(request): ApiJson<SetJoinSoundRequest>,
) -> Result<StatusCode, ApiError> {
    limit_guild_writes(&state, user.id)?;
    state
        .guilds
        .set_join_sound(user.id, guild_id, request.sound_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/v1/sounds/{id}`: the WAV, to members of the sound's guild only.
pub(crate) async fn get_sound(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(id): ApiPath<Snowflake>,
) -> Result<Response, ApiError> {
    limit_sound_downloads(&state, user.id)?;
    let file_id = state.guilds.readable_sound_file_id(user.id, id).await?;
    let (bytes, permit) = state.profile_images.read_file(file_id).await?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("audio/wav")),
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
        crate::http::attachment_routes::body_keeping_permit(bytes, permit),
    )
        .into_response())
}
