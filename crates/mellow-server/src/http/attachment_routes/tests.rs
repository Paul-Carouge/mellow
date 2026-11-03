use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use mellow_protocol::{ApiErrorBody, ApiErrorCode, CurrentUser};
use tower::ServiceExt as _;

use super::*;
use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::migrated_database;
use crate::http::test_support::*;
use crate::http::*;

#[test]
fn large_uploads_get_time_at_a_slow_speed_and_small_ones_a_floor() {
    assert_eq!(upload_deadline(25 * 1024 * 1024).as_secs(), 400);
    assert_eq!(upload_deadline(1024), MINIMUM_UPLOAD_DEADLINE);
}

#[test]
fn if_none_match_matches_the_tag_in_a_list_weakly_or_any() {
    let tag = HeaderValue::from_static("\"7-42\"");
    let matches =
        |condition: &'static str| is_entity_tag_matched(&HeaderValue::from_static(condition), &tag);
    assert!(matches("\"7-42\""));
    assert!(matches("W/\"7-42\""));
    assert!(matches("\"1-1\", \"7-42\""));
    assert!(matches("*"));
    assert!(!matches("\"7-43\""));
    assert!(!matches("7-42"));
    assert!(!matches(""));
}

#[test]
fn file_names_cannot_break_out_of_the_disposition_header() {
    assert_eq!(percent_encode_file_name("photo.png"), "photo.png");
    assert_eq!(
        percent_encode_file_name("a\"b;c=d é.txt"),
        "a%22b%3Bc%3Dd%20%C3%A9.txt"
    );
}

