//! Message history, sending, editing, deleting, reactions, pins and read states. All require a
//! signed-in user.

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use mellow_protocol::{
    AcknowledgeRequest, DirectMessageChannel, EditMessageRequest, Message, MessageHistoryQuery,
    OpenDirectMessageRequest, PublicUser, ReactionUsersQuery, ReadState, SendMessageRequest,
    Snowflake,
};

use crate::domain::clock::now_unix_milliseconds;
use crate::domain::messages::ReactionRemoval;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiJson, ApiPath, ApiQuery};
use crate::http::authentication::AuthenticatedUser;
use crate::http::rate_limit::{
    DIRECT_MESSAGE_OPENS_PER_USER, DIRECT_MESSAGE_OPENS_PER_USER_PER_DAY, MESSAGE_WRITES_PER_USER,
    MESSAGE_WRITES_PER_USER_PER_HOUR, MESSAGES_PER_CHANNEL, REACTION_WRITES_PER_USER,
    REACTION_WRITES_PER_USER_PER_HOUR, READ_STATE_WRITES_PER_USER, TYPING_INDICATORS_PER_USER,
};

/// Send, edit and delete share the per-user budgets (burst and hourly).
fn limit_message_writes(state: &AppState, user_id: Snowflake) -> Result<(), ApiError> {
    let now = Instant::now();
    let within_burst = state.rate_limiter.record_event(
        &format!("message-write:{user_id}"),
        MESSAGE_WRITES_PER_USER,
        now,
    );
    let within_hour = state.rate_limiter.record_event(
        &format!("message-write-hour:{user_id}"),
        MESSAGE_WRITES_PER_USER_PER_HOUR,
        now,
    );
    if within_burst && within_hour {
        Ok(())
    } else {
        Err(ApiError::rate_limited())
    }
}

/// Adding and removing reactions share the per-user budgets (burst and hourly).
fn limit_reaction_writes(state: &AppState, user_id: Snowflake) -> Result<(), ApiError> {
    let now = Instant::now();
    let within_burst = state.rate_limiter.record_event(
        &format!("reaction-write:{user_id}"),
        REACTION_WRITES_PER_USER,
        now,
    );
    let within_hour = state.rate_limiter.record_event(
        &format!("reaction-write-hour:{user_id}"),
        REACTION_WRITES_PER_USER_PER_HOUR,
        now,
    );
    if within_burst && within_hour {
        Ok(())
    } else {
        Err(ApiError::rate_limited())
    }
}

