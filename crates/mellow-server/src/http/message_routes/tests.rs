use mellow_protocol::{ApiErrorBody, ApiErrorCode};

use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::tests::migrated_database;
use crate::http::test_support::*;
use crate::http::*;

#[tokio::test]
async fn messages_over_http() {
    use mellow_protocol::{Message, ReadState};

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let messages_path = format!("/api/v1/channels/{general}/messages");

    let (status, body) = send(
        &state,
        authorized(
            "POST",
            &messages_path,
            &owner,
            Some(serde_json::json!({"content": "hello", "nonce": "n1"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let sent: Message = json(&body);
    assert_eq!(sent.nonce.as_deref(), Some("n1"));
    assert_eq!(sent.author.username, "owner");

    let (status, body) = send(
        &state,
        authorized("GET", &format!("{messages_path}?limit=10"), &owner, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json::<Vec<Message>>(&body)[0].content, "hello");

    let (status, body) = send(
        &state,
        authorized("GET", &format!("{messages_path}?limit=nope"), &owner, None),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        json::<ApiErrorBody>(&body).code,
        ApiErrorCode::ValidationFailed
    );

    let message_path = format!("{messages_path}/{}", sent.id);
    let (status, body) = send(
        &state,
        authorized(
            "PATCH",
            &message_path,
            &owner,
            Some(serde_json::json!({"content": "hello, edited"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(json::<Message>(&body).edited_at_unix_milliseconds.is_some());

    let (status, _) = send(
        &state,
        authorized(
            "PUT",
            &format!("/api/v1/channels/{general}/read-state"),
            &owner,
            Some(serde_json::json!({"last_read_message_id": sent.id.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, body) = send(
        &state,
        authorized("GET", "/api/v1/users/@me/read-states", &owner, None),
    )
    .await;
    assert_eq!(
        json::<Vec<ReadState>>(&body)[0].last_read_message_id,
        sent.id
    );

    let (status, _) = send(&state, authorized("DELETE", &message_path, &owner, None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = send(&state, authorized("DELETE", &message_path, &owner, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json::<ApiErrorBody>(&body).code, ApiErrorCode::NotFound);

    database.close().await;
}

#[tokio::test]
async fn sending_messages_is_rate_limited_per_user() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let messages_path = format!("/api/v1/channels/{general}/messages");
    for _ in 0..rate_limit::MESSAGE_WRITES_PER_USER.maximum_events {
        send(
            &state,
            authorized(
                "POST",
                &messages_path,
                &owner,
                Some(serde_json::json!({"content": "spam"})),
            ),
        )
        .await;
    }
    let (status, _) = send(
        &state,
        authorized(
            "POST",
            &messages_path,
            &owner,
            Some(serde_json::json!({"content": "one more"})),
        ),
    )
    .await;
    database.close().await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn history_cursors_over_http() {
    use mellow_protocol::Message;

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let messages_path = format!("/api/v1/channels/{general}/messages");
    let (_, body) = send(
        &state,
        authorized(
            "POST",
            &messages_path,
            &owner,
            Some(serde_json::json!({"content": "first"})),
        ),
    )
    .await;
    let sent: Message = json(&body);
    let (_, body) = send(
        &state,
        authorized(
            "POST",
            &messages_path,
            &owner,
            Some(serde_json::json!({"content": "second"})),
        ),
    )
    .await;
    let second: Message = json(&body);

    let (_, body) = send(
        &state,
        authorized(
            "GET",
            &format!("{messages_path}?before={}&limit=1", second.id),
            &owner,
            None,
        ),
    )
    .await;
    assert_eq!(
        json::<Vec<Message>>(&body)[0].id,
        sent.id,
        "query-string cursors parse"
    );
    let (_, body) = send(
        &state,
        authorized(
            "GET",
            &format!("{messages_path}?after=18446744073709551615"),
            &owner,
            None,
        ),
    )
    .await;
    assert_eq!(json::<Vec<Message>>(&body), Vec::new());
    database.close().await;
}

#[tokio::test]
async fn typing_needs_membership_and_is_rate_limited() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let outsider = registered_token(&state, "outsider").await;
    let typing_path = format!("/api/v1/channels/{general}/typing");

    let (status, _) = send(&state, authorized("POST", &typing_path, &owner, None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(&state, authorized("POST", &typing_path, &outsider, None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for _ in 1..rate_limit::TYPING_INDICATORS_PER_USER.maximum_events {
        send(&state, authorized("POST", &typing_path, &owner, None)).await;
    }
    let (status, _) = send(&state, authorized("POST", &typing_path, &owner, None)).await;
    database.close().await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn reactions_and_pins_over_http() {
    use mellow_protocol::{Guild, Message, PublicUser};

    const THUMBS_UP: &str = "%F0%9F%91%8D";
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, guest_id) = guild_with_guest(&state).await;
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
            Some(serde_json::json!({"content": "react to me"})),
        ),
    )
    .await;
    let message: Message = json(&body);
    let reactions_path = format!("{messages_path}/{}/reactions", message.id);
    let mine = format!("{reactions_path}/{THUMBS_UP}/@me");

    let mut statuses = Vec::new();
    for _ in 0..2 {
        statuses.push(send(&state, authorized("PUT", &mine, &guest, None)).await.0);
    }
    let (_, body) = send(
        &state,
        authorized("GET", &format!("{messages_path}?limit=10"), &guest, None),
    )
    .await;
    let history: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let (status, body) = send(
        &state,
        authorized(
            "GET",
            &format!("{reactions_path}/{THUMBS_UP}"),
            &owner,
            None,
        ),
    )
    .await;
    let users: Vec<PublicUser> = json(&body);
    let (bad_emoji, bad_emoji_body) = send(
        &state,
        authorized("PUT", &format!("{reactions_path}/lol/@me"), &guest, None),
    )
    .await;
    let removed = send(&state, authorized("DELETE", &mine, &guest, None))
        .await
        .0;
    database.close().await;

    assert_eq!(statuses, [StatusCode::NO_CONTENT, StatusCode::NO_CONTENT]);
    assert_eq!(
        history[0]["reactions"],
        serde_json::json!([{"emoji": "👍", "count": 1, "me": true}])
    );
    assert_eq!(history[0]["pinned"], false);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        users.iter().map(|user| user.id).collect::<Vec<_>>(),
        [guest_id]
    );
    assert_eq!(bad_emoji, StatusCode::BAD_REQUEST);
    assert_eq!(
        json::<ApiErrorBody>(&bad_emoji_body).code,
        ApiErrorCode::ValidationFailed
    );
    assert_eq!(removed, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn pins_over_http() {
    use mellow_protocol::{Guild, Message};

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
    let (_, body) = send(
        &state,
        authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &owner, None),
    )
    .await;
    let general = json::<Guild>(&body).channels[1].id;
    let (_, body) = send(
        &state,
        authorized(
            "POST",
            &format!("/api/v1/channels/{general}/messages"),
            &guest,
            Some(serde_json::json!({"content": "pin me"})),
        ),
    )
    .await;
    let message: Message = json(&body);
    let pin_path = format!("/api/v1/channels/{general}/pins/{}", message.id);
    let guest_pins = send(&state, authorized("PUT", &pin_path, &guest, None)).await;
    let owner_pins = send(&state, authorized("PUT", &pin_path, &owner, None))
        .await
        .0;
    let pins_path = format!("/api/v1/channels/{general}/pins");
    let (_, body) = send(&state, authorized("GET", &pins_path, &guest, None)).await;
    let pins: Vec<Message> = json(&body);
    let unpinned = send(&state, authorized("DELETE", &pin_path, &owner, None))
        .await
        .0;
    let (_, body) = send(&state, authorized("GET", &pins_path, &guest, None)).await;
    let pins_after: Vec<Message> = json(&body);
    database.close().await;

    assert_eq!(guest_pins.0, StatusCode::FORBIDDEN);
    assert_eq!(
        json::<ApiErrorBody>(&guest_pins.1).code,
        ApiErrorCode::MissingPermissions
    );
    assert_eq!(owner_pins, StatusCode::NO_CONTENT);
    assert_eq!(pins.len(), 1);
    assert!(pins[0].pinned);
    assert_eq!(unpinned, StatusCode::NO_CONTENT);
    assert_eq!(pins_after, Vec::new());
}

#[tokio::test]
async fn message_search_over_http() {
    use mellow_protocol::MessageSearchResults;

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let (_, body) = send(
        &state,
        authorized("GET", "/api/v1/users/@me/guilds", &owner, None),
    )
    .await;
    let guild_id = json::<Vec<mellow_protocol::GuildSummary>>(&body)[0].id;
    send(
        &state,
        authorized(
            "POST",
            &format!("/api/v1/channels/{general}/messages"),
            &owner,
            Some(serde_json::json!({"content": "Crème brûlée tonight?"})),
        ),
    )
    .await;
    let search = |body: serde_json::Value| {
        authorized(
            "POST",
            &format!("/api/v1/guilds/{guild_id}/messages/search"),
            &owner,
            Some(body),
        )
    };
    let (status, body) = send(
        &state,
        search(serde_json::json!({"q": "creme -pizza", "sort": "newest", "limit": 5})),
    )
    .await;
    let found: MessageSearchResults = json(&body);
    let unknown_field = send(&state, search(serde_json::json!({"q": "creme", "page": 2})))
        .await
        .0;
    let as_get = send(
        &state,
        authorized(
            "GET",
            &format!("/api/v1/guilds/{guild_id}/messages/search?q=creme"),
            &owner,
            None,
        ),
    )
    .await
    .0;
    let (_, body) = send(
        &state,
        authorized(
            "POST",
            "/api/v1/users/@me/messages/search",
            &owner,
            Some(serde_json::json!({"q": "creme"})),
        ),
    )
    .await;
    let in_conversations: MessageSearchResults = json(&body);
    for _ in 2..rate_limit::MESSAGE_SEARCHES_PER_USER.maximum_events {
        send(&state, search(serde_json::json!({"q": "creme"}))).await;
    }
    let limited = send(&state, search(serde_json::json!({"q": "creme"})))
        .await
        .0;
    database.close().await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(found.total_count, 1);
    assert_eq!(found.hits[0].message.content, "Crème brûlée tonight?");
    assert_eq!(found.hits[0].highlights.len(), 1);
    assert_eq!(unknown_field, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(as_get, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(in_conversations.total_count, 0);
    assert_eq!(limited, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn reactions_are_rate_limited_per_user() {
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, general) = owner_and_general_channel(&state).await;
    let (_, body) = send(
        &state,
        authorized(
            "POST",
            &format!("/api/v1/channels/{general}/messages"),
            &owner,
            Some(serde_json::json!({"content": "hi"})),
        ),
    )
    .await;
    let message: mellow_protocol::Message = json(&body);
    let mine = format!(
        "/api/v1/channels/{general}/messages/{}/reactions/%F0%9F%91%8D/@me",
        message.id
    );
    for _ in 0..rate_limit::REACTION_WRITES_PER_USER.maximum_events {
        send(&state, authorized("PUT", &mine, &owner, None)).await;
    }
    let (status, _) = send(&state, authorized("DELETE", &mine, &owner, None)).await;
    database.close().await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn reactions_are_rate_limited_per_channel() {
    use mellow_protocol::{Guild, Invite, Message};

    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, _) = guild_with_guest(&state).await;
    let (_, body) = send(
        &state,
        authorized("GET", &format!("/api/v1/guilds/{guild_id}"), &owner, None),
    )
    .await;
    let general = json::<Guild>(&body).channels[1].id;
    let (_, body) = send(
        &state,
        authorized(
            "POST",
            &format!("/api/v1/guilds/{guild_id}/invites"),
            &owner,
            Some(serde_json::json!({})),
        ),
    )
    .await;
    let code = json::<Invite>(&body).code;
    let mut members = vec![owner.clone(), guest];
    for name in ["third", "fourth"] {
        let token = registered_token(&state, name).await;
        send(
            &state,
            authorized("POST", &format!("/api/v1/invites/{code}"), &token, None),
        )
        .await;
        members.push(token);
    }
    let (_, body) = send(
        &state,
        authorized(
            "POST",
            &format!("/api/v1/channels/{general}/messages"),
            &owner,
            Some(serde_json::json!({"content": "hi"})),
        ),
    )
    .await;
    let message: Message = json(&body);
    let reaction = format!(
        "/api/v1/channels/{general}/messages/{}/reactions/%F0%9F%91%8D/@me",
        message.id
    );
    let per_user = rate_limit::REACTION_WRITES_PER_USER.maximum_events;
    let per_channel = rate_limit::REACTION_WRITES_PER_CHANNEL.maximum_events;
    let (last, others) = members.split_last().unwrap();
    let mut accepted = 0;
    for token in others {
        for _ in 0..per_user {
            let (status, _) = send(&state, authorized("PUT", &reaction, token, None)).await;
            accepted += u32::from(status == StatusCode::NO_CONTENT);
        }
    }
    let (status, _) = send(&state, authorized("PUT", &reaction, last, None)).await;
    database.close().await;

    assert_eq!(
        accepted, per_channel,
        "three members fill the channel's budget"
    );
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn moderators_remove_reactions_over_http() {
    use mellow_protocol::{Guild, Message};

    const THUMBS_UP: &str = "%F0%9F%91%8D";
    let (_directory, database) = migrated_database().await;
    let state = test_state(database.clone(), RegistrationMode::Open);
    let (owner, guest, guild_id, guest_id) = guild_with_guest(&state).await;
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
            Some(serde_json::json!({"content": "hi"})),
        ),
    )
    .await;
    let message: Message = json(&body);
    let reactions = format!("{messages_path}/{}/reactions", message.id);
    let guest_reacts = authorized("PUT", &format!("{reactions}/{THUMBS_UP}/@me"), &guest, None);
    send(&state, guest_reacts).await;

    let mut statuses = Vec::new();
    for (token, path) in [
        (&guest, reactions.clone()),
        (&owner, format!("{reactions}/{THUMBS_UP}/{guest_id}")),
        (&owner, format!("{reactions}/{THUMBS_UP}")),
        (&owner, reactions.clone()),
    ] {
        statuses.push(
            send(&state, authorized("DELETE", &path, token, None))
                .await
                .0,
        );
    }
    database.close().await;

    assert_eq!(
        statuses,
        [
            StatusCode::FORBIDDEN,
            StatusCode::NO_CONTENT,
            StatusCode::NO_CONTENT,
            StatusCode::NO_CONTENT
        ]
    );
}
