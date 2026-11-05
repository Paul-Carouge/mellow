//! Provider embeds (ADR-0044) from GitHub's API and Wikipedia's summaries, both served by a local
//! site: the banner's facts, and the page's card while GitHub's budget is spent.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::http::{StatusCode, header};
use axum::response::IntoResponse as _;
use mellow_protocol::{
    GitHubIssueEmbed, GitHubIssueState, GitHubRepositoryEmbed, LinkEmbed, LinkPreview, Message,
    WikipediaArticleEmbed,
};
use url::Url;

use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::migrated_database;
use crate::domain::link_previews::fetcher::LinkFetcher;
use crate::domain::link_previews::providers::LinkProviders;
use crate::domain::messages::fetch_queued_link_previews;
use crate::http::AppState;
use crate::http::test_support::*;

/// Fetches of the stub site's GitHub API and of its pages.
#[derive(Default)]
struct Fetches {
    api: AtomicUsize,
    pages: AtomicUsize,
}

fn json_answer(body: &serde_json::Value) -> axum::response::Response {
    (
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (
                header::HeaderName::from_static("x-ratelimit-remaining"),
                "59",
            ),
        ],
        body.to_string(),
    )
        .into_response()
}

fn png_answer(width: u32, height: u32) -> axum::response::Response {
    (
        [(header::CONTENT_TYPE, "image/png")],
        crate::domain::image_processing::tests::png(width, height),
    )
        .into_response()
}

