//! Avatars, profile banners, and guild and channel icons (ADR-0021).

use std::time::Instant;

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use mellow_protocol::{Channel, CurrentUser, Guild, IconCropQuery, Snowflake};

use crate::domain::clock::now_unix_milliseconds;
use crate::domain::guilds::IconUpload;
use crate::domain::image_processing::SquareCrop;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiPath, ApiQuery};
use crate::http::attachment_routes::read_upload_body;
use crate::http::authentication::AuthenticatedUser;
use crate::http::rate_limit::{PROFILE_IMAGE_DOWNLOADS_PER_USER, PROFILE_IMAGE_WRITES_PER_USER};
use crate::storage::profile_images::ProfileImageSlot;

/// Pictures larger than this are refused before processing (ADR-0021).
pub(crate) const MAXIMUM_PROFILE_IMAGE_BYTES: usize = 8 * 1024 * 1024;
/// Each version has its own id and never changes.
const CACHE_IMMUTABLE: &str = "private, max-age=31536000, immutable";

/// Counts a picture change of `user_id`, whatever it is for (avatars, banners, icons, emoji),
/// before anything is read or processed.
pub(crate) fn record_picture_write(state: &AppState, user_id: Snowflake) -> Result<(), ApiError> {
    if state.rate_limiter.record_event(
        &format!("profile-image:{user_id}"),
        PROFILE_IMAGE_WRITES_PER_USER,
        Instant::now(),
    ) {
        Ok(())
    } else {
        Err(ApiError::rate_limited())
    }
}

/// Counts a picture change of `user_id`, whatever it is for, and reads the uploaded picture.
async fn read_upload(
    state: &AppState,
    user_id: Snowflake,
    body: Option<Body>,
) -> Result<Option<Vec<u8>>, ApiError> {
    record_picture_write(state, user_id)?;
    let Some(body) = body else {
        return Ok(None);
    };
    let bytes = read_upload_body(body, MAXIMUM_PROFILE_IMAGE_BYTES).await?;
    if bytes.is_empty() {
        return Err(ApiError::validation("a picture cannot be empty".into()));
    }
    Ok(Some(Vec::from(bytes)))
}

async fn replace(
    state: &AppState,
    user: CurrentUser,
    slot: ProfileImageSlot,
    body: Option<Body>,
) -> Result<Json<CurrentUser>, ApiError> {
    let bytes = read_upload(state, user.id, body).await?;
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

fn record_download(state: &AppState, user_id: Snowflake) -> Result<(), ApiError> {
    if state.rate_limiter.record_event(
        &format!("profile-image-download:{user_id}"),
        PROFILE_IMAGE_DOWNLOADS_PER_USER,
        Instant::now(),
    ) {
        Ok(())
    } else {
        Err(ApiError::rate_limited())
    }
}

async fn serve(
    state: &AppState,
    user_id: Snowflake,
    id: Snowflake,
    is_still_wanted: bool,
) -> Result<Response, ApiError> {
    record_download(state, user_id)?;
    let (bytes, permit) = state.profile_images.read(id, is_still_wanted).await?;
    Ok(picture_response(bytes, permit))
}

fn picture_response(bytes: Vec<u8>, permit: tokio::sync::OwnedSemaphorePermit) -> Response {
    (
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
        crate::http::attachment_routes::body_keeping_permit(bytes, permit),
    )
        .into_response()
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

/// All three crop values or none, as `IconCropQuery` says.
fn square_crop_from_query(query: IconCropQuery) -> Result<Option<SquareCrop>, ApiError> {
    match (query.crop_x, query.crop_y, query.crop_size) {
        (Some(x), Some(y), Some(size)) if size > 0 => Ok(Some(SquareCrop { x, y, size })),
        (None, None, None) => Ok(None),
        _ => Err(ApiError::validation(
            "crop_x, crop_y and crop_size go together, with a size of at least 1".into(),
        )),
    }
}

async fn icon_upload(
    state: &AppState,
    user_id: Snowflake,
    query: Option<IconCropQuery>,
    body: Option<Body>,
) -> Result<Option<IconUpload>, ApiError> {
    let crop = query.map(square_crop_from_query).transpose()?.flatten();
    Ok(read_upload(state, user_id, body)
        .await?
        .map(|bytes| IconUpload { bytes, crop }))
}

/// `PUT /api/v1/guilds/{guild_id}/icon`, the picture as the raw body, cropped by the query.
pub(crate) async fn put_guild_icon(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
    ApiQuery(query): ApiQuery<IconCropQuery>,
    body: Body,
) -> Result<Json<Guild>, ApiError> {
    let upload = icon_upload(&state, user.id, Some(query), Some(body)).await?;
    replace_guild_icon(&state, user.id, guild_id, upload).await
}

pub(crate) async fn delete_guild_icon(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
) -> Result<Json<Guild>, ApiError> {
    let upload = icon_upload(&state, user.id, None, None).await?;
    replace_guild_icon(&state, user.id, guild_id, upload).await
}

async fn replace_guild_icon(
    state: &AppState,
    user_id: Snowflake,
    guild_id: Snowflake,
    upload: Option<IconUpload>,
) -> Result<Json<Guild>, ApiError> {
    let guild = state
        .guilds
        .replace_guild_icon(
            &state.profile_images,
            user_id,
            guild_id,
            upload,
            now_unix_milliseconds(),
        )
        .await?;
    Ok(Json(guild))
}

/// `PUT /api/v1/channels/{channel_id}/icon`, the picture as the raw body, cropped by the query.
pub(crate) async fn put_channel_icon(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(channel_id): ApiPath<Snowflake>,
    ApiQuery(query): ApiQuery<IconCropQuery>,
    body: Body,
) -> Result<Json<Channel>, ApiError> {
    let upload = icon_upload(&state, user.id, Some(query), Some(body)).await?;
    replace_channel_icon(&state, user.id, channel_id, upload).await
}

pub(crate) async fn delete_channel_icon(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(channel_id): ApiPath<Snowflake>,
) -> Result<Json<Channel>, ApiError> {
    let upload = icon_upload(&state, user.id, None, None).await?;
    replace_channel_icon(&state, user.id, channel_id, upload).await
}

async fn replace_channel_icon(
    state: &AppState,
    user_id: Snowflake,
    channel_id: Snowflake,
    upload: Option<IconUpload>,
) -> Result<Json<Channel>, ApiError> {
    let channel = state
        .guilds
        .replace_channel_icon(
            &state.profile_images,
            user_id,
            channel_id,
            upload,
            now_unix_milliseconds(),
        )
        .await?;
    Ok(Json(channel))
}

async fn serve_icon(
    state: &AppState,
    user_id: Snowflake,
    id: Snowflake,
    is_still_wanted: bool,
) -> Result<Response, ApiError> {
    record_download(state, user_id)?;
    let file_id = state
        .guilds
        .readable_icon_file_id(user_id, id, is_still_wanted)
        .await?;
    let (bytes, permit) = state.profile_images.read_file(file_id).await?;
    Ok(picture_response(bytes, permit))
}

/// `GET /api/v1/icons/{id}`: animated when the version is.
pub(crate) async fn get_icon(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(id): ApiPath<Snowflake>,
) -> Result<Response, ApiError> {
    serve_icon(&state, user.id, id, false).await
}

/// `GET /api/v1/icons/{id}/still`: the first frame of an animated version.
pub(crate) async fn get_icon_still(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(id): ApiPath<Snowflake>,
) -> Result<Response, ApiError> {
    serve_icon(&state, user.id, id, true).await
}
