use mellow_protocol::{ApiErrorBody, ApiErrorCode, Snowflake};

use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::migrated_database;
use crate::http::test_support::*;
use crate::http::*;

#[tokio::test]
async fn guild_lifecycle_over_http() {
    use mellow_protocol::{Guild, GuildSummary, Invite, Member, Permissions};

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let owner = registered_token(&state, "owner").await;
    let guest = registered_token(&state, "guest").await;

    let (status, body) = send(
        &state,
        authorized(
            "POST",
            "/api/v1/guilds",
            &owner,
            Some(serde_json::json!({"name": "Mellow Dev"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let guild: Guild = json(&body);
    let guild_path = format!("/api/v1/guilds/{}", guild.id);

    let (status, body) = send(&state, authorized("GET", &guild_path, &guest, None)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "outsiders cannot see the guild"
    );
    assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::NotFound);

    let (status, body) = send(
        &state,
        authorized(
            "POST",
            &format!("{guild_path}/invites"),
            &owner,
            Some(serde_json::json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let invite: Invite = json(&body);

    let (status, body) = send(
        &state,
        authorized(
            "POST",
            &format!("/api/v1/invites/{}", invite.code),
            &guest,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json::<Guild>(&body).my_permissions,
        Permissions::DEFAULT_EVERYONE
    );

    let (status, body) = send(
        &state,
        authorized(
            "POST",
            &format!("{guild_path}/channels"),
            &guest,
            Some(serde_json::json!({"kind": "text", "name": "nope", "parent_id": null})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        json::<ApiErrorBody>(&body).code,
        ApiErrorCode::MissingPermissions
    );

    let (_, body) = send(
        &state,
        authorized("GET", &format!("{guild_path}/members"), &guest, None),
    )
    .await;
    assert_eq!(json::<Vec<Member>>(&body).len(), 2);
    let (_, body) = send(
        &state,
        authorized("GET", "/api/v1/users/@me/guilds", &guest, None),
    )
    .await;
    assert_eq!(json::<Vec<GuildSummary>>(&body)[0].name, "Mellow Dev");

    let (status, body) = send(
        &state,
        authorized("GET", "/api/v1/guilds/not-an-id", &owner, None),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        json::<ApiErrorBody>(&body).code,
        ApiErrorCode::ValidationFailed
    );

    let (status, _) = send(
        &state,
        authorized("GET", &guild_path, "mlw_not-a-real-token", None),
    )
    .await;
    database.close().await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn out_of_range_ids_are_not_found_and_invite_guessing_is_rate_limited() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let owner = registered_token(&state, "owner").await;
    let guest = registered_token(&state, "guest").await;

    let (status, _) = send(
        &state,
        authorized("GET", "/api/v1/guilds/18446744073709551615", &owner, None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "ids beyond SQLite's range are simply not found"
    );

    for _ in 0..rate_limit::INVITE_ATTEMPTS_PER_ADDRESS.maximum_events {
        send(
            &state,
            authorized("POST", "/api/v1/invites/guessing", &guest, None),
        )
        .await;
    }
    let (status, _) = send(
        &state,
        authorized("POST", "/api/v1/invites/guessing", &guest, None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "invite codes cannot be brute forced"
    );
    database.close().await;
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One scenario: create, list, edit, assign, delete roles.
async fn roles_over_http() {
    use mellow_protocol::{Guild, Permissions, Role};

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, guest_id) = guild_with_guest(&state).await;
    let guild_path = format!("/api/v1/guilds/{guild_id}");

    let kick = Permissions::KICK_MEMBERS.bits().to_string();
    let (status, body) = send(
        &state,
        authorized(
            "POST",
            &format!("{guild_path}/roles"),
            &owner,
            Some(
                serde_json::json!({"name": "Moderator", "color": 0x2f_a8_6b, "permissions": kick}),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let role: Role = json(&body);
    assert_eq!((role.position, role.color), (1, 0x2f_a8_6b));
    let role_path = format!("{guild_path}/roles/{}", role.id);

    let (status, _) = send(
        &state,
        authorized(
            "PUT",
            &format!("{guild_path}/members/{guest_id}/roles/{}", role.id),
            &owner,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, body) = send(&state, authorized("GET", &guild_path, &guest, None)).await;
    let seen_by_guest: Guild = json(&body);
    assert_eq!(seen_by_guest.my_role_ids, vec![role.id]);
    assert!(
        seen_by_guest
            .my_permissions
            .contains(Permissions::KICK_MEMBERS)
    );
    assert_eq!(seen_by_guest.roles.len(), 2);

    let (status, body) = send(
        &state,
        authorized(
            "PATCH",
            &role_path,
            &owner,
            Some(serde_json::json!({"name": "Mods"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json::<Role>(&body).name, "Mods");

    let unknown_bit = (1_u64 << 63).to_string();
    let roles_path = format!("{guild_path}/roles");
    let everyone_path = format!("{guild_path}/roles/{guild_id}");
    for (method, path, body, expected_status) in [
        (
            "POST",
            &roles_path,
            Some(serde_json::json!({"name": "x", "permissions": unknown_bit})),
            StatusCode::BAD_REQUEST,
        ),
        (
            "POST",
            &roles_path,
            Some(serde_json::json!({"name": "x", "hoist": true})),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("DELETE", &everyone_path, None, StatusCode::BAD_REQUEST),
    ] {
        let (status, response) = send(&state, authorized(method, path, &owner, body)).await;
        assert_eq!(status, expected_status, "{method} {path}");
        assert_eq!(
            json::<ApiErrorBody>(&response).code,
            ApiErrorCode::ValidationFailed
        );
    }
    let (status, body) = send(&state, authorized("DELETE", &role_path, &guest, None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        json::<ApiErrorBody>(&body).code,
        ApiErrorCode::MissingPermissions
    );

    let (status, _) = send(
        &state,
        authorized(
            "DELETE",
            &format!("{guild_path}/members/{guest_id}/roles/{}", role.id),
            &owner,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(&state, authorized("DELETE", &role_path, &owner, None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(&state, authorized("DELETE", &role_path, &owner, None)).await;
    database.close().await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn only_instance_administrators_create_servers_when_the_instance_says_so() {
    let (_directory, database) = migrated_database().await;
    let mut state = test_state(database.clone(), RegistrationMode::Open);
    state.guilds = Arc::new(Guilds::new(
        database.clone(),
        Arc::new(SnowflakeGenerator::default()),
        Arc::new(EventHub::default()),
        crate::domain::account_rules::GuildCreationPolicy::Administrators,
    ));
    let member = registered_token(&state, "member").await;
    let administrator = registered_token(&state, "admin").await;
    sqlx::query("UPDATE users SET is_instance_administrator = 1 WHERE username = 'admin'")
        .execute(database.writer())
        .await
        .unwrap();
    let create = |token: String| {
        let state = state.clone();
        async move {
            send(
                &state,
                authorized(
                    "POST",
                    "/api/v1/guilds",
                    &token,
                    Some(serde_json::json!({"name": "Club"})),
                ),
            )
            .await
            .0
        }
    };

    let by_member = create(member).await;
    let by_administrator = create(administrator).await;
    database.close().await;

    assert_eq!(by_member, StatusCode::FORBIDDEN);
    assert_eq!(by_administrator, StatusCode::CREATED);
}

#[tokio::test]
async fn channels_are_reordered_over_http() {
    use mellow_protocol::Channel;

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
    let (_, body) = send(
        &state,
        authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &owner, None),
    )
    .await;
    let channels = json::<mellow_protocol::Guild>(&body).channels;
    let [category, general] = [channels[0].id, channels[1].id];
    let order_path = format!("/api/v1/guilds/{guild_id}/channels/order");
    // `general` leaves its category and moves above it.
    let order = serde_json::json!({"channels": [
        {"id": general, "parent_id": null},
        {"id": category, "parent_id": null},
    ]});
    let (status, body) = send(
        &state,
        authorized("PUT", &order_path, &owner, Some(order.clone())),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let placed: Vec<(Snowflake, Option<Snowflake>)> = json::<Vec<Channel>>(&body)
        .into_iter()
        .map(|channel| (channel.id, channel.parent_id))
        .collect();
    assert_eq!(placed, [(general, None), (category, None)]);

    let (status, _) = send(&state, authorized("PUT", &order_path, &guest, Some(order))).await;
    database.close().await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn roles_are_reordered_over_http() {
    use mellow_protocol::Role;

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
    let roles_path = format!("/api/v1/guilds/{guild_id}/roles");
    let mut created = Vec::new();
    for name in ["first", "second"] {
        let (_, body) = send(
            &state,
            authorized(
                "POST",
                &roles_path,
                &owner,
                Some(serde_json::json!({"name": name})),
            ),
        )
        .await;
        created.push(json::<Role>(&body).id);
    }
    let order_path = format!("{roles_path}/order");
    // Created last, "second" is lowest: put "first" back below it.
    let order = serde_json::json!({"role_ids": [created[0], created[1]]});
    let (status, body) = send(
        &state,
        authorized("PUT", &order_path, &owner, Some(order.clone())),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<String> = json::<Vec<Role>>(&body)
        .into_iter()
        .map(|role| role.name)
        .collect();
    assert_eq!(names, ["@everyone", "first", "second"]);

    let (status, body) = send(&state, authorized("PUT", &order_path, &guest, Some(order))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        json::<ApiErrorBody>(&body).code,
        ApiErrorCode::MissingPermissions
    );
    let (status, body) = send(
        &state,
        authorized(
            "PUT",
            &order_path,
            &owner,
            Some(serde_json::json!({"role_ids": [created[0]]})),
        ),
    )
    .await;
    database.close().await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        json::<ApiErrorBody>(&body).code,
        ApiErrorCode::ValidationFailed
    );
}

#[tokio::test]
async fn overwrites_hide_a_channel_over_http() {
    use mellow_protocol::{Channel, Guild, Message, ReadState};

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
    let (_, body) = send(
        &state,
        authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &owner, None),
    )
    .await;
    let general = json::<Guild>(&body).channels[1].id;
    let messages_path = format!("/api/v1/channels/{general}/messages");
    let (_, body) = send(
        &state,
        authorized(
            "POST",
            &messages_path,
            &owner,
            Some(serde_json::json!({"content": "hello"})),
        ),
    )
    .await;
    let message_id = json::<Message>(&body).id;
    send(
        &state,
        authorized(
            "PUT",
            &format!("/api/v1/channels/{general}/read-state"),
            &guest,
            Some(serde_json::json!({"last_read_message_id": message_id})),
        ),
    )
    .await;

    let (_, body) = send(
        &state,
        authorized("GET", "/api/v1/users/@me/read-states", &guest, None),
    )
    .await;
    assert_eq!(json::<Vec<ReadState>>(&body).len(), 1, "read before hiding");

    let overwrite_path = format!("/api/v1/channels/{general}/permissions/{guild_id}");
    let view = mellow_protocol::Permissions::VIEW_CHANNEL
        .bits()
        .to_string();
    let (status, body) = send(
        &state,
        authorized(
            "PUT",
            &overwrite_path,
            &owner,
            Some(serde_json::json!({"kind": "role", "deny": view})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json::<Channel>(&body).permission_overwrites.len(), 1);

    let (status, _) = send(&state, authorized("GET", &messages_path, &guest, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, body) = send(
        &state,
        authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &guest, None),
    )
    .await;
    assert!(
        json::<Guild>(&body)
            .channels
            .iter()
            .all(|channel| channel.id != general)
    );
    let (_, body) = send(
        &state,
        authorized("GET", "/api/v1/users/@me/read-states", &guest, None),
    )
    .await;
    assert_eq!(json::<Vec<ReadState>>(&body), Vec::new());

    let (status, body) = send(
        &state,
        authorized(
            "PUT",
            &overwrite_path,
            &owner,
            Some(serde_json::json!({"kind": "role", "allow": view, "deny": view})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        json::<ApiErrorBody>(&body).code,
        ApiErrorCode::ValidationFailed
    );

    let (status, _) = send(&state, authorized("DELETE", &overwrite_path, &owner, None)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(&state, authorized("GET", &messages_path, &guest, None)).await;
    database.close().await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn kicks_and_bans_over_http() {
    use mellow_protocol::{Ban, Invite};

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, guest_id) = guild_with_guest(&state).await;
    let guild_path = format!("/api/v1/guilds/{guild_id}");
    let (status, _) = send(
        &state,
        authorized(
            "DELETE",
            &format!("{guild_path}/members/{guest_id}"),
            &owner,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let ban_path = format!("{guild_path}/bans/{guest_id}");
    let (status, _) = send(
        &state,
        authorized(
            "PUT",
            &ban_path,
            &owner,
            Some(serde_json::json!({"reason": "spam"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, body) = send(
        &state,
        authorized("GET", &format!("{guild_path}/bans"), &owner, None),
    )
    .await;
    assert_eq!(json::<Vec<Ban>>(&body)[0].reason, "spam");

    let (_, body) = send(
        &state,
        authorized(
            "POST",
            &format!("{guild_path}/invites"),
            &owner,
            Some(serde_json::json!({})),
        ),
    )
    .await;
    let invite_path = format!("/api/v1/invites/{}", json::<Invite>(&body).code);
    let (status, body) = send(&state, authorized("POST", &invite_path, &guest, None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::Banned);

    let (status, _) = send(&state, authorized("DELETE", &ban_path, &owner, None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(&state, authorized("POST", &invite_path, &guest, None)).await;
    database.close().await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_audit_log_over_http() {
    use mellow_protocol::{AuditAction, AuditLogEntry};

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, guest_id) = guild_with_guest(&state).await;
    let guild_path = format!("/api/v1/guilds/{guild_id}");
    send(
        &state,
        authorized(
            "DELETE",
            &format!("{guild_path}/members/{guest_id}"),
            &owner,
            None,
        ),
    )
    .await;
    let (status, body) = send(
        &state,
        authorized(
            "GET",
            &format!("{guild_path}/audit-log?limit=10"),
            &owner,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entries: Vec<AuditLogEntry> = json(&body);
    assert_eq!(entries[0].action, AuditAction::MemberKick);
    assert_eq!(entries[0].target_id, Some(guest_id));
    let (status, _) = send(
        &state,
        authorized("GET", &format!("{guild_path}/audit-log"), &guest, None),
    )
    .await;
    let (bad_status, _) = send(
        &state,
        authorized(
            "GET",
            &format!("{guild_path}/audit-log?limit=101"),
            &owner,
            None,
        ),
    )
    .await;
    database.close().await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the kicked guest is no longer a member"
    );
    assert_eq!(bad_status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn server_settings_over_http() {
    use mellow_protocol::{Guild, GuildInvite};

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
    let guild_path = format!("/api/v1/guilds/{guild_id}");
    let invites_path = format!("{guild_path}/invites");
    let rename = || Some(serde_json::json!({"name": "Book club"}));

    let (guest_rename, body) =
        send(&state, authorized("PATCH", &guild_path, &guest, rename())).await;
    let guest_rename_code = json::<ApiErrorBody>(&body).code;
    let (unknown_rename, _) = send(
        &state,
        authorized("PATCH", "/api/v1/guilds/42", &owner, rename()),
    )
    .await;
    let (unknown_field, _) = send(
        &state,
        authorized(
            "PATCH",
            &guild_path,
            &owner,
            Some(serde_json::json!({"icon": "x"})),
        ),
    )
    .await;
    let (_, body) = send(&state, authorized("PATCH", &guild_path, &owner, rename())).await;
    let renamed: Guild = json(&body);

    let (guest_list, _) = send(&state, authorized("GET", &invites_path, &guest, None)).await;
    let (_, body) = send(&state, authorized("GET", &invites_path, &owner, None)).await;
    let invites: Vec<GuildInvite> = json(&body);
    let code = invites[0].code.clone();
    let invite_path = format!("/api/v1/invites/{code}");
    let (guest_revoke, _) = send(&state, authorized("DELETE", &invite_path, &guest, None)).await;
    let (revoked, _) = send(&state, authorized("DELETE", &invite_path, &owner, None)).await;
    let (revoked_again, _) = send(&state, authorized("DELETE", &invite_path, &owner, None)).await;

    let (guest_delete, _) = send(&state, authorized("DELETE", &guild_path, &guest, None)).await;
    let (deleted, _) = send(&state, authorized("DELETE", &guild_path, &owner, None)).await;
    let (after_delete, _) = send(&state, authorized("GET", &guild_path, &owner, None)).await;
    database.close().await;

    assert_eq!(guest_rename, StatusCode::FORBIDDEN);
    assert_eq!(guest_rename_code, ApiErrorCode::MissingPermissions);
    assert_eq!(unknown_rename, StatusCode::NOT_FOUND);
    assert_eq!(unknown_field, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(renamed.name, "Book club");
    assert_eq!(guest_list, StatusCode::FORBIDDEN);
    assert_eq!(invites.len(), 1);
    assert_eq!((invites[0].uses, invites[0].max_uses), (1, None));
    assert_eq!(invites[0].creator.username, "owner");
    assert_eq!(guest_revoke, StatusCode::FORBIDDEN);
    assert_eq!(revoked, StatusCode::NO_CONTENT);
    assert_eq!(revoked_again, StatusCode::NOT_FOUND);
    assert_eq!(guest_delete, StatusCode::FORBIDDEN);
    assert_eq!(deleted, StatusCode::NO_CONTENT);
    assert_eq!(after_delete, StatusCode::NOT_FOUND);
}