/// GitHub's API answers, trimmed to the fields the card reads.
fn github_api_answer(path: &str, avatar: &str) -> axum::response::Response {
    match path {
        "mellow-org/mellow" => json_answer(&serde_json::json!({
            "name": "Mellow",
            "owner": { "login": "Mellow-Org", "avatar_url": avatar },
            "description": "A self-hostable place\nto talk.",
            "stargazers_count": 104_238,
            "forks_count": 13_612,
            "language": "Rust",
            "topics": ["chat", "rust", "self-hosted", "svelte", "voice", "webrtc"],
            "license": { "spdx_id": "AGPL-3.0" },
            "archived": false,
        })),
        "mellow-org/mellow/issues/7" => json_answer(&serde_json::json!({
            "title": "Dark icon on light docks",
            "body": "<!-- template -->\n## What happens\nThe icon disappears.",
            "state": "closed",
            "state_reason": "not_planned",
            "user": { "login": "ana-dev" },
            "comments": 3,
            "created_at": "2026-11-03T10:00:00Z",
            "repository_url": "https://api.github.com/repos/Mellow-Org/Mellow",
        })),
        "mellow-org/mellow/issues/8" => json_answer(&serde_json::json!({
            "title": "Stabilise async closures",
            "state": "closed",
            "user": { "login": "paul" },
            "comments": 42,
            "created_at": "2026-11-03T10:00:00Z",
            "draft": false,
            "pull_request": { "merged_at": "2026-11-05T08:30:00Z" },
            "repository_url": "https://api.github.com/repos/Mellow-Org/Mellow",
        })),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Wikipedia's `page/summary` answers, trimmed to the fields the card reads.
fn wikipedia_summary_answer(title: &str, thumbnail: &str) -> axum::response::Response {
    match title {
        "Crépuscule" => json_answer(&serde_json::json!({
            "type": "standard",
            "title": "Crépuscule",
            "titles": { "normalized": "Crépuscule" },
            "extract": "Le crépuscule est la lueur atmosphérique présente avant le lever du soleil.",
            "thumbnail": { "source": thumbnail, "width": 320, "height": 240 },
        })),
        "Mercury" => json_answer(&serde_json::json!({
            "type": "disambiguation",
            "title": "Mercury",
            "extract": "Mercury usually refers to:",
        })),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// A stub of GitHub (API under `/repos`, pages at `/{owner}/{name}`) and of Wikipedia's
/// summaries; `refusal`, when set, is the API's answer to its first request.
async fn stub_site(refusal: Option<axum::response::Response>) -> (Url, Arc<Fetches>) {
    let site_fetches = Arc::new(Fetches::default());
    let refusal = Arc::new(std::sync::Mutex::new(refusal));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let site = format!("http://{}/", listener.local_addr().unwrap());
    let avatar = format!("{site}avatar.png");
    let thumbnail = format!("{site}thumbnail.png");
    let api_fetches = site_fetches.clone();
    let page_fetches = site_fetches.clone();
    let page = r#"<html><head><meta property="og:title" content="The page's card"></head></html>"#;
    let router = axum::Router::new()
        .route(
            "/repos/{*path}",
            axum::routing::get(
                move |axum::extract::Path(path): axum::extract::Path<String>| {
                    api_fetches.api.fetch_add(1, Ordering::SeqCst);
                    let refused = refusal.lock().unwrap().take();
                    let answer = refused.unwrap_or_else(|| github_api_answer(&path, &avatar));
                    async move { answer }
                },
            ),
        )
        .route(
            "/api/rest_v1/page/summary/{title}",
            axum::routing::get(
                move |axum::extract::Path(title): axum::extract::Path<String>| {
                    let answer = wikipedia_summary_answer(&title, &thumbnail);
                    async move { answer }
                },
            ),
        )
        .route(
            "/avatar.png",
            axum::routing::get(|| async { png_answer(80, 80) }),
        )
        .route(
            "/thumbnail.png",
            axum::routing::get(|| async { png_answer(320, 240) }),
        )
        .route(
            "/{owner}/{name}",
            axum::routing::get(move || {
                page_fetches.pages.fetch_add(1, Ordering::SeqCst);
                async move { ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], page) }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, router).await });
    (Url::parse(&site).unwrap(), site_fetches)
}

/// Sends `content` to the general channel and returns the message's previews once fetched.
async fn previews_of_message_with(
    state: &AppState,
    fetcher: &LinkFetcher,
    owner: &str,
    general: mellow_protocol::Snowflake,
    content: &str,
) -> Vec<LinkPreview> {
    let messages_path = format!("/api/v1/channels/{general}/messages");
    let (status, body) = send(
        state,
        authorized(
            "POST",
            &messages_path,
            owner,
            Some(serde_json::json!({ "content": content })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let sent: Message = json(&body);
    fetch_queued_link_previews(&state.messages, &state.attachments, fetcher).await;
    let (_, body) = send(state, authorized("GET", &messages_path, owner, None)).await;
    let history: Vec<Message> = json(&body);
    history
        .into_iter()
        .find(|message| message.id == sent.id)
        .unwrap()
        .link_previews
}

#[tokio::test]
async fn github_repository_issue_and_pull_request_links_get_their_banners() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let (site, site_fetches) = stub_site(None).await;
    let fetcher = LinkFetcher::allowing_loopback_for_tests()
        .with_providers_for_tests(LinkProviders::served_by_for_tests(&site));
    state.messages.enable_link_previews();

    let previews = previews_of_message_with(
        &state,
        &fetcher,
        &owner,
        general,
        &format!("{site}mellow-org/mellow, {site}mellow-org/mellow/issues/7 and {site}mellow-org/mellow/pull/8"),
    )
    .await;
    database.close().await;

    assert_eq!(
        site_fetches.pages.load(Ordering::SeqCst),
        0,
        "the API alone"
    );
    assert_eq!(site_fetches.api.load(Ordering::SeqCst), 3);
    let [repository, issue, pull_request] = previews.as_slice() else {
        panic!("{previews:?}");
    };
    assert_eq!(repository.title.as_deref(), Some("Mellow-Org/Mellow"));
    assert_eq!(
        repository.description.as_deref(),
        Some("A self-hostable place to talk.")
    );
    assert_eq!(repository.site_name.as_deref(), Some("GitHub"));
    let avatar = repository.image.unwrap();
    assert_eq!((avatar.width, avatar.height), (80, 80));
    assert_eq!(
        repository.embed,
        Some(LinkEmbed::GitHubRepository(GitHubRepositoryEmbed {
            owner: "Mellow-Org".into(),
            name: "Mellow".into(),
            stars: 104_238,
            forks: 13_612,
            language: Some("Rust".into()),
            language_color: None,
            topics: ["chat", "rust", "self-hosted", "svelte", "voice"]
                .map(String::from)
                .to_vec(),
            licence_spdx: Some("AGPL-3.0".into()),
            is_archived: false,
        }))
    );
    assert_eq!(issue.title.as_deref(), Some("Dark icon on light docks"));
    assert_eq!(
        issue.description.as_deref(),
        Some("What happens The icon disappears.")
    );
    assert_eq!(issue.image, None, "no author avatar on issues");
    assert_eq!(
        issue.embed,
        Some(LinkEmbed::GitHubIssue(GitHubIssueEmbed {
            owner: "Mellow-Org".into(),
            repository: "Mellow".into(),
            number: 7,
            state: GitHubIssueState::NotPlanned,
            author: "ana-dev".into(),
            opened_at_unix_milliseconds: Some(1_793_700_000_000),
            merged_at_unix_milliseconds: None,
            comments: 3,
        }))
    );
    let Some(LinkEmbed::GitHubPullRequest(pull_request_embed)) = &pull_request.embed else {
        panic!("{pull_request:?}");
    };
    assert_eq!(pull_request_embed.state, GitHubIssueState::Merged);
    assert_eq!(pull_request_embed.number, 8);
    assert_eq!(
        pull_request_embed.merged_at_unix_milliseconds,
        Some(1_793_867_400_000)
    );
}

/// GitHub answers `refusal` once its budget is spent: the link shows the page's card for an hour
/// only, and no other request reaches the API before the reset.
async fn a_spent_github_budget_shows_the_page_card_until_the_reset(
    refusal: axum::response::Response,
) {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let (site, site_fetches) = stub_site(Some(refusal)).await;
    let fetcher = LinkFetcher::allowing_loopback_for_tests()
        .with_providers_for_tests(LinkProviders::served_by_for_tests(&site));
    state.messages.enable_link_previews();

    let refused = previews_of_message_with(
        &state,
        &fetcher,
        &owner,
        general,
        &format!("{site}mellow-org/mellow"),
    )
    .await;
    let paused = previews_of_message_with(
        &state,
        &fetcher,
        &owner,
        general,
        &format!("{site}mellow-org/other"),
    )
    .await;
    let lifetimes: Vec<i64> = sqlx::query_scalar(
        "SELECT expires_at_unix_milliseconds - fetched_at_unix_milliseconds FROM link_previews",
    )
    .fetch_all(database.readers())
    .await
    .unwrap();
    database.close().await;

    assert_eq!(
        site_fetches.api.load(Ordering::SeqCst),
        1,
        "paused after the refusal"
    );
    assert_eq!(site_fetches.pages.load(Ordering::SeqCst), 2);
    for previews in [&refused, &paused] {
        assert_eq!(previews.len(), 1);
        assert_eq!(previews[0].title.as_deref(), Some("The page's card"));
        assert_eq!(previews[0].embed, None);
    }
    assert_eq!(lifetimes, [60 * 60 * 1_000, 60 * 60 * 1_000]);
}

/// Seconds since 1970 an hour from now: GitHub's next reset as its header writes it.
fn reset_in_an_hour() -> String {
    (crate::domain::clock::now_unix_milliseconds() / 1_000 + 60 * 60).to_string()
}

#[tokio::test]
async fn a_403_for_a_spent_budget_shows_the_page_card_until_the_reset() {
    let refusal = (
        StatusCode::FORBIDDEN,
        [
            ("x-ratelimit-remaining", "0".to_owned()),
            ("x-ratelimit-reset", reset_in_an_hour()),
        ],
    )
        .into_response();
    a_spent_github_budget_shows_the_page_card_until_the_reset(refusal).await;
}

#[tokio::test]
async fn a_429_with_retry_after_shows_the_page_card_until_then() {
    let refusal = (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "600")]).into_response();
    a_spent_github_budget_shows_the_page_card_until_the_reset(refusal).await;
}

#[tokio::test]
async fn wikipedia_links_get_the_article_summary_and_thumbnail() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let (site, _) = stub_site(None).await;
    let fetcher = LinkFetcher::allowing_loopback_for_tests()
        .with_providers_for_tests(LinkProviders::served_by_for_tests(&site));
    state.messages.enable_link_previews();

    let previews = previews_of_message_with(
        &state,
        &fetcher,
        &owner,
        general,
        "https://fr.m.wikipedia.org/wiki/Crépuscule?useskin=vector and https://en.wikipedia.org/wiki/Mercury",
    )
    .await;
    database.close().await;

    let [article, disambiguation] = previews.as_slice() else {
        panic!("{previews:?}");
    };
    assert_eq!(
        article.url, "https://fr.wikipedia.org/wiki/Cr%C3%A9puscule",
        "the desktop site's canonical link"
    );
    assert_eq!(article.title.as_deref(), Some("Crépuscule"));
    assert_eq!(
        article.description.as_deref(),
        Some("Le crépuscule est la lueur atmosphérique présente avant le lever du soleil.")
    );
    assert_eq!(article.site_name.as_deref(), Some("Wikipedia"));
    let thumbnail = article.image.unwrap();
    assert_eq!((thumbnail.width, thumbnail.height), (320, 240));
    assert_eq!(
        article.embed,
        Some(LinkEmbed::WikipediaArticle(WikipediaArticleEmbed {
            language: "fr".into(),
            is_disambiguation: false,
        }))
    );
    assert_eq!(
        disambiguation.embed,
        Some(LinkEmbed::WikipediaArticle(WikipediaArticleEmbed {
            language: "en".into(),
            is_disambiguation: true,
        }))
    );
}

#[tokio::test]
async fn links_github_does_not_know_show_the_page_card_for_a_day_without_spending_the_budget() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let (site, site_fetches) = stub_site(None).await;
    let fetcher = LinkFetcher::allowing_loopback_for_tests()
        .with_providers_for_tests(LinkProviders::served_by_for_tests(&site));
    state.messages.enable_link_previews();

    let previews = previews_of_message_with(
        &state,
        &fetcher,
        &owner,
        general,
        &format!("{site}mellow-org/gone and {site}topics/rust"),
    )
    .await;
    let lifetimes: Vec<i64> = sqlx::query_scalar(
        "SELECT expires_at_unix_milliseconds - fetched_at_unix_milliseconds FROM link_previews",
    )
    .fetch_all(database.readers())
    .await
    .unwrap();
    database.close().await;

    assert_eq!(
        site_fetches.api.load(Ordering::SeqCst),
        1,
        "GitHub's own pages never reach the API"
    );
    assert_eq!(site_fetches.pages.load(Ordering::SeqCst), 2);
    for preview in &previews {
        assert_eq!(preview.title.as_deref(), Some("The page's card"));
        assert_eq!(preview.embed, None);
    }
    assert_eq!(previews.len(), 2);
    assert_eq!(lifetimes, [24 * 60 * 60 * 1_000, 24 * 60 * 60 * 1_000]);
}
