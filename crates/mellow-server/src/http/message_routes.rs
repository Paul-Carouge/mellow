//! Message history, sending, editing, deleting and read states. All require a signed-in user.

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use mellow_protocol::{
    AcknowledgeRequest, EditMessageRequest, Message, MessageHistoryQuery, ReadState,
    SendMessageRequest, Snowflake,
};

use crate::domain::clock::now_unix_milliseconds;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiJson, ApiPath, ApiQuery};
use crate::http::authentication::AuthenticatedUser;
use crate::http::rate_limit::{
    MESSAGE_WRITES_PER_USER, MESSAGE_WRITES_PER_USER_PER_HOUR, MESSAGES_PER_CHANNEL,
    READ_STATE_WRITES_PER_USER,
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
