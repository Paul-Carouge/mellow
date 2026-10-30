//! Guild, channel, role, invite and member endpoints. All require a signed-in user.

use std::net::SocketAddr;
use std::time::Instant;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use mellow_protocol::{
    AuditLogEntry, AuditLogQuery, Ban, Channel, CreateBanRequest, CreateChannelRequest,
    CreateGuildRequest, CreateInviteRequest, CreateRoleRequest, Guild, GuildInvite, GuildSummary,
    Invite, Member, ReorderChannelsRequest, ReorderRolesRequest, Role,
    SetPermissionOverwriteRequest, Snowflake, UpdateChannelRequest, UpdateGuildRequest,
    UpdateRoleRequest,
};

use crate::domain::clock::now_unix_milliseconds;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiJson, ApiPath, ApiQuery};
use crate::http::authentication::AuthenticatedUser;
use crate::http::client_address::client_address_key;
use crate::http::rate_limit::{
    GUILD_WRITES_PER_USER, INVITE_ATTEMPTS_PER_ADDRESS, INVITE_ATTEMPTS_PER_USER,
};

pub(super) fn limit_guild_writes(state: &AppState, user_id: Snowflake) -> Result<(), ApiError> {
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
        .create_guild(
            user.id,
            user.is_instance_administrator,
            request,
            now_unix_milliseconds(),
        )
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

pub(crate) async fn update_guild(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
    ApiJson(request): ApiJson<UpdateGuildRequest>,
) -> Result<Json<Guild>, ApiError> {
    limit_guild_writes(&state, user.id)?;
    Ok(Json(
        state
            .guilds
            .update_guild(user.id, guild_id, request)
            .await?,
    ))
}

pub(crate) async fn delete_guild(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
) -> Result<StatusCode, ApiError> {
    limit_guild_writes(&state, user.id)?;
    state.guilds.delete_guild(user.id, guild_id).await?;
    Ok(StatusCode::NO_CONTENT)
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

pub(crate) async fn list_invites(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
) -> Result<Json<Vec<GuildInvite>>, ApiError> {
    Ok(Json(
        state
            .guilds
            .list_invites(user.id, guild_id, now_unix_milliseconds())
            .await?,
    ))
}

pub(crate) async fn delete_invite(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(code): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    limit_guild_writes(&state, user.id)?;
    state.guilds.delete_invite(user.id, &code).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Rate limited per address and per user so invite codes cannot be guessed by brute force.
pub(crate) async fn join_with_invite(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(code): ApiPath<String>,
) -> Result<Json<Guild>, ApiError> {
    let address = client_address_key(peer, &headers, &state.trusted_proxy_networks);
    let now = Instant::now();
    let is_allowed = state.rate_limiter.record_event(
        &format!("invite:{address}"),
        INVITE_ATTEMPTS_PER_ADDRESS,
        now,
    ) && state.rate_limiter.record_event(
        &format!("invite-user:{}", user.id),
        INVITE_ATTEMPTS_PER_USER,
        now,
    );
    if !is_allowed {
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

pub(crate) async fn reorder_channels(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
    ApiJson(request): ApiJson<ReorderChannelsRequest>,
) -> Result<Json<Vec<Channel>>, ApiError> {
    limit_guild_writes(&state, user.id)?;
    Ok(Json(
        state
            .guilds
            .reorder_channels(user.id, guild_id, request)
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

pub(crate) async fn kick_member(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((guild_id, member_id)): ApiPath<(Snowflake, Snowflake)>,
) -> Result<StatusCode, ApiError> {
    limit_guild_writes(&state, user.id)?;
    state
        .guilds
        .kick_member(user.id, guild_id, member_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn ban_member(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((guild_id, target_id)): ApiPath<(Snowflake, Snowflake)>,
    ApiJson(request): ApiJson<CreateBanRequest>,
) -> Result<StatusCode, ApiError> {
    limit_guild_writes(&state, user.id)?;
    state
        .guilds
        .ban_member(
            user.id,
            guild_id,
            target_id,
            request,
            now_unix_milliseconds(),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn unban_member(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((guild_id, target_id)): ApiPath<(Snowflake, Snowflake)>,
) -> Result<StatusCode, ApiError> {
    limit_guild_writes(&state, user.id)?;
    state
        .guilds
        .unban_member(user.id, guild_id, target_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list_bans(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
) -> Result<Json<Vec<Ban>>, ApiError> {
    Ok(Json(state.guilds.list_bans(user.id, guild_id).await?))
}

pub(crate) async fn list_audit_log(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(guild_id): ApiPath<Snowflake>,
    ApiQuery(query): ApiQuery<AuditLogQuery>,
) -> Result<Json<Vec<AuditLogEntry>>, ApiError> {
    Ok(Json(
        state
            .guilds
            .list_audit_log(user.id, guild_id, query)
            .await?,
    ))
}
