//! REST API (axum). Handlers validate input and map domain errors to HTTP; no SQL or business
//! rules live here (ARCHITECTURE.md §4.1).

mod account_routes;
pub(crate) mod api_error;
mod attachment_routes;
pub(crate) mod authentication;
pub(crate) mod client_address;
mod guild_routes;
mod message_routes;
mod profile_image_routes;
pub(crate) mod rate_limit;
pub(crate) mod web_client;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use ipnet::IpNet;
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
    pub(crate) attachments: Arc<crate::domain::attachments::Attachments>,
    pub(crate) profile_images: Arc<crate::domain::profile_images::ProfileImages>,
    pub(crate) maximum_attachment_bytes: u64,
    pub(crate) events: Arc<EventHub>,
    pub(crate) gateway_sessions: Arc<crate::gateway::SessionRegistry>,
    pub(crate) rate_limiter: Arc<RateLimiter>,
    /// Peers whose `X-Forwarded-For` names the client; empty without a reverse proxy.
    pub(crate) trusted_proxy_networks: Arc<[IpNet]>,
}

impl AppState {
    pub(crate) fn new(
        pool: SqlitePool,
        config: &ServerConfig,
        attachment_files: crate::storage::attachment_files::AttachmentFiles,
    ) -> Self {
        let ids = Arc::new(SnowflakeGenerator::default());
        let events = Arc::new(EventHub::default());
        let guilds = Arc::new(Guilds::new(pool.clone(), ids.clone(), events.clone()));
        let messages = Arc::new(Messages::new(pool.clone(), ids.clone(), guilds.clone()));
        let attachments = Arc::new(crate::domain::attachments::Attachments::new(
            pool.clone(),
            ids.clone(),
            messages.clone(),
            attachment_files.clone(),
            config.maximum_attachment_storage_bytes_per_user,
        ));
        Self {
            profile_images: Arc::new(crate::domain::profile_images::ProfileImages::new(
                pool.clone(),
                ids.clone(),
                attachment_files,
                attachments.clone(),
                events.clone(),
            )),
            attachments,
            maximum_attachment_bytes: config.maximum_attachment_bytes,
            accounts: Arc::new(Accounts::new(
                pool.clone(),
                config.registration_mode,
                ids.clone(),
                events.clone(),
            )),
            messages,
            events,
            gateway_sessions: Arc::default(),
            guilds,
            pool,
            rate_limiter: Arc::default(),
            trusted_proxy_networks: client_address::build_trusted_proxy_networks(
                config.behind_tls_reverse_proxy,
                &config.trusted_proxy_networks,
            )
            .into(),
        }
    }
}

/// `/api/v1/auth/*` and the signed-in user's own account.
fn account_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/register", post(account_routes::register))
        .route("/api/v1/auth/login", post(account_routes::login))
        .route("/api/v1/auth/logout", post(account_routes::logout))
        .route(
            "/api/v1/users/@me",
            get(account_routes::current_user).patch(account_routes::update_current_user),
        )
        .route(
            "/api/v1/users/@me/password",
            post(account_routes::change_password),
        )
        .route(
            "/api/v1/users/@me/avatar",
            put(profile_image_routes::put_avatar)
                .delete(profile_image_routes::delete_avatar)
                .layer(DefaultBodyLimit::max(
                    profile_image_routes::MAXIMUM_PROFILE_IMAGE_BYTES,
                )),
        )
        .route(
            "/api/v1/users/@me/banner",
            put(profile_image_routes::put_banner)
                .delete(profile_image_routes::delete_banner)
                .layer(DefaultBodyLimit::max(
                    profile_image_routes::MAXIMUM_PROFILE_IMAGE_BYTES,
                )),
        )
        .route(
            "/api/v1/profile-images/{id}",
            get(profile_image_routes::get_profile_image),
        )
        .route(
            "/api/v1/profile-images/{id}/still",
            get(profile_image_routes::get_profile_image_still),
        )
}

