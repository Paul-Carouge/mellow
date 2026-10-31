//! REST API (axum). Handlers validate input and map domain errors to HTTP; no SQL or business
//! rules live here (ARCHITECTURE.md §4.1).

mod account_routes;
pub(crate) mod api_error;
mod attachment_routes;
pub(crate) mod authentication;
pub(crate) mod client_address;
mod emoji_routes;
mod guild_routes;
mod message_routes;
mod profile_image_routes;
pub(crate) mod rate_limit;
mod relationship_routes;
pub(crate) mod web_client;

use std::net::SocketAddr;
use std::sync::Arc;

use crate::storage::Database;
use axum::extract::DefaultBodyLimit;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use ipnet::IpNet;
use mellow_protocol::{HealthResponse, HealthStatus};

use crate::config::ServerConfig;
use crate::domain::accounts::Accounts;
use crate::domain::guild_rules::MAXIMUM_EMOJI_UPLOAD_BYTES;
use crate::domain::guilds::Guilds;
use crate::domain::messages::Messages;
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::events::EventHub;
use crate::http::rate_limit::RateLimiter;

#[derive(Debug, Clone)]
pub(crate) struct AppState {
    pub(crate) database: Database,
    pub(crate) accounts: Arc<Accounts>,
    pub(crate) guilds: Arc<Guilds>,
    pub(crate) messages: Arc<Messages>,
    pub(crate) relationships: Arc<crate::domain::relationships::Relationships>,
    pub(crate) attachments: Arc<crate::domain::attachments::Attachments>,
    pub(crate) profile_images: Arc<crate::domain::profile_images::ProfileImages>,
    pub(crate) maximum_attachment_bytes: u64,
    pub(crate) events: Arc<EventHub>,
    pub(crate) gateway_sessions: Arc<crate::gateway::SessionRegistry>,
    /// Limits of signed-in requests: keyed by the user, one of their channels, or the address of
    /// a request that carries a valid session.
    pub(crate) rate_limiter: Arc<RateLimiter>,
    /// Limits keyed by address or login name, before any session: kept apart so a flood of
    /// sign-in attempts cannot fill the table that signed-in users' limits need.
    pub(crate) unauthenticated_rate_limiter: Arc<RateLimiter>,
    /// Gateway connections per address, apart again: their keys last seconds, so a sign-in flood
    /// that fills the table above cannot stop signed-in users from reconnecting.
    pub(crate) gateway_rate_limiter: Arc<RateLimiter>,
    /// Peers whose `X-Forwarded-For` names the client; empty without a reverse proxy.
    pub(crate) trusted_proxy_networks: Arc<[IpNet]>,
    /// Voice channels (ADR-0030), signalled over the gateway.
    pub(crate) voice: crate::voice::VoiceService,
}

impl AppState {
    pub(crate) fn new(
        database: Database,
        config: &ServerConfig,
        attachment_files: crate::storage::attachment_files::AttachmentFiles,
    ) -> Self {
        let ids = Arc::new(SnowflakeGenerator::default());
        let events = Arc::new(EventHub::default());
        let guilds = Arc::new(Guilds::new(
            database.clone(),
            ids.clone(),
            events.clone(),
            config.guild_creation,
        ));
        let messages = Arc::new(Messages::new(database.clone(), ids.clone(), guilds.clone()));
        let voice = crate::voice::VoiceService::start(
            &config.voice,
            config.is_development_listener(),
            guilds.clone(),
            events.clone(),
        );
        let attachments = Arc::new(crate::domain::attachments::Attachments::new(
            database.clone(),
            ids.clone(),
            messages.clone(),
            attachment_files.clone(),
            config.maximum_attachment_storage_bytes_per_user,
        ));
        Self {
            profile_images: Arc::new(crate::domain::profile_images::ProfileImages::new(
                database.clone(),
                ids.clone(),
                attachment_files,
                attachments.clone(),
                events.clone(),
            )),
            attachments,
            maximum_attachment_bytes: config.maximum_attachment_bytes,
            accounts: Arc::new(Accounts::new(
                database.clone(),
                config.registration_mode,
                ids.clone(),
                events.clone(),
            )),
            messages,
            relationships: Arc::new(crate::domain::relationships::Relationships::new(
                database.clone(),
                events.clone(),
            )),
            events,
            gateway_sessions: Arc::default(),
            guilds,
            database,
            rate_limiter: Arc::default(),
            unauthenticated_rate_limiter: Arc::default(),
            gateway_rate_limiter: Arc::default(),
            trusted_proxy_networks: client_address::build_trusted_proxy_networks(
                config.behind_tls_reverse_proxy,
                &config.trusted_proxy_networks,
            )
            .into(),
            voice,
        }
    }
}

/// Group conversations (ADR-0026); their messages use the channel routes.
fn group_conversation_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/group-conversations",
            post(message_routes::create_group_conversation),
        )
        .route(
            "/api/v1/group-conversations/{channel_id}",
            patch(message_routes::rename_group_conversation),
        )
        .route(
            "/api/v1/group-conversations/{channel_id}/recipients/{user_id}",
            put(message_routes::add_group_participant)
                .delete(message_routes::remove_group_participant),
        )
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
            "/api/v1/users/@me/presence",
            put(account_routes::update_presence),
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
        .route(
            "/api/v1/guilds/{guild_id}/icon",
            put(profile_image_routes::put_guild_icon)
                .delete(profile_image_routes::delete_guild_icon)
                .layer(DefaultBodyLimit::max(
                    profile_image_routes::MAXIMUM_PROFILE_IMAGE_BYTES,
                )),
        )
        .route(
            "/api/v1/channels/{channel_id}/icon",
            put(profile_image_routes::put_channel_icon)
                .delete(profile_image_routes::delete_channel_icon)
                .layer(DefaultBodyLimit::max(
                    profile_image_routes::MAXIMUM_PROFILE_IMAGE_BYTES,
                )),
        )
        .route(
            "/api/v1/guilds/{guild_id}/emojis",
            get(emoji_routes::list_emojis)
                .post(emoji_routes::create_emoji)
                .layer(DefaultBodyLimit::max(MAXIMUM_EMOJI_UPLOAD_BYTES)),
        )
        .route(
            "/api/v1/guilds/{guild_id}/emojis/{emoji_id}",
            patch(emoji_routes::rename_emoji).delete(emoji_routes::delete_emoji),
        )
        .route("/api/v1/icons/{id}", get(profile_image_routes::get_icon))
        .route(
            "/api/v1/icons/{id}/still",
            get(profile_image_routes::get_icon_still),
        )
}

