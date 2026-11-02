use mellow_protocol::{CurrentUser, Snowflake};

use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::migrated_database;
use crate::http::test_support::*;
use crate::http::*;

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
    let (member_read, animated) = send(&state, authorized("GET", &icon_path, &guest, None)).await;
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
    let (after_sweep, _) = send(&state, authorized("GET", &channel_icon_path, &owner, None)).await;
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
            None,
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
                crate::storage::attachments::newest_attachments_awaiting_optimization(&database, 1)
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
async fn pictures_stored_before_adr_0033_are_recompressed_one_by_one_and_resume_after_a_restart() {
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
    let new_channel_icon = crate::storage::guild_channels::find_guild_channel(&database, general)
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

#[tokio::test]
async fn avatars_and_banners_keep_the_part_chosen_in_the_crop_dialog() {
    use crate::domain::image_processing::tests::{png, stored_dimensions};

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let owner = registered_token(&state, "owner").await;
    let upload = |path: &str| picture_request("PUT", path, &owner, png(1_200, 800));
    let stored = |user: CurrentUser, banner: bool| {
        let image = if banner { user.banner } else { user.avatar };
        format!("/api/v1/profile-images/{}", image.unwrap().id)
    };

    let (half_avatar_crop, _) =
        send(&state, upload("/api/v1/users/@me/avatar?crop_x=0&crop_y=0")).await;
    let (half_banner_crop, _) =
        send(&state, upload("/api/v1/users/@me/banner?crop_width=500")).await;
    let (square_banner_crop, _) = send(
        &state,
        upload("/api/v1/users/@me/banner?crop_x=0&crop_y=0&crop_size=500"),
    )
    .await;
    let (status, body) = send(
        &state,
        upload("/api/v1/users/@me/avatar?crop_x=100&crop_y=50&crop_size=600"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let (_, avatar) = send(
        &state,
        authorized("GET", &stored(json(&body), false), &owner, None),
    )
    .await;
    let (status, body) = send(
        &state,
        upload("/api/v1/users/@me/banner?crop_x=0&crop_y=300&crop_width=500"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let (_, banner) = send(
        &state,
        authorized("GET", &stored(json(&body), true), &owner, None),
    )
    .await;
    database.close().await;

    assert_eq!(half_avatar_crop, StatusCode::BAD_REQUEST);
    assert_eq!(half_banner_crop, StatusCode::BAD_REQUEST);
    assert_eq!(
        square_banner_crop,
        StatusCode::BAD_REQUEST,
        "a banner's crop is a width"
    );
    assert_eq!(stored_dimensions(&avatar), (256, 256));
    assert_eq!(
        stored_dimensions(&banner),
        (500, 200),
        "5:2, never scaled up"
    );
}
