//! REST API (axum). Handlers validate input and map domain errors to HTTP; no SQL or business
//! rules live here (ARCHITECTURE.md §4.1).

mod account_routes;
pub(crate) mod api_error;
mod authentication;
pub(crate) mod client_address;
mod guild_routes;
mod message_routes;
pub(crate) mod rate_limit;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use mellow_protocol::{HealthResponse, HealthStatus};
use sqlx::SqlitePool;

use crate::config::ServerConfig;
use crate::domain::accounts::Accounts;
use crate::domain::guilds::Guilds;
use crate::domain::messages::Messages;
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::events::EventHub;
use crate::http::rate_limit::RateLimiter;

#[derive(Debug, Clone)]
pub(crate) struct AppState {
    pub(crate) pool: SqlitePool,
    pub(crate) accounts: Arc<Accounts>,
    pub(crate) guilds: Arc<Guilds>,
    pub(crate) messages: Arc<Messages>,
    pub(crate) events: Arc<EventHub>,
    pub(crate) gateway_sessions: Arc<crate::gateway::SessionRegistry>,
    pub(crate) rate_limiter: Arc<RateLimiter>,
    pub(crate) is_behind_tls_reverse_proxy: bool,
}

impl AppState {
    pub(crate) fn new(pool: SqlitePool, config: &ServerConfig) -> Self {
        let ids = Arc::new(SnowflakeGenerator::default());
        let events = Arc::new(EventHub::default());
        let guilds = Arc::new(Guilds::new(pool.clone(), ids.clone(), events.clone()));
        Self {
            accounts: Arc::new(Accounts::new(
                pool.clone(),
                config.registration_mode,
                ids.clone(),
                events.clone(),
            )),
            messages: Arc::new(Messages::new(
                pool.clone(),
                ids,
                guilds.clone(),
                events.clone(),
            )),
            events,
            gateway_sessions: Arc::default(),
            guilds,
            pool,
            rate_limiter: Arc::default(),
            is_behind_tls_reverse_proxy: config.behind_tls_reverse_proxy,
        }
    }
}

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/auth/register", post(account_routes::register))
        .route("/api/v1/auth/login", post(account_routes::login))
        .route("/api/v1/auth/logout", post(account_routes::logout))
        .route("/api/v1/users/@me", get(account_routes::current_user))
        .route(
            "/api/v1/users/@me/guilds",
            get(guild_routes::list_my_guilds),
        )
        .route("/api/v1/guilds", post(guild_routes::create_guild))
        .route("/api/v1/guilds/{guild_id}", get(guild_routes::get_guild))
        .route(
            "/api/v1/guilds/{guild_id}/channels",
            post(guild_routes::create_channel),
        )
        .route(
            "/api/v1/guilds/{guild_id}/invites",
            post(guild_routes::create_invite),
        )
        .route(
            "/api/v1/guilds/{guild_id}/members",
            get(guild_routes::list_members),
        )
        .route(
            "/api/v1/guilds/{guild_id}/members/@me",
            delete(guild_routes::leave_guild),
        )
        .route(
            "/api/v1/channels/{channel_id}",
            patch(guild_routes::update_channel).delete(guild_routes::delete_channel),
        )
        .route(
            "/api/v1/invites/{code}",
            post(guild_routes::join_with_invite),
        )
        .route(
            "/api/v1/channels/{channel_id}/messages",
            get(message_routes::list_messages).post(message_routes::send_message),
        )
        .route(
            "/api/v1/channels/{channel_id}/messages/{message_id}",
            patch(message_routes::edit_message).delete(message_routes::delete_message),
        )
        .route(
            "/api/v1/channels/{channel_id}/read-state",
            put(message_routes::acknowledge_read_state),
        )
        .route(
            "/api/v1/users/@me/read-states",
            get(message_routes::list_my_read_states),
        )
        .route(
            "/api/v1/channels/{channel_id}/typing",
            post(message_routes::start_typing),
        )
        .route("/gateway", get(crate::gateway::upgrade))
        .fallback(|| async { api_error::ApiError::not_found() })
        .method_not_allowed_fallback(|| async { api_error::ApiError::method_not_allowed() })
        .with_state(state)
}

/// What `axum::serve` needs: the router plus the peer address of each connection (for rate limits).
pub(crate) fn make_service(
    state: AppState,
) -> axum::extract::connect_info::IntoMakeServiceWithConnectInfo<Router, SocketAddr> {
    router(state).into_make_service_with_connect_info::<SocketAddr>()
}

