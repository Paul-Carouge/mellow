//! Friends, friend requests and blocks of the signed-in user (ADR-0025).

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use mellow_protocol::{Relationship, SendFriendRequest, SetRelationshipRequest, Snowflake};

use crate::domain::clock::now_unix_milliseconds;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiJson, ApiPath};
use crate::http::authentication::AuthenticatedUser;
use crate::http::rate_limit::{
    FRIEND_REQUESTS_PER_USER, FRIEND_REQUESTS_PER_USER_PER_DAY, RELATIONSHIP_WRITES_PER_USER,
};

fn limit_relationship_writes(state: &AppState, user_id: Snowflake) -> Result<(), ApiError> {
    if state.rate_limiter.record_event(
        &format!("relationship-write:{user_id}"),
        RELATIONSHIP_WRITES_PER_USER,
        Instant::now(),
    ) {
        Ok(())
    } else {
        Err(ApiError::rate_limited())
    }
}

pub(crate) async fn list_relationships(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<Relationship>>, ApiError> {
    Ok(Json(state.relationships.list(user.id).await?))
}

/// Each request names a username, so both budgets also bound how fast accounts can be probed.
pub(crate) async fn send_friend_request(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiJson(request): ApiJson<SendFriendRequest>,
) -> Result<Json<Relationship>, ApiError> {
    let now = Instant::now();
    let within_burst = state.rate_limiter.record_event(
        &format!("friend-request:{}", user.id),
        FRIEND_REQUESTS_PER_USER,
        now,
    );
    let within_day = state.rate_limiter.record_event(
        &format!("friend-request-day:{}", user.id),
        FRIEND_REQUESTS_PER_USER_PER_DAY,
        now,
    );
    if !(within_burst && within_day) {
        return Err(ApiError::rate_limited());
    }
    Ok(Json(
        state
            .relationships
            .send_friend_request(user.id, request, now_unix_milliseconds())
            .await?,
    ))
}

pub(crate) async fn set_relationship(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(other_user_id): ApiPath<Snowflake>,
    ApiJson(request): ApiJson<SetRelationshipRequest>,
) -> Result<Json<Relationship>, ApiError> {
    limit_relationship_writes(&state, user.id)?;
    Ok(Json(
        state
            .relationships
            .set(
                user.id,
                other_user_id,
                request.kind,
                now_unix_milliseconds(),
            )
            .await?,
    ))
}

pub(crate) async fn remove_relationship(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(other_user_id): ApiPath<Snowflake>,
) -> Result<StatusCode, ApiError> {
    limit_relationship_writes(&state, user.id)?;
    state.relationships.remove(user.id, other_user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
