//! Role icons over HTTP (ADR-0050): who may set them, what they may be, who may see them.

use mellow_protocol::{
    ApiErrorBody, ApiErrorCode, AuditAction, AuditChangeKey, AuditLogEntry, Role, Snowflake,
};

use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::migrated_database;
use crate::domain::image_processing::tests::{animated_gif, png};
use crate::http::test_support::*;
use crate::http::*;

async fn create_role(state: &AppState, owner: &str, guild_id: Snowflake) -> Role {
    let (_, body) = send(
        state,
        authorized(
            "POST",
            &format!("/api/v1/guilds/{guild_id}/roles"),
            owner,
            Some(serde_json::json!({ "name": "Helpers" })),
        ),
    )
    .await;
    json(&body)
}

fn set_emoji(path: &str, token: &str, emoji: &str) -> axum::http::Request<axum::body::Body> {
    authorized(
        "PUT",
        &format!("{path}/emoji"),
        token,
        Some(serde_json::json!({ "unicode_emoji": emoji })),
    )
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One scenario: picture, emoji, removal, refusals, deletion.
async fn role_icons_are_pictures_or_emoji_set_by_role_managers_and_shown_to_members() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
    let stranger = registered_token(&state, "stranger").await;
    let role = create_role(&state, &owner, guild_id).await;
    let icon_path = format!("/api/v1/guilds/{guild_id}/roles/{}/icon", role.id);

    let (by_guest, _) = send(
        &state,
        picture_request("PUT", &icon_path, &guest, png(80, 80)),
    )
    .await;
    let (_, body) = send(
        &state,
        picture_request("PUT", &icon_path, &owner, animated_gif(80, 80, 3)),
    )
    .await;
    let pictured: Role = json(&body);
    let icon = pictured.icon.unwrap();
    let icon_url = format!("/api/v1/icons/{}", icon.id);
    let (seen_by_member, _) = send(&state, authorized("GET", &icon_url, &guest, None)).await;
    let (seen_by_stranger, _) = send(&state, authorized("GET", &icon_url, &stranger, None)).await;
    let (_, body) = send(&state, set_emoji(&icon_path, &owner, "\u{2764}\u{FE0F}")).await;
    let with_emoji: Role = json(&body);
    let (old_picture, _) = send(&state, authorized("GET", &icon_url, &guest, None)).await;
    let mut refusals = Vec::new();
    for emoji in [":123", "two words", "\u{1F525}\u{1F525}"] {
        let (status, body) = send(&state, set_emoji(&icon_path, &owner, emoji)).await;
        refusals.push((status, json::<ApiErrorBody>(&body).code));
    }
    let everyone_path = format!("/api/v1/guilds/{guild_id}/roles/{guild_id}/icon");
    let (status, body) = send(&state, set_emoji(&everyone_path, &owner, "\u{1F525}")).await;
    refusals.push((status, json::<ApiErrorBody>(&body).code));
    let (too_large, _) = send(
        &state,
        picture_request("PUT", &icon_path, &owner, vec![0; 256 * 1024 + 1]),
    )
    .await;
    let (_, body) = send(&state, authorized("DELETE", &icon_path, &owner, None)).await;
    let removed: Role = json(&body);
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
    let icon_changes: Vec<(String, String)> = json::<Vec<AuditLogEntry>>(&body)
        .into_iter()
        .filter(|entry| entry.action == AuditAction::RoleUpdate)
        .flat_map(|entry| entry.changes)
        .filter(|change| change.key == AuditChangeKey::Icon)
        .map(|change| (change.old_value, change.new_value))
        .collect();
    // A role deleted with its picture takes the picture's row with it.
    let (_, body) = send(
        &state,
        picture_request("PUT", &icon_path, &owner, png(8, 8)),
    )
    .await;
    let last_icon = json::<Role>(&body).icon.unwrap();
    send(
        &state,
        authorized(
            "DELETE",
            &format!("/api/v1/guilds/{guild_id}/roles/{}", role.id),
            &owner,
            None,
        ),
    )
    .await;
    let after_deletion =
        crate::storage::guild_images::find_guild_image(&state.database, last_icon.id)
            .await
            .unwrap();
    database.close().await;

    assert_eq!(by_guest, StatusCode::FORBIDDEN);
    assert!(icon.is_animated, "animated like an emoji");
    assert_eq!(pictured.unicode_emoji, None);
    assert_eq!(seen_by_member, StatusCode::OK);
    assert_eq!(seen_by_stranger, StatusCode::NOT_FOUND);
    assert_eq!(with_emoji.icon, None, "an emoji replaces the picture");
    assert_eq!(with_emoji.unicode_emoji.as_deref(), Some("\u{2764}"));
    assert_eq!(old_picture, StatusCode::NOT_FOUND);
    assert!(
        refusals
            .iter()
            .all(|(status, code)| *status == StatusCode::BAD_REQUEST
                && *code == ApiErrorCode::ValidationFailed),
        "{refusals:?}"
    );
    assert_eq!(too_large, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!((removed.icon, removed.unicode_emoji), (None, None));
    // Newest first: removal, emoji, picture.
    assert_eq!(
        icon_changes,
        [
            ("\u{2764}".to_owned(), String::new()),
            (icon.id.to_string(), "\u{2764}".to_owned()),
            (String::new(), icon.id.to_string()),
        ]
    );
    assert_eq!(after_deletion, None);
}
