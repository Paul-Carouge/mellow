//! REST API (axum). Handlers validate input and map domain errors to HTTP; no SQL or business
//! rules live here (ARCHITECTURE.md §4.1).

mod account_routes;
mod api_error;
mod client_address;
mod rate_limit;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use mellow_protocol::{HealthResponse, HealthStatus};
use sqlx::SqlitePool;

use crate::config::ServerConfig;
use crate::domain::accounts::Accounts;
use crate::http::rate_limit::RateLimiter;

#[derive(Debug, Clone)]
pub(crate) struct AppState {
    pub(crate) pool: SqlitePool,
    pub(crate) accounts: Arc<Accounts>,
    pub(crate) rate_limiter: Arc<RateLimiter>,
    pub(crate) is_behind_tls_reverse_proxy: bool,
}

impl AppState {
    pub(crate) fn new(pool: SqlitePool, config: &ServerConfig) -> Self {
        Self {
            accounts: Arc::new(Accounts::new(pool.clone(), config.registration_mode)),
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
mod tests {
    use axum::body::Body;
    use axum::extract::connect_info::MockConnectInfo;
    use axum::http::{HeaderMap, Request, header};
    use mellow_protocol::{ApiErrorBody, ApiErrorCode, CurrentUser, SessionCreated};
    use serde::de::DeserializeOwned;
    use tower::ServiceExt as _;

    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::tests::{PASSWORD, migrated_pool};

    const TEST_PEER: &str = "192.0.2.10:40000";

    fn test_state(pool: SqlitePool, registration_mode: RegistrationMode) -> AppState {
        AppState {
            accounts: Arc::new(Accounts::new(pool.clone(), registration_mode)),
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
}
