//! REST API (axum). Handlers validate input and map domain errors to HTTP; no SQL or business
//! rules live here (ARCHITECTURE.md §4.1).

mod account_routes;
pub(crate) mod api_error;
pub(crate) mod attachment_routes;
pub(crate) mod authentication;
pub(crate) mod client_address;
mod emoji_routes;
mod guild_routes;
mod message_routes;
mod profile_image_routes;
pub(crate) mod rate_limit;
mod relationship_routes;
mod sound_routes;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod web_client;

use std::net::SocketAddr;
use std::sync::Arc;

use crate::storage::Database;
use axum::extract::DefaultBodyLimit;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse as _, Response};
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
use crate::domain::sound_rules::MAXIMUM_SOUND_FILE_BYTES;
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
        .route(
            "/api/v1/guilds/{guild_id}/sounds",
            get(sound_routes::list_sounds)
                .post(sound_routes::create_sound)
                .layer(DefaultBodyLimit::max(MAXIMUM_SOUND_FILE_BYTES)),
        )
        .route(
            "/api/v1/guilds/{guild_id}/sounds/{sound_id}",
            patch(sound_routes::update_sound).delete(sound_routes::delete_sound),
        )
        .route(
            "/api/v1/guilds/{guild_id}/members/@me/join-sound",
            patch(sound_routes::set_join_sound),
        )
        .route("/api/v1/sounds/{id}", get(sound_routes::get_sound))
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
        .route(
            "/api/v1/attachments/{attachment_id}/preview",
            get(attachment_routes::download_preview),
        )
}

fn relationship_router() -> Router<AppState> {
    Router::new()
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
}

pub(crate) fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .merge(account_router())
        .route(
            "/api/v1/users/@me/guilds",
            get(guild_routes::list_my_guilds),
        )
        .merge(relationship_router())
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
        .layer(middleware::from_fn(copy_request_authority_into_host))
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

