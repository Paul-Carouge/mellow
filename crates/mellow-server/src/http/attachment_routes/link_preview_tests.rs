//! Link previews through edits, slow sites, busy picture slots, floods and lost access
//! (ADR-0041). Sites are local servers; a job's budget is ended by an event, never by time.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use mellow_protocol::{CurrentUser, GuildSummary, Message};
use tokio::sync::{Notify, mpsc};

use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::migrated_database;
use crate::domain::link_previews::fetcher::LinkFetcher;
use crate::domain::messages::{fetch_queued_link_previews, run_queued_link_preview_jobs};
use crate::events::SharedEvent;
use crate::http::test_support::*;
use crate::http::*;

/// A site whose `/card` page (any query) has a title and no picture; it counts the page's fetches.
async fn card_site() -> (String, Arc<AtomicUsize>) {
    let card_fetches = Arc::new(AtomicUsize::new(0));
    let counter = card_fetches.clone();
    let site = axum::Router::new().route(
        "/card",
        axum::routing::get(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            async {
                (
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    "<html><head><title>A card</title></head></html>",
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, site).await });
    (format!("http://{address}"), card_fetches)
}

/// A site that accepts connections and never answers; `Notify` tells each connection.
async fn silent_site() -> (String, Arc<Notify>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let connected = Arc::new(Notify::new());
    let notifier = connected.clone();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
            notifier.notify_one();
        }
    });
    (format!("http://{address}"), connected)
}

/// A live gateway session of the token's user, subscribed to their first guild.
async fn live_session(state: &AppState, token: &str, key: u8) -> mpsc::Receiver<SharedEvent> {
    let (_, body) = send(state, authorized("GET", "/api/v1/users/@me", token, None)).await;
    let user_id = json::<CurrentUser>(&body).id;
    let (_, body) = send(
        state,
        authorized("GET", "/api/v1/users/@me/guilds", token, None),
    )
    .await;
    let guild_id = json::<Vec<GuildSummary>>(&body)[0].id;
    let (session, events) = state
        .events
        .register_session(user_id, [key; 32], 1)
        .unwrap();
    state.events.activate_session(session, &[guild_id]);
    events
}

/// The `data` of each event of `event_type` queued for the session.
fn events_of_type(
    events: &mut mpsc::Receiver<SharedEvent>,
    event_type: &str,
) -> Vec<serde_json::Value> {
    let mut found = Vec::new();
    while let Ok(event) = events.try_recv() {
        let mut event: serde_json::Value = serde_json::from_str(&event.payload_json).unwrap();
        if event["t"] == event_type {
            found.push(event["data"].take());
        }
    }
    found
}

fn post_message(path: &str, token: &str, content: &str) -> axum::http::Request<axum::body::Body> {
    authorized(
        "POST",
        path,
        token,
        Some(serde_json::json!({ "content": content })),
    )
}

async fn history(state: &AppState, path: &str, token: &str) -> Vec<Message> {
    let (_, body) = send(state, authorized("GET", path, token, None)).await;
    json(&body)
}

/// What becomes of the job an edit queues.
enum EditJob {
    PreviewsTurnedOff,
    QueueFull,
    Fails,
    OutOfTime,
}

