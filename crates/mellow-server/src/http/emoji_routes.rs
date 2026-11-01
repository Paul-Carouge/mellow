//! A guild's custom emoji (ADR-0028). Their pictures are served by the icon routes.

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use mellow_protocol::{CreateEmojiQuery, CustomEmoji, Snowflake, UpdateEmojiRequest};

use crate::domain::clock::now_unix_milliseconds;
use crate::domain::guild_rules::MAXIMUM_EMOJI_UPLOAD_BYTES;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiJson, ApiPath, ApiQuery};
use crate::http::attachment_routes::read_upload_body;
use crate::http::authentication::AuthenticatedUser;
use crate::http::guild_routes::limit_guild_writes;
use crate::http::profile_image_routes::record_picture_write;

/// `GET /api/v1/guilds/{guild_id}/emojis`: members only.
pub(crate) async fn list_emojis(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
) -> Result<Json<Vec<CustomEmoji>>, ApiError> {
    Ok(Json(state.guilds.list_emojis(user.id, guild_id).await?))
}

/// `POST /api/v1/guilds/{guild_id}/emojis?name=…`, the picture as the raw body.
pub(crate) async fn create_emoji(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
    ApiQuery(query): ApiQuery<CreateEmojiQuery>,
    body: Body,
) -> Result<(StatusCode, Json<CustomEmoji>), ApiError> {
    // The picture budget, not guild writes: each upload decodes and encodes a picture.
    record_picture_write(&state, user.id)?;
    let bytes = read_upload_body(body, MAXIMUM_EMOJI_UPLOAD_BYTES).await?;
    if bytes.is_empty() {
        return Err(ApiError::validation("a picture cannot be empty".into()));
    }
    let emoji = state
        .guilds
        .create_emoji(
            &state.profile_images,
            user.id,
            guild_id,
            &query.name,
            Vec::from(bytes),
            now_unix_milliseconds(),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(emoji)))
}

/// `PATCH /api/v1/guilds/{guild_id}/emojis/{emoji_id}`: renames it.
pub(crate) async fn rename_emoji(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((guild_id, emoji_id)): ApiPath<(Snowflake, Snowflake)>,
    ApiJson(request): ApiJson<UpdateEmojiRequest>,
) -> Result<Json<CustomEmoji>, ApiError> {
    limit_guild_writes(&state, user.id)?;
    Ok(Json(
        state
            .guilds
            .rename_emoji(user.id, guild_id, emoji_id, &request.name)
            .await?,
    ))
}

/// `DELETE /api/v1/guilds/{guild_id}/emojis/{emoji_id}`.
pub(crate) async fn delete_emoji(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((guild_id, emoji_id)): ApiPath<(Snowflake, Snowflake)>,
) -> Result<StatusCode, ApiError> {
    limit_guild_writes(&state, user.id)?;
    state
        .guilds
        .delete_emoji(&state.profile_images, user.id, guild_id, emoji_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
