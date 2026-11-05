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

#[tokio::test]
async fn a_body_keeping_its_permit_has_an_exact_length_and_frees_the_permit_once_dropped() {
    use http_body::Body as _;
    let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
    let permit = slots.clone().try_acquire_owned().unwrap();
    let body = body_keeping_permit(vec![7; 5], permit);
    // The exact length is what arms the throughput floor for avatars and sounds (ADR-0038).
    assert_eq!(body.size_hint().exact(), Some(5));
    assert_eq!(slots.available_permits(), 0);
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    assert_eq!(bytes.as_ref(), [7; 5]);
    assert_eq!(slots.available_permits(), 1);
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
    use super::{MAXIMUM_RANGE_RESPONSE_BYTES, RequestedRange::*, byte_range};
    assert_eq!(byte_range("bytes=0-", 10), Satisfiable(0..=9));
    assert_eq!(byte_range("bytes=2-4", 10), Satisfiable(2..=4));
    assert_eq!(byte_range("bytes=8-99", 10), Satisfiable(8..=9));
    assert_eq!(byte_range("bytes=-3", 10), Satisfiable(7..=9));
    assert_eq!(byte_range("bytes=-30", 10), Satisfiable(0..=9));
    assert_eq!(byte_range("bytes=10-", 10), Unsatisfiable);
    assert_eq!(byte_range("bytes=-0", 10), Unsatisfiable);
    assert_eq!(byte_range("bytes=0-", 0), Unsatisfiable);
    assert_eq!(byte_range("Bytes=2-4", 10), Satisfiable(2..=4));
    let large = 3 * MAXIMUM_RANGE_RESPONSE_BYTES;
    assert_eq!(
        byte_range("bytes=0-", large),
        Satisfiable(0..=MAXIMUM_RANGE_RESPONSE_BYTES - 1),
        "a player asks for the next part itself"
    );
    assert_eq!(
        byte_range("bytes=-10", large),
        Satisfiable(large - 10..=large - 1)
    );
    assert_eq!(
        byte_range(&format!("bytes=-{large}"), large),
        Satisfiable(0..=MAXIMUM_RANGE_RESPONSE_BYTES - 1)
    );
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

    // Every slot of the reader held by a body not sent yet: revalidating needs none.
    let mut held = Vec::new();
    for _ in 0..crate::domain::attachments::CONCURRENT_DOWNLOADS_PER_READER {
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
#[allow(clippy::too_many_lines)] // One scenario: upload, send, then every range answer.
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
    let ((whole_status, _), whole_headers) =
        send_from(&state, TEST_PEER, authorized("GET", &path, &owner, None)).await;
    let entity_tag = whole_headers.get(header::ETAG).unwrap().to_str().unwrap();
    let ((same_version_status, _), _) =
        send_from(&state, TEST_PEER, ranged("bytes=4-11", Some(entity_tag))).await;
    let mut revalidation = ranged("bytes=4-11", None);
    revalidation.headers_mut().insert(
        header::IF_NONE_MATCH,
        HeaderValue::from_str(entity_tag).unwrap(),
    );
    let ((revalidation_status, _), _) = send_from(&state, TEST_PEER, revalidation).await;
    database.close().await;

    assert_eq!(whole_status, StatusCode::OK);
    assert_eq!(
        whole_headers.get(header::CONTENT_LENGTH).unwrap(),
        &VIDEO.len().to_string()
    );
    assert_eq!(same_version_status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        revalidation_status,
        StatusCode::NOT_MODIFIED,
        "a revalidation wins over a range"
    );

    assert_eq!(video.content_type, "video/mp4");
    assert_eq!(ranged_status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(&ranged_bytes[..], b"ftypisom");
    let total = VIDEO.len();
    assert_eq!(
        ranged_headers.get(header::CONTENT_RANGE).unwrap(),
        &format!("bytes 4-11/{total}")
    );
    assert_eq!(ranged_headers.get(header::ACCEPT_RANGES).unwrap(), "bytes");
    assert_eq!(ranged_headers.get(header::CONTENT_LENGTH).unwrap(), "8");
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

fn mp4_box(kind: &[u8], payload: &[u8]) -> Vec<u8> {
    let size = u32::try_from(payload.len() + 8).unwrap();
    [&size.to_be_bytes()[..], kind, payload].concat()
}

/// A phone's portrait video: a 1920 × 1080 track turned 90°, filmed at `place`.
fn portrait_video_filmed_at(place: &[u8]) -> &'static [u8] {
    let mut track_header = vec![0; 24 + 16];
    for word in [0, 0x0001_0000, 0, 0xFFFF_0000_u32, 0, 0, 0, 0, 0x4000_0000] {
        track_header.extend(word.to_be_bytes());
    }
    track_header.extend((1920_u32 << 16).to_be_bytes());
    track_header.extend((1080_u32 << 16).to_be_bytes());
    let handler = mp4_box(b"hdlr", &[&[0; 8][..], b"vide", &[0; 12]].concat());
    let track = mp4_box(
        b"trak",
        &[mp4_box(b"tkhd", &track_header), mp4_box(b"mdia", &handler)].concat(),
    );
    let place = mp4_box(
        b"udta",
        &mp4_box(b"\xa9xyz", &[&[0, 18, 0, 0][..], place].concat()),
    );
    let file = [
        mp4_box(b"ftyp", b"mp42\0\0\0\0isommp42"),
        mp4_box(b"moov", &[track, place].concat()),
        mp4_box(b"mdat", b"frames"),
    ]
    .concat();
    file.leak()
}

#[tokio::test]
async fn a_portrait_video_keeps_its_shape_and_loses_its_place_before_anyone_downloads_it() {
    use mellow_protocol::Attachment;

    const PLACE: &[u8] = b"+48.8584+002.2945/";
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let upload_path = format!("/api/v1/channels/{general}/attachments?file_name=clip.mp4");
    let video_bytes = portrait_video_filmed_at(PLACE);
    let (_, body) = send(&state, upload_request(&upload_path, &owner, video_bytes)).await;
    let video: Attachment = json(&body);
    let mut malformed = video_bytes.to_vec();
    // The `trak` box claims more than its `moov` holds.
    let track_at = malformed.windows(4).position(|w| w == b"trak").unwrap() - 4;
    malformed[track_at..track_at + 4].copy_from_slice(&0x7FFF_u32.to_be_bytes());
    let (malformed_status, _) = send(
        &state,
        upload_request(&upload_path, &owner, malformed.leak()),
    )
    .await;
    let (status, _) = send(
        &state,
        authorized(
            "POST",
            &format!("/api/v1/channels/{general}/messages"),
            &owner,
            Some(serde_json::json!({"content": "", "attachment_ids": [video.id.to_string()]})),
        ),
    )
    .await;
    let ((download_status, served), _) = send_from(
        &state,
        TEST_PEER,
        authorized(
            "GET",
            &format!("/api/v1/attachments/{}", video.id),
            &owner,
            None,
        ),
    )
    .await;
    database.close().await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(video.content_type, "video/mp4");
    assert_eq!((video.width, video.height), (Some(1080), Some(1920)));
    assert_eq!(download_status, StatusCode::OK);
    assert_eq!(served.len(), video_bytes.len());
    assert!(!served.windows(PLACE.len()).any(|w| w == PLACE));
    assert!(!served.windows(4).any(|w| w == b"udta"));
    assert_eq!(
        malformed_status,
        StatusCode::BAD_REQUEST,
        "boxes that cannot be walked may hide a place"
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One scenario: forward, compress the original, delete it, purge.
async fn a_forward_shares_the_original_files_without_quota_and_keeps_them_once_it_is_deleted() {
    use mellow_protocol::{Attachment, Message};

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let stranger = registered_token(&state, "stranger").await;
    let upload_path = format!("/api/v1/channels/{general}/attachments?file_name=a.png");
    let (_, body) = send(&state, upload_request(&upload_path, &owner, PNG)).await;
    let uploaded: Attachment = json(&body);
    let messages_path = format!("/api/v1/channels/{general}/messages");
    let (_, body) = send(
        &state,
        authorized(
            "POST",
            &messages_path,
            &owner,
            Some(serde_json::json!({"content": "", "attachment_ids": [uploaded.id.to_string()]})),
        ),
    )
    .await;
    let original: Message = json(&body);
    let forward_body = serde_json::json!({
        "content": "",
        "forwarded_from": {"channel_id": general.to_string(), "message_id": original.id.to_string()},
    });
    let (stranger_status, _) = send(
        &state,
        authorized(
            "POST",
            &messages_path,
            &stranger,
            Some(forward_body.clone()),
        ),
    )
    .await;
    let (status, body) = send(
        &state,
        authorized("POST", &messages_path, &owner, Some(forward_body)),
    )
    .await;
    let forward: Message = json(&body);
    let copy = forward.attachments[0].clone();
    let owner_id = json::<CurrentUser>(
        &send(&state, authorized("GET", "/api/v1/users/@me", &owner, None))
            .await
            .1,
    )
    .id;
    let quota_bytes = crate::storage::attachments::stored_bytes_of_uploader(&database, owner_id)
        .await
        .unwrap();
    // A smaller file replacing the original's (ADR-0033) reaches the copy in the same statement.
    let stored = crate::storage::attachments::find_attachment(&database, uploaded.id)
        .await
        .unwrap()
        .unwrap();
    let replacement = Snowflake::from_raw(stored.file_id.raw() + 1);
    state
        .attachments
        .write_file(replacement, PNG.to_vec())
        .await;
    let moved = crate::storage::attachments::update_attachment_file(
        &database,
        stored.file_id,
        replacement,
        "a.png",
        "image/png",
        PNG.len() as u64,
    )
    .await
    .unwrap();
    let preview = crate::storage::attachments::StoredPreview {
        file_id: Snowflake::from_raw(replacement.raw() + 1),
        content_type: "image/webp".into(),
        size_bytes: 1,
    };
    crate::storage::attachments::record_attachment_preview(&database, uploaded.id, &preview)
        .await
        .unwrap();
    let stored_copy = crate::storage::attachments::find_attachment(&database, copy.id)
        .await
        .unwrap()
        .unwrap();
    let copy_file_id = stored_copy.file_id;
    let (deleted, _) = send(
        &state,
        authorized(
            "DELETE",
            &format!("{messages_path}/{}", original.id),
            &owner,
            None,
        ),
    )
    .await;
    state
        .attachments
        .purge(
            crate::domain::clock::now_unix_milliseconds()
                + crate::domain::attachments::UNSENT_UPLOAD_LIFETIME_MILLISECONDS * 2,
        )
        .await
        .unwrap();
    // Deleting the original must not free the quota while a copy keeps its file on disk.
    let quota_bytes_after_delete =
        crate::storage::attachments::stored_bytes_of_uploader(&database, owner_id)
            .await
            .unwrap();
    let (download_status, downloaded) = send(
        &state,
        authorized(
            "GET",
            &format!("/api/v1/attachments/{}", copy.id),
            &owner,
            None,
        ),
    )
    .await;
    database.close().await;

    assert_eq!(stranger_status, StatusCode::NOT_FOUND);
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(forward.content, "");
    assert_eq!(
        forward
            .forwarded_from
            .map(|origin| (origin.channel_id, origin.message_id)),
        Some((general, original.id))
    );
    assert_ne!(copy.id, uploaded.id);
    assert_eq!(
        (copy.file_name, copy.size_bytes),
        (uploaded.file_name, uploaded.size_bytes)
    );
    assert_eq!(quota_bytes, PNG.len() as u64, "the copy takes no quota");
    assert!(moved);
    assert_eq!(copy_file_id, replacement);
    assert_eq!(
        stored_copy.preview,
        Some(preview),
        "the preview reaches the copy"
    );
    assert_eq!(deleted, StatusCode::NO_CONTENT);
    assert_eq!(
        quota_bytes_after_delete,
        PNG.len() as u64,
        "a copy takes over the quota"
    );
    assert_eq!(download_status, StatusCode::OK);
    assert_eq!(downloaded, PNG);
}

/// A site on this machine for link previews: a page with Open Graph tags and a picture, a
/// redirect into a private network, plain text; it counts the page's fetches.
pub(super) async fn local_site() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use axum::response::IntoResponse as _;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let page_fetches = std::sync::Arc::new(AtomicUsize::new(0));
    let counter = page_fetches.clone();
    let site = axum::Router::new()
        .route(
            "/page",
            axum::routing::get(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async {
                    (
                        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                        r##"<html><head><title>Plain</title>
                        <meta property="og:title" content="Local &amp; page">
                        <meta property="og:description" content="What it says">
                        <meta property="og:site_name" content="Local">
                        <meta property="og:image" content="/picture.png">
                        <meta name="theme-color" content="#3366cc"></head><body></body></html>"##,
                    )
                }
            }),
        )
        .route(
            "/picture.png",
            axum::routing::get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/png")],
                    crate::domain::image_processing::tests::png(120, 90),
                )
                    .into_response()
            }),
        )
        .route(
            "/private",
            axum::routing::get(|| async {
                (
                    StatusCode::FOUND,
                    [(header::LOCATION, "http://10.0.0.1/admin")],
                )
            }),
        )
        .route(
            "/text",
            axum::routing::get(|| async { ([(header::CONTENT_TYPE, "text/plain")], "plain") }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, site).await });
    (format!("http://{address}"), page_fetches)
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One scenario: fetch, publish, serve, cache, edit away.
async fn link_previews_are_fetched_once_shown_in_history_and_served_to_readers_only() {
    use mellow_protocol::Message;
    use std::sync::atomic::Ordering;

    use crate::domain::link_previews::fetcher::LinkFetcher;
    use crate::domain::messages::fetch_queued_link_previews;

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let stranger = registered_token(&state, "stranger").await;
    let (site, page_fetches) = local_site().await;
    let fetcher = LinkFetcher::allowing_loopback_for_tests();
    state.messages.enable_link_previews();
    let messages_path = format!("/api/v1/channels/{general}/messages");
    let post = |content: String| {
        authorized(
            "POST",
            &messages_path,
            &owner,
            Some(serde_json::json!({ "content": content })),
        )
    };
    let (status, body) = send(
        &state,
        post(format!(
            "see {site}/page, {site}/private and {site}/text, not <{site}/hidden>"
        )),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let sent: Message = json(&body);
    assert!(sent.link_previews.is_empty(), "fetched after the answer");
    fetch_queued_link_previews(&state.messages, &state.attachments, &fetcher).await;
    let (_, body) = send(&state, authorized("GET", &messages_path, &owner, None)).await;
    let history: Vec<Message> = json(&body);
    let previews = &history[0].link_previews;
    assert_eq!(
        previews.len(),
        1,
        "a private redirect and plain text give none"
    );
    let preview = &previews[0];
    assert_eq!(preview.url, format!("{site}/page"));
    assert_eq!(preview.title.as_deref(), Some("Local & page"));
    assert_eq!(preview.description.as_deref(), Some("What it says"));
    assert_eq!(preview.site_name.as_deref(), Some("Local"));
    assert_eq!(preview.theme_color, Some(0x0033_66cc));
    let image = preview.image.unwrap();
    assert_eq!((image.width, image.height), (120, 90));

    let image_path = format!(
        "/api/v1/channels/{general}/messages/{}/link-previews/{}/image",
        sent.id, preview.id
    );
    let ((status, bytes), headers) = send_from(
        &state,
        TEST_PEER,
        authorized("GET", &image_path, &owner, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("image/")
    );
    assert!(!bytes.is_empty());
    let entity_tag = headers[header::ETAG].clone();
    let mut revalidation = authorized("GET", &image_path, &owner, None);
    revalidation
        .headers_mut()
        .insert(header::IF_NONE_MATCH, entity_tag);
    let (status, _) = send(&state, revalidation).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    let (status, _) = send(&state, authorized("GET", &image_path, &stranger, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "only readers of the channel");

    let (_, body) = send(&state, post(format!("again {site}/page"))).await;
    let again: Message = json(&body);
    fetch_queued_link_previews(&state.messages, &state.attachments, &fetcher).await;
    assert_eq!(
        page_fetches.load(Ordering::SeqCst),
        1,
        "a fresh preview is reused"
    );

    let (status, _) = send(
        &state,
        authorized(
            "PATCH",
            &format!("{messages_path}/{}", again.id),
            &owner,
            Some(serde_json::json!({ "content": "no link any more" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    fetch_queued_link_previews(&state.messages, &state.attachments, &fetcher).await;
    let (_, body) = send(&state, authorized("GET", &messages_path, &owner, None)).await;
    let history: Vec<Message> = json(&body);
    database.close().await;

    let edited = history
        .iter()
        .find(|message| message.id == again.id)
        .unwrap();
    assert!(
        edited.link_previews.is_empty(),
        "an edit drops the previews"
    );
    assert_eq!(
        history
            .iter()
            .find(|message| message.id == sent.id)
            .unwrap()
            .link_previews
            .len(),
        1
    );
}