/// An edit replacing a message's previewed link by another drops the old link's preview from
/// history and from `MESSAGE_UPDATE`, whatever becomes of the edit's own job.
async fn an_edit_drops_the_preview_of_the_link_it_removed(edit_job: EditJob) {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let mut owner_events = live_session(&state, &owner, 1).await;
    let (site, _) = card_site().await;
    let (silent, connected) = silent_site().await;
    let fetcher = LinkFetcher::allowing_loopback_for_tests();
    state.messages.enable_link_previews();
    let messages_path = format!("/api/v1/channels/{general}/messages");
    let (_, body) = send(
        &state,
        post_message(&messages_path, &owner, &format!("see {site}/card?old")),
    )
    .await;
    let message: Message = json(&body);
    fetch_queued_link_previews(&state.messages, &state.attachments, &fetcher).await;
    let before_edit = history(&state, &messages_path, &owner).await;
    let new_link = match edit_job {
        EditJob::PreviewsTurnedOff => {
            state.messages.disable_link_previews();
            format!("{site}/card?new")
        }
        EditJob::QueueFull => {
            state.messages.fill_link_preview_queue();
            format!("{site}/card?new")
        }
        EditJob::Fails => {
            sqlx::raw_sql(
                "CREATE TRIGGER refuse_link_previews BEFORE INSERT ON link_previews \
                 BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )
            .execute(database.writer())
            .await
            .unwrap();
            format!("{site}/card?new")
        }
        EditJob::OutOfTime => format!("{silent}/never"),
    };
    events_of_type(&mut owner_events, "");

    let (status, _) = send(
        &state,
        authorized(
            "PATCH",
            &format!("{messages_path}/{}", message.id),
            &owner,
            Some(serde_json::json!({ "content": format!("now {new_link}") })),
        ),
    )
    .await;
    let updates = events_of_type(&mut owner_events, "MESSAGE_UPDATE");
    let results = run_queued_link_preview_jobs(
        &state.messages,
        &state.attachments,
        &fetcher,
        connected.notified(),
    )
    .await;
    let after_jobs = history(&state, &messages_path, &owner).await;
    let preview_updates = events_of_type(&mut owner_events, "MESSAGE_LINK_PREVIEWS_UPDATE");
    database.close().await;

    assert_eq!(before_edit[0].link_previews.len(), 1);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updates.len(), 1);
    assert_eq!(
        updates[0].get("link_previews"),
        None,
        "an empty list is left out"
    );
    assert!(
        after_jobs[0].link_previews.is_empty(),
        "{:?}",
        after_jobs[0]
    );
    assert_eq!(preview_updates.len(), 0, "the new link gave nothing");
    let failures = results.iter().filter(|result| result.is_err()).count();
    match edit_job {
        EditJob::PreviewsTurnedOff => assert_eq!(results.len(), 0),
        EditJob::QueueFull => assert_eq!(failures, 0, "only the filler jobs ran"),
        EditJob::Fails => assert_eq!((results.len(), failures), (1, 1)),
        EditJob::OutOfTime => assert_eq!((results.len(), failures), (1, 0)),
    }
}

#[tokio::test]
async fn an_edit_drops_a_removed_links_preview_with_previews_turned_off() {
    an_edit_drops_the_preview_of_the_link_it_removed(EditJob::PreviewsTurnedOff).await;
}

#[tokio::test]
async fn an_edit_drops_a_removed_links_preview_when_the_queue_is_full() {
    an_edit_drops_the_preview_of_the_link_it_removed(EditJob::QueueFull).await;
}

#[tokio::test]
async fn an_edit_drops_a_removed_links_preview_when_its_job_fails() {
    an_edit_drops_the_preview_of_the_link_it_removed(EditJob::Fails).await;
}

#[tokio::test]
async fn an_edit_drops_a_removed_links_preview_when_its_job_runs_out_of_time() {
    an_edit_drops_the_preview_of_the_link_it_removed(EditJob::OutOfTime).await;
}

#[tokio::test]
async fn a_hanging_link_does_not_lose_the_preview_of_the_link_before_it() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let mut owner_events = live_session(&state, &owner, 1).await;
    let (site, _) = card_site().await;
    let (silent, connected) = silent_site().await;
    let fetcher = LinkFetcher::allowing_loopback_for_tests();
    state.messages.enable_link_previews();
    let messages_path = format!("/api/v1/channels/{general}/messages");
    send(
        &state,
        post_message(
            &messages_path,
            &owner,
            &format!("see {site}/card and {silent}/never"),
        ),
    )
    .await;
    events_of_type(&mut owner_events, "");

    // The budget runs out once the second link's site is reached, whatever the clock says.
    let results = run_queued_link_preview_jobs(
        &state.messages,
        &state.attachments,
        &fetcher,
        connected.notified(),
    )
    .await;
    let after = history(&state, &messages_path, &owner).await;
    let published = events_of_type(&mut owner_events, "MESSAGE_LINK_PREVIEWS_UPDATE");
    database.close().await;

    assert_eq!(results.len(), 1);
    assert!(results[0].is_ok());
    let urls: Vec<&str> = after[0]
        .link_previews
        .iter()
        .map(|preview| preview.url.as_str())
        .collect();
    assert_eq!(urls, [format!("{site}/card")]);
    assert_eq!(published.len(), 1);
    assert_eq!(published[0]["link_previews"][0]["title"], "A card");
}