/// `PUT /api/v1/channels/{channel_id}/messages/{message_id}/reactions/{emoji}/@me`. The emoji is
/// percent-encoded in the path; the router decodes it.
pub(crate) async fn add_reaction(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((channel_id, message_id, emoji)): ApiPath<(Snowflake, Snowflake, String)>,
) -> Result<StatusCode, ApiError> {
    limit_reaction_writes(&state, user.id)?;
    state
        .messages
        .add_reaction(user.id, channel_id, message_id, &emoji)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/channels/{channel_id}/messages/{message_id}/reactions/{emoji}/@me`.
pub(crate) async fn remove_own_reaction(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((channel_id, message_id, emoji)): ApiPath<(Snowflake, Snowflake, String)>,
) -> Result<StatusCode, ApiError> {
    limit_reaction_writes(&state, user.id)?;
    state
        .messages
        .remove_own_reaction(user.id, channel_id, message_id, &emoji)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/channels/{channel_id}/messages/{message_id}/reactions/{emoji}/{user_id}`: a
/// moderator removes someone's reaction.
pub(crate) async fn remove_user_reaction(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((channel_id, message_id, emoji, user_id)): ApiPath<(
        Snowflake,
        Snowflake,
        String,
        Snowflake,
    )>,
) -> Result<StatusCode, ApiError> {
    remove_reactions(
        &state,
        user.id,
        channel_id,
        message_id,
        ReactionRemoval::User { user_id, emoji },
    )
    .await
}

/// `DELETE /api/v1/channels/{channel_id}/messages/{message_id}/reactions/{emoji}`: a moderator
/// removes every reaction with this emoji.
pub(crate) async fn remove_emoji_reactions(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((channel_id, message_id, emoji)): ApiPath<(Snowflake, Snowflake, String)>,
) -> Result<StatusCode, ApiError> {
    remove_reactions(
        &state,
        user.id,
        channel_id,
        message_id,
        ReactionRemoval::Emoji(emoji),
    )
    .await
}

/// `DELETE /api/v1/channels/{channel_id}/messages/{message_id}/reactions`: a moderator removes
/// every reaction of the message.
pub(crate) async fn remove_all_reactions(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((channel_id, message_id)): ApiPath<(Snowflake, Snowflake)>,
) -> Result<StatusCode, ApiError> {
    remove_reactions(
        &state,
        user.id,
        channel_id,
        message_id,
        ReactionRemoval::All,
    )
    .await
}

async fn remove_reactions(
    state: &AppState,
    moderator_id: Snowflake,
    channel_id: Snowflake,
    message_id: Snowflake,
    removal: ReactionRemoval,
) -> Result<StatusCode, ApiError> {
    limit_reaction_writes(state, moderator_id)?;
    state
        .messages
        .remove_reactions(moderator_id, channel_id, message_id, removal)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/v1/channels/{channel_id}/messages/{message_id}/reactions/{emoji}`.
pub(crate) async fn list_reacting_users(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((channel_id, message_id, emoji)): ApiPath<(Snowflake, Snowflake, String)>,
    ApiQuery(query): ApiQuery<ReactionUsersQuery>,
) -> Result<Json<Vec<PublicUser>>, ApiError> {
    Ok(Json(
        state
            .messages
            .list_reacting_users(user.id, channel_id, message_id, &emoji, query)
            .await?,
    ))
}

/// `GET /api/v1/channels/{channel_id}/pins`.
pub(crate) async fn list_pins(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(channel_id): ApiPath<Snowflake>,
) -> Result<Json<Vec<Message>>, ApiError> {
    Ok(Json(state.messages.list_pins(user.id, channel_id).await?))
}

/// `PUT /api/v1/channels/{channel_id}/pins/{message_id}`. Counts as a message write.
pub(crate) async fn pin_message(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((channel_id, message_id)): ApiPath<(Snowflake, Snowflake)>,
) -> Result<StatusCode, ApiError> {
    limit_message_writes(&state, user.id)?;
    state
        .messages
        .pin(user.id, channel_id, message_id, now_unix_milliseconds())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/channels/{channel_id}/pins/{message_id}`. Counts as a message write.
pub(crate) async fn unpin_message(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((channel_id, message_id)): ApiPath<(Snowflake, Snowflake)>,
) -> Result<StatusCode, ApiError> {
    limit_message_writes(&state, user.id)?;
    state
        .messages
        .unpin(user.id, channel_id, message_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/users/@me/channels`: the conversation with that user, opened if needed.
pub(crate) async fn open_direct_message(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiJson(request): ApiJson<OpenDirectMessageRequest>,
) -> Result<Json<DirectMessageChannel>, ApiError> {
    let now = Instant::now();
    let within_burst = state.rate_limiter.record_event(
        &format!("direct-message-open:{}", user.id),
        DIRECT_MESSAGE_OPENS_PER_USER,
        now,
    );
    let within_day = state.rate_limiter.record_event(
        &format!("direct-message-open-day:{}", user.id),
        DIRECT_MESSAGE_OPENS_PER_USER_PER_DAY,
        now,
    );
    if !(within_burst && within_day) {
        return Err(ApiError::rate_limited());
    }
    Ok(Json(
        state
            .messages
            .open_direct_message(user.id, request, now_unix_milliseconds())
            .await?,
    ))
}

pub(crate) async fn send_message(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(channel_id): ApiPath<Snowflake>,
    ApiJson(request): ApiJson<SendMessageRequest>,
) -> Result<(StatusCode, Json<Message>), ApiError> {
    limit_message_writes(&state, user.id)?;
    if !state.rate_limiter.record_event(
        &format!("message-channel:{channel_id}"),
        MESSAGES_PER_CHANNEL,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    let message = state
        .messages
        .send(user.id, channel_id, request, now_unix_milliseconds())
        .await?;
    Ok((StatusCode::CREATED, Json(message)))
}

pub(crate) async fn list_messages(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(channel_id): ApiPath<Snowflake>,
    ApiQuery(query): ApiQuery<MessageHistoryQuery>,
) -> Result<Json<Vec<Message>>, ApiError> {
    Ok(Json(
        state
            .messages
            .list_messages(user.id, channel_id, query)
            .await?,
    ))
}

pub(crate) async fn edit_message(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((channel_id, message_id)): ApiPath<(Snowflake, Snowflake)>,
    ApiJson(request): ApiJson<EditMessageRequest>,
) -> Result<Json<Message>, ApiError> {
    limit_message_writes(&state, user.id)?;
    Ok(Json(
        state
            .messages
            .edit(
                user.id,
                channel_id,
                message_id,
                request,
                now_unix_milliseconds(),
            )
            .await?,
    ))
}

pub(crate) async fn delete_message(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath((channel_id, message_id)): ApiPath<(Snowflake, Snowflake)>,
) -> Result<StatusCode, ApiError> {
    limit_message_writes(&state, user.id)?;
    state
        .messages
        .delete(user.id, channel_id, message_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn acknowledge_read_state(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(channel_id): ApiPath<Snowflake>,
    ApiJson(request): ApiJson<AcknowledgeRequest>,
) -> Result<StatusCode, ApiError> {
    if !state.rate_limiter.record_event(
        &format!("read-state:{}", user.id),
        READ_STATE_WRITES_PER_USER,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    state
        .messages
        .acknowledge_read_state(user.id, channel_id, request)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list_my_read_states(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<ReadState>>, ApiError> {
    Ok(Json(state.messages.list_my_read_states(user.id).await?))
}

/// Rate limited per user: typing indicators are frequent but cheap to throttle.
pub(crate) async fn start_typing(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(channel_id): ApiPath<Snowflake>,
) -> Result<StatusCode, ApiError> {
    if !state.rate_limiter.record_event(
        &format!("typing:{}", user.id),
        TYPING_INDICATORS_PER_USER,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    state.messages.start_typing(user.id, channel_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