/// Messages, reactions, pins, read states, typing and attachments of a channel or conversation.
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
            "/api/v1/channels/{channel_id}/messages/{message_id}/reactions",
            delete(message_routes::remove_all_reactions),
        )
        .route(
            "/api/v1/channels/{channel_id}/messages/{message_id}/reactions/{emoji}",
            get(message_routes::list_reacting_users).delete(message_routes::remove_emoji_reactions),
        )
        .route(
            "/api/v1/channels/{channel_id}/messages/{message_id}/reactions/{emoji}/@me",
            put(message_routes::add_reaction).delete(message_routes::remove_own_reaction),
        )
        .route(
            "/api/v1/channels/{channel_id}/messages/{message_id}/reactions/{emoji}/{user_id}",
            delete(message_routes::remove_user_reaction),
        )
        .route(
            "/api/v1/channels/{channel_id}/pins",
            get(message_routes::list_pins),
        )
        .route(
            "/api/v1/channels/{channel_id}/pins/{message_id}",
            put(message_routes::pin_message).delete(message_routes::unpin_message),
        )
        .route(
            "/api/v1/channels/{channel_id}/read-state",
            put(message_routes::acknowledge_read_state),
        )
        .route(
            "/api/v1/guilds/{guild_id}/messages/search",
            post(message_routes::search_guild_messages),
        )
        .route(
            "/api/v1/users/@me/messages/search",
            post(message_routes::search_my_conversation_messages),
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
            "/api/v1/users/@me/relationships",
            get(relationship_routes::list_relationships)
                .post(relationship_routes::send_friend_request),
        )
        .route(
            "/api/v1/users/@me/relationships/{user_id}",
            put(relationship_routes::set_relationship)
                .delete(relationship_routes::remove_relationship),
        )
        .merge(group_conversation_router())
        .route(
            "/api/v1/users/@me/channels",
            post(message_routes::open_direct_message),
        )
        .route("/api/v1/guilds", post(guild_routes::create_guild))
        .route(
            "/api/v1/guilds/{guild_id}",
            get(guild_routes::get_guild)
                .patch(guild_routes::update_guild)
                .delete(guild_routes::delete_guild),
        )
        .route(
            "/api/v1/guilds/{guild_id}/channels",
            post(guild_routes::create_channel),
        )
        .route(
            "/api/v1/guilds/{guild_id}/channels/order",
            put(guild_routes::reorder_channels),
        )
        .route(
            "/api/v1/guilds/{guild_id}/invites",
            get(guild_routes::list_invites).post(guild_routes::create_invite),
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
            post(guild_routes::join_with_invite).delete(guild_routes::delete_invite),
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
    let database_answers = match state.database.check_that_readers_answer().await {
        Ok(()) => true,
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
    use crate::domain::accounts::tests::{PASSWORD, migrated_database};

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

    pub(crate) fn test_state(database: Database, registration_mode: RegistrationMode) -> AppState {
        let ids = Arc::new(SnowflakeGenerator::default());
        let events = Arc::new(EventHub::default());
        let guilds = Arc::new(Guilds::new(
            database.clone(),
            ids.clone(),
            events.clone(),
            crate::domain::account_rules::GuildCreationPolicy::Everyone,
        ));
        let messages = Arc::new(Messages::new(database.clone(), ids.clone(), guilds.clone()));
        let voice = crate::voice::VoiceService::disabled(guilds.clone(), events.clone());
        let files = test_attachment_files();
        let attachments = Arc::new(crate::domain::attachments::Attachments::new(
            database.clone(),
            ids.clone(),
            messages.clone(),
            files.clone(),
            crate::config::DEFAULT_MAXIMUM_ATTACHMENT_STORAGE_BYTES_PER_USER,
        ));
        AppState {
            accounts: Arc::new(Accounts::new(
                database.clone(),
                registration_mode,
                ids.clone(),
                events.clone(),
            )),
            profile_images: Arc::new(crate::domain::profile_images::ProfileImages::new(
                database.clone(),
                ids.clone(),
                files.clone(),
                attachments.clone(),
                events.clone(),
            )),
            attachments,
            maximum_attachment_bytes: crate::config::DEFAULT_MAXIMUM_ATTACHMENT_BYTES,
            messages,
            relationships: Arc::new(crate::domain::relationships::Relationships::new(
                database.clone(),
                events.clone(),
            )),
            events,
            gateway_sessions: Arc::default(),
            guilds,
            database,
            rate_limiter: Arc::default(),
            unauthenticated_rate_limiter: Arc::default(),
            gateway_rate_limiter: Arc::default(),
            trusted_proxy_networks: Arc::default(),
            voice,
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
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Closed);

        let (status, body) = send(
            &state,
            Request::get("/api/v1/health").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let health: HealthResponse = json(&body);
        assert_eq!(health.status, HealthStatus::Ok);
        assert_eq!(health.version, env!("CARGO_PKG_VERSION"));

        database.close().await;
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
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);

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
        database.close().await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::Unauthorized);
    }

    #[tokio::test]
    async fn the_web_client_signs_in_with_a_cookie_usable_only_from_this_origin() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;

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
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Closed);
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
        database.close().await;

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
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;
    }

    #[tokio::test]
    async fn authentication_attempts_are_rate_limited_per_address() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Closed);
        let attempt = || post_json("/api/v1/auth/register", &registration("aurore"));
        for _ in 0..rate_limit::AUTHENTICATION_ATTEMPTS_PER_ADDRESS.maximum_events {
            let (status, _) = send(&state, attempt()).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "registration is closed");
        }
        let (status, body) = send(&state, attempt()).await;
        database.close().await;
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
    async fn one_address_cycling_login_names_cannot_fill_the_rate_limiter() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Closed);
        let attempts = rate_limit::AUTHENTICATION_ATTEMPTS_PER_ADDRESS.maximum_events * 3;
        for index in 0..attempts {
            send_from(
                &state,
                "198.51.100.1:1000",
                wrong_login(&format!("n{index}")),
            )
            .await;
        }
        database.close().await;
        // The address key, then two keys per attempt the address limit let through: refused
        // attempts add none, and signed-in users' limits live in their own table.
        let allowed = rate_limit::AUTHENTICATION_ATTEMPTS_PER_ADDRESS.maximum_events as usize;
        assert_eq!(
            state.unauthenticated_rate_limiter.tracked_key_count(),
            1 + 2 * allowed
        );
        assert_eq!(state.rate_limiter.tracked_key_count(), 0);
    }

    #[tokio::test]
    async fn login_attempts_are_limited_per_account_and_address_before_the_password_check() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    }

    #[tokio::test]
    async fn forwarded_for_from_the_loopback_proxy_separates_clients() {
        let (_directory, database) = migrated_database().await;
        let mut state = test_state(database.clone(), RegistrationMode::Closed);
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
        database.close().await;
        assert_eq!(limited, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(other_client, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn malformed_bodies_and_unknown_routes_answer_with_json_errors() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;
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

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn out_of_range_ids_are_not_found_and_invite_guessing_is_rate_limited() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;
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
    async fn history_gives_each_message_its_own_attachments_in_order() {
        use mellow_protocol::{Attachment, Message};

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let mut uploaded = Vec::new();
        for name in ["a1.png", "b1.png", "a2.png", "b2.png"] {
            let path = format!("/api/v1/channels/{general}/attachments?file_name={name}");
            let (_, body) = send(&state, upload_request(&path, &owner, PNG)).await;
            uploaded.push(json::<Attachment>(&body).id);
        }
        let messages_path = format!("/api/v1/channels/{general}/messages");
        // Ids interleave between the two messages: A gets the 1st and 3rd, B the 2nd and 4th.
        let expected = [
            vec![uploaded[0], uploaded[2]],
            vec![uploaded[1], uploaded[3]],
            vec![],
        ];
        for (index, attachment_ids) in expected.iter().enumerate() {
            let ids: Vec<String> = attachment_ids.iter().map(ToString::to_string).collect();
            let (status, _) = send(
                &state,
                authorized(
                    "POST",
                    &messages_path,
                    &owner,
                    Some(serde_json::json!({
                        "content": format!("message {index}"),
                        "attachment_ids": ids,
                        "nonce": format!("nonce-{index}"),
                    })),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
        }
        let (_, body) = send(&state, authorized("GET", &messages_path, &owner, None)).await;
        database.close().await;
        let history: Vec<Message> = json(&body);
        let sent = &history[history.len() - expected.len()..];
        let attachment_ids: Vec<Vec<_>> = sent
            .iter()
            .map(|message| {
                message
                    .attachments
                    .iter()
                    .map(|attachment| attachment.id)
                    .collect()
            })
            .collect();
        assert_eq!(attachment_ids, expected);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // One scenario: upload, send, list, download, refusals.
    async fn attachments_upload_send_and_download_safely() {
        use mellow_protocol::{Attachment, Message};

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;

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
        let (_directory, database) = migrated_database().await;
        let mut state = test_state(database.clone(), RegistrationMode::Open);
        state.attachments = Arc::new(crate::domain::attachments::Attachments::new(
            database.clone(),
            Arc::new(SnowflakeGenerator::default()),
            state.messages.clone(),
            test_attachment_files(),
            PNG.len() as u64 + 1,
        ));
        let (owner, general) = owner_and_general_channel(&state).await;
        let path = format!("/api/v1/channels/{general}/attachments?file_name=a.png");
        let (first, _) = send(&state, upload_request(&path, &owner, PNG)).await;
        let (second, body) = send(&state, upload_request(&path, &owner, PNG)).await;
        database.close().await;

        assert_eq!(first, StatusCode::CREATED);
        assert_eq!(second, StatusCode::CONFLICT);
        assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::LimitReached);
    }

    #[tokio::test]
    async fn attachments_over_the_limit_are_refused_before_being_read() {
        let (_directory, database) = migrated_database().await;
        let mut state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn messages_over_http() {
        use mellow_protocol::{Message, ReadState};

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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

        database.close().await;
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

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn only_instance_administrators_create_servers_when_the_instance_says_so() {
        let (_directory, database) = migrated_database().await;
        let mut state = test_state(database.clone(), RegistrationMode::Open);
        state.guilds = Arc::new(Guilds::new(
            database.clone(),
            Arc::new(SnowflakeGenerator::default()),
            Arc::new(EventHub::default()),
            crate::domain::account_rules::GuildCreationPolicy::Administrators,
        ));
        let member = registered_token(&state, "member").await;
        let administrator = registered_token(&state, "admin").await;
        sqlx::query("UPDATE users SET is_instance_administrator = 1 WHERE username = 'admin'")
            .execute(database.writer())
            .await
            .unwrap();
        let create = |token: String| {
            let state = state.clone();
            async move {
                send(
                    &state,
                    authorized(
                        "POST",
                        "/api/v1/guilds",
                        &token,
                        Some(serde_json::json!({"name": "Club"})),
                    ),
                )
                .await
                .0
            }
        };

        let by_member = create(member).await;
        let by_administrator = create(administrator).await;
        database.close().await;

        assert_eq!(by_member, StatusCode::FORBIDDEN);
        assert_eq!(by_administrator, StatusCode::CREATED);
    }

    #[tokio::test]
    async fn channels_are_reordered_over_http() {
        use mellow_protocol::Channel;

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
        let (_, body) = send(
            &state,
            authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &owner, None),
        )
        .await;
        let channels = json::<mellow_protocol::Guild>(&body).channels;
        let [category, general] = [channels[0].id, channels[1].id];
        let order_path = format!("/api/v1/guilds/{guild_id}/channels/order");
        // `general` leaves its category and moves above it.
        let order = serde_json::json!({"channels": [
            {"id": general, "parent_id": null},
            {"id": category, "parent_id": null},
        ]});
        let (status, body) = send(
            &state,
            authorized("PUT", &order_path, &owner, Some(order.clone())),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let placed: Vec<(Snowflake, Option<Snowflake>)> = json::<Vec<Channel>>(&body)
            .into_iter()
            .map(|channel| (channel.id, channel.parent_id))
            .collect();
        assert_eq!(placed, [(general, None), (category, None)]);

        let (status, _) = send(&state, authorized("PUT", &order_path, &guest, Some(order))).await;
        database.close().await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn roles_are_reordered_over_http() {
        use mellow_protocol::Role;

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json::<ApiErrorBody>(&body).code,
            ApiErrorCode::ValidationFailed
        );
    }

    #[tokio::test]
    async fn overwrites_hide_a_channel_over_http() {
        use mellow_protocol::{Channel, Guild, Message, ReadState};

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn kicks_and_bans_over_http() {
        use mellow_protocol::{Ban, Invite};

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn the_audit_log_over_http() {
        use mellow_protocol::{AuditAction, AuditLogEntry};

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "the kicked guest is no longer a member"
        );
        assert_eq!(bad_status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn server_settings_over_http() {
        use mellow_protocol::{Guild, GuildInvite};

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
        let guild_path = format!("/api/v1/guilds/{guild_id}");
        let invites_path = format!("{guild_path}/invites");
        let rename = || Some(serde_json::json!({"name": "Book club"}));

        let (guest_rename, body) =
            send(&state, authorized("PATCH", &guild_path, &guest, rename())).await;
        let guest_rename_code = json::<ApiErrorBody>(&body).code;
        let (unknown_rename, _) = send(
            &state,
            authorized("PATCH", "/api/v1/guilds/42", &owner, rename()),
        )
        .await;
        let (unknown_field, _) = send(
            &state,
            authorized(
                "PATCH",
                &guild_path,
                &owner,
                Some(serde_json::json!({"icon": "x"})),
            ),
        )
        .await;
        let (_, body) = send(&state, authorized("PATCH", &guild_path, &owner, rename())).await;
        let renamed: Guild = json(&body);

        let (guest_list, _) = send(&state, authorized("GET", &invites_path, &guest, None)).await;
        let (_, body) = send(&state, authorized("GET", &invites_path, &owner, None)).await;
        let invites: Vec<GuildInvite> = json(&body);
        let code = invites[0].code.clone();
        let invite_path = format!("/api/v1/invites/{code}");
        let (guest_revoke, _) =
            send(&state, authorized("DELETE", &invite_path, &guest, None)).await;
        let (revoked, _) = send(&state, authorized("DELETE", &invite_path, &owner, None)).await;
        let (revoked_again, _) =
            send(&state, authorized("DELETE", &invite_path, &owner, None)).await;

        let (guest_delete, _) = send(&state, authorized("DELETE", &guild_path, &guest, None)).await;
        let (deleted, _) = send(&state, authorized("DELETE", &guild_path, &owner, None)).await;
        let (after_delete, _) = send(&state, authorized("GET", &guild_path, &owner, None)).await;
        database.close().await;

        assert_eq!(guest_rename, StatusCode::FORBIDDEN);
        assert_eq!(guest_rename_code, ApiErrorCode::MissingPermissions);
        assert_eq!(unknown_rename, StatusCode::NOT_FOUND);
        assert_eq!(unknown_field, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(renamed.name, "Book club");
        assert_eq!(guest_list, StatusCode::FORBIDDEN);
        assert_eq!(invites.len(), 1);
        assert_eq!((invites[0].uses, invites[0].max_uses), (1, None));
        assert_eq!(invites[0].creator.username, "owner");
        assert_eq!(guest_revoke, StatusCode::FORBIDDEN);
        assert_eq!(revoked, StatusCode::NO_CONTENT);
        assert_eq!(revoked_again, StatusCode::NOT_FOUND);
        assert_eq!(guest_delete, StatusCode::FORBIDDEN);
        assert_eq!(deleted, StatusCode::NO_CONTENT);
        assert_eq!(after_delete, StatusCode::NOT_FOUND);
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
    #[allow(clippy::too_many_lines)] // One scenario: who may set, crop, read and remove icons.
    async fn guild_and_channel_icons_are_cropped_served_to_members_only_and_audited() {
        use crate::domain::image_processing::tests::{animated_gif, png};
        use mellow_protocol::{AuditChangeKey, AuditLogEntry, Channel, Guild, GuildSummary};

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
        let stranger = registered_token(&state, "stranger").await;
        let guild_icon_path = format!("/api/v1/guilds/{guild_id}/icon");
        let (_, body) = send(
            &state,
            authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &owner, None),
        )
        .await;
        let channels = json::<Guild>(&body).channels;
        let (category, general) = (channels[0].id, channels[1].id);

        let (guest_upload, _) = send(
            &state,
            picture_request("PUT", &guild_icon_path, &guest, png(64, 64)),
        )
        .await;
        let (half_crop, _) = send(
            &state,
            picture_request(
                "PUT",
                &format!("{guild_icon_path}?crop_x=1"),
                &owner,
                png(64, 64),
            ),
        )
        .await;
        let (status, body) = send(
            &state,
            picture_request("PUT", &guild_icon_path, &owner, animated_gif(600, 400, 3)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let guild_icon = json::<Guild>(&body).icon.unwrap();
        let (_, body) = send(
            &state,
            authorized("GET", "/api/v1/users/@me/guilds", &guest, None),
        )
        .await;
        let guest_sees = json::<Vec<GuildSummary>>(&body)[0].icon;
        let icon_path = format!("/api/v1/icons/{}", guild_icon.id);
        let (member_read, animated) =
            send(&state, authorized("GET", &icon_path, &guest, None)).await;
        let (still_read, still) = send(
            &state,
            authorized("GET", &format!("{icon_path}/still"), &guest, None),
        )
        .await;
        let (stranger_read, _) = send(&state, authorized("GET", &icon_path, &stranger, None)).await;

        let (status, body) = send(
            &state,
            picture_request(
                "PUT",
                &format!("/api/v1/channels/{general}/icon?crop_x=10&crop_y=0&crop_size=40"),
                &owner,
                png(64, 40),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let channel_icon = json::<Channel>(&body).icon.unwrap();
        let (_, channel_icon_bytes) = send(
            &state,
            authorized(
                "GET",
                &format!("/api/v1/icons/{}", channel_icon.id),
                &guest,
                None,
            ),
        )
        .await;
        let channel_icon_path = format!("/api/v1/icons/{}", channel_icon.id);
        let (status, _) = send(
            &state,
            authorized(
                "PUT",
                &format!("/api/v1/channels/{general}/permissions/{guild_id}"),
                &owner,
                Some(serde_json::json!({"kind": "role", "deny": "2"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (hidden_from_guest, _) =
            send(&state, authorized("GET", &channel_icon_path, &guest, None)).await;
        let (hidden_still, _) = send(
            &state,
            authorized("GET", &format!("{channel_icon_path}/still"), &guest, None),
        )
        .await;
        let (shown_to_owner, _) =
            send(&state, authorized("GET", &channel_icon_path, &owner, None)).await;
        // Icon files are not orphans: the sweep keeps them however old they are.
        state
            .attachments
            .purge(
                crate::domain::clock::now_unix_milliseconds()
                    + crate::domain::attachments::UNSENT_UPLOAD_LIFETIME_MILLISECONDS * 2,
            )
            .await
            .unwrap();
        let (after_sweep, _) =
            send(&state, authorized("GET", &channel_icon_path, &owner, None)).await;
        let (category_upload, _) = send(
            &state,
            picture_request(
                "PUT",
                &format!("/api/v1/channels/{category}/icon"),
                &owner,
                png(64, 64),
            ),
        )
        .await;

        let (_, body) = send(&state, authorized("DELETE", &guild_icon_path, &owner, None)).await;
        let after_removal = json::<Guild>(&body).icon;
        let (removed_read, _) = send(&state, authorized("GET", &icon_path, &guest, None)).await;
        let (_, body) = send(
            &state,
            authorized(
                "GET",
                &format!("/api/v1/guilds/{guild_id}/audit-log"),
                &owner,
                None,
            ),
        )
        .await;
        let icon_changes = json::<Vec<AuditLogEntry>>(&body)
            .into_iter()
            .flat_map(|entry| entry.changes)
            .filter(|change| change.key == AuditChangeKey::Icon)
            .count();
        database.close().await;

        assert_eq!(guest_upload, StatusCode::FORBIDDEN);
        assert_eq!(half_crop, StatusCode::BAD_REQUEST);
        assert!(guild_icon.is_animated);
        assert_eq!(guest_sees, Some(guild_icon));
        assert_eq!(member_read, StatusCode::OK);
        let animated = webp_animation::Decoder::new(&animated).unwrap();
        assert_eq!(animated.dimensions(), (256, 256));
        assert_eq!(animated.into_iter().count(), 3);
        assert_eq!(still_read, StatusCode::OK);
        assert_eq!(image::load_from_memory(&still).unwrap().width(), 256);
        assert_eq!(stranger_read, StatusCode::NOT_FOUND, "members only");
        assert!(!channel_icon.is_animated);
        let channel_picture = image::load_from_memory(&channel_icon_bytes).unwrap();
        assert_eq!(
            (channel_picture.width(), channel_picture.height()),
            (40, 40)
        );
        assert_eq!(hidden_from_guest, StatusCode::NOT_FOUND, "viewers only");
        assert_eq!(hidden_still, StatusCode::NOT_FOUND);
        assert_eq!(shown_to_owner, StatusCode::OK);
        assert_eq!(after_sweep, StatusCode::OK);
        assert_eq!(category_upload, StatusCode::BAD_REQUEST);
        assert_eq!(after_removal, None);
        assert_eq!(removed_read, StatusCode::NOT_FOUND);
        assert_eq!(icon_changes, 3, "set, channel set, removed");
    }

    #[tokio::test]
    async fn emoji_uploads_spend_the_picture_budget_before_their_body_is_read() {
        use crate::domain::image_processing::tests::png;
        use crate::http::rate_limit::PROFILE_IMAGE_WRITES_PER_USER;

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (_, guest, guild_id, _) = guild_with_guest(&state).await;
        let path = format!("/api/v1/guilds/{guild_id}/emojis?name=party");
        let mut answers = Vec::new();
        for _ in 0..PROFILE_IMAGE_WRITES_PER_USER.maximum_events {
            answers.push(
                send(&state, picture_request("POST", &path, &guest, png(8, 8)))
                    .await
                    .0,
            );
        }
        let (over_budget, _) =
            send(&state, picture_request("POST", &path, &guest, png(8, 8))).await;
        let (avatar_after, _) = send(
            &state,
            picture_request("PUT", "/api/v1/users/@me/avatar", &guest, png(8, 8)),
        )
        .await;
        database.close().await;

        assert!(
            answers
                .iter()
                .all(|status| *status == StatusCode::FORBIDDEN),
            "refused uploads count too"
        );
        assert_eq!(over_budget, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            avatar_after,
            StatusCode::TOO_MANY_REQUESTS,
            "one budget for every picture"
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // One scenario: add, list, serve, react, rename, delete.
    async fn custom_emoji_are_managed_served_to_members_and_reacted_with_over_http() {
        use crate::domain::guild_rules::MAXIMUM_EMOJI_UPLOAD_BYTES;
        use crate::domain::image_processing::tests::{animated_gif, png};
        use mellow_protocol::{CustomEmoji, Guild, Message};

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
        let stranger = registered_token(&state, "stranger").await;
        let emojis_path = format!("/api/v1/guilds/{guild_id}/emojis");

        let (by_guest, _) = send(
            &state,
            picture_request(
                "POST",
                &format!("{emojis_path}?name=party"),
                &guest,
                png(8, 8),
            ),
        )
        .await;
        let (too_large, _) = send(
            &state,
            picture_request(
                "POST",
                &format!("{emojis_path}?name=party"),
                &owner,
                vec![0; MAXIMUM_EMOJI_UPLOAD_BYTES + 1],
            ),
        )
        .await;
        let (without_name, _) = send(
            &state,
            picture_request("POST", &emojis_path, &owner, png(8, 8)),
        )
        .await;
        let (status, body) = send(
            &state,
            picture_request(
                "POST",
                &format!("{emojis_path}?name=party"),
                &owner,
                png(8, 8),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "{}",
            String::from_utf8_lossy(&body)
        );
        let party: CustomEmoji = json(&body);
        let (_, body) = send(
            &state,
            picture_request(
                "POST",
                &format!("{emojis_path}?name=parrot"),
                &owner,
                animated_gif(300, 200, 3),
            ),
        )
        .await;
        let parrot: CustomEmoji = json(&body);
        let (_, body) = send(&state, authorized("GET", &emojis_path, &guest, None)).await;
        let listed: Vec<CustomEmoji> = json(&body);
        let (listed_to_stranger, _) =
            send(&state, authorized("GET", &emojis_path, &stranger, None)).await;
        let (_, body) = send(
            &state,
            authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &guest, None),
        )
        .await;
        let guild: Guild = json(&body);
        let parrot_path = format!("/api/v1/icons/{}", parrot.id);
        let (_, animated) = send(&state, authorized("GET", &parrot_path, &guest, None)).await;
        let (still_status, still) = send(
            &state,
            authorized("GET", &format!("{parrot_path}/still"), &guest, None),
        )
        .await;
        let (served_to_stranger, _) =
            send(&state, authorized("GET", &parrot_path, &stranger, None)).await;

        let general = guild.channels[1].id;
        let (_, body) = send(
            &state,
            authorized(
                "POST",
                &format!("/api/v1/channels/{general}/messages"),
                &owner,
                Some(serde_json::json!({"content": format!("<a:parrot:{}>", parrot.id)})),
            ),
        )
        .await;
        let message: Message = json(&body);
        let (reacted, _) = send(
            &state,
            authorized(
                "PUT",
                &format!(
                    "/api/v1/channels/{general}/messages/{}/reactions/%3A{}/@me",
                    message.id, parrot.id
                ),
                &guest,
                None,
            ),
        )
        .await;

        let party_path = format!("{emojis_path}/{}", party.id);
        let (by_guest_rename, _) = send(
            &state,
            authorized(
                "PATCH",
                &party_path,
                &guest,
                Some(serde_json::json!({"name": "x_x"})),
            ),
        )
        .await;
        let (_, body) = send(
            &state,
            authorized(
                "PATCH",
                &party_path,
                &owner,
                Some(serde_json::json!({"name": "fiesta"})),
            ),
        )
        .await;
        let renamed: CustomEmoji = json(&body);
        let (taken, body) = send(
            &state,
            authorized(
                "PATCH",
                &party_path,
                &owner,
                Some(serde_json::json!({"name": "PARROT"})),
            ),
        )
        .await;
        let taken_code = json::<ApiErrorBody>(&body).code;
        let (deleted, _) = send(&state, authorized("DELETE", &party_path, &owner, None)).await;
        let (deleted_picture, _) = send(
            &state,
            authorized("GET", &format!("/api/v1/icons/{}", party.id), &owner, None),
        )
        .await;
        let (deleted_again, _) =
            send(&state, authorized("DELETE", &party_path, &owner, None)).await;
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
        let reactions = json::<Vec<Message>>(&body).remove(0).reactions;
        database.close().await;

        assert_eq!(by_guest, StatusCode::FORBIDDEN);
        assert_eq!(too_large, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(without_name, StatusCode::BAD_REQUEST);
        assert!(!party.is_animated);
        assert!(parrot.is_animated);
        assert_eq!(listed, [party.clone(), parrot.clone()]);
        assert_eq!(guild.emojis, listed);
        assert_eq!(listed_to_stranger, StatusCode::NOT_FOUND);
        assert_eq!(
            webp_animation::Decoder::new(&animated)
                .unwrap()
                .dimensions(),
            (128, 128)
        );
        assert_eq!(still_status, StatusCode::OK);
        assert_eq!(image::load_from_memory(&still).unwrap().width(), 128);
        assert_eq!(served_to_stranger, StatusCode::NOT_FOUND, "members only");
        assert_eq!(reacted, StatusCode::NO_CONTENT);
        assert_eq!(reactions[0].emoji, format!(":{}", parrot.id));
        assert_eq!(by_guest_rename, StatusCode::FORBIDDEN);
        assert_eq!(renamed.name, "fiesta");
        assert_eq!(taken, StatusCode::BAD_REQUEST);
        assert_eq!(taken_code, ApiErrorCode::ValidationFailed);
        assert_eq!(deleted, StatusCode::NO_CONTENT);
        assert_eq!(deleted_picture, StatusCode::NOT_FOUND);
        assert_eq!(deleted_again, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // One scenario: upload, serve, show everywhere, replace, remove.
    async fn animated_avatars_and_banners_are_stored_served_and_shown_with_their_owner() {
        use crate::domain::image_processing::tests::{animated_gif, png};
        use mellow_protocol::{Member, Message};

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        let first_avatar = json::<CurrentUser>(&body).avatar.unwrap();
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
        // A request that started before the banner was set must not take it away.
        let request_started_before_banner = CurrentUser {
            banner: None,
            ..me.clone()
        };
        let after_stale_request = state
            .profile_images
            .replace(
                request_started_before_banner,
                crate::storage::profile_images::ProfileImageSlot::Avatar,
                Some(animated_gif(64, 64, 3)),
                crate::domain::clock::now_unix_milliseconds(),
            )
            .await
            .unwrap();
        let avatar = after_stale_request.avatar.unwrap();

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
        database.close().await;

        assert!(avatar.is_animated);
        assert!(!banner.is_animated);
        assert_eq!(refused, StatusCode::BAD_REQUEST, "not a picture");
        assert_eq!(me.banner, Some(banner));
        assert_eq!(me.avatar, Some(first_avatar));
        assert_ne!(avatar.id, first_avatar.id, "each version its own id");
        assert_eq!(after_stale_request.banner, Some(banner));
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

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        let (_, body) = send(
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
        let sent: mellow_protocol::Message = json(&body);
        // What the background worker runs for this upload.
        state.attachments.optimize(uploaded.id).await.unwrap();
        let ((_, bytes), headers) = send_from(
            &state,
            TEST_PEER,
            authorized(
                "GET",
                &format!("/api/v1/attachments/{}", uploaded.id),
                &owner,
                None,
            ),
        )
        .await;
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
        let history: Vec<mellow_protocol::Message> = json(&body);
        database.close().await;

        assert_eq!(uploaded.content_type, "image/gif", "answered as received");
        assert_eq!(sent.attachments[0].file_name, "party.gif");
        assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "image/webp");
        assert!(bytes.len() < gif.len());
        let stored = &history[0].attachments[0];
        assert_eq!(stored.file_name, "party.webp");
        assert_eq!(stored.content_type, "image/webp");
        assert_eq!(stored.size_bytes, bytes.len() as u64);
    }

    #[tokio::test]
    async fn a_download_keeps_its_slot_until_its_body_is_dropped_and_uploads_go_one_per_user() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let (_, body) = send(
            &state,
            upload_request(
                &format!("/api/v1/channels/{general}/attachments?file_name=a.png"),
                &owner,
                PNG,
            ),
        )
        .await;
        let uploaded: mellow_protocol::Attachment = json(&body);
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
        let all_slots = state.attachments.free_download_slots();
        let response = router(state.clone())
            .layer(MockConnectInfo(TEST_PEER.parse::<SocketAddr>().unwrap()))
            .oneshot(authorized(
                "GET",
                &format!("/api/v1/attachments/{}", uploaded.id),
                &owner,
                None,
            ))
            .await
            .unwrap();
        let slots_while_unsent = state.attachments.free_download_slots();
        drop(response);
        let slots_after = state.attachments.free_download_slots();

        let (_, body) = send(&state, authorized("GET", "/api/v1/users/@me", &owner, None)).await;
        let owner_id = json::<CurrentUser>(&body).id;
        let first = state
            .attachments
            .start_upload(owner_id, general, "first.txt")
            .await
            .unwrap();
        let second = state
            .attachments
            .start_upload(owner_id, general, "second.txt")
            .await;
        drop(first);
        let after_first_ended = state
            .attachments
            .start_upload(owner_id, general, "third.txt")
            .await;
        database.close().await;

        assert_eq!(slots_while_unsent, all_slots - 1);
        assert_eq!(slots_after, all_slots);
        assert!(matches!(
            second,
            Err(crate::domain::messages::MessageError::Busy)
        ));
        assert!(after_first_ended.is_ok());
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
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;
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

    #[tokio::test]
    async fn the_chosen_status_is_stored_and_validated() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let token = registered_token(&state, "aurore").await;
        let put = |body: serde_json::Value| {
            authorized("PUT", "/api/v1/users/@me/presence", &token, Some(body))
        };

        let (status, body) = send(
            &state,
            put(serde_json::json!({"status": "do_not_disturb", "custom_status": "  Focus  "})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let updated = json::<CurrentUser>(&body);
        assert_eq!(updated.status, mellow_protocol::ChosenStatus::DoNotDisturb);
        assert_eq!(updated.custom_status.as_deref(), Some("Focus"));
        let (status, body) = send(
            &state,
            put(serde_json::json!({"status": "online", "custom_status": "x".repeat(129)})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json::<ApiErrorBody>(&body).code,
            ApiErrorCode::ValidationFailed
        );
        let (status, _) = send(&state, put(serde_json::json!({"status": "away"}))).await;
        assert!(status.is_client_error());
        let (_, body) = send(&state, authorized("GET", "/api/v1/users/@me", &token, None)).await;
        let stored = json::<CurrentUser>(&body);
        assert_eq!(stored.status, mellow_protocol::ChosenStatus::DoNotDisturb);
        assert_eq!(stored.custom_status.as_deref(), Some("Focus"));

        let (_, body) = send(&state, put(serde_json::json!({"status": "invisible"}))).await;
        let cleared = json::<CurrentUser>(&body);
        assert_eq!(cleared.status, mellow_protocol::ChosenStatus::Invisible);
        assert_eq!(cleared.custom_status, None);
        database.close().await;
    }

    /// A stolen session that uses up its password-check budget must not keep the owner's other
    /// devices from changing the password (and so revoking it).
    #[tokio::test]
    async fn password_checks_and_profile_writes_are_rate_limited_per_session() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;

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

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;
    }

    #[tokio::test]
    async fn typing_needs_membership_and_is_rate_limited() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
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
        database.close().await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn reactions_and_pins_over_http() {
        use mellow_protocol::{Guild, Message, PublicUser};

        const THUMBS_UP: &str = "%F0%9F%91%8D";
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, guest_id) = guild_with_guest(&state).await;
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
                Some(serde_json::json!({"content": "react to me"})),
            ),
        )
        .await;
        let message: Message = json(&body);
        let reactions_path = format!("{messages_path}/{}/reactions", message.id);
        let mine = format!("{reactions_path}/{THUMBS_UP}/@me");

        let mut statuses = Vec::new();
        for _ in 0..2 {
            statuses.push(send(&state, authorized("PUT", &mine, &guest, None)).await.0);
        }
        let (_, body) = send(
            &state,
            authorized("GET", &format!("{messages_path}?limit=10"), &guest, None),
        )
        .await;
        let history: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let (status, body) = send(
            &state,
            authorized(
                "GET",
                &format!("{reactions_path}/{THUMBS_UP}"),
                &owner,
                None,
            ),
        )
        .await;
        let users: Vec<PublicUser> = json(&body);
        let (bad_emoji, bad_emoji_body) = send(
            &state,
            authorized("PUT", &format!("{reactions_path}/lol/@me"), &guest, None),
        )
        .await;
        let removed = send(&state, authorized("DELETE", &mine, &guest, None))
            .await
            .0;
        database.close().await;

        assert_eq!(statuses, [StatusCode::NO_CONTENT, StatusCode::NO_CONTENT]);
        assert_eq!(
            history[0]["reactions"],
            serde_json::json!([{"emoji": "👍", "count": 1, "me": true}])
        );
        assert_eq!(history[0]["pinned"], false);
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            users.iter().map(|user| user.id).collect::<Vec<_>>(),
            [guest_id]
        );
        assert_eq!(bad_emoji, StatusCode::BAD_REQUEST);
        assert_eq!(
            json::<ApiErrorBody>(&bad_emoji_body).code,
            ApiErrorCode::ValidationFailed
        );
        assert_eq!(removed, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn pins_over_http() {
        use mellow_protocol::{Guild, Message};

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
        let (_, body) = send(
            &state,
            authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &owner, None),
        )
        .await;
        let general = json::<Guild>(&body).channels[1].id;
        let (_, body) = send(
            &state,
            authorized(
                "POST",
                &format!("/api/v1/channels/{general}/messages"),
                &guest,
                Some(serde_json::json!({"content": "pin me"})),
            ),
        )
        .await;
        let message: Message = json(&body);
        let pin_path = format!("/api/v1/channels/{general}/pins/{}", message.id);
        let guest_pins = send(&state, authorized("PUT", &pin_path, &guest, None)).await;
        let owner_pins = send(&state, authorized("PUT", &pin_path, &owner, None))
            .await
            .0;
        let pins_path = format!("/api/v1/channels/{general}/pins");
        let (_, body) = send(&state, authorized("GET", &pins_path, &guest, None)).await;
        let pins: Vec<Message> = json(&body);
        let unpinned = send(&state, authorized("DELETE", &pin_path, &owner, None))
            .await
            .0;
        let (_, body) = send(&state, authorized("GET", &pins_path, &guest, None)).await;
        let pins_after: Vec<Message> = json(&body);
        database.close().await;

        assert_eq!(guest_pins.0, StatusCode::FORBIDDEN);
        assert_eq!(
            json::<ApiErrorBody>(&guest_pins.1).code,
            ApiErrorCode::MissingPermissions
        );
        assert_eq!(owner_pins, StatusCode::NO_CONTENT);
        assert_eq!(pins.len(), 1);
        assert!(pins[0].pinned);
        assert_eq!(unpinned, StatusCode::NO_CONTENT);
        assert_eq!(pins_after, Vec::new());
    }

    #[tokio::test]
    async fn message_search_over_http() {
        use mellow_protocol::MessageSearchResults;

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let (_, body) = send(
            &state,
            authorized("GET", "/api/v1/users/@me/guilds", &owner, None),
        )
        .await;
        let guild_id = json::<Vec<mellow_protocol::GuildSummary>>(&body)[0].id;
        send(
            &state,
            authorized(
                "POST",
                &format!("/api/v1/channels/{general}/messages"),
                &owner,
                Some(serde_json::json!({"content": "Crème brûlée tonight?"})),
            ),
        )
        .await;
        let search = |body: serde_json::Value| {
            authorized(
                "POST",
                &format!("/api/v1/guilds/{guild_id}/messages/search"),
                &owner,
                Some(body),
            )
        };
        let (status, body) = send(
            &state,
            search(serde_json::json!({"q": "creme -pizza", "sort": "newest", "limit": 5})),
        )
        .await;
        let found: MessageSearchResults = json(&body);
        let unknown_field = send(&state, search(serde_json::json!({"q": "creme", "page": 2})))
            .await
            .0;
        let as_get = send(
            &state,
            authorized(
                "GET",
                &format!("/api/v1/guilds/{guild_id}/messages/search?q=creme"),
                &owner,
                None,
            ),
        )
        .await
        .0;
        let (_, body) = send(
            &state,
            authorized(
                "POST",
                "/api/v1/users/@me/messages/search",
                &owner,
                Some(serde_json::json!({"q": "creme"})),
            ),
        )
        .await;
        let in_conversations: MessageSearchResults = json(&body);
        for _ in 2..rate_limit::MESSAGE_SEARCHES_PER_USER.maximum_events {
            send(&state, search(serde_json::json!({"q": "creme"}))).await;
        }
        let limited = send(&state, search(serde_json::json!({"q": "creme"})))
            .await
            .0;
        database.close().await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(found.total_count, 1);
        assert_eq!(found.hits[0].message.content, "Crème brûlée tonight?");
        assert_eq!(found.hits[0].highlights.len(), 1);
        assert_eq!(unknown_field, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(as_get, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(in_conversations.total_count, 0);
        assert_eq!(limited, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn reactions_are_rate_limited_per_user() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let (_, body) = send(
            &state,
            authorized(
                "POST",
                &format!("/api/v1/channels/{general}/messages"),
                &owner,
                Some(serde_json::json!({"content": "hi"})),
            ),
        )
        .await;
        let message: mellow_protocol::Message = json(&body);
        let mine = format!(
            "/api/v1/channels/{general}/messages/{}/reactions/%F0%9F%91%8D/@me",
            message.id
        );
        for _ in 0..rate_limit::REACTION_WRITES_PER_USER.maximum_events {
            send(&state, authorized("PUT", &mine, &owner, None)).await;
        }
        let (status, _) = send(&state, authorized("DELETE", &mine, &owner, None)).await;
        database.close().await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn reactions_are_rate_limited_per_channel() {
        use mellow_protocol::{Guild, Invite, Message};

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
        let (_, body) = send(
            &state,
            authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &owner, None),
        )
        .await;
        let general = json::<Guild>(&body).channels[1].id;
        let (_, body) = send(
            &state,
            authorized(
                "POST",
                &format!("/api/v1/guilds/{guild_id}/invites"),
                &owner,
                Some(serde_json::json!({})),
            ),
        )
        .await;
        let code = json::<Invite>(&body).code;
        let mut members = vec![owner.clone(), guest];
        for name in ["third", "fourth"] {
            let token = registered_token(&state, name).await;
            send(
                &state,
                authorized("POST", &format!("/api/v1/invites/{code}"), &token, None),
            )
            .await;
            members.push(token);
        }
        let (_, body) = send(
            &state,
            authorized(
                "POST",
                &format!("/api/v1/channels/{general}/messages"),
                &owner,
                Some(serde_json::json!({"content": "hi"})),
            ),
        )
        .await;
        let message: Message = json(&body);
        let reaction = format!(
            "/api/v1/channels/{general}/messages/{}/reactions/%F0%9F%91%8D/@me",
            message.id
        );
        let per_user = rate_limit::REACTION_WRITES_PER_USER.maximum_events;
        let per_channel = rate_limit::REACTION_WRITES_PER_CHANNEL.maximum_events;
        let (last, others) = members.split_last().unwrap();
        let mut accepted = 0;
        for token in others {
            for _ in 0..per_user {
                let (status, _) = send(&state, authorized("PUT", &reaction, token, None)).await;
                accepted += u32::from(status == StatusCode::NO_CONTENT);
            }
        }
        let (status, _) = send(&state, authorized("PUT", &reaction, last, None)).await;
        database.close().await;

        assert_eq!(
            accepted, per_channel,
            "three members fill the channel's budget"
        );
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn moderators_remove_reactions_over_http() {
        use mellow_protocol::{Guild, Message};

        const THUMBS_UP: &str = "%F0%9F%91%8D";
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, guest_id) = guild_with_guest(&state).await;
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
                Some(serde_json::json!({"content": "hi"})),
            ),
        )
        .await;
        let message: Message = json(&body);
        let reactions = format!("{messages_path}/{}/reactions", message.id);
        let guest_reacts = authorized("PUT", &format!("{reactions}/{THUMBS_UP}/@me"), &guest, None);
        send(&state, guest_reacts).await;

        let mut statuses = Vec::new();
        for (token, path) in [
            (&guest, reactions.clone()),
            (&owner, format!("{reactions}/{THUMBS_UP}/{guest_id}")),
            (&owner, format!("{reactions}/{THUMBS_UP}")),
            (&owner, reactions.clone()),
        ] {
            statuses.push(
                send(&state, authorized("DELETE", &path, token, None))
                    .await
                    .0,
            );
        }
        database.close().await;

        assert_eq!(
            statuses,
            [
                StatusCode::FORBIDDEN,
                StatusCode::NO_CONTENT,
                StatusCode::NO_CONTENT,
                StatusCode::NO_CONTENT
            ]
        );
    }
}