#[tokio::test]
async fn one_authors_flood_of_links_leaves_room_for_another_members_preview() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
    let (_, body) = send(
        &state,
        authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &owner, None),
    )
    .await;
    let general = json::<mellow_protocol::Guild>(&body).channels[1].id;
    let (site, card_fetches) = card_site().await;
    let fetcher = LinkFetcher::allowing_loopback_for_tests();
    state.messages.enable_link_previews();
    let messages_path = format!("/api/v1/channels/{general}/messages");
    for index in 0..8 {
        send(
            &state,
            post_message(&messages_path, &owner, &format!("{site}/card?{index}")),
        )
        .await;
    }
    let queued_by_the_flood = state.messages.queued_link_preview_jobs();
    send(
        &state,
        post_message(&messages_path, &guest, &format!("{site}/card?guest")),
    )
    .await;
    let queued_with_the_guest = state.messages.queued_link_preview_jobs();

    fetch_queued_link_previews(&state.messages, &state.attachments, &fetcher).await;
    let after = history(&state, &messages_path, &owner).await;
    // The author's slots are free again once their jobs ended.
    send(
        &state,
        post_message(&messages_path, &owner, &format!("{site}/card?later")),
    )
    .await;
    let queued_later = state.messages.queued_link_preview_jobs();
    database.close().await;

    assert_eq!(queued_by_the_flood, 4, "the author's cap");
    assert_eq!(queued_with_the_guest, 5);
    assert_eq!(card_fetches.load(Ordering::SeqCst), 5);
    let with_previews: Vec<&str> = after
        .iter()
        .filter(|message| !message.link_previews.is_empty())
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(with_previews.len(), 5);
    assert!(with_previews.contains(&format!("{site}/card?guest").as_str()));
    assert_eq!(queued_later, 1);
}

#[tokio::test]
async fn a_member_who_lost_access_meanwhile_does_not_receive_the_previews() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
    let mut owner_events = live_session(&state, &owner, 1).await;
    let mut guest_events = live_session(&state, &guest, 2).await;
    let (_, body) = send(
        &state,
        authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &owner, None),
    )
    .await;
    let general = json::<mellow_protocol::Guild>(&body).channels[1].id;
    let (site, _) = card_site().await;
    let fetcher = LinkFetcher::allowing_loopback_for_tests();
    state.messages.enable_link_previews();
    send(
        &state,
        post_message(
            &format!("/api/v1/channels/{general}/messages"),
            &owner,
            &format!("see {site}/card"),
        ),
    )
    .await;
    let view = mellow_protocol::Permissions::VIEW_CHANNEL
        .bits()
        .to_string();
    let (status, _) = send(
        &state,
        authorized(
            "PUT",
            &format!("/api/v1/channels/{general}/permissions/{guild_id}"),
            &owner,
            Some(serde_json::json!({"kind": "role", "deny": view})),
        ),
    )
    .await;
    let guest_saw_the_message = events_of_type(&mut guest_events, "MESSAGE_CREATE").len();
    events_of_type(&mut owner_events, "");

    fetch_queued_link_previews(&state.messages, &state.attachments, &fetcher).await;
    let owner_previews = events_of_type(&mut owner_events, "MESSAGE_LINK_PREVIEWS_UPDATE");
    let guest_previews = events_of_type(&mut guest_events, "MESSAGE_LINK_PREVIEWS_UPDATE");
    database.close().await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(guest_saw_the_message, 1, "a viewer when it was sent");
    assert_eq!(owner_previews.len(), 1);
    assert_eq!(guest_previews.len(), 0);
}