#[test]
fn header_file_names_decode_as_encode_uri_component_writes_them() {
    assert_eq!(
        percent_decode("vacances%20d%27%C3%A9t%C3%A9.png").as_deref(),
        Some("vacances d'été.png")
    );
    assert_eq!(percent_decode("%ZZ"), None);
    assert_eq!(percent_decode("%C3"), None, "not UTF-8");
    assert_eq!(percent_decode("cut%4"), None);
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot really an image";

const HTML: &[u8] = b"<html><script>alert(1)</script></html>";

#[test]
fn byte_ranges_follow_rfc_9110_for_one_range() {
    use super::{RequestedRange::*, byte_range};
    assert_eq!(byte_range("bytes=0-", 10), Satisfiable(0..=9));
    assert_eq!(byte_range("bytes=2-4", 10), Satisfiable(2..=4));
    assert_eq!(byte_range("bytes=8-99", 10), Satisfiable(8..=9));
    assert_eq!(byte_range("bytes=-3", 10), Satisfiable(7..=9));
    assert_eq!(byte_range("bytes=-30", 10), Satisfiable(0..=9));
    assert_eq!(byte_range("bytes=10-", 10), Unsatisfiable);
    assert_eq!(byte_range("bytes=-0", 10), Unsatisfiable);
    assert_eq!(byte_range("bytes=0-", 0), Unsatisfiable);
    for ignored in [
        "items=0-1",
        "bytes=0-1,4-5",
        "bytes=5-2",
        "bytes=-",
        "bytes=a-1",
        "bytes=+1-2",
        "bytes=99999999999999999999999-",
    ] {
        assert_eq!(byte_range(ignored, 10), Ignored, "{ignored}");
    }
}

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
async fn pictures_carry_their_dimensions_from_upload_to_history() {
    use mellow_protocol::{Attachment, Message};

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let picture = crate::domain::image_processing::tests::png(30, 10).leak();
    let mut uploaded = Vec::new();
    for (name, bytes) in [("wide.png", &*picture), ("page.html", HTML)] {
        let path = format!("/api/v1/channels/{general}/attachments?file_name={name}");
        let (_, body) = send(&state, upload_request(&path, &owner, bytes)).await;
        uploaded.push(json::<Attachment>(&body));
    }
    let messages_path = format!("/api/v1/channels/{general}/messages");
    let ids: Vec<String> = uploaded.iter().map(|file| file.id.to_string()).collect();
    let (status, _) = send(
        &state,
        authorized(
            "POST",
            &messages_path,
            &owner,
            Some(serde_json::json!({"content": "", "attachment_ids": ids})),
        ),
    )
    .await;
    let (_, body) = send(&state, authorized("GET", &messages_path, &owner, None)).await;
    database.close().await;

    assert_eq!(status, StatusCode::CREATED);
    let dimensions = |file: &Attachment| (file.width, file.height);
    assert_eq!(
        uploaded.iter().map(dimensions).collect::<Vec<_>>(),
        [(Some(30), Some(10)), (None, None)]
    );
    let history: Vec<Message> = json(&body);
    let listed = &history.last().unwrap().attachments;
    assert_eq!(listed, &uploaded);
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

    let (status, body) = send(&state, upload_request(&upload_path("cat.png"), &owner, PNG)).await;
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
            Some(serde_json::json!({"content": "", "attachment_ids": [uploaded.id.to_string()]})),
        ),
    )
    .await;
    let sent: mellow_protocol::Message = json(&body);
    let ((_, _), original_headers) = send_from(
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
    let original_entity_tag = original_headers.get(header::ETAG).unwrap().clone();
    // What the background worker runs for this upload.
    state.attachments.optimize(uploaded.id).await.unwrap();
    let mut revalidation = authorized(
        "GET",
        &format!("/api/v1/attachments/{}", uploaded.id),
        &owner,
        None,
    );
    revalidation
        .headers_mut()
        .insert(header::IF_NONE_MATCH, original_entity_tag.clone());
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
    let (revalidated, _) = send(&state, revalidation).await;
    database.close().await;

    assert_ne!(headers.get(header::ETAG), Some(&original_entity_tag));
    assert_eq!(
        revalidated,
        StatusCode::OK,
        "a copy of the original is stale"
    );
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
#[allow(clippy::too_many_lines)] // One scenario: before and after the worker, refusals, purge.
async fn a_large_picture_gets_a_preview_served_like_its_attachment_and_purged_with_it() {
    use crate::domain::image_processing::tests::{encoded, photo, stored_dimensions};
    use mellow_protocol::Attachment;

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let stranger = registered_token(&state, "stranger").await;
    // Wide and short: debug builds encode slowly, and only the width needs a preview.
    let picture = encoded(&photo(1_200, 150), image::ImageFormat::Jpeg, None);
    let upload = |name: &'static str| {
        let mut request = picture_request(
            "POST",
            &format!("/api/v1/channels/{general}/attachments"),
            &owner,
            picture.clone(),
        );
        request
            .headers_mut()
            .insert("x-mellow-file-name", HeaderValue::from_static(name));
        request
    };
    let (_, body) = send(&state, upload("photo.png")).await;
    let sent_picture: Attachment = json(&body);
    let (_, body) = send(&state, upload("draft.png")).await;
    let unsent_picture: Attachment = json(&body);
    send(
        &state,
        authorized(
            "POST",
            &format!("/api/v1/channels/{general}/messages"),
            &owner,
            Some(serde_json::json!({
                "content": "",
                "attachment_ids": [sent_picture.id.to_string()],
            })),
        ),
    )
    .await;
    let get = |path: &str, token: &str| authorized("GET", path, token, None);
    let preview_path = format!("/api/v1/attachments/{}/preview", sent_picture.id);
    let attachment_path = format!("/api/v1/attachments/{}", sent_picture.id);

    // Before the worker ran, the preview is the attachment itself.
    let ((_, early_preview), early_headers) =
        send_from(&state, TEST_PEER, get(&preview_path, &owner)).await;
    let ((_, _), early_attachment_headers) =
        send_from(&state, TEST_PEER, get(&attachment_path, &owner)).await;
    state.attachments.optimize(sent_picture.id).await.unwrap();
    state.attachments.optimize(unsent_picture.id).await.unwrap();
    let ((preview_status, preview), headers) =
        send_from(&state, TEST_PEER, get(&preview_path, &owner)).await;
    let ((_, attachment), attachment_headers) =
        send_from(&state, TEST_PEER, get(&attachment_path, &owner)).await;
    let mut revalidation = get(&preview_path, &owner);
    revalidation.headers_mut().insert(
        header::IF_NONE_MATCH,
        headers.get(header::ETAG).unwrap().clone(),
    );
    let (revalidated, _) = send(&state, revalidation).await;
    let (stranger_status, _) = send(&state, get(&preview_path, &stranger)).await;
    let preview_of = |id| {
        let database = database.clone();
        async move {
            crate::storage::attachments::find_attachment(&database, id)
                .await
                .unwrap()
                .unwrap()
                .preview
                .unwrap()
        }
    };
    let unsent_preview = preview_of(unsent_picture.id).await;
    let kept_preview = preview_of(sent_picture.id).await;
    state
        .attachments
        .purge(
            crate::domain::clock::now_unix_milliseconds()
                + crate::domain::attachments::UNSENT_UPLOAD_LIFETIME_MILLISECONDS * 2,
        )
        .await
        .unwrap();
    let remaining_files = state.attachments.stored_file_ids().await;
    database.close().await;

    assert_eq!(early_preview, picture);
    assert_eq!(
        early_headers.get(header::ETAG),
        early_attachment_headers.get(header::ETAG)
    );
    assert_eq!(preview_status, StatusCode::OK);
    assert_eq!(stored_dimensions(&preview), (800, 100));
    assert!(
        preview.len() < attachment.len(),
        "smaller than the stored file"
    );
    let preview_type = headers.get(header::CONTENT_TYPE).unwrap();
    assert!(["image/avif", "image/webp"].contains(&preview_type.to_str().unwrap()));
    assert_ne!(
        headers.get(header::ETAG),
        attachment_headers.get(header::ETAG)
    );
    for name in [
        header::X_CONTENT_TYPE_OPTIONS,
        header::CONTENT_SECURITY_POLICY,
        header::CACHE_CONTROL,
    ] {
        assert_eq!(headers.get(&name), attachment_headers.get(&name), "{name}");
    }
    assert!(
        headers
            .get(header::CONTENT_DISPOSITION)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("inline")
    );
    assert_eq!(revalidated, StatusCode::NOT_MODIFIED);
    assert_eq!(stranger_status, StatusCode::NOT_FOUND);
    assert!(!remaining_files.contains(&unsent_preview.file_id), "purged");
    assert!(remaining_files.contains(&kept_preview.file_id));
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
            Some(serde_json::json!({"content": "", "attachment_ids": [uploaded.id.to_string()]})),
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

#[tokio::test]
async fn attachments_revalidate_without_a_slot_and_downloads_wait_for_one() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let stranger = registered_token(&state, "stranger").await;
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
            Some(serde_json::json!({"content": "", "attachment_ids": [uploaded.id.to_string()]})),
        ),
    )
    .await;
    let path = format!("/api/v1/attachments/{}", uploaded.id);
    let get = |token: &str, entity_tag: Option<&HeaderValue>| {
        let mut request = authorized("GET", &path, token, None);
        if let Some(entity_tag) = entity_tag {
            request
                .headers_mut()
                .insert(header::IF_NONE_MATCH, entity_tag.clone());
        }
        request
    };
    let ((_, _), first_headers) = send_from(&state, TEST_PEER, get(&owner, None)).await;
    let entity_tag = first_headers.get(header::ETAG).unwrap().clone();

    // Every slot held by a body not sent yet: revalidating needs none.
    let mut held = Vec::new();
    for _ in 0..state.attachments.free_download_slots() {
        held.push(
            router(state.clone())
                .layer(MockConnectInfo(TEST_PEER.parse::<SocketAddr>().unwrap()))
                .oneshot(get(&owner, None))
                .await
                .unwrap(),
        );
    }
    let ((revalidated, revalidated_body), revalidated_headers) =
        send_from(&state, TEST_PEER, get(&owner, Some(&entity_tag))).await;
    let (stranger_revalidated, _) = send(&state, get(&stranger, Some(&entity_tag))).await;
    let waiting = tokio::spawn({
        let state = state.clone();
        let request = get(&owner, None);
        async move { send(&state, request).await.0 }
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let was_waiting = !waiting.is_finished();
    held.pop();
    let after_a_slot_freed = waiting.await.unwrap();
    drop(held);
    database.close().await;

    assert_eq!(
        entity_tag,
        format!("\"{}-{}\"", uploaded.id, PNG.len()).as_str()
    );
    assert_eq!(revalidated, StatusCode::NOT_MODIFIED);
    assert!(revalidated_body.is_empty());
    assert_eq!(revalidated_headers.get(header::ETAG), Some(&entity_tag));
    assert_eq!(
        revalidated_headers.get(header::CACHE_CONTROL).unwrap(),
        "private, no-cache"
    );
    assert_eq!(
        stranger_revalidated,
        StatusCode::NOT_FOUND,
        "a matching tag never skips the permission check"
    );
    assert!(was_waiting, "a download waits for a slot, not 503");
    assert_eq!(after_a_slot_freed, StatusCode::OK);
}

#[tokio::test]
async fn videos_play_inline_and_media_players_get_byte_ranges() {
    use mellow_protocol::Attachment;

    const VIDEO: &[u8] = b"\0\0\0\x20ftypisom\0\0\x02\0not really a video";
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let (_, body) = send(
        &state,
        upload_request(
            &format!("/api/v1/channels/{general}/attachments?file_name=clip.mp4"),
            &owner,
            VIDEO,
        ),
    )
    .await;
    let video: Attachment = json(&body);
    let (status, body) = send(
        &state,
        authorized(
            "POST",
            &format!("/api/v1/channels/{general}/messages"),
            &owner,
            Some(serde_json::json!({"content": "", "attachment_ids": [video.id.to_string()]})),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let path = format!("/api/v1/attachments/{}", video.id);
    let ranged = |range: &str, if_range: Option<&str>| {
        let mut request = authorized("GET", &path, &owner, None);
        let headers = request.headers_mut();
        headers.insert(header::RANGE, HeaderValue::from_str(range).unwrap());
        if let Some(if_range) = if_range {
            headers.insert(header::IF_RANGE, HeaderValue::from_str(if_range).unwrap());
        }
        request
    };
    let ((ranged_status, ranged_bytes), ranged_headers) =
        send_from(&state, TEST_PEER, ranged("bytes=4-11", None)).await;
    let ((beyond_end_status, _), beyond_end_headers) =
        send_from(&state, TEST_PEER, ranged("bytes=999-", None)).await;
    let ((other_version_status, whole_file), _) =
        send_from(&state, TEST_PEER, ranged("bytes=4-11", Some("\"0-1\""))).await;
    database.close().await;

    assert_eq!(video.content_type, "video/mp4");
    assert_eq!(ranged_status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(&ranged_bytes[..], b"ftypisom");
    let total = VIDEO.len();
    assert_eq!(
        ranged_headers.get(header::CONTENT_RANGE).unwrap(),
        &format!("bytes 4-11/{total}")
    );
    assert_eq!(ranged_headers.get(header::ACCEPT_RANGES).unwrap(), "bytes");
    assert_eq!(
        ranged_headers.get(header::CONTENT_TYPE).unwrap(),
        "video/mp4"
    );
    assert!(
        ranged_headers
            .get(header::CONTENT_DISPOSITION)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("inline")
    );
    assert_eq!(beyond_end_status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        beyond_end_headers.get(header::CONTENT_RANGE).unwrap(),
        &format!("bytes */{total}")
    );
    assert_eq!(
        other_version_status,
        StatusCode::OK,
        "If-Range names another version"
    );
    assert_eq!(&whole_file[..], VIDEO);
}
