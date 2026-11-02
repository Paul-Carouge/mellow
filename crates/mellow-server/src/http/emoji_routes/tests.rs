use mellow_protocol::{ApiErrorBody, ApiErrorCode};

use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::migrated_database;
use crate::http::test_support::*;
use crate::http::*;

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
    let (over_budget, _) = send(&state, picture_request("POST", &path, &guest, png(8, 8))).await;
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
    let (deleted_again, _) = send(&state, authorized("DELETE", &party_path, &owner, None)).await;
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