#[tokio::test]
async fn a_preview_never_takes_the_picture_slot_an_avatar_upload_needs() {
    use crate::domain::image_processing::tests::png;

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let (site, _) = super::tests::local_site().await;
    let fetcher = LinkFetcher::allowing_loopback_for_tests();
    state.messages.enable_link_previews();
    let messages_path = format!("/api/v1/channels/{general}/messages");
    // The background compression holds one slot, a preview picture being made the other.
    let _optimizer = state
        .attachments
        .wait_for_image_processing_permit()
        .await
        .unwrap();
    let _preview_picture = state.attachments.take_link_preview_picture_slot();

    send(
        &state,
        post_message(&messages_path, &owner, &format!("see {site}/page")),
    )
    .await;
    fetch_queued_link_previews(&state.messages, &state.attachments, &fetcher).await;
    let after = history(&state, &messages_path, &owner).await;
    let lifetimes: Vec<i64> = sqlx::query_scalar(
        "SELECT expires_at_unix_milliseconds - fetched_at_unix_milliseconds FROM link_previews",
    )
    .fetch_all(database.readers())
    .await
    .unwrap();
    let (avatar_status, body) = send(
        &state,
        picture_request("PUT", "/api/v1/users/@me/avatar", &owner, png(64, 64)),
    )
    .await;
    database.close().await;

    let preview = &after[0].link_previews[0];
    assert_eq!(preview.title.as_deref(), Some("Local & page"), "text kept");
    assert_eq!(preview.image, None, "its picture skipped, not waited for");
    assert_eq!(
        lifetimes,
        [60 * 60 * 1_000],
        "fetched again within the hour, not kept without its picture for a day"
    );
    assert_eq!(
        avatar_status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&body)
    );
}

#[tokio::test]
async fn an_error_on_a_later_link_keeps_the_preview_of_the_link_before_it() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let mut owner_events = live_session(&state, &owner, 1).await;
    let (site, _) = card_site().await;
    let fetcher = LinkFetcher::allowing_loopback_for_tests();
    state.messages.enable_link_previews();
    let messages_path = format!("/api/v1/channels/{general}/messages");
    // Recording the second link's preview fails, as a full disk or a database error would.
    sqlx::raw_sql(
        "CREATE TRIGGER refuse_second_link BEFORE INSERT ON link_previews \
         WHEN NEW.url LIKE '%?second' BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .execute(database.writer())
    .await
    .unwrap();
    send(
        &state,
        post_message(
            &messages_path,
            &owner,
            &format!("see {site}/card?first and {site}/card?second"),
        ),
    )
    .await;
    events_of_type(&mut owner_events, "");

    let results = run_queued_link_preview_jobs(
        &state.messages,
        &state.attachments,
        &fetcher,
        std::future::pending(),
    )
    .await;
    let after = history(&state, &messages_path, &owner).await;
    let published = events_of_type(&mut owner_events, "MESSAGE_LINK_PREVIEWS_UPDATE");
    database.close().await;

    assert_eq!(results.len(), 1);
    assert!(results[0].is_err(), "the error is still reported");
    let urls: Vec<&str> = after[0]
        .link_previews
        .iter()
        .map(|preview| preview.url.as_str())
        .collect();
    assert_eq!(urls, [format!("{site}/card?first")]);
    assert_eq!(published.len(), 1);
    assert_eq!(published[0]["link_previews"][0]["title"], "A card");
}

