//! Profiles' text and links over HTTP, and their sites' icons fetched from local sites standing
//! in for real hosts (ADR-0050). Nothing waits on a clock: the queued fetches run when the test
//! asks for them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use mellow_protocol::{ApiErrorBody, ApiErrorCode, Snowflake, UserProfile};

use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::migrated_database;
use crate::domain::clock::now_unix_milliseconds;
use crate::domain::image_processing::tests::png;
use crate::domain::link_previews::fetcher::LinkFetcher;
use crate::domain::profiles::fetch_queued_link_icons;
use crate::domain::profiles::site_icons_tests::ico_file;
use crate::http::test_support::*;
use crate::http::*;

fn put_profile(token: &str, body: serde_json::Value) -> axum::http::Request<axum::body::Body> {
    authorized("PUT", "/api/v1/users/@me/profile", token, Some(body))
}

async fn profile_of(
    state: &AppState,
    token: &str,
    path: &str,
) -> (StatusCode, Option<UserProfile>) {
    let (status, body) = send(state, authorized("GET", path, token, None)).await;
    (status, (status == StatusCode::OK).then(|| json(&body)))
}

async fn user_id_of(state: &AppState, token: &str) -> Snowflake {
    let (_, own) = profile_of(state, token, "/api/v1/users/@me/profile").await;
    own.unwrap().user_id
}

#[tokio::test]
async fn profiles_are_checked_saved_and_shown_only_to_people_who_share_something() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, _, guest_id) = guild_with_guest(&state).await;
    let stranger = registered_token(&state, "stranger").await;

    let (saved_status, body) = send(
        &state,
        put_profile(
            &guest,
            serde_json::json!({
                "about_me": "  Writes **notes**\r\nand tests  ",
                "links": [
                    { "url": "https://Example.org/ada", "label": " Blog " },
                    { "url": "https://example.com/" }
                ]
            }),
        ),
    )
    .await;
    let saved: UserProfile = json(&body);
    let guest_path = format!("/api/v1/users/{guest_id}/profile");
    let (by_owner_status, by_owner) = profile_of(&state, &owner, &guest_path).await;
    let (by_stranger, _) = profile_of(&state, &stranger, &guest_path).await;
    let (unknown, _) = profile_of(&state, &owner, "/api/v1/users/123456789/profile").await;
    let (own_status, own) = profile_of(&state, &guest, "/api/v1/users/@me/profile").await;
    let (empty_status, empty) = profile_of(&state, &owner, "/api/v1/users/@me/profile").await;
    let mut refusals = Vec::new();
    for body in [
        serde_json::json!({ "about_me": "a".repeat(191) }),
        serde_json::json!({ "links": [{ "url": "http://example.org/" }] }),
        serde_json::json!({ "links": [{ "url": "https://192.168.1.18/" }] }),
        serde_json::json!({ "links": vec![serde_json::json!({ "url": "https://example.org/" }); 6] }),
        serde_json::json!({ "links": [{ "url": "https://example.org/", "label": "x".repeat(41) }] }),
        serde_json::json!({ "about_me": "\u{0007}" }),
    ] {
        let (status, body) = send(&state, put_profile(&guest, body)).await;
        refusals.push((status, json::<ApiErrorBody>(&body).code));
    }
    let (_, after_refusals) = profile_of(&state, &guest, "/api/v1/users/@me/profile").await;
    database.close().await;

    assert_eq!(saved_status, StatusCode::OK);
    assert_eq!(saved.user_id, guest_id);
    assert_eq!(saved.about_me, "Writes **notes**\nand tests");
    assert_eq!(saved.links.len(), 2);
    assert_eq!(saved.links[0].url, "https://example.org/ada");
    assert_eq!(saved.links[0].label.as_deref(), Some("Blog"));
    assert_eq!(saved.links[1].label, None);
    assert!(
        saved.links.iter().all(|link| link.icon_id.is_none()),
        "icons are off"
    );
    assert_eq!(by_owner_status, StatusCode::OK);
    assert_eq!(by_owner.as_ref(), Some(&saved));
    assert_eq!(by_stranger, StatusCode::NOT_FOUND, "nothing shared");
    assert_eq!(unknown, StatusCode::NOT_FOUND);
    assert_eq!((own_status, own.as_ref()), (StatusCode::OK, Some(&saved)));
    assert_eq!(empty_status, StatusCode::OK);
    let empty = empty.unwrap();
    assert!(empty.about_me.is_empty() && empty.links.is_empty());
    assert!(
        refusals
            .iter()
            .all(|(status, code)| *status == StatusCode::BAD_REQUEST
                && *code == ApiErrorCode::ValidationFailed),
        "{refusals:?}"
    );
    assert_eq!(
        after_refusals.as_ref(),
        Some(&saved),
        "refusals change nothing"
    );
}