/// Messages, read states, typing and attachments of a channel or conversation.
fn message_router(maximum_attachment_bytes: u64) -> Router<AppState> {
    Router::new()
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
        .route(
            "/api/v1/channels/{channel_id}/attachments",
            post(attachment_routes::upload).layer(DefaultBodyLimit::max(attachment_body_limit(
                maximum_attachment_bytes,
            ))),
        )
        .route(
            "/api/v1/attachments/{attachment_id}",
            get(attachment_routes::download),
        )
}

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .merge(account_router())
        .route(
            "/api/v1/users/@me/guilds",
            get(guild_routes::list_my_guilds),
        )
        .route(
            "/api/v1/users/@me/channels",
            post(message_routes::open_direct_message),
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
            "/api/v1/guilds/{guild_id}/members/{user_id}",
            delete(guild_routes::kick_member),
        )
        .route(
            "/api/v1/guilds/{guild_id}/audit-log",
            get(guild_routes::list_audit_log),
        )
        .route(
            "/api/v1/guilds/{guild_id}/bans",
            get(guild_routes::list_bans),
        )
        .route(
            "/api/v1/guilds/{guild_id}/bans/{user_id}",
            put(guild_routes::ban_member).delete(guild_routes::unban_member),
        )
        .route(
            "/api/v1/guilds/{guild_id}/roles",
            post(guild_routes::create_role),
        )
        .route(
            "/api/v1/guilds/{guild_id}/roles/order",
            put(guild_routes::reorder_roles),
        )
        .route(
            "/api/v1/guilds/{guild_id}/roles/{role_id}",
            patch(guild_routes::update_role).delete(guild_routes::delete_role),
        )
        .route(
            "/api/v1/guilds/{guild_id}/members/{user_id}/roles/{role_id}",
            put(guild_routes::add_member_role).delete(guild_routes::remove_member_role),
        )
        .route(
            "/api/v1/channels/{channel_id}",
            patch(guild_routes::update_channel).delete(guild_routes::delete_channel),
        )
        .route(
            "/api/v1/channels/{channel_id}/permissions/{target_id}",
            put(guild_routes::set_permission_overwrite)
                .delete(guild_routes::delete_permission_overwrite),
        )
        .route(
            "/api/v1/invites/{code}",
            post(guild_routes::join_with_invite),
        )
        .merge(message_router(state.maximum_attachment_bytes))
        .route("/gateway", get(crate::gateway::upgrade))
        .fallback(web_client::serve)
        .method_not_allowed_fallback(|| async { api_error::ApiError::method_not_allowed() })
        .layer(middleware::from_fn(mark_api_responses_private))
        .with_state(state)
}

fn attachment_body_limit(maximum_attachment_bytes: u64) -> usize {
    usize::try_from(maximum_attachment_bytes).unwrap_or(usize::MAX)
}

/// A shared cache must never keep a user's answer: unlike `Authorization`, a session cookie does
/// not stop caches from storing a response (RFC 9111 §3.5).
async fn mark_api_responses_private(request: Request, next: Next) -> Response {
    let is_api = request.uri().path().starts_with("/api/");
    let mut response = next.run(request).await;
    if is_api {
        let headers = response.headers_mut();
        headers
            .entry(header::CACHE_CONTROL)
            .or_insert(HeaderValue::from_static("no-store"));
        headers.append(header::VARY, HeaderValue::from_static("Cookie"));
    }
    response
}

/// With built-in TLS, browsers must never come back over plain HTTP: a network attacker would
/// answer there in the instance's name (ADR-0019). Behind a reverse proxy, the proxy sends it.
async fn require_https_from_now_on(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::STRICT_TRANSPORT_SECURITY,
        HeaderValue::from_static("max-age=31536000"),
    );
    response
}

/// What `axum::serve` needs: the router plus the peer address of each connection (for rate limits).
pub(crate) fn make_service(
    state: AppState,
) -> axum::extract::connect_info::IntoMakeServiceWithConnectInfo<Router, SocketAddr> {
    router(state).into_make_service_with_connect_info::<SocketAddr>()
}