#[tokio::test]
async fn an_edit_carries_the_card_a_job_recorded_while_the_edit_waited_for_the_writer() {
    const KEPT_LINK: &str = "https://card.example/";
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let mut owner_events = live_session(&state, &owner, 1).await;
    let messages_path = format!("/api/v1/channels/{general}/messages");
    let (_, body) = send(
        &state,
        post_message(&messages_path, &owner, &format!("see {KEPT_LINK}")),
    )
    .await;
    let message: Message = json(&body);
    events_of_type(&mut owner_events, "");
    // The send's job records the kept link's card while holding the writer, as it would while
    // the edit reads what to publish and then waits for the writer.
    let mut job_transaction = database
        .writer()
        .begin_with("BEGIN IMMEDIATE")
        .await
        .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "INSERT INTO link_previews (id, url_sha256, url, title, is_empty, \
         fetched_at_unix_milliseconds, expires_at_unix_milliseconds) \
         VALUES (77, zeroblob(32), '{KEPT_LINK}', 'A card', 0, 0, 9000000000000); \
         INSERT INTO message_link_previews (message_id, position, link_preview_id) \
         VALUES ({}, 0, 77);",
        message.id
    )))
    .execute(&mut *job_transaction)
    .await
    .unwrap();
    let job_commits_later = async {
        // Lets the edit run as far as it can without the writer before the job commits.
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        job_transaction.commit().await.unwrap();
    };

    let ((status, _), ()) = tokio::join!(
        send(
            &state,
            authorized(
                "PATCH",
                &format!("{messages_path}/{}", message.id),
                &owner,
                Some(serde_json::json!({ "content": format!("fixed: see {KEPT_LINK}") })),
            ),
        ),
        job_commits_later,
    );
    let updates = events_of_type(&mut owner_events, "MESSAGE_UPDATE");
    database.close().await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["link_previews"][0]["title"], "A card");
}

/// A stand-in for YouTube's oEmbed endpoint and thumbnail host (`LinkFetcher`'s stand-in site);
/// it counts oEmbed's answers.
async fn stand_in_youtube() -> (String, Arc<AtomicUsize>) {
    use axum::response::IntoResponse as _;

    let oembed_answers = Arc::new(AtomicUsize::new(0));
    let counter = oembed_answers.clone();
    let site = axum::Router::new()
        .route(
            "/www.youtube.com/oembed",
            axum::routing::get(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async {
                    (
                        [(header::CONTENT_TYPE, "application/json")],
                        r#"{"title":"Les Cités d'or","author_name":"Noëlle Dupuis - Topic"}"#,
                    )
                }
            }),
        )
        .route(
            "/i.ytimg.com/vi/{id}/maxresdefault.jpg",
            axum::routing::get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/png")],
                    crate::domain::image_processing::tests::png(160, 90),
                )
                    .into_response()
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, site).await });
    (format!("http://{address}"), oembed_answers)
}

/// YouTube links get their card and embed from oEmbed, never from the page behind the consent
/// wall, and two sharers' spellings of one track share one preview (ADR-0044).
#[tokio::test]
async fn youtube_links_get_their_embed_from_oembed_once_for_every_spelling() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let (site, oembed_answers) = stand_in_youtube().await;
    let fetcher = LinkFetcher::stand_in_for_every_host_for_tests(&site);
    state.messages.enable_link_previews();
    let messages_path = format!("/api/v1/channels/{general}/messages");

    for content in [
        "https://music.youtube.com/watch?v=lYBUbBu4W08&si=first-sharer",
        "https://music.youtube.com/watch?si=second-sharer&v=lYBUbBu4W08",
    ] {
        send(&state, post_message(&messages_path, &owner, content)).await;
        fetch_queued_link_previews(&state.messages, &state.attachments, &fetcher).await;
    }
    let messages = history(&state, &messages_path, &owner).await;
    database.close().await;

    assert_eq!(oembed_answers.load(Ordering::SeqCst), 1, "one cached row");
    let previews: Vec<_> = messages
        .iter()
        .map(|message| message.link_previews[0].clone())
        .collect();
    assert_eq!(previews[0], previews[1]);
    let preview = &previews[0];
    assert_eq!(preview.url, "https://music.youtube.com/watch?v=lYBUbBu4W08");
    assert_eq!(preview.title.as_deref(), Some("Les Cités d'or"));
    assert_eq!(preview.site_name.as_deref(), Some("YouTube Music"));
    assert_eq!(
        preview.image.map(|image| (image.width, image.height)),
        Some((160, 90))
    );
    assert_eq!(
        preview.embed,
        Some(mellow_protocol::LinkEmbed::YouTubeMusicTrack(
            mellow_protocol::YouTubeMusicTrackEmbed {
                video_id: "lYBUbBu4W08".into(),
                artist: "Noëlle Dupuis".into(),
                album: None,
                duration_seconds: None,
            }
        ))
    );
}

