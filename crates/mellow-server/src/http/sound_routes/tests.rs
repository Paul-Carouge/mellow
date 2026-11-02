use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::migrated_database;
use crate::http::test_support::*;
use crate::http::*;

#[tokio::test]
#[allow(clippy::too_many_lines)] // One scenario: upload, serve, choose, edit, delete.
async fn sounds_are_uploaded_served_to_members_as_wav_and_chosen_as_join_sounds_over_http() {
    use mellow_protocol::{Guild, SoundboardSound};

    use crate::domain::sound_rules::MAXIMUM_SOUND_FILE_BYTES;
    use crate::domain::sound_rules::tests::canonical_wav;

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
    let stranger = registered_token(&state, "stranger").await;
    let upload_path = format!("/api/v1/guilds/{guild_id}/sounds?name=Horn&emoji=%F0%9F%93%AF");
    let (by_guest, _) = send(
        &state,
        picture_request("POST", &upload_path, &guest, canonical_wav(480)),
    )
    .await;
    let (too_large, _) = send(
        &state,
        picture_request(
            "POST",
            &upload_path,
            &owner,
            vec![0; MAXIMUM_SOUND_FILE_BYTES + 1],
        ),
    )
    .await;
    let (status, body) = send(
        &state,
        picture_request("POST", &upload_path, &owner, canonical_wav(480)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let sound: SoundboardSound = json(&body);
    let sound_path = format!("/api/v1/sounds/{}", sound.id);
    let ((download_status, download), headers) = send_from(
        &state,
        TEST_PEER,
        authorized("GET", &sound_path, &guest, None),
    )
    .await;
    let (stranger_download, _) =
        send(&state, authorized("GET", &sound_path, &stranger, None)).await;
    let join_sound_path = format!("/api/v1/guilds/{guild_id}/members/@me/join-sound");
    let (chosen, _) = send(
        &state,
        authorized(
            "PATCH",
            &join_sound_path,
            &guest,
            Some(serde_json::json!({ "sound_id": sound.id.to_string() })),
        ),
    )
    .await;
    let (_, body) = send(
        &state,
        authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &guest, None),
    )
    .await;
    let guild: Guild = json(&body);
    let (renamed, body) = send(
        &state,
        authorized(
            "PATCH",
            &format!("/api/v1/guilds/{guild_id}/sounds/{}", sound.id),
            &owner,
            Some(serde_json::json!({ "name": "Big horn", "emoji": null })),
        ),
    )
    .await;
    let renamed_sound: SoundboardSound = json(&body);
    let (deleted, _) = send(
        &state,
        authorized(
            "DELETE",
            &format!("/api/v1/guilds/{guild_id}/sounds/{}", sound.id),
            &owner,
            None,
        ),
    )
    .await;
    let (_, body) = send(
        &state,
        authorized(
            "GET",
            &format!("/api/v1/guilds/{guild_id}/sounds"),
            &guest,
            None,
        ),
    )
    .await;
    let remaining: Vec<SoundboardSound> = json(&body);
    database.close().await;

    assert_eq!(by_guest, StatusCode::FORBIDDEN);
    assert_eq!(too_large, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        (sound.name.as_str(), sound.emoji.as_deref()),
        ("Horn", Some("📯"))
    );
    assert_eq!(download_status, StatusCode::OK);
    assert_eq!(download.as_ref(), canonical_wav(480).as_slice());
    assert_eq!(headers[header::CONTENT_TYPE], "audio/wav");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(stranger_download, StatusCode::NOT_FOUND);
    assert_eq!(chosen, StatusCode::NO_CONTENT);
    assert_eq!(guild.my_join_sound_id, Some(sound.id));
    assert_eq!(guild.sounds, std::slice::from_ref(&sound));
    assert_eq!(renamed, StatusCode::OK);
    assert_eq!(
        (renamed_sound.name.as_str(), renamed_sound.emoji),
        ("Big horn", None)
    );
    assert_eq!(deleted, StatusCode::NO_CONTENT);
    assert_eq!(remaining, []);
}
