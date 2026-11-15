//! Private calls in conversations (ADR-0062): starting one and declining one. Answering, joining
//! and hanging up go through the gateway's voice frames with the conversation's id.

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use mellow_protocol::{Call, Snowflake};

use crate::domain::clock::now_unix_milliseconds;
use crate::domain::guilds::GuildError;
use crate::http::AppState;
use crate::http::api_error::{ApiError, ApiPath};
use crate::http::authentication::AuthenticatedUser;
use crate::http::rate_limit::{
    CALL_DECLINES_PER_USER, CALL_STARTS_PER_CONVERSATION, CALL_STARTS_PER_USER,
    CALL_STARTS_PER_USER_PER_HOUR,
};
use crate::voice::CallError;

impl From<CallError> for ApiError {
    fn from(error: CallError) -> Self {
        match error {
            CallError::NotFound => Self::not_found(),
            CallError::VoiceUnavailable => {
                Self::validation("Voice is off on this instance.".into())
            }
            CallError::LimitReached => GuildError::LimitReached("calls going on at once").into(),
            CallError::AlreadyCalling => GuildError::Conflict.into(),
            CallError::Message(error) => error.into(),
        }
    }
}

/// Starts a call in the conversation, or answers the one going on there; the caller then joins
/// it with `voice_join`. Each start posts a message and rings people: limited per user (minute
/// and hour) and per conversation, whoever calls, so a group cannot be rung in a loop.
pub(crate) async fn start_call(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(channel_id): ApiPath<Snowflake>,
) -> Result<Json<Call>, ApiError> {
    let now = Instant::now();
    let within_burst = state.rate_limiter.record_event(
        &format!("call-start:{}", user.id),
        CALL_STARTS_PER_USER,
        now,
    );
    let within_hour = state.rate_limiter.record_event(
        &format!("call-start-hour:{}", user.id),
        CALL_STARTS_PER_USER_PER_HOUR,
        now,
    );
    let within_conversation = state.rate_limiter.record_event(
        &format!("call-start-channel:{channel_id}"),
        CALL_STARTS_PER_CONVERSATION,
        now,
    );
    if !(within_burst && within_hour && within_conversation) {
        return Err(ApiError::rate_limited());
    }
    Ok(Json(
        state
            .voice
            .start_call(user.id, channel_id, now_unix_milliseconds())
            .await?,
    ))
}

/// Stops the call ringing on all the user's devices. Always `204`: it tells nothing about a
/// call that does not ring them.
pub(crate) async fn decline_call(
    State(state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    ApiPath(channel_id): ApiPath<Snowflake>,
) -> Result<StatusCode, ApiError> {
    if !state.rate_limiter.record_event(
        &format!("call-decline:{}", user.id),
        CALL_DECLINES_PER_USER,
        Instant::now(),
    ) {
        return Err(ApiError::rate_limited());
    }
    state.voice.decline_call(user.id, channel_id).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use mellow_protocol::{ApiErrorBody, ApiErrorCode};

    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::tests::migrated_database;
    use crate::http::rate_limit::CALL_STARTS_PER_USER;
    use crate::http::test_support::{
        authorized, json, owner_and_general_channel, send, test_state,
    };

    #[tokio::test]
    async fn calls_answer_without_voice_decline_tells_nothing_and_starts_are_limited() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let call_path = format!("/api/v1/channels/{general}/call");

        let (status, body) = send(&state, authorized("POST", &call_path, &owner, None)).await;
        let without_voice = (status, json::<ApiErrorBody>(&body).code);
        let (declined, _) = send(
            &state,
            authorized("POST", &format!("{call_path}/decline"), &owner, None),
        )
        .await;
        let (unauthenticated, _) = send(
            &state,
            axum::http::Request::post(&call_path)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
        for _ in 1..CALL_STARTS_PER_USER.maximum_events {
            send(&state, authorized("POST", &call_path, &owner, None)).await;
        }
        let (over_the_limit, _) = send(&state, authorized("POST", &call_path, &owner, None)).await;
        database.close().await;

        assert_eq!(
            without_voice,
            (StatusCode::BAD_REQUEST, ApiErrorCode::ValidationFailed)
        );
        assert_eq!(declined, StatusCode::NO_CONTENT);
        assert_eq!(unauthenticated, StatusCode::UNAUTHORIZED);
        assert_eq!(over_the_limit, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn a_conversation_rings_a_bounded_number_of_times_whoever_calls() {
        use crate::http::test_support::guild_with_guest;

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
        // Any channel id works: the limit is counted before the conversation is read.
        let call_path = format!("/api/v1/channels/{guild_id}/call");
        let mut statuses = Vec::new();
        for caller in [&owner, &guest, &owner, &guest] {
            statuses.push(
                send(&state, authorized("POST", &call_path, caller, None))
                    .await
                    .0,
            );
        }
        database.close().await;

        assert_eq!(statuses[3], StatusCode::TOO_MANY_REQUESTS, "{statuses:?}");
        assert!(
            statuses[..3]
                .iter()
                .all(|status| *status != StatusCode::TOO_MANY_REQUESTS),
            "{statuses:?}"
        );
    }
}
