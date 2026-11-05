//! Role icons (ADR-0050): a picture or a Unicode emoji, set by whoever manages the role.

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use mellow_protocol::{IconCropQuery, Role, SetRoleEmojiRequest, Snowflake};

use crate::domain::clock::now_unix_milliseconds;
use crate::domain::guild_rules::MAXIMUM_ROLE_ICON_UPLOAD_BYTES;
use crate::domain::guilds::{IconUpload, RoleIconRequest};
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiJson, ApiPath, ApiQuery};
use crate::http::attachment_routes::read_upload_body;
use crate::http::authentication::AuthenticatedUser;
use crate::http::guild_routes::limit_guild_writes;
use crate::http::profile_image_routes::{record_picture_write, square_crop_from_query};

async fn replace(
    state: &AppState,
    user_id: Snowflake,
    (guild_id, role_id): (Snowflake, Snowflake),
    request: RoleIconRequest,
) -> Result<Json<Role>, ApiError> {
    let role = state
        .guilds
        .replace_role_icon(
            &state.profile_images,
            user_id,
            guild_id,
            role_id,
            request,
            now_unix_milliseconds(),
        )
        .await?;
    Ok(Json(role))
}

/// `PUT /api/v1/guilds/{guild_id}/roles/{role_id}/icon`, the picture as the raw body (at most
/// 256 KiB), cropped by the query.
pub(crate) async fn put_role_icon(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(ids): ApiPath<(Snowflake, Snowflake)>,
    ApiQuery(query): ApiQuery<IconCropQuery>,
    body: Body,
) -> Result<Json<Role>, ApiError> {
    let crop = square_crop_from_query(query)?;
    // The picture budget, not guild writes: each upload decodes and encodes a picture.
    record_picture_write(&state, user.id)?;
    let bytes = read_upload_body(body, MAXIMUM_ROLE_ICON_UPLOAD_BYTES).await?;
    if bytes.is_empty() {
        return Err(ApiError::validation("a picture cannot be empty".into()));
    }
    let upload = IconUpload {
        bytes: Vec::from(bytes),
        crop,
    };
    replace(&state, user.id, ids, RoleIconRequest::Picture(upload)).await
}

/// `PUT /api/v1/guilds/{guild_id}/roles/{role_id}/icon/emoji`.
pub(crate) async fn put_role_emoji(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(ids): ApiPath<(Snowflake, Snowflake)>,
    ApiJson(request): ApiJson<SetRoleEmojiRequest>,
) -> Result<Json<Role>, ApiError> {
    limit_guild_writes(&state, user.id)?;
    let request = RoleIconRequest::Emoji(request.unicode_emoji);
    replace(&state, user.id, ids, request).await
}

/// `DELETE /api/v1/guilds/{guild_id}/roles/{role_id}/icon`: removes the picture or the emoji.
pub(crate) async fn delete_role_icon(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(ids): ApiPath<(Snowflake, Snowflake)>,
) -> Result<Json<Role>, ApiError> {
    limit_guild_writes(&state, user.id)?;
    replace(&state, user.id, ids, RoleIconRequest::Remove).await
}

#[cfg(test)]
mod tests;