/// `make_service` for the built-in TLS listener: every answer carries HSTS.
pub(crate) fn make_tls_service(
    state: AppState,
) -> axum::extract::connect_info::IntoMakeServiceWithConnectInfo<Router, SocketAddr> {
    router(state)
        .layer(middleware::from_fn(require_https_from_now_on))
        .into_make_service_with_connect_info::<SocketAddr>()
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
    use axum::http::HeaderMap;
    use mellow_protocol::{ApiErrorBody, ApiErrorCode, CurrentUser, SessionCreated, Snowflake};
    use serde::de::DeserializeOwned;
    use tower::ServiceExt as _;

    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::tests::{PASSWORD, migrated_pool};

    const TEST_PEER: &str = "192.0.2.10:40000";

    /// A fresh directory per state, kept for the test process's life.
    pub(crate) fn test_attachment_files() -> crate::storage::attachment_files::AttachmentFiles {
        let directory = tempfile::tempdir().unwrap().keep();
        crate::storage::attachment_files::AttachmentFiles::new(
            directory,
            &zeroize::Zeroizing::new([3u8; 32]),
        )
        .unwrap()
    }

    pub(crate) fn test_state(pool: SqlitePool, registration_mode: RegistrationMode) -> AppState {
        let ids = Arc::new(SnowflakeGenerator::default());
        let events = Arc::new(EventHub::default());
        let guilds = Arc::new(Guilds::new(pool.clone(), ids.clone(), events.clone()));
        let messages = Arc::new(Messages::new(pool.clone(), ids.clone(), guilds.clone()));
        let files = test_attachment_files();
        let attachments = Arc::new(crate::domain::attachments::Attachments::new(
            pool.clone(),
            ids.clone(),
            messages.clone(),
            files.clone(),
            crate::config::DEFAULT_MAXIMUM_ATTACHMENT_STORAGE_BYTES_PER_USER,
        ));
        AppState {
            accounts: Arc::new(Accounts::new(
                pool.clone(),
                registration_mode,
                ids.clone(),
                events.clone(),
            )),
            profile_images: Arc::new(crate::domain::profile_images::ProfileImages::new(
                pool.clone(),
                ids.clone(),
                files.clone(),
                attachments.clone(),
                events.clone(),
            )),
            attachments,
            maximum_attachment_bytes: crate::config::DEFAULT_MAXIMUM_ATTACHMENT_BYTES,
            messages,
            events,
            gateway_sessions: Arc::default(),
            guilds,
            pool,
            rate_limiter: Arc::default(),
            trusted_proxy_networks: Arc::default(),
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
    async fn the_web_client_signs_in_with_a_cookie_usable_only_from_this_origin() {
        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let mut register = post_json("/api/v1/auth/register", &registration("ada"));
        register.headers_mut().insert(
            "x-mellow-session-transport",
            HeaderValue::from_static("cookie"),
        );
        let ((status, body), headers) = send_from(&state, TEST_PEER, register).await;
        assert_eq!(status, StatusCode::CREATED);
        let session: SessionCreated = json(&body);
        assert_eq!(session.token, "", "the token stays out of page scripts");
        let set_cookie = headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
        assert!(set_cookie.contains("HttpOnly") && set_cookie.contains("SameSite=Strict"));
        let cookie = set_cookie.split(';').next().unwrap().to_owned();

        let request_from = |origin: &str| {
            Request::get("/api/v1/users/@me")
                .header(header::COOKIE, &cookie)
                .header(header::HOST, "chat.example.org")
                .header(header::ORIGIN, origin)
                .body(Body::empty())
                .unwrap()
        };
        let (same_origin, _) = send(&state, request_from("https://chat.example.org")).await;
        let (other_origin, _) = send(&state, request_from("https://evil.example")).await;
        let logout = Request::post("/api/v1/auth/logout")
            .header(header::COOKIE, &cookie)
            .header(header::HOST, "chat.example.org")
            .header(header::ORIGIN, "https://chat.example.org")
            .body(Body::empty())
            .unwrap();
        let ((logged_out, _), logout_headers) = send_from(&state, TEST_PEER, logout).await;
        let (after_logout, _) = send(&state, request_from("https://chat.example.org")).await;
        pool.close().await;

        assert_eq!(same_origin, StatusCode::OK);
        assert_eq!(other_origin, StatusCode::UNAUTHORIZED);
        assert_eq!(logged_out, StatusCode::NO_CONTENT);
        assert!(
            logout_headers
                .get(header::SET_COOKIE)
                .unwrap()
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );
        assert_eq!(after_logout, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn api_answers_are_private_and_the_tls_listener_adds_hsts() {
        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Closed);
        let ((_, _), plain) = send_from(
            &state,
            TEST_PEER,
            Request::get("/api/v1/health").body(Body::empty()).unwrap(),
        )
        .await;
        let tls_response = router(state.clone())
            .layer(middleware::from_fn(require_https_from_now_on))
            .layer(MockConnectInfo(TEST_PEER.parse::<SocketAddr>().unwrap()))
            .oneshot(Request::get("/api/v1/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        pool.close().await;

        assert_eq!(plain.get(header::CACHE_CONTROL).unwrap(), "no-store");
        assert_eq!(plain.get(header::VARY).unwrap(), "Cookie");
        assert!(plain.get(header::STRICT_TRANSPORT_SECURITY).is_none());
        assert_eq!(
            tls_response
                .headers()
                .get(header::STRICT_TRANSPORT_SECURITY)
                .unwrap(),
            "max-age=31536000"
        );
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
        state.trusted_proxy_networks =
            client_address::build_trusted_proxy_networks(true, &[]).into();
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

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot really an image";
    const HTML: &[u8] = b"<html><script>alert(1)</script></html>";

    /// `path` ends with `?file_name=<name>` for readability; the name goes into the header.
    fn upload_request(path: &str, token: &str, bytes: &'static [u8]) -> Request<Body> {
        let (path, file_name) = path.split_once("?file_name=").unwrap_or((path, "file"));
        Request::post(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header("x-mellow-file-name", file_name)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(bytes))
            .unwrap()
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // One scenario: upload, send, list, download, refusals.
    async fn attachments_upload_send_and_download_safely() {
        use mellow_protocol::{Attachment, Message};

        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let stranger = registered_token(&state, "stranger").await;
        let upload_path =
            |name: &str| format!("/api/v1/channels/{general}/attachments?file_name={name}");

        let (status, body) =
            send(&state, upload_request(&upload_path("cat.png"), &owner, PNG)).await;
        assert_eq!(status, StatusCode::CREATED);
        let image: Attachment = json(&body);
        assert_eq!(image.content_type, "image/png");
        let (_, body) = send(
            &state,
            upload_request(&upload_path("page.html"), &owner, HTML),
        )
        .await;
        let page: Attachment = json(&body);
        assert_eq!(page.content_type, "application/octet-stream");
        let (stranger_upload, _) = send(
            &state,
            upload_request(&upload_path("x.png"), &stranger, PNG),
        )
        .await;
        let (_, body) = send(
            &state,
            upload_request(&upload_path("theirs.png"), &stranger, PNG),
        )
        .await;
        let _ = body;

        // Not sent yet: nobody can download it.
        let (unsent_download, _) = send(
            &state,
            authorized(
                "GET",
                &format!("/api/v1/attachments/{}", image.id),
                &owner,
                None,
            ),
        )
        .await;

        let messages_path = format!("/api/v1/channels/{general}/messages");
        let (status, body) = send(
            &state,
            authorized(
                "POST",
                &messages_path,
                &owner,
                Some(serde_json::json!({
                    "content": "",
                    "attachment_ids": [image.id.to_string(), page.id.to_string()],
                })),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "{}",
            String::from_utf8_lossy(&body)
        );
        let sent: Message = json(&body);
        let (reused, _) = send(
            &state,
            authorized(
                "POST",
                &messages_path,
                &owner,
                Some(serde_json::json!({"content": "again", "attachment_ids": [image.id.to_string()]})),
            ),
        )
        .await;
        let (_, body) = send(&state, authorized("GET", &messages_path, &owner, None)).await;
        let history: Vec<Message> = json(&body);

        let ((image_status, image_bytes), image_headers) = send_from(
            &state,
            TEST_PEER,
            authorized(
                "GET",
                &format!("/api/v1/attachments/{}", image.id),
                &owner,
                None,
            ),
        )
        .await;
        let ((_, _), page_headers) = send_from(
            &state,
            TEST_PEER,
            authorized(
                "GET",
                &format!("/api/v1/attachments/{}", page.id),
                &owner,
                None,
            ),
        )
        .await;
        let (stranger_download, _) = send(
            &state,
            authorized(
                "GET",
                &format!("/api/v1/attachments/{}", image.id),
                &stranger,
                None,
            ),
        )
        .await;
        pool.close().await;

        assert_eq!(
            stranger_upload,
            StatusCode::NOT_FOUND,
            "not a member of the guild"
        );
        assert_eq!(unsent_download, StatusCode::NOT_FOUND);
        assert_eq!(sent.attachments.len(), 2);
        assert_eq!(sent.content, "");
        assert_eq!(
            reused,
            StatusCode::BAD_REQUEST,
            "an attachment goes with one message"
        );
        assert_eq!(history[0].attachments, sent.attachments);
        assert_eq!(image_status, StatusCode::OK);
        assert_eq!(&image_bytes[..], PNG);
        assert_eq!(
            image_headers.get(header::CONTENT_TYPE).unwrap(),
            "image/png"
        );
        assert!(
            image_headers
                .get(header::CONTENT_DISPOSITION)
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("inline")
        );
        assert_eq!(
            page_headers.get(header::CONTENT_TYPE).unwrap(),
            "application/octet-stream"
        );
        assert!(
            page_headers
                .get(header::CONTENT_DISPOSITION)
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("attachment")
        );
        assert_eq!(
            page_headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
            "nosniff"
        );
        assert!(
            page_headers
                .get(header::CONTENT_SECURITY_POLICY)
                .unwrap()
                .to_str()
                .unwrap()
                .contains("sandbox")
        );
        assert_eq!(stranger_download, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn uploads_stop_at_the_storage_quota_of_each_account() {
        let (_directory, pool) = migrated_pool().await;
        let mut state = test_state(pool.clone(), RegistrationMode::Open);
        state.attachments = Arc::new(crate::domain::attachments::Attachments::new(
            pool.clone(),
            Arc::new(SnowflakeGenerator::default()),
            state.messages.clone(),
            test_attachment_files(),
            PNG.len() as u64 + 1,
        ));
        let (owner, general) = owner_and_general_channel(&state).await;
        let path = format!("/api/v1/channels/{general}/attachments?file_name=a.png");
        let (first, _) = send(&state, upload_request(&path, &owner, PNG)).await;
        let (second, body) = send(&state, upload_request(&path, &owner, PNG)).await;
        pool.close().await;

        assert_eq!(first, StatusCode::CREATED);
        assert_eq!(second, StatusCode::CONFLICT);
        assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::LimitReached);
    }

    #[tokio::test]
    async fn attachments_over_the_limit_are_refused_before_being_read() {
        let (_directory, pool) = migrated_pool().await;
        let mut state = test_state(pool.clone(), RegistrationMode::Open);
        state.maximum_attachment_bytes = 8;
        let (owner, general) = owner_and_general_channel(&state).await;
        let (status, _) = send(
            &state,
            upload_request(
                &format!("/api/v1/channels/{general}/attachments?file_name=big.bin"),
                &owner,
                b"0123456789",
            ),
        )
        .await;
        pool.close().await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
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

    /// Owner token, guest token, the owner's guild with the guest as a member, and the guest's id.
    async fn guild_with_guest(state: &AppState) -> (String, String, Snowflake, Snowflake) {
        let (owner, _) = owner_and_general_channel(state).await;
        let guest = registered_token(state, "guest").await;
        let (_, body) = send(
            state,
            authorized("GET", "/api/v1/users/@me/guilds", &owner, None),
        )
        .await;
        let guild_id = json::<Vec<mellow_protocol::GuildSummary>>(&body)[0].id;
        let (_, body) = send(
            state,
            authorized(
                "POST",
                &format!("/api/v1/guilds/{guild_id}/invites"),
                &owner,
                Some(serde_json::json!({})),
            ),
        )
        .await;
        let code = json::<mellow_protocol::Invite>(&body).code;
        send(
            state,
            authorized("POST", &format!("/api/v1/invites/{code}"), &guest, None),
        )
        .await;
        let (_, body) = send(state, authorized("GET", "/api/v1/users/@me", &guest, None)).await;
        let guest_id = json::<CurrentUser>(&body).id;
        (owner, guest, guild_id, guest_id)
    }

    #[tokio::test]
    async fn roles_over_http() {
        use mellow_protocol::{Guild, Permissions, Role};

        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, guest_id) = guild_with_guest(&state).await;
        let guild_path = format!("/api/v1/guilds/{guild_id}");

        let kick = Permissions::KICK_MEMBERS.bits().to_string();
        let (status, body) = send(
            &state,
            authorized(
                "POST",
                &format!("{guild_path}/roles"),
                &owner,
                Some(serde_json::json!({"name": "Moderator", "color": 0x2f_a8_6b, "permissions": kick})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let role: Role = json(&body);
        assert_eq!((role.position, role.color), (1, 0x2f_a8_6b));
        let role_path = format!("{guild_path}/roles/{}", role.id);

        let (status, _) = send(
            &state,
            authorized(
                "PUT",
                &format!("{guild_path}/members/{guest_id}/roles/{}", role.id),
                &owner,
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, body) = send(&state, authorized("GET", &guild_path, &guest, None)).await;
        let seen_by_guest: Guild = json(&body);
        assert_eq!(seen_by_guest.my_role_ids, vec![role.id]);
        assert!(
            seen_by_guest
                .my_permissions
                .contains(Permissions::KICK_MEMBERS)
        );
        assert_eq!(seen_by_guest.roles.len(), 2);

        let (status, body) = send(
            &state,
            authorized(
                "PATCH",
                &role_path,
                &owner,
                Some(serde_json::json!({"name": "Mods"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json::<Role>(&body).name, "Mods");

        let unknown_bit = (1_u64 << 63).to_string();
        let roles_path = format!("{guild_path}/roles");
        let everyone_path = format!("{guild_path}/roles/{guild_id}");
        for (method, path, body, expected_status) in [
            (
                "POST",
                &roles_path,
                Some(serde_json::json!({"name": "x", "permissions": unknown_bit})),
                StatusCode::BAD_REQUEST,
            ),
            (
                "POST",
                &roles_path,
                Some(serde_json::json!({"name": "x", "hoist": true})),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            ("DELETE", &everyone_path, None, StatusCode::BAD_REQUEST),
        ] {
            let (status, response) = send(&state, authorized(method, path, &owner, body)).await;
            assert_eq!(status, expected_status, "{method} {path}");
            assert_eq!(
                json::<ApiErrorBody>(&response).code,
                ApiErrorCode::ValidationFailed
            );
        }
        let (status, body) = send(&state, authorized("DELETE", &role_path, &guest, None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            json::<ApiErrorBody>(&body).code,
            ApiErrorCode::MissingPermissions
        );

        let (status, _) = send(
            &state,
            authorized(
                "DELETE",
                &format!("{guild_path}/members/{guest_id}/roles/{}", role.id),
                &owner,
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = send(&state, authorized("DELETE", &role_path, &owner, None)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = send(&state, authorized("DELETE", &role_path, &owner, None)).await;
        pool.close().await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn roles_are_reordered_over_http() {
        use mellow_protocol::Role;

        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
        let roles_path = format!("/api/v1/guilds/{guild_id}/roles");
        let mut created = Vec::new();
        for name in ["first", "second"] {
            let (_, body) = send(
                &state,
                authorized(
                    "POST",
                    &roles_path,
                    &owner,
                    Some(serde_json::json!({"name": name})),
                ),
            )
            .await;
            created.push(json::<Role>(&body).id);
        }
        let order_path = format!("{roles_path}/order");
        // Created last, "second" is lowest: put "first" back below it.
        let order = serde_json::json!({"role_ids": [created[0], created[1]]});
        let (status, body) = send(
            &state,
            authorized("PUT", &order_path, &owner, Some(order.clone())),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let names: Vec<String> = json::<Vec<Role>>(&body)
            .into_iter()
            .map(|role| role.name)
            .collect();
        assert_eq!(names, ["@everyone", "first", "second"]);

        let (status, body) =
            send(&state, authorized("PUT", &order_path, &guest, Some(order))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            json::<ApiErrorBody>(&body).code,
            ApiErrorCode::MissingPermissions
        );
        let (status, body) = send(
            &state,
            authorized(
                "PUT",
                &order_path,
                &owner,
                Some(serde_json::json!({"role_ids": [created[0]]})),
            ),
        )
        .await;
        pool.close().await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json::<ApiErrorBody>(&body).code,
            ApiErrorCode::ValidationFailed
        );
    }

    #[tokio::test]
    async fn overwrites_hide_a_channel_over_http() {
        use mellow_protocol::{Channel, Guild, Message, ReadState};

        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
        let (_, body) = send(
            &state,
            authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &owner, None),
        )
        .await;
        let general = json::<Guild>(&body).channels[1].id;
        let messages_path = format!("/api/v1/channels/{general}/messages");
        let (_, body) = send(
            &state,
            authorized(
                "POST",
                &messages_path,
                &owner,
                Some(serde_json::json!({"content": "hello"})),
            ),
        )
        .await;
        let message_id = json::<Message>(&body).id;
        send(
            &state,
            authorized(
                "PUT",
                &format!("/api/v1/channels/{general}/read-state"),
                &guest,
                Some(serde_json::json!({"last_read_message_id": message_id})),
            ),
        )
        .await;

        let (_, body) = send(
            &state,
            authorized("GET", "/api/v1/users/@me/read-states", &guest, None),
        )
        .await;
        assert_eq!(json::<Vec<ReadState>>(&body).len(), 1, "read before hiding");

        let overwrite_path = format!("/api/v1/channels/{general}/permissions/{guild_id}");
        let view = mellow_protocol::Permissions::VIEW_CHANNEL
            .bits()
            .to_string();
        let (status, body) = send(
            &state,
            authorized(
                "PUT",
                &overwrite_path,
                &owner,
                Some(serde_json::json!({"kind": "role", "deny": view})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json::<Channel>(&body).permission_overwrites.len(), 1);

        let (status, _) = send(&state, authorized("GET", &messages_path, &guest, None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (_, body) = send(
            &state,
            authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &guest, None),
        )
        .await;
        assert!(
            json::<Guild>(&body)
                .channels
                .iter()
                .all(|channel| channel.id != general)
        );
        let (_, body) = send(
            &state,
            authorized("GET", "/api/v1/users/@me/read-states", &guest, None),
        )
        .await;
        assert_eq!(json::<Vec<ReadState>>(&body), Vec::new());

        let (status, body) = send(
            &state,
            authorized(
                "PUT",
                &overwrite_path,
                &owner,
                Some(serde_json::json!({"kind": "role", "allow": view, "deny": view})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json::<ApiErrorBody>(&body).code,
            ApiErrorCode::ValidationFailed
        );

        let (status, _) = send(&state, authorized("DELETE", &overwrite_path, &owner, None)).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send(&state, authorized("GET", &messages_path, &guest, None)).await;
        pool.close().await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn kicks_and_bans_over_http() {
        use mellow_protocol::{Ban, Invite};

        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, guest_id) = guild_with_guest(&state).await;
        let guild_path = format!("/api/v1/guilds/{guild_id}");
        let (status, _) = send(
            &state,
            authorized(
                "DELETE",
                &format!("{guild_path}/members/{guest_id}"),
                &owner,
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let ban_path = format!("{guild_path}/bans/{guest_id}");
        let (status, _) = send(
            &state,
            authorized(
                "PUT",
                &ban_path,
                &owner,
                Some(serde_json::json!({"reason": "spam"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, body) = send(
            &state,
            authorized("GET", &format!("{guild_path}/bans"), &owner, None),
        )
        .await;
        assert_eq!(json::<Vec<Ban>>(&body)[0].reason, "spam");

        let (_, body) = send(
            &state,
            authorized(
                "POST",
                &format!("{guild_path}/invites"),
                &owner,
                Some(serde_json::json!({})),
            ),
        )
        .await;
        let invite_path = format!("/api/v1/invites/{}", json::<Invite>(&body).code);
        let (status, body) = send(&state, authorized("POST", &invite_path, &guest, None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::Banned);

        let (status, _) = send(&state, authorized("DELETE", &ban_path, &owner, None)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = send(&state, authorized("POST", &invite_path, &guest, None)).await;
        pool.close().await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn the_audit_log_over_http() {
        use mellow_protocol::{AuditAction, AuditLogEntry};

        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, guest_id) = guild_with_guest(&state).await;
        let guild_path = format!("/api/v1/guilds/{guild_id}");
        send(
            &state,
            authorized(
                "DELETE",
                &format!("{guild_path}/members/{guest_id}"),
                &owner,
                None,
            ),
        )
        .await;
        let (status, body) = send(
            &state,
            authorized(
                "GET",
                &format!("{guild_path}/audit-log?limit=10"),
                &owner,
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let entries: Vec<AuditLogEntry> = json(&body);
        assert_eq!(entries[0].action, AuditAction::MemberKick);
        assert_eq!(entries[0].target_id, Some(guest_id));
        let (status, _) = send(
            &state,
            authorized("GET", &format!("{guild_path}/audit-log"), &guest, None),
        )
        .await;
        let (bad_status, _) = send(
            &state,
            authorized(
                "GET",
                &format!("{guild_path}/audit-log?limit=101"),
                &owner,
                None,
            ),
        )
        .await;
        pool.close().await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "the kicked guest is no longer a member"
        );
        assert_eq!(bad_status, StatusCode::BAD_REQUEST);
    }

    fn picture_request(method: &str, path: &str, token: &str, bytes: Vec<u8>) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(bytes))
            .unwrap()
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // One scenario: upload, serve, show everywhere, replace, remove.
    async fn animated_avatars_and_banners_are_stored_served_and_shown_with_their_owner() {
        use crate::domain::image_processing::tests::{animated_gif, png};
        use mellow_protocol::{Member, Message};

        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let stranger = registered_token(&state, "stranger").await;

        let (status, body) = send(
            &state,
            picture_request(
                "PUT",
                "/api/v1/users/@me/avatar",
                &owner,
                animated_gif(64, 64, 3),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let avatar = json::<CurrentUser>(&body).avatar.unwrap();
        let (_, body) = send(
            &state,
            picture_request("PUT", "/api/v1/users/@me/banner", &owner, png(300, 100)),
        )
        .await;
        let banner = json::<CurrentUser>(&body).banner.unwrap();
        let (refused, _) = send(
            &state,
            picture_request(
                "PUT",
                "/api/v1/users/@me/avatar",
                &owner,
                b"<svg/>".to_vec(),
            ),
        )
        .await;
        let (_, body) = send(&state, authorized("GET", "/api/v1/users/@me", &owner, None)).await;
        let me: CurrentUser = json(&body);

        let image_path = format!("/api/v1/profile-images/{}", avatar.id);
        let ((animated_status, animated), animated_headers) = send_from(
            &state,
            TEST_PEER,
            authorized("GET", &image_path, &stranger, None),
        )
        .await;
        let (_, still) = send(
            &state,
            authorized("GET", &format!("{image_path}/still"), &stranger, None),
        )
        .await;

        let (_, body) = send(
            &state,
            authorized(
                "POST",
                &format!("/api/v1/channels/{general}/messages"),
                &owner,
                Some(serde_json::json!({"content": "new look"})),
            ),
        )
        .await;
        let sent: Message = json(&body);
        let (_, body) = send(
            &state,
            authorized(
                "GET",
                &format!("/api/v1/channels/{general}/messages"),
                &owner,
                None,
            ),
        )
        .await;
        let history: Vec<Message> = json(&body);
        let guild_id = me_guild_id(&state, &owner).await;
        let (_, body) = send(
            &state,
            authorized(
                "GET",
                &format!("/api/v1/guilds/{guild_id}/members"),
                &owner,
                None,
            ),
        )
        .await;
        let members: Vec<Member> = json(&body);

        let (_, body) = send(
            &state,
            authorized("DELETE", "/api/v1/users/@me/avatar", &owner, None),
        )
        .await;
        let without_avatar: CurrentUser = json(&body);
        let (removed, _) = send(&state, authorized("GET", &image_path, &owner, None)).await;
        pool.close().await;

        assert!(avatar.is_animated);
        assert!(!banner.is_animated);
        assert_eq!(refused, StatusCode::BAD_REQUEST, "not a picture");
        assert_eq!((me.avatar, me.banner), (Some(avatar), Some(banner)));
        assert_eq!(animated_status, StatusCode::OK);
        assert_eq!(&animated[..4], b"RIFF");
        assert_eq!(&animated[8..12], b"WEBP");
        assert_ne!(animated, still, "the still is its first frame only");
        assert_eq!(
            animated_headers.get(header::CONTENT_TYPE).unwrap(),
            "image/webp"
        );
        assert!(
            animated_headers
                .get(header::CACHE_CONTROL)
                .unwrap()
                .to_str()
                .unwrap()
                .contains("immutable")
        );
        assert_eq!(sent.author.avatar, Some(avatar));
        assert_eq!(history[0].author.banner, Some(banner));
        assert!(
            members
                .iter()
                .any(|member| member.user.avatar == Some(avatar))
        );
        assert_eq!(without_avatar.avatar, None);
        assert_eq!(without_avatar.banner, Some(banner));
        assert_eq!(removed, StatusCode::NOT_FOUND, "a replaced version is gone");
    }

    #[tokio::test]
    async fn an_animated_gif_attachment_becomes_a_smaller_webp_in_the_background() {
        use crate::domain::image_processing::tests::animated_gif;
        use mellow_protocol::Attachment;

        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let optimizer = tokio::spawn(
            crate::domain::attachments::optimize_attachments_in_background(
                state.attachments.clone(),
            ),
        );
        let (owner, general) = owner_and_general_channel(&state).await;
        let gif = animated_gif(120, 80, 4);
        let mut upload = picture_request(
            "POST",
            &format!("/api/v1/channels/{general}/attachments"),
            &owner,
            gif.clone(),
        );
        upload
            .headers_mut()
            .insert("x-mellow-file-name", HeaderValue::from_static("party.gif"));
        let (_, body) = send(&state, upload).await;
        let uploaded: Attachment = json(&body);
        send(
            &state,
            authorized(
                "POST",
                &format!("/api/v1/channels/{general}/messages"),
                &owner,
                Some(
                    serde_json::json!({"content": "", "attachment_ids": [uploaded.id.to_string()]}),
                ),
            ),
        )
        .await;

        let download_path = format!("/api/v1/attachments/{}", uploaded.id);
        let mut downloaded = (Vec::new(), String::new());
        for _ in 0..200 {
            let ((_, bytes), headers) = send_from(
                &state,
                TEST_PEER,
                authorized("GET", &download_path, &owner, None),
            )
            .await;
            let content_type = headers
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            downloaded = (bytes.to_vec(), content_type);
            if downloaded.1 == "image/webp" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        optimizer.abort();
        let _cancelled = optimizer.await;
        pool.close().await;

        assert_eq!(uploaded.content_type, "image/gif", "answered as received");
        assert_eq!(downloaded.1, "image/webp");
        assert!(downloaded.0.len() < gif.len());
    }

    async fn me_guild_id(state: &AppState, token: &str) -> Snowflake {
        let (_, body) = send(
            state,
            authorized("GET", "/api/v1/users/@me/guilds", token, None),
        )
        .await;
        json::<Vec<mellow_protocol::GuildSummary>>(&body)[0].id
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

    async fn logged_in_token(state: &AppState, username: &str, device_name: &str) -> String {
        let (_, body) = send(
            state,
            post_json(
                "/api/v1/auth/login",
                &serde_json::json!({"login": username, "password": PASSWORD, "device_name": device_name}),
            ),
        )
        .await;
        json::<SessionCreated>(&body).token
    }

    /// A stolen session that uses up its password-check budget must not keep the owner's other
    /// devices from changing the password (and so revoking it).
    #[tokio::test]
    async fn password_checks_and_profile_writes_are_rate_limited_per_session() {
        let (_directory, pool) = migrated_pool().await;
        let state = test_state(pool.clone(), RegistrationMode::Open);
        let stolen = registered_token(&state, "aurore").await;
        let own = logged_in_token(&state, "aurore", "own").await;
        let more_sessions = [
            logged_in_token(&state, "aurore", "third").await,
            logged_in_token(&state, "aurore", "fourth").await,
        ];
        let change = |token: &str, current: &str| {
            authorized(
                "POST",
                "/api/v1/users/@me/password",
                token,
                Some(serde_json::json!({
                    "current_password": current,
                    "new_password": "a brand new password",
                })),
            )
        };
        let rename = |token: &str, display_name: String| {
            authorized(
                "PATCH",
                "/api/v1/users/@me",
                token,
                Some(serde_json::json!({ "display_name": display_name })),
            )
        };

        // More than the user's budget too: refused checks must not count against it.
        let mut guesses = Vec::new();
        for _ in 0..=rate_limit::PASSWORD_CHECKS_PER_USER.maximum_events {
            guesses.push(send(&state, change(&stolen, "a wrong guess")).await.0);
        }
        let mut stolen_renames = Vec::new();
        for attempt in 0..=rate_limit::PROFILE_WRITES_PER_SESSION.maximum_events {
            stolen_renames.push(
                send(&state, rename(&stolen, format!("Thief {attempt}")))
                    .await
                    .0,
            );
        }
        let (stolen_with_right_password, _) = send(&state, change(&stolen, PASSWORD)).await;
        let mut renames = Vec::new();
        for attempt in 0..=rate_limit::PROFILE_WRITES_PER_SESSION.maximum_events {
            renames.push(
                send(&state, rename(&own, format!("Aurore {attempt}")))
                    .await
                    .0,
            );
        }
        // Sessions together hit the user's ceiling: 10 + 10 allowed above, 10 more here.
        let mut third_renames = Vec::new();
        for attempt in 0..rate_limit::PROFILE_WRITES_PER_SESSION.maximum_events {
            third_renames.push(
                send(
                    &state,
                    rename(&more_sessions[0], format!("Third {attempt}")),
                )
                .await
                .0,
            );
        }
        let (fourth_rename, _) = send(&state, rename(&more_sessions[1], "Fourth".into())).await;
        let (own_change, _) = send(&state, change(&own, PASSWORD)).await;
        let (stolen_after_change, _) =
            send(&state, with_bearer("/api/v1/users/@me", "GET", &stolen)).await;
        pool.close().await;

        let session_budget = rate_limit::PASSWORD_CHECKS_PER_SESSION.maximum_events as usize;
        let (checked, refused) = guesses.split_at(session_budget);
        assert!(
            checked
                .iter()
                .all(|status| *status == StatusCode::UNAUTHORIZED)
        );
        assert!(
            refused
                .iter()
                .all(|status| *status == StatusCode::TOO_MANY_REQUESTS)
        );
        assert_eq!(stolen_renames.last(), Some(&StatusCode::TOO_MANY_REQUESTS));
        assert_eq!(stolen_with_right_password, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(own_change, StatusCode::NO_CONTENT);
        assert_eq!(stolen_after_change, StatusCode::UNAUTHORIZED);
        assert!(third_renames.iter().all(|status| *status == StatusCode::OK));
        assert_eq!(fourth_rename, StatusCode::TOO_MANY_REQUESTS);
        let (last, allowed) = renames.split_last().unwrap();
        assert!(allowed.iter().all(|status| *status == StatusCode::OK));
        assert_eq!(*last, StatusCode::TOO_MANY_REQUESTS);
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
