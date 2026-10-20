//! Guild, channel, role, invite and member endpoints. All require a signed-in user.

use std::net::SocketAddr;
use std::time::Instant;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use mellow_protocol::{
    Channel, CreateChannelRequest, CreateGuildRequest, CreateInviteRequest, CreateRoleRequest,
    Guild, GuildSummary, Invite, Member, ReorderRolesRequest, Role, SetPermissionOverwriteRequest,
    Snowflake, UpdateChannelRequest, UpdateRoleRequest,
};

use crate::domain::clock::now_unix_milliseconds;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiJson, ApiPath};
use crate::http::authentication::AuthenticatedUser;
use crate::http::client_address::client_address_key;
use crate::http::rate_limit::{GUILD_WRITES_PER_USER, INVITE_ATTEMPTS_PER_ADDRESS};

fn limit_guild_writes(state: &AppState, user_id: Snowflake) -> Result<(), ApiError> {
    if state.rate_limiter.record_event(
        &format!("guild-write:{user_id}"),
        GUILD_WRITES_PER_USER,
        Instant::now(),
    ) {
        Ok(())
    } else {
        Err(ApiError::rate_limited())
    }
}

pub(crate) async fn create_guild(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiJson(request): ApiJson<CreateGuildRequest>,
) -> Result<(StatusCode, Json<Guild>), ApiError> {
    limit_guild_writes(&state, user.id)?;
    let guild = state
        .guilds
        .create_guild(user.id, request, now_unix_milliseconds())
        .await?;
    Ok((StatusCode::CREATED, Json(guild)))
}

pub(crate) async fn list_my_guilds(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<GuildSummary>>, ApiError> {
    Ok(Json(state.guilds.list_my_guilds(user.id).await?))
}

pub(crate) async fn get_guild(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
) -> Result<Json<Guild>, ApiError> {
    Ok(Json(state.guilds.get_guild(user.id, guild_id).await?))
}

pub(crate) async fn create_channel(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
    ApiJson(request): ApiJson<CreateChannelRequest>,
) -> Result<(StatusCode, Json<Channel>), ApiError> {
    limit_guild_writes(&state, user.id)?;
    let channel = state
        .guilds
        .create_channel(user.id, guild_id, request, now_unix_milliseconds())
        .await?;
    Ok((StatusCode::CREATED, Json(channel)))
}

pub(crate) async fn update_channel(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(channel_id): ApiPath<Snowflake>,
    ApiJson(request): ApiJson<UpdateChannelRequest>,
) -> Result<Json<Channel>, ApiError> {
    limit_guild_writes(&state, user.id)?;
    Ok(Json(
        state
            .guilds
            .update_channel(user.id, channel_id, request)
            .await?,
    ))
}

pub(crate) async fn delete_channel(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(channel_id): ApiPath<Snowflake>,
) -> Result<StatusCode, ApiError> {
    limit_guild_writes(&state, user.id)?;
    state.guilds.delete_channel(user.id, channel_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn create_invite(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
    ApiJson(request): ApiJson<CreateInviteRequest>,
) -> Result<(StatusCode, Json<Invite>), ApiError> {
    limit_guild_writes(&state, user.id)?;
    let invite = state
        .guilds
        .create_invite(user.id, guild_id, request, now_unix_milliseconds())
        .await?;
    Ok((StatusCode::CREATED, Json(invite)))
}

/// Rate limited per address so invite codes cannot be guessed by brute force.
pub(crate) async fn join_with_invite(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(code): ApiPath<String>,
) -> Result<Json<Guild>, ApiError> {
    let address = client_address_key(peer, &headers, &state.trusted_proxy_networks);
    if !state.rate_limiter.record_event(
        &format!("invite:{address}"),
        INVITE_ATTEMPTS_PER_ADDRESS,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    Ok(Json(
        state
            .guilds
            .join_with_invite(user.id, &code, now_unix_milliseconds())
            .await?,
    ))
}

pub(crate) async fn list_members(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
) -> Result<Json<Vec<Member>>, ApiError> {
    Ok(Json(state.guilds.list_members(user.id, guild_id).await?))
}

pub(crate) async fn leave_guild(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
) -> Result<StatusCode, ApiError> {
    state.guilds.leave_guild(user.id, guild_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn create_role(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
    ApiJson(request): ApiJson<CreateRoleRequest>,
) -> Result<(StatusCode, Json<Role>), ApiError> {
    limit_guild_writes(&state, user.id)?;
    let role = state
        .guilds
        .create_role(user.id, guild_id, request, now_unix_milliseconds())
        .await?;
    Ok((StatusCode::CREATED, Json(role)))
}

pub(crate) async fn update_role(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((guild_id, role_id)): ApiPath<(Snowflake, Snowflake)>,
    ApiJson(request): ApiJson<UpdateRoleRequest>,
) -> Result<Json<Role>, ApiError> {
    limit_guild_writes(&state, user.id)?;
    Ok(Json(
        state
            .guilds
            .update_role(user.id, guild_id, role_id, request)
            .await?,
    ))
}

pub(crate) async fn delete_role(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((guild_id, role_id)): ApiPath<(Snowflake, Snowflake)>,
) -> Result<StatusCode, ApiError> {
    limit_guild_writes(&state, user.id)?;
    state.guilds.delete_role(user.id, guild_id, role_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn add_member_role(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((guild_id, member_id, role_id)): ApiPath<(Snowflake, Snowflake, Snowflake)>,
) -> Result<StatusCode, ApiError> {
    limit_guild_writes(&state, user.id)?;
    state
        .guilds
        .add_member_role(user.id, guild_id, member_id, role_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn remove_member_role(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((guild_id, member_id, role_id)): ApiPath<(Snowflake, Snowflake, Snowflake)>,
) -> Result<StatusCode, ApiError> {
    limit_guild_writes(&state, user.id)?;
    state
        .guilds
        .remove_member_role(user.id, guild_id, member_id, role_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn reorder_roles(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
    ApiJson(request): ApiJson<ReorderRolesRequest>,
) -> Result<Json<Vec<Role>>, ApiError> {
    limit_guild_writes(&state, user.id)?;
    Ok(Json(
        state
            .guilds
            .reorder_roles(user.id, guild_id, request)
            .await?,
    ))
}

pub(crate) async fn set_permission_overwrite(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((channel_id, target_id)): ApiPath<(Snowflake, Snowflake)>,
    ApiJson(request): ApiJson<SetPermissionOverwriteRequest>,
) -> Result<Json<Channel>, ApiError> {
    limit_guild_writes(&state, user.id)?;
    Ok(Json(
        state
            .guilds
            .set_permission_overwrite(user.id, channel_id, target_id, request)
            .await?,
    ))
}

pub(crate) async fn delete_permission_overwrite(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((channel_id, target_id)): ApiPath<(Snowflake, Snowflake)>,
) -> Result<Json<Channel>, ApiError> {
    limit_guild_writes(&state, user.id)?;
    Ok(Json(
        state
            .guilds
            .delete_permission_overwrite(user.id, channel_id, target_id)
            .await?,
    ))
}