#[tokio::test]
async fn blocking_someone_never_opens_their_profile() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let target = registered_token(&state, "target").await;
    let stranger = registered_token(&state, "stranger").await;
    let target_id = user_id_of(&state, &target).await;
    send(
        &state,
        put_profile(&target, serde_json::json!({ "about_me": "Private" })),
    )
    .await;

    let (blocked_status, _) = send(
        &state,
        authorized(
            "PUT",
            &format!("/api/v1/users/@me/relationships/{target_id}"),
            &stranger,
            Some(serde_json::json!({ "kind": "blocked" })),
        ),
    )
    .await;
    let (by_blocker, _) = profile_of(
        &state,
        &stranger,
        &format!("/api/v1/users/{target_id}/profile"),
    )
    .await;
    database.close().await;

    assert_eq!(blocked_status, StatusCode::OK);
    assert_eq!(by_blocker, StatusCode::NOT_FOUND);
}

/// What the stand-in sites served.
#[derive(Default)]
struct SiteRequests {
    pictured_icon: AtomicUsize,
    other_favicon: AtomicUsize,
    /// `flaky.example` answers 503 while set.
    is_flaky_site_down: AtomicBool,
}

/// One server standing in for every host (`LinkFetcher::stand_in_for_every_host_for_tests`):
/// `pictured.example`'s home page names a PNG icon, `ico.example` has only an ICO favicon holding
/// a PNG, `bare.example` has nothing, `flaky.example` has a PNG favicon unless it is down.
async fn stand_in_sites() -> (String, Arc<SiteRequests>) {
    use axum::routing::get;

    let requests = Arc::new(SiteRequests::default());
    let pictured = requests.clone();
    let favicon = requests.clone();
    let flaky = requests.clone();
    let site = axum::Router::new()
        .route(
            "/pictured.example/",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<html><head><link rel="icon" href="/icon.png"></head></html>"#,
                )
            }),
        )
        .route(
            "/pictured.example/icon.png",
            get(move || {
                pictured.pictured_icon.fetch_add(1, Ordering::SeqCst);
                async { ([(header::CONTENT_TYPE, "image/png")], png(48, 48)) }
            }),
        )
        .route(
            "/ico.example/favicon.ico",
            get(move || {
                favicon.other_favicon.fetch_add(1, Ordering::SeqCst);
                async {
                    (
                        [(header::CONTENT_TYPE, "image/x-icon")],
                        ico_file(&[(32, &png(32, 32))]),
                    )
                }
            }),
        )
        .route(
            "/flaky.example/favicon.ico",
            get(move || {
                let is_down = flaky.is_flaky_site_down.load(Ordering::SeqCst);
                async move {
                    if is_down {
                        StatusCode::SERVICE_UNAVAILABLE.into_response()
                    } else {
                        ([(header::CONTENT_TYPE, "image/png")], png(32, 32)).into_response()
                    }
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, site).await });
    (format!("http://{address}/"), requests)
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One scenario: save, fetch, serve, reuse, drop.
async fn site_icons_are_fetched_once_per_host_served_for_good_and_dropped_with_their_last_link() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, _, guest_id) = guild_with_guest(&state).await;
    let (site, requests) = stand_in_sites().await;
    let fetcher = LinkFetcher::stand_in_for_every_host_for_tests(&site);
    state.profiles.enable_link_icons();
    let links = serde_json::json!({
        "links": [
            { "url": "https://pictured.example/a" },
            { "url": "https://pictured.example/b" },
            { "url": "https://ico.example/" },
            { "url": "https://bare.example/" }
        ]
    });

    send(&state, put_profile(&guest, links.clone())).await;
    fetch_queued_link_icons(&state.profiles, &fetcher).await;
    let guest_path = format!("/api/v1/users/{guest_id}/profile");
    let (_, after_fetch) = profile_of(&state, &owner, &guest_path).await;
    // Fresh icons are not fetched again, however often the profile is read or saved.
    send(&state, put_profile(&guest, links)).await;
    fetch_queued_link_icons(&state.profiles, &fetcher).await;
    let after_fetch = after_fetch.unwrap();
    let pictured_icon = after_fetch.links[0].icon_id.unwrap();
    let ico_icon = after_fetch.links[2].icon_id.unwrap();
    let ((served_status, served), headers) = send_from(
        &state,
        TEST_PEER,
        authorized(
            "GET",
            &format!("/api/v1/link-icons/{pictured_icon}"),
            &owner,
            None,
        ),
    )
    .await;
    // The guest keeps only the ICO site: the other sites' icons go with their last link.
    send(
        &state,
        put_profile(
            &guest,
            serde_json::json!({ "links": [{ "url": "https://ico.example/" }] }),
        ),
    )
    .await;
    let (dropped, _) = send(
        &state,
        authorized(
            "GET",
            &format!("/api/v1/link-icons/{pictured_icon}"),
            &owner,
            None,
        ),
    )
    .await;
    let (kept, _) = send(
        &state,
        authorized(
            "GET",
            &format!("/api/v1/link-icons/{ico_icon}"),
            &owner,
            None,
        ),
    )
    .await;
    let remaining_files = crate::storage::user_profiles::all_link_icon_file_ids(&state.database)
        .await
        .unwrap();
    database.close().await;

    assert_eq!(
        after_fetch.links[1].icon_id,
        Some(pictured_icon),
        "one icon per host"
    );
    assert_eq!(after_fetch.links[3].icon_id, None, "a site without an icon");
    assert_ne!(pictured_icon, ico_icon);
    assert_eq!(requests.pictured_icon.load(Ordering::SeqCst), 1);
    assert_eq!(requests.other_favicon.load(Ordering::SeqCst), 1);
    assert_eq!(served_status, StatusCode::OK);
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "private, max-age=31536000, immutable"
    );
    let content_type = headers[header::CONTENT_TYPE].to_str().unwrap();
    assert!(
        content_type == "image/webp" || content_type == "image/avif",
        "{content_type}"
    );
    assert!(served.len() < 64 * 1024);
    assert_eq!(dropped, StatusCode::NOT_FOUND);
    assert_eq!(kept, StatusCode::OK);
    assert_eq!(remaining_files, [ico_icon]);
}