/// HTTP/2 carries the host in `:authority`, which lands in the URI and never in `Host`, yet the
/// origin check (CSRF, cross-site WebSocket hijacking) and the voice candidate read `Host`. A
/// request naming two different hosts is refused: which one counts would be ambiguous
/// (RFC 9113 §8.3.1).
async fn copy_request_authority_into_host(mut request: Request, next: Next) -> Response {
    if let Some(authority) = request.uri().authority() {
        let Ok(authority) = HeaderValue::from_str(authority.as_str()) else {
            return api_error::ApiError::validation("Invalid request authority.".into())
                .into_response();
        };
        match request.headers().get(header::HOST) {
            None => {
                request.headers_mut().insert(header::HOST, authority);
            }
            Some(host) if *host == authority => {}
            Some(_) => {
                return api_error::ApiError::validation(
                    "The Host header and the request authority differ.".into(),
                )
                .into_response();
            }
        }
    }
    next.run(request).await
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
mod tests {
    use axum::body::Body;
    use axum::extract::connect_info::MockConnectInfo;
    use mellow_protocol::{ApiErrorBody, ApiErrorCode, CurrentUser, Snowflake};
    use tower::ServiceExt as _;

    use super::test_support::*;
    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::tests::{PASSWORD, migrated_database};

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
    #[allow(clippy::too_many_lines)] // One scenario: upload, serve, choose, edit, delete.
    async fn sounds_are_uploaded_served_to_members_as_wav_and_chosen_as_join_sounds_over_http() {
        use mellow_protocol::{Guild, SoundboardSound};

        use crate::domain::sound_rules::MAXIMUM_SOUND_FILE_BYTES;
        use crate::domain::sound_rules::tests::canonical_wav;

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
        let stranger = registered_token(&state, "stranger").await;
        let upload_path = format!("/api/v1/guilds/{guild_id}/sounds?name=Horn&emoji=%F0%9F%93%AF");
        let (by_guest, _) = send(
            &state,
            picture_request("POST", &upload_path, &guest, canonical_wav(480)),
        )
        .await;
        let (too_large, _) = send(
            &state,
            picture_request(
                "POST",
                &upload_path,
                &owner,
                vec![0; MAXIMUM_SOUND_FILE_BYTES + 1],
            ),
        )
        .await;
        let (status, body) = send(
            &state,
            picture_request("POST", &upload_path, &owner, canonical_wav(480)),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "{}",
            String::from_utf8_lossy(&body)
        );
        let sound: SoundboardSound = json(&body);
        let sound_path = format!("/api/v1/sounds/{}", sound.id);
        let ((download_status, download), headers) = send_from(
            &state,
            TEST_PEER,
            authorized("GET", &sound_path, &guest, None),
        )
        .await;
        let (stranger_download, _) =
            send(&state, authorized("GET", &sound_path, &stranger, None)).await;
        let join_sound_path = format!("/api/v1/guilds/{guild_id}/members/@me/join-sound");
        let (chosen, _) = send(
            &state,
            authorized(
                "PATCH",
                &join_sound_path,
                &guest,
                Some(serde_json::json!({ "sound_id": sound.id.to_string() })),
            ),
        )
        .await;
        let (_, body) = send(
            &state,
            authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &guest, None),
        )
        .await;
        let guild: Guild = json(&body);
        let (renamed, body) = send(
            &state,
            authorized(
                "PATCH",
                &format!("/api/v1/guilds/{guild_id}/sounds/{}", sound.id),
                &owner,
                Some(serde_json::json!({ "name": "Big horn", "emoji": null })),
            ),
        )
        .await;
        let renamed_sound: SoundboardSound = json(&body);
        let (deleted, _) = send(
            &state,
            authorized(
                "DELETE",
                &format!("/api/v1/guilds/{guild_id}/sounds/{}", sound.id),
                &owner,
                None,
            ),
        )
        .await;
        let (_, body) = send(
            &state,
            authorized(
                "GET",
                &format!("/api/v1/guilds/{guild_id}/sounds"),
                &guest,
                None,
            ),
        )
        .await;
        let remaining: Vec<SoundboardSound> = json(&body);
        database.close().await;

        assert_eq!(by_guest, StatusCode::FORBIDDEN);
        assert_eq!(too_large, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            (sound.name.as_str(), sound.emoji.as_deref()),
            ("Horn", Some("📯"))
        );
        assert_eq!(download_status, StatusCode::OK);
        assert_eq!(download.as_ref(), canonical_wav(480).as_slice());
        assert_eq!(headers[header::CONTENT_TYPE], "audio/wav");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(stranger_download, StatusCode::NOT_FOUND);
        assert_eq!(chosen, StatusCode::NO_CONTENT);
        assert_eq!(guild.my_join_sound_id, Some(sound.id));
        assert_eq!(guild.sounds, std::slice::from_ref(&sound));
        assert_eq!(renamed, StatusCode::OK);
        assert_eq!(
            (renamed_sound.name.as_str(), renamed_sound.emoji),
            ("Big horn", None)
        );
        assert_eq!(deleted, StatusCode::NO_CONTENT);
        assert_eq!(remaining, []);
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
                crate::domain::profile_images::ProfileImageSlot::Avatar,
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
        let ((_, still), still_headers) = send_from(
            &state,
            TEST_PEER,
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
        let still_type = crate::domain::message_rules::sniff_content_type(&still);
        assert!(["image/avif", "image/webp"].contains(&still_type));
        assert_eq!(
            still_headers.get(header::CONTENT_TYPE).unwrap(),
            still_type,
            "served as the type it is stored in"
        );
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
    async fn pictures_whose_compression_was_lost_in_a_restart_are_compressed_after_it() {
        use crate::domain::image_processing::tests::png;

        const UPLOADS_BEFORE_THE_RESTART: usize = 100;
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let picture = png(64, 40);
        let mut uploaded = Vec::new();
        let (_, body) = send(&state, authorized("GET", "/api/v1/users/@me", &owner, None)).await;
        let owner_id = json::<CurrentUser>(&body).id;
        // Through the domain: the HTTP route would rate-limit this many uploads. No worker runs:
        // past the queue's capacity jobs are dropped, and the restart loses the queued ones too.
        for _ in 0..UPLOADS_BEFORE_THE_RESTART {
            let slot = state
                .attachments
                .start_upload(owner_id, general, "photo.png")
                .await
                .unwrap();
            let attachment = state
                .attachments
                .finish_upload(
                    slot,
                    picture.clone(),
                    crate::domain::clock::now_unix_milliseconds(),
                )
                .await
                .unwrap();
            uploaded.push(attachment.id);
        }

        state
            .attachments
            .restarted()
            .optimize_every_attachment_awaiting()
            .await;
        let mut stored = Vec::new();
        for id in &uploaded {
            stored.push(
                crate::storage::attachments::find_attachment(&database, *id)
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
        let awaiting =
            crate::storage::attachments::newest_attachments_awaiting_optimization(&database, 1_000)
                .await
                .unwrap();
        database.close().await;

        assert!(awaiting.is_empty(), "every attempt is recorded");
        let original_size = u64::try_from(picture.len()).unwrap();
        assert!(
            stored
                .iter()
                .all(|attachment| attachment.size_bytes < original_size),
            "every upload was compressed"
        );
    }

    #[tokio::test]
    async fn a_picture_whose_compression_died_with_the_process_is_not_tried_again() {
        use crate::domain::image_processing::tests::png;

        /// Queries before giving up on the attempt being recorded; each waits on the database.
        const MAXIMUM_CHECKS: usize = 10_000;
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let (_, body) = send(&state, authorized("GET", "/api/v1/users/@me", &owner, None)).await;
        let owner_id = json::<CurrentUser>(&body).id;
        let picture = png(64, 40);
        let slot = state
            .attachments
            .start_upload(owner_id, general, "photo.png")
            .await
            .unwrap();
        let attachment = state
            .attachments
            .finish_upload(
                slot,
                picture.clone(),
                crate::domain::clock::now_unix_milliseconds(),
            )
            .await
            .unwrap();

        // Every slot taken: the worker stops inside the job, which is where the process dies.
        let slots = [
            state.attachments.image_processing_permit().unwrap(),
            state.attachments.image_processing_permit().unwrap(),
        ];
        let is_attempt_recorded = async {
            for _ in 0..MAXIMUM_CHECKS {
                let awaiting =
                    crate::storage::attachments::newest_attachments_awaiting_optimization(
                        &database, 1,
                    )
                    .await
                    .unwrap();
                if awaiting.is_empty() {
                    return;
                }
                tokio::task::yield_now().await;
            }
            panic!("the attempt is recorded before the job runs");
        };
        let worker = state.attachments.optimize_every_attachment_awaiting();
        tokio::select! {
            () = worker => panic!("the job cannot end without a slot"),
            () = is_attempt_recorded => {}
        }
        // The worker is gone mid-job, as in a crash; the restarted instance must not retry it.
        drop(slots);
        state
            .attachments
            .restarted()
            .optimize_every_attachment_awaiting()
            .await;
        let stored = crate::storage::attachments::find_attachment(&database, attachment.id)
            .await
            .unwrap()
            .unwrap();
        database.close().await;

        assert_eq!(
            stored.size_bytes,
            u64::try_from(picture.len()).unwrap(),
            "kept as uploaded"
        );
    }

    async fn me_guild_id(state: &AppState, token: &str) -> Snowflake {
        let (_, body) = send(
            state,
            authorized("GET", "/api/v1/users/@me/guilds", token, None),
        )
        .await;
        json::<Vec<mellow_protocol::GuildSummary>>(&body)[0].id
    }

    #[tokio::test]
    async fn stalled_picture_and_sound_uploads_are_answered_busy_at_their_deadline() {
        use futures_util::StreamExt as _;

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let owner = registered_token(&state, "owner").await;
        let (_, body) = send(
            &state,
            authorized(
                "POST",
                "/api/v1/guilds",
                &owner,
                Some(serde_json::json!({"name": "Club"})),
            ),
        )
        .await;
        let guild_id = json::<mellow_protocol::Guild>(&body).id;
        let uploads = [
            ("PUT", "/api/v1/users/@me/avatar".to_owned()),
            (
                "POST",
                format!("/api/v1/guilds/{guild_id}/emojis?name=party"),
            ),
            (
                "POST",
                format!("/api/v1/guilds/{guild_id}/sounds?name=party"),
            ),
        ];
        for (method, path) in uploads {
            // One byte, then nothing: the client stalls mid-body.
            let (started, body_started) = tokio::sync::oneshot::channel::<()>();
            let trickle = futures_util::stream::once(async move {
                let _ = started.send(());
                Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"x"))
            })
            .chain(futures_util::stream::pending());
            let request = Request::builder()
                .method(method)
                .uri(&path)
                .header(header::AUTHORIZATION, format!("Bearer {owner}"))
                .body(Body::from_stream(trickle))
                .unwrap();
            let upload = tokio::spawn({
                let state = state.clone();
                async move { send(&state, request).await }
            });
            body_started.await.unwrap();
            // Paused only while the handler waits on the body, so database work runs in real time.
            tokio::time::pause();
            tokio::time::advance(std::time::Duration::from_mins(10)).await;
            let (status, body) = tokio::time::timeout(std::time::Duration::from_secs(1), upload)
                .await
                .expect("the handler still waits for the stalled body")
                .unwrap();
            tokio::time::resume();
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
            assert_eq!(
                json::<ApiErrorBody>(&body).code,
                ApiErrorCode::ServerBusy,
                "{path}"
            );
        }
        database.close().await;
    }

    /// A picture as the lossless pipeline before ADR-0033 stored avatars, banners and icons.
    fn lossless_webp(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
            .encode(
                crate::domain::image_processing::tests::photo(width, height).as_raw(),
                width,
                height,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        bytes
    }

    /// Writes `bytes` as a file stored before ADR-0033, with its row in `table` (`profile_images`
    /// or `guild_images`, whose columns `owner_columns` and `owner_values` name), and answers its id.
    async fn store_legacy_picture(
        state: &AppState,
        bytes: Vec<u8>,
        table: &str,
        owner_columns: &str,
        owner_values: &str,
    ) -> Snowflake {
        let size_bytes = bytes.len();
        let id = state
            .profile_images
            .store_file(bytes, crate::domain::clock::now_unix_milliseconds())
            .await
            .unwrap();
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {table} (id, {owner_columns}, size_bytes, created_at_unix_milliseconds, \
             compression_version) VALUES ({id}, {owner_values}, {size_bytes}, 0, 0)"
        )))
        .execute(state.database.writer())
        .await
        .unwrap();
        id
    }

    #[tokio::test]
    async fn an_icon_is_recompressed_while_every_read_slot_serves_a_download() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (_owner, general) = owner_and_general_channel(&state).await;
        let guild_id: i64 = sqlx::query_scalar("SELECT guild_id FROM channels WHERE id = ?")
            .bind(i64::try_from(general.raw()).unwrap())
            .fetch_one(database.readers())
            .await
            .unwrap();
        let icon = store_legacy_picture(
            &state,
            lossless_webp(256, 256),
            "guild_images",
            "guild_id",
            &guild_id.to_string(),
        )
        .await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "UPDATE guilds SET icon_id = {icon} WHERE id = {guild_id};"
        )))
        .execute(database.writer())
        .await
        .unwrap();
        let mut downloads = Vec::new();
        for _ in 0..crate::domain::profile_images::CONCURRENT_READS {
            downloads.push(state.profile_images.read_file(icon).await.unwrap().1);
        }

        let claimed = crate::storage::image_recompression::claim_next_icon(&database)
            .await
            .unwrap()
            .unwrap();
        let recompressed = state
            .guilds
            .recompress_icon(&state.profile_images, claimed)
            .await;
        drop(downloads);
        let new_icon: i64 = sqlx::query_scalar("SELECT icon_id FROM guilds WHERE id = ?")
            .bind(guild_id)
            .fetch_one(database.readers())
            .await
            .unwrap();
        database.close().await;

        recompressed.unwrap();
        assert_ne!(new_icon, i64::try_from(icon.raw()).unwrap());
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // One scenario: an instance's history, a crash, a restart.
    async fn pictures_stored_before_adr_0033_are_recompressed_one_by_one_and_resume_after_a_restart()
     {
        use crate::domain::image_processing::tests::{encoded, photo};
        use crate::domain::image_recompression::ImageRecompression;
        use crate::storage::image_recompression as recompression;

        /// More than the pictures stored: the loop must end on its own well before.
        const MAXIMUM_STEPS: usize = 20;
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let (_, body) = send(&state, authorized("GET", "/api/v1/users/@me", &owner, None)).await;
        let owner_id = json::<CurrentUser>(&body).id;
        let guild_id: i64 = sqlx::query_scalar("SELECT guild_id FROM channels WHERE id = ?")
            .bind(i64::try_from(general.raw()).unwrap())
            .fetch_one(database.readers())
            .await
            .unwrap();
        let guild_id = Snowflake::from_raw(u64::try_from(guild_id).unwrap());

        // What an instance holds after a year of the lossless pipeline: a 512 px avatar, a banner
        // wider than today's 960 px, a 256 px guild icon, a 96 px channel icon, an emoji and two
        // pictures sent in a message (the newer one is the job that kills the process).
        let avatar_bytes = lossless_webp(512, 512);
        let banner_bytes = lossless_webp(1_200, 480);
        let avatar = store_legacy_picture(
            &state,
            avatar_bytes.clone(),
            "profile_images",
            "user_id",
            &owner_id.to_string(),
        )
        .await;
        let banner = store_legacy_picture(
            &state,
            banner_bytes.clone(),
            "profile_images",
            "user_id",
            &owner_id.to_string(),
        )
        .await;
        let guild_icon = store_legacy_picture(
            &state,
            lossless_webp(256, 256),
            "guild_images",
            "guild_id",
            &guild_id.to_string(),
        )
        .await;
        let channel_icon = store_legacy_picture(
            &state,
            lossless_webp(96, 96),
            "guild_images",
            "guild_id, channel_id",
            &format!("{guild_id}, {general}"),
        )
        .await;
        let emoji = store_legacy_picture(
            &state,
            lossless_webp(128, 128),
            "guild_images",
            "guild_id",
            &guild_id.to_string(),
        )
        .await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "UPDATE users SET avatar_id = {avatar}, banner_id = {banner} WHERE id = {owner_id}; \
             UPDATE guilds SET icon_id = {guild_icon} WHERE id = {guild_id}; \
             UPDATE channels SET icon_id = {channel_icon} WHERE id = {general}; \
             INSERT INTO guild_emojis (id, guild_id, name) VALUES ({emoji}, {guild_id}, 'wave');"
        )))
        .execute(database.writer())
        .await
        .unwrap();
        let mut attachment_ids = Vec::new();
        for (name, bytes) in [
            (
                "wide.png",
                encoded(&photo(1_200, 150), image::ImageFormat::Png, None),
            ),
            (
                "photo.jpg",
                encoded(&photo(600, 400), image::ImageFormat::Jpeg, None),
            ),
        ] {
            let slot = state
                .attachments
                .start_upload(owner_id, general, name)
                .await
                .unwrap();
            let uploaded = state
                .attachments
                .finish_upload(slot, bytes, crate::domain::clock::now_unix_milliseconds())
                .await
                .unwrap();
            attachment_ids.push(uploaded.id);
        }
        let [wide_picture, killing_picture] = attachment_ids[..] else {
            unreachable!("two uploads")
        };
        send(
            &state,
            authorized(
                "POST",
                &format!("/api/v1/channels/{general}/messages"),
                &owner,
                Some(serde_json::json!({
                    "content": "",
                    "attachment_ids": [wide_picture.to_string(), killing_picture.to_string()],
                })),
            ),
        )
        .await;
        sqlx::raw_sql(
            "UPDATE attachments SET compression_version = 0, \
             optimization_attempted_at_unix_milliseconds = created_at_unix_milliseconds",
        )
        .execute(database.writer())
        .await
        .unwrap();
        let stored_before = |id| {
            let database = database.clone();
            async move {
                crate::storage::attachments::find_attachment(&database, id)
                    .await
                    .unwrap()
                    .unwrap()
            }
        };
        let wide_before = stored_before(wide_picture).await;
        let killing_before = stored_before(killing_picture).await;
        let (session, mut events) = state
            .events
            .register_session(owner_id, [7; 32], 100)
            .unwrap();
        state.events.activate_session(session, &[guild_id]);
        let waiting_before = recompression::pictures_to_recompress(&database)
            .await
            .unwrap();

        let recompression_for = |attachments| {
            ImageRecompression::new(
                database.clone(),
                attachments,
                state.profile_images.clone(),
                state.guilds.clone(),
            )
        };
        let unrequested = recompression_for(state.attachments.clone())
            .recompress_next()
            .await;
        recompression::request_recompression(&database, 0)
            .await
            .unwrap();
        // The process dies on the newest attachment, its claim recorded; then it restarts, does
        // one picture and is stopped again.
        recompression::claim_next_attachment(&database)
            .await
            .unwrap();
        let did_one = recompression_for(Arc::new(state.attachments.restarted()))
            .recompress_next()
            .await;
        let resumed = recompression_for(Arc::new(state.attachments.restarted()));
        let mut steps_after_restart = 0;
        while resumed.recompress_next().await {
            steps_after_restart += 1;
            assert!(
                steps_after_restart < MAXIMUM_STEPS,
                "the recompression ends"
            );
        }
        let is_still_requested = recompression::is_recompression_requested(&database)
            .await
            .unwrap();
        let waiting_after = recompression::pictures_to_recompress(&database)
            .await
            .unwrap();
        let profile = crate::domain::profile_images::profile_images_of_user(&database, owner_id)
            .await
            .unwrap();
        let (new_avatar, new_banner) = (profile.avatar.unwrap().id, profile.banner.unwrap().id);
        let (new_avatar_bytes, _) = state.profile_images.read(new_avatar, false).await.unwrap();
        let (new_banner_bytes, _) = state.profile_images.read(new_banner, false).await.unwrap();
        let old_avatar_read = state.profile_images.read(avatar, false).await;
        let new_guild_icon = crate::storage::guilds::find_guild_summary(&database, guild_id)
            .await
            .unwrap()
            .unwrap()
            .icon
            .unwrap()
            .id;
        let new_channel_icon =
            crate::storage::guild_channels::find_guild_channel(&database, general)
                .await
                .unwrap()
                .unwrap()
                .icon
                .unwrap()
                .id;
        let emojis = crate::storage::guild_emojis::list_guild_emojis(&database, guild_id)
            .await
            .unwrap();
        let wide_after = stored_before(wide_picture).await;
        let killing_after = stored_before(killing_picture).await;
        let ((download_status, downloaded), _) = send_from(
            &state,
            TEST_PEER,
            authorized(
                "GET",
                &format!("/api/v1/attachments/{wide_picture}"),
                &owner,
                None,
            ),
        )
        .await;
        let stored_files: std::collections::HashSet<Snowflake> = state
            .attachments
            .stored_file_ids()
            .await
            .into_iter()
            .collect();
        let files_in_use = crate::domain::attachments::file_ids_in_use(&database)
            .await
            .unwrap();
        let mut announced = Vec::new();
        while let Ok(event) = events.try_recv() {
            announced.push(event.payload_json.to_string());
        }
        database.close().await;

        assert!(!unrequested, "nothing happens before an administrator asks");
        assert_eq!(waiting_before.profile_images.count, 2);
        assert_eq!(waiting_before.icons.count, 2, "the emoji is left out");
        assert_eq!(waiting_before.attachments.count, 2);
        assert!(did_one);
        assert_eq!(
            steps_after_restart, 4,
            "the avatar, both icons and the wide picture"
        );
        assert!(!is_still_requested, "the request ends with the work");
        assert_eq!(waiting_after.count(), 0);

        assert!(![avatar, banner].contains(&new_avatar) && ![avatar, banner].contains(&new_banner));
        assert!(new_avatar_bytes.len() < avatar_bytes.len());
        assert!(new_banner_bytes.len() < banner_bytes.len());
        assert!(old_avatar_read.is_err(), "the old version is gone");
        assert_ne!(new_guild_icon, guild_icon);
        assert_ne!(new_channel_icon, channel_icon);
        assert_eq!(
            emojis[0].id, emoji,
            "an emoji keeps the id messages name it by"
        );

        assert_ne!(
            wide_after.file_id, wide_before.file_id,
            "written beside the old one"
        );
        assert!(wide_after.size_bytes < wide_before.size_bytes);
        assert!(["image/avif", "image/webp"].contains(&wide_after.content_type.as_str()));
        assert!(
            wide_after.preview.is_some(),
            "an old picture gets its preview"
        );
        assert_eq!(download_status, StatusCode::OK);
        assert_eq!(downloaded.len() as u64, wide_after.size_bytes);
        assert_eq!(
            killing_after, killing_before,
            "the picture that killed it is kept"
        );

        assert_eq!(
            stored_files, files_in_use,
            "no old file is left, none is missing"
        );
        for id in [new_avatar, new_banner, new_guild_icon, new_channel_icon] {
            assert!(
                announced
                    .iter()
                    .any(|event| event.contains(&id.to_string())),
                "{id} is announced"
            );
        }
    }
}
