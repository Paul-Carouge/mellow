//! REST API (axum). Handlers validate input and map domain errors to HTTP; no SQL or business
//! rules live here (ARCHITECTURE.md §4.1).

mod account_routes;
pub(crate) mod api_error;
pub(crate) mod attachment_routes;
pub(crate) mod authentication;
pub(crate) mod client_address;
mod emoji_routes;
mod gif_routes;
mod guild_routes;
mod message_routes;
mod profile_image_routes;
mod profile_routes;
pub(crate) mod rate_limit;
mod relationship_routes;
mod role_icon_routes;
mod sound_routes;
#[cfg(test)]
pub(crate) mod test_support;
mod two_factor_routes;
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
use crate::domain::guild_rules::{MAXIMUM_EMOJI_UPLOAD_BYTES, MAXIMUM_ROLE_ICON_UPLOAD_BYTES};
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
    /// Two-factor sign-in (ADR-0049), with the sign-ins waiting for their code.
    pub(crate) two_factor: Arc<crate::domain::two_factor::TwoFactor>,
    pub(crate) guilds: Arc<Guilds>,
    pub(crate) messages: Arc<Messages>,
    pub(crate) relationships: Arc<crate::domain::relationships::Relationships>,
    pub(crate) attachments: Arc<crate::domain::attachments::Attachments>,
    pub(crate) profile_images: Arc<crate::domain::profile_images::ProfileImages>,
    /// "About me" texts and profile links (ADR-0050).
    pub(crate) profiles: Arc<crate::domain::profiles::Profiles>,
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
    /// GIFs from Klipy (ADR-0042), when the owner set a key.
    pub(crate) gifs: Option<Arc<crate::domain::gifs::Gifs>>,
    /// The web page's policy with Klipy's hosts and this key's API path, when GIFs are on.
    pub(crate) gif_content_security_policy: Option<HeaderValue>,
}

impl AppState {
    /// Turns GIFs on (ADR-0042): `serve` calls it when `mellow.toml` holds a Klipy key.
    ///
    /// # Panics
    /// When the key holds characters a header cannot: never for a key `GifsConfig` validated.
    pub(crate) fn with_gifs(mut self, gifs: crate::domain::gifs::Gifs) -> Self {
        // Invariant: `GifsConfig::validate` allows only letters, digits, `-` and `_`.
        #[allow(clippy::expect_used)]
        let policy = HeaderValue::from_str(&web_client::content_security_policy(Some(
            gifs.klipy_app_key(),
        )))
        .expect("a validated Klipy key fits in a header");
        self.gif_content_security_policy = Some(policy);
        self.gifs = Some(Arc::new(gifs));
        self
    }

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
        let profile_images = Arc::new(crate::domain::profile_images::ProfileImages::new(
            database.clone(),
            ids.clone(),
            attachment_files,
            attachments.clone(),
            events.clone(),
        ));
        Self {
            profiles: Arc::new(crate::domain::profiles::Profiles::new(
                database.clone(),
                profile_images.clone(),
            )),
            profile_images,
            attachments,
            maximum_attachment_bytes: config.maximum_attachment_bytes,
            two_factor: Arc::new(crate::domain::two_factor::TwoFactor::new(database.clone())),
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
            gifs: None,
            gif_content_security_policy: None,
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
        .merge(two_factor_routes::router())
        .route(
            "/api/v1/users/@me",
            get(account_routes::current_user).patch(account_routes::update_current_user),
        )
        .route(
            "/api/v1/users/name-availability",
            get(account_routes::name_availability),
        )
        .route(
            "/api/v1/users/@me/password",
            post(account_routes::change_password),
        )
        .route(
            "/api/v1/users/@me/presence",
            put(account_routes::update_presence),
        )
        .route("/api/v1/gifs/settings", get(gif_routes::settings))
        .route(
            "/api/v1/users/@me/gif-favourites",
            get(gif_routes::favourites),
        )
        .route(
            "/api/v1/users/@me/gif-favourites/{slug}",
            put(gif_routes::add_favourite).delete(gif_routes::remove_favourite),
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
            "/api/v1/channels/{channel_id}/messages/{message_id}/poll/answers/{answer_id}/votes/@me",
            put(message_routes::add_poll_vote).delete(message_routes::remove_poll_vote),
        )
        .route(
            "/api/v1/channels/{channel_id}/messages/{message_id}/checklist/items",
            post(message_routes::add_checklist_item),
        )
        .route(
            "/api/v1/channels/{channel_id}/messages/{message_id}/checklist/items/{item_id}",
            patch(message_routes::rename_checklist_item)
                .delete(message_routes::remove_checklist_item),
        )
        .route(
            "/api/v1/channels/{channel_id}/messages/{message_id}/checklist/items/{item_id}/check",
            put(message_routes::tick_checklist_item).delete(message_routes::untick_checklist_item),
        )
        .route(
            "/api/v1/channels/{channel_id}/messages/{message_id}/progress-bar",
            patch(message_routes::update_progress_bar),
        )
        .route(
            "/api/v1/channels/{channel_id}/messages/{message_id}/poll/answers/{answer_id}/voters",
            get(message_routes::list_poll_voters),
        )
        .route(
            "/api/v1/channels/{channel_id}/messages/{message_id}/poll/end",
            post(message_routes::end_poll),
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
        .route(
            "/api/v1/channels/{channel_id}/messages/{message_id}/link-previews/{preview_id}/image",
            get(attachment_routes::download_link_preview_image),
        )
}

/// Profiles' text and links (ADR-0050), and role icons.
fn profile_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/users/@me/profile",
            get(profile_routes::get_my_profile).put(profile_routes::put_my_profile),
        )
        .route(
            "/api/v1/users/{user_id}/profile",
            get(profile_routes::get_user_profile),
        )
        .route(
            "/api/v1/link-icons/{id}",
            get(profile_routes::get_link_icon),
        )
        .route(
            "/api/v1/guilds/{guild_id}/roles/{role_id}/icon",
            put(role_icon_routes::put_role_icon)
                .delete(role_icon_routes::delete_role_icon)
                .layer(DefaultBodyLimit::max(MAXIMUM_ROLE_ICON_UPLOAD_BYTES)),
        )
        .route(
            "/api/v1/guilds/{guild_id}/roles/{role_id}/icon/emoji",
            put(role_icon_routes::put_role_emoji),
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
        .merge(profile_router())
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
    use mellow_protocol::{ApiErrorBody, ApiErrorCode};
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
}