#[tokio::test]
async fn without_link_previews_no_site_is_ever_asked_for_its_icon() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let guest = registered_token(&state, "guest").await;
    let (site, requests) = stand_in_sites().await;
    let fetcher = LinkFetcher::stand_in_for_every_host_for_tests(&site);

    send(
        &state,
        put_profile(
            &guest,
            serde_json::json!({ "links": [{ "url": "https://pictured.example/" }] }),
        ),
    )
    .await;
    fetch_queued_link_icons(&state.profiles, &fetcher).await;
    let (_, profile) = profile_of(&state, &guest, "/api/v1/users/@me/profile").await;
    database.close().await;

    assert_eq!(requests.pictured_icon.load(Ordering::SeqCst), 0);
    assert_eq!(profile.unwrap().links[0].icon_id, None);
}

#[tokio::test]
async fn an_icon_lasts_a_week_and_a_site_that_stops_answering_keeps_it_and_is_asked_within_an_hour()
{
    const HOUR_MILLISECONDS: u64 = 60 * 60 * 1_000;
    const WEEK_MILLISECONDS: u64 = 7 * 24 * HOUR_MILLISECONDS;
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let guest = registered_token(&state, "guest").await;
    let guest_id = user_id_of(&state, &guest).await;
    let (site, requests) = stand_in_sites().await;
    let fetcher = LinkFetcher::stand_in_for_every_host_for_tests(&site);
    state.profiles.enable_link_icons();
    let stored_link = || async {
        crate::storage::user_profiles::profile_of_user(&state.database, guest_id)
            .await
            .unwrap()
            .links
            .remove(0)
    };

    let first_fetch_start = now_unix_milliseconds();
    send(
        &state,
        put_profile(
            &guest,
            serde_json::json!({ "links": [{ "url": "https://flaky.example/" }] }),
        ),
    )
    .await;
    fetch_queued_link_icons(&state.profiles, &fetcher).await;
    let first_fetch_end = now_unix_milliseconds();
    let first_fetch = stored_link().await;
    // The week has passed, and the site is down.
    crate::storage::user_profiles::postpone_link_icon_refresh(&state.database, "flaky.example", 0)
        .await
        .unwrap();
    requests.is_flaky_site_down.store(true, Ordering::SeqCst);
    let retry_start = now_unix_milliseconds();
    profile_of(&state, &guest, "/api/v1/users/@me/profile").await;
    fetch_queued_link_icons(&state.profiles, &fetcher).await;
    let retry_end = now_unix_milliseconds();
    let after_failure = stored_link().await;
    let ((kept_status, _), _) = send_from(
        &state,
        TEST_PEER,
        authorized(
            "GET",
            &format!("/api/v1/link-icons/{}", first_fetch.icon_file_id.unwrap()),
            &guest,
            None,
        ),
    )
    .await;
    database.close().await;

    let lifetime = first_fetch.icon_refresh_after_unix_milliseconds.unwrap();
    assert!(
        (first_fetch_start + WEEK_MILLISECONDS..=first_fetch_end + WEEK_MILLISECONDS)
            .contains(&lifetime)
    );
    assert_eq!(after_failure.icon_file_id, first_fetch.icon_file_id, "kept");
    let retry = after_failure.icon_refresh_after_unix_milliseconds.unwrap();
    assert!((retry_start + HOUR_MILLISECONDS..=retry_end + HOUR_MILLISECONDS).contains(&retry));
    assert_eq!(kept_status, StatusCode::OK);
}