/// Healthy only if the database answers, so a monitor sees a broken disk or key, not just a live process.
async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let database_answers = match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(%error, "health check: the database did not answer");
            false
        }
    };
    let (status_code, status) = if database_answers {
        (StatusCode::OK, HealthStatus::Ok)
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            HealthStatus::DatabaseUnavailable,
        )
    };
    (
        status_code,
        Json(HealthResponse {
            status,
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use axum::body::Body;
    use axum::extract::connect_info::MockConnectInfo;
    use axum::http::{HeaderMap, Request, header};
    use mellow_protocol::{ApiErrorBody, ApiErrorCode, CurrentUser, SessionCreated, Snowflake};
    use serde::de::DeserializeOwned;
    use tower::ServiceExt as _;

    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::tests::{PASSWORD, migrated_pool};

    const TEST_PEER: &str = "192.0.2.10:40000";

    pub(crate) fn test_state(pool: SqlitePool, registration_mode: RegistrationMode) -> AppState {
        let ids = Arc::new(SnowflakeGenerator::default());
        let events = Arc::new(EventHub::default());
        let guilds = Arc::new(Guilds::new(pool.clone(), ids.clone(), events.clone()));
        AppState {
            accounts: Arc::new(Accounts::new(
                pool.clone(),
                registration_mode,
                ids.clone(),
                events.clone(),
            )),
            messages: Arc::new(Messages::new(
                pool.clone(),
                ids,
                guilds.clone(),
                events.clone(),
            )),
            events,
            gateway_sessions: Arc::default(),
            guilds,
            pool,
            rate_limiter: Arc::default(),
            is_behind_tls_reverse_proxy: false,
        }
    }

    async fn send(state: &AppState, request: Request<Body>) -> (StatusCode, axum::body::Bytes) {
        send_from(state, TEST_PEER, request).await.0
    }

    async fn send_from(
        state: &AppState,
        peer: &str,
        request: Request<Body>,
    ) -> ((StatusCode, axum::body::Bytes), HeaderMap) {
        let response = router(state.clone())
            .layer(MockConnectInfo(peer.parse::<SocketAddr>().unwrap()))
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        ((status, body), headers)
    }

    fn json<T: DeserializeOwned>(body: &[u8]) -> T {
        serde_json::from_slice(body).unwrap()
    }

    fn post_json(path: &str, body: &serde_json::Value) -> Request<Body> {
        Request::post(path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn with_bearer(path: &str, method: &str, token: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    fn registration(username: &str) -> serde_json::Value {
        serde_json::json!({
            "username": username,
            "email": format!("{username}@example.org"),
            "password": PASSWORD,
            "device_name": "Test device",
        })
    }

    #[tokio::test]
    async fn health_reports_ok_while_the_database_answers_and_unavailable_once_it_is_closed() {
        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Closed);

        let (status, body) = send(
            &state,
            Request::get("/api/v1/health").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let health: HealthResponse = json(&body);
        assert_eq!(health.status, HealthStatus::Ok);
        assert_eq!(health.version, env!("CARGO_PKG_VERSION"));

        pool.close().await;
        let (status, body) = send(
            &state,
            Request::get("/api/v1/health").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            json::<HealthResponse>(&body).status,
            HealthStatus::DatabaseUnavailable
        );
    }

    #[tokio::test]
    async fn register_me_logout_round_trip() {
        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);

        let (status, body) = send(
            &state,
            post_json("/api/v1/auth/register", &registration("aurore")),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let session: SessionCreated = json(&body);

        let (status, body) = send(
            &state,
            with_bearer("/api/v1/users/@me", "GET", &session.token),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json::<CurrentUser>(&body), session.user);

        let (status, _) = send(
            &state,
            with_bearer("/api/v1/auth/logout", "POST", &session.token),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) = send(
            &state,
            with_bearer("/api/v1/users/@me", "GET", &session.token),
        )
        .await;
        pool.close().await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::Unauthorized);
    }

    #[tokio::test]
    async fn errors_use_stable_codes() {
        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        send(
            &state,
            post_json("/api/v1/auth/register", &registration("aurore")),
        )
        .await;

        let cases = [
            (
                post_json(
                    "/api/v1/auth/login",
                    &serde_json::json!({"login": "aurore", "password": "wrong password!", "device_name": "d"}),
                ),
                StatusCode::UNAUTHORIZED,
                ApiErrorCode::InvalidCredentials,
            ),
            (
                post_json(
                    "/api/v1/auth/register",
                    &serde_json::json!({
                        "username": "aurore",
                        "email": "another@example.org",
                        "password": PASSWORD,
                        "device_name": "Test device",
                    }),
                ),
                StatusCode::CONFLICT,
                ApiErrorCode::UsernameTaken,
            ),
            (
                post_json(
                    "/api/v1/auth/register",
                    &serde_json::json!({"username": "x"}),
                ),
                StatusCode::UNPROCESSABLE_ENTITY,
                ApiErrorCode::ValidationFailed,
            ),
            (
                Request::get("/api/v1/users/@me")
                    .body(Body::empty())
                    .unwrap(),
                StatusCode::UNAUTHORIZED,
                ApiErrorCode::Unauthorized,
            ),
        ];
        for (request, expected_status, expected_code) in cases {
            let path = request.uri().to_string();
            let (status, body) = send(&state, request).await;
            assert_eq!(status, expected_status, "{path}");
            assert_eq!(json::<ApiErrorBody>(&body).code, expected_code, "{path}");
        }
        pool.close().await;
    }

    #[tokio::test]
    async fn authentication_attempts_are_rate_limited_per_address() {
        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Closed);
        let attempt = || post_json("/api/v1/auth/register", &registration("aurore"));
        for _ in 0..rate_limit::AUTHENTICATION_ATTEMPTS_PER_ADDRESS.maximum_events {
            let (status, _) = send(&state, attempt()).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "registration is closed");
        }
        let (status, body) = send(&state, attempt()).await;
        pool.close().await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::RateLimited);
    }

    fn wrong_login(login: &str) -> Request<Body> {
        post_json(
            "/api/v1/auth/login",
            &serde_json::json!({"login": login, "password": "wrong password!", "device_name": "d"}),
        )
    }

    #[tokio::test]
    async fn login_attempts_are_limited_per_account_and_address_before_the_password_check() {
        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        send(
            &state,
            post_json("/api/v1/auth/register", &registration("aurore")),
        )
        .await;
        let attacker = "198.51.100.1:1000";
        for _ in 0..rate_limit::LOGIN_ATTEMPTS_PER_ACCOUNT_AND_ADDRESS.maximum_events {
            let ((status, _), _) = send_from(&state, attacker, wrong_login("aurore")).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        // Even the right password is refused from that address now: the attempt is counted first.
        let right_password = post_json(
            "/api/v1/auth/login",
            &serde_json::json!({"login": "Aurore", "password": PASSWORD, "device_name": "d"}),
        );
        let ((status, body), _) = send_from(&state, attacker, right_password).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::RateLimited);

        // The real user, from another address, is not locked out by the attacker.
        let from_home = post_json(
            "/api/v1/auth/login",
            &serde_json::json!({"login": "aurore", "password": PASSWORD, "device_name": "d"}),
        );
        let ((status, _), headers) = send_from(&state, "203.0.113.50:2000", from_home).await;
        pool.close().await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    }

    #[tokio::test]
    async fn forwarded_for_from_the_loopback_proxy_separates_clients() {
        let (_directory, pool) = migrated_pool().await;
        let mut state = test_state(pool.clone(), RegistrationMode::Closed);
        state.is_behind_tls_reverse_proxy = true;
        let via_proxy = |client: &str| {
            let mut request = wrong_login("nobody");
            request
                .headers_mut()
                .insert("x-forwarded-for", client.parse().unwrap());
            request
        };
        for _ in 0..rate_limit::AUTHENTICATION_ATTEMPTS_PER_ADDRESS.maximum_events {
            send_from(&state, "127.0.0.1:3000", via_proxy("203.0.113.1")).await;
        }
        let ((limited, _), _) = send_from(&state, "127.0.0.1:3000", via_proxy("203.0.113.1")).await;
        let ((other_client, _), _) =
            send_from(&state, "127.0.0.1:3000", via_proxy("203.0.113.2")).await;
        pool.close().await;
        assert_eq!(limited, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(other_client, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn malformed_bodies_and_unknown_routes_answer_with_json_errors() {
        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let mut unknown_field = registration("aurore");
        unknown_field["is_instance_administrator"] = serde_json::json!(true);
        let cases = [
            (
                Request::post("/api/v1/auth/register")
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from(registration("aurore").to_string()))
                    .unwrap(),
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                ApiErrorCode::ValidationFailed,
            ),
            (
                post_json("/api/v1/auth/register", &unknown_field),
                StatusCode::UNPROCESSABLE_ENTITY,
                ApiErrorCode::ValidationFailed,
            ),
            (
                wrong_login(&"a".repeat(1_000)),
                StatusCode::BAD_REQUEST,
                ApiErrorCode::ValidationFailed,
            ),
            (
                Request::get("/api/v1/nothing-here")
                    .body(Body::empty())
                    .unwrap(),
                StatusCode::NOT_FOUND,
                ApiErrorCode::NotFound,
            ),
            (
                Request::get("/api/v1/auth/login")
                    .body(Body::empty())
                    .unwrap(),
                StatusCode::METHOD_NOT_ALLOWED,
                ApiErrorCode::NotFound,
            ),
        ];
        for (request, expected_status, expected_code) in cases {
            let path = request.uri().to_string();
            let (status, body) = send(&state, request).await;
            assert_eq!(status, expected_status, "{path}");
            assert_eq!(json::<ApiErrorBody>(&body).code, expected_code, "{path}");
        }
        pool.close().await;
    }

    async fn registered_token(state: &AppState, username: &str) -> String {
        let (_, body) = send(
            state,
            post_json("/api/v1/auth/register", &registration(username)),
        )
        .await;
        json::<SessionCreated>(&body).token
    }

    fn authorized(
        method: &str,
        path: &str,
        token: &str,
        body: Option<serde_json::Value>,
    ) -> Request<Body> {
        let builder = Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"));
        match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        }
    }

    #[tokio::test]
    async fn guild_lifecycle_over_http() {
        use mellow_protocol::{Guild, GuildSummary, Invite, Member, Permissions};

        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let owner = registered_token(&state, "owner").await;
        let guest = registered_token(&state, "guest").await;

        let (status, body) = send(
            &state,
            authorized(
                "POST",
                "/api/v1/guilds",
                &owner,
                Some(serde_json::json!({"name": "Mellow Dev"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let guild: Guild = json(&body);
        let guild_path = format!("/api/v1/guilds/{}", guild.id);

        let (status, body) = send(&state, authorized("GET", &guild_path, &guest, None)).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "outsiders cannot see the guild"
        );
        assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::NotFound);

        let (status, body) = send(
            &state,
            authorized(
                "POST",
                &format!("{guild_path}/invites"),
                &owner,
                Some(serde_json::json!({})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let invite: Invite = json(&body);

        let (status, body) = send(
            &state,
            authorized(
                "POST",
                &format!("/api/v1/invites/{}", invite.code),
                &guest,
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json::<Guild>(&body).my_permissions,
            Permissions::DEFAULT_EVERYONE
        );

        let (status, body) = send(
            &state,
            authorized(
                "POST",
                &format!("{guild_path}/channels"),
                &guest,
                Some(serde_json::json!({"kind": "text", "name": "nope", "parent_id": null})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            json::<ApiErrorBody>(&body).code,
            ApiErrorCode::MissingPermissions
        );

        let (_, body) = send(
            &state,
            authorized("GET", &format!("{guild_path}/members"), &guest, None),
        )
        .await;
        assert_eq!(json::<Vec<Member>>(&body).len(), 2);
        let (_, body) = send(
            &state,
            authorized("GET", "/api/v1/users/@me/guilds", &guest, None),
        )
        .await;
        assert_eq!(json::<Vec<GuildSummary>>(&body)[0].name, "Mellow Dev");

        let (status, body) = send(
            &state,
            authorized("GET", "/api/v1/guilds/not-an-id", &owner, None),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json::<ApiErrorBody>(&body).code,
            ApiErrorCode::ValidationFailed
        );

        let (status, _) = send(
            &state,
            authorized("GET", &guild_path, "mlw_not-a-real-token", None),
        )
        .await;
        pool.close().await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn out_of_range_ids_are_not_found_and_invite_guessing_is_rate_limited() {
        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let owner = registered_token(&state, "owner").await;
        let guest = registered_token(&state, "guest").await;

        let (status, _) = send(
            &state,
            authorized("GET", "/api/v1/guilds/18446744073709551615", &owner, None),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "ids beyond SQLite's range are simply not found"
        );

        for _ in 0..rate_limit::INVITE_ATTEMPTS_PER_ADDRESS.maximum_events {
            send(
                &state,
                authorized("POST", "/api/v1/invites/guessing", &guest, None),
            )
            .await;
        }
        let (status, _) = send(
            &state,
            authorized("POST", "/api/v1/invites/guessing", &guest, None),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "invite codes cannot be brute forced"
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn messages_over_http() {
        use mellow_protocol::{Message, ReadState};

        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let messages_path = format!("/api/v1/channels/{general}/messages");

        let (status, body) = send(
            &state,
            authorized(
                "POST",
                &messages_path,
                &owner,
                Some(serde_json::json!({"content": "hello", "nonce": "n1"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let sent: Message = json(&body);
        assert_eq!(sent.nonce.as_deref(), Some("n1"));
        assert_eq!(sent.author.username, "owner");

        let (status, body) = send(
            &state,
            authorized("GET", &format!("{messages_path}?limit=10"), &owner, None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json::<Vec<Message>>(&body)[0].content, "hello");

        let (status, body) = send(
            &state,
            authorized("GET", &format!("{messages_path}?limit=nope"), &owner, None),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json::<ApiErrorBody>(&body).code,
            ApiErrorCode::ValidationFailed
        );

        let message_path = format!("{messages_path}/{}", sent.id);
        let (status, body) = send(
            &state,
            authorized(
                "PATCH",
                &message_path,
                &owner,
                Some(serde_json::json!({"content": "hello, edited"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(json::<Message>(&body).edited_at_unix_milliseconds.is_some());

        let (status, _) = send(
            &state,
            authorized(
                "PUT",
                &format!("/api/v1/channels/{general}/read-state"),
                &owner,
                Some(serde_json::json!({"last_read_message_id": sent.id.to_string()})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, body) = send(
            &state,
            authorized("GET", "/api/v1/users/@me/read-states", &owner, None),
        )
        .await;
        assert_eq!(
            json::<Vec<ReadState>>(&body)[0].last_read_message_id,
            sent.id
        );

        let (status, _) = send(&state, authorized("DELETE", &message_path, &owner, None)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) = send(&state, authorized("DELETE", &message_path, &owner, None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::NotFound);

        pool.close().await;
    }

    async fn owner_and_general_channel(state: &AppState) -> (String, Snowflake) {
        let owner = registered_token(state, "owner").await;
        let (_, body) = send(
            state,
            authorized(
                "POST",
                "/api/v1/guilds",
                &owner,
                Some(serde_json::json!({"name": "Club"})),
            ),
        )
        .await;
        let general = json::<mellow_protocol::Guild>(&body).channels[1].id;
        (owner, general)
    }

    #[tokio::test]
    async fn sending_messages_is_rate_limited_per_user() {
        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let messages_path = format!("/api/v1/channels/{general}/messages");
        for _ in 0..rate_limit::MESSAGE_WRITES_PER_USER.maximum_events {
            send(
                &state,
                authorized(
                    "POST",
                    &messages_path,
                    &owner,
                    Some(serde_json::json!({"content": "spam"})),
                ),
            )
            .await;
        }
        let (status, _) = send(
            &state,
            authorized(
                "POST",
                &messages_path,
                &owner,
                Some(serde_json::json!({"content": "one more"})),
            ),
        )
        .await;
        pool.close().await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn history_cursors_over_http() {
        use mellow_protocol::Message;

        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let messages_path = format!("/api/v1/channels/{general}/messages");
        let (_, body) = send(
            &state,
            authorized(
                "POST",
                &messages_path,
                &owner,
                Some(serde_json::json!({"content": "first"})),
            ),
        )
        .await;
        let sent: Message = json(&body);
        let (_, body) = send(
            &state,
            authorized(
                "POST",
                &messages_path,
                &owner,
                Some(serde_json::json!({"content": "second"})),
            ),
        )
        .await;
        let second: Message = json(&body);

        let (_, body) = send(
            &state,
            authorized(
                "GET",
                &format!("{messages_path}?before={}&limit=1", second.id),
                &owner,
                None,
            ),
        )
        .await;
        assert_eq!(
            json::<Vec<Message>>(&body)[0].id,
            sent.id,
            "query-string cursors parse"
        );
        let (_, body) = send(
            &state,
            authorized(
                "GET",
                &format!("{messages_path}?after=18446744073709551615"),
                &owner,
                None,
            ),
        )
        .await;
        assert_eq!(json::<Vec<Message>>(&body), Vec::new());
        pool.close().await;
    }

    #[tokio::test]
    async fn typing_needs_membership_and_is_rate_limited() {
        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let outsider = registered_token(&state, "outsider").await;
        let typing_path = format!("/api/v1/channels/{general}/typing");

        let (status, _) = send(&state, authorized("POST", &typing_path, &owner, None)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = send(&state, authorized("POST", &typing_path, &outsider, None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        for _ in 1..rate_limit::TYPING_INDICATORS_PER_USER.maximum_events {
            send(&state, authorized("POST", &typing_path, &owner, None)).await;
        }
        let (status, _) = send(&state, authorized("POST", &typing_path, &owner, None)).await;
        pool.close().await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }
}
