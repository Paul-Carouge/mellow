//! Fixtures shared by the HTTP route tests.

use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::HeaderMap;
use mellow_protocol::{CurrentUser, SessionCreated, Snowflake};
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;

use super::*;
use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::PASSWORD;

pub(crate) const TEST_PEER: &str = "192.0.2.10:40000";

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
        gifs: None,
        gif_content_security_policy: None,
    }
}

pub(crate) async fn send(
    state: &AppState,
    request: Request<Body>,
) -> (StatusCode, axum::body::Bytes) {
    send_from(state, TEST_PEER, request).await.0
}

pub(crate) async fn send_from(
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

pub(crate) fn json<T: DeserializeOwned>(body: &[u8]) -> T {
    serde_json::from_slice(body).unwrap()
}

pub(crate) fn post_json(path: &str, body: &serde_json::Value) -> Request<Body> {
    Request::post(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

pub(crate) fn registration(username: &str) -> serde_json::Value {
    serde_json::json!({
        "username": username,
        "email": format!("{username}@example.org"),
        "password": PASSWORD,
        "device_name": "Test device",
    })
}

pub(crate) fn wrong_login(login: &str) -> Request<Body> {
    post_json(
        "/api/v1/auth/login",
        &serde_json::json!({"login": login, "password": "wrong password!", "device_name": "d"}),
    )
}

pub(crate) async fn registered_token(state: &AppState, username: &str) -> String {
    let (_, body) = send(
        state,
        post_json("/api/v1/auth/register", &registration(username)),
    )
    .await;
    json::<SessionCreated>(&body).token
}

pub(crate) fn authorized(
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

/// Owner token, guest token, the owner's guild with the guest as a member, and the guest's id.
pub(crate) async fn guild_with_guest(state: &AppState) -> (String, String, Snowflake, Snowflake) {
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

pub(crate) fn picture_request(
    method: &str,
    path: &str,
    token: &str,
    bytes: Vec<u8>,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes))
        .unwrap()
}

pub(crate) async fn owner_and_general_channel(state: &AppState) -> (String, Snowflake) {
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