/// A stand-in for YouTube whose oEmbed endpoint answers `oembed_status` with no body, and whose
/// watch page has an Open Graph title; it counts the page's fetches.
async fn stand_in_youtube_without_oembed(oembed_status: StatusCode) -> (String, Arc<AtomicUsize>) {
    let page_fetches = Arc::new(AtomicUsize::new(0));
    let counter = page_fetches.clone();
    let site = axum::Router::new()
        .route(
            "/www.youtube.com/oembed",
            axum::routing::get(move || async move { oembed_status }),
        )
        .route(
            "/www.youtube.com/watch",
            axum::routing::get(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async {
                    (
                        [(header::CONTENT_TYPE, "text/html")],
                        r#"<html><head><meta property="og:title" content="A private video"></head></html>"#,
                    )
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, site).await });
    (format!("http://{address}"), page_fetches)
}

/// The link's one recorded preview and how long it is kept, after one message sharing it.
async fn youtube_preview_recorded_with_oembed_answering(
    oembed_status: StatusCode,
) -> (Vec<Message>, i64, usize) {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let (site, page_fetches) = stand_in_youtube_without_oembed(oembed_status).await;
    let fetcher = LinkFetcher::stand_in_for_every_host_for_tests(&site);
    state.messages.enable_link_previews();
    let messages_path = format!("/api/v1/channels/{general}/messages");

    send(
        &state,
        post_message(
            &messages_path,
            &owner,
            "https://www.youtube.com/watch?v=lYBUbBu4W08",
        ),
    )
    .await;
    fetch_queued_link_previews(&state.messages, &state.attachments, &fetcher).await;
    let messages = history(&state, &messages_path, &owner).await;
    let lifetime: i64 = sqlx::query_scalar(
        "SELECT expires_at_unix_milliseconds - fetched_at_unix_milliseconds FROM link_previews",
    )
    .fetch_one(database.readers())
    .await
    .unwrap();
    database.close().await;
    (messages, lifetime, page_fetches.load(Ordering::SeqCst))
}

/// A video oEmbed refuses (private, removed) gets the page's card, without an embed.
#[tokio::test]
async fn a_youtube_video_oembed_refuses_gets_the_page_card_without_an_embed() {
    let (messages, lifetime, page_fetches) =
        youtube_preview_recorded_with_oembed_answering(StatusCode::NOT_FOUND).await;

    assert_eq!(page_fetches, 1);
    let preview = &messages[0].link_previews[0];
    assert_eq!(preview.title.as_deref(), Some("A private video"));
    assert_eq!(preview.embed, None);
    assert_eq!(lifetime, 24 * 60 * 60 * 1_000);
}

/// A busy or broken oEmbed leaves the link without a card for now and asks again within the
/// hour: the page, which can be YouTube's consent wall, is not shown for a day instead.
#[tokio::test]
async fn a_youtube_link_whose_oembed_is_busy_is_asked_again_within_the_hour() {
    let (messages, lifetime, page_fetches) =
        youtube_preview_recorded_with_oembed_answering(StatusCode::SERVICE_UNAVAILABLE).await;

    assert_eq!(page_fetches, 0, "the page is not asked");
    assert!(messages[0].link_previews.is_empty(), "no card for now");
    assert_eq!(lifetime, 60 * 60 * 1_000);
}
