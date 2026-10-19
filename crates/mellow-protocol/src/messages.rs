use serde::{Deserialize, Serialize};

use crate::{PublicUser, Snowflake};

/// Body of `POST /api/v1/channels/{channel_id}/messages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendMessageRequest {
    pub content: String,
    /// The message this one answers; must be in the same channel.
    pub reply_to_id: Option<Snowflake>,
    /// Client-generated, echoed back so an offline outbox can match its pending copy
    /// (ARCHITECTURE.md §6). Not stored.
    pub nonce: Option<String>,
}

/// Body of `PATCH /api/v1/channels/{channel_id}/messages/{message_id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditMessageRequest {
    pub content: String,
}

/// Query of `GET /api/v1/channels/{channel_id}/messages`. With neither cursor, the newest
/// messages are returned. Results are always in chronological order (oldest first).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageHistoryQuery {
    /// Messages strictly older than this id.
    pub before: Option<Snowflake>,
    /// Messages strictly newer than this id.
    pub after: Option<Snowflake>,
    /// 1 to 100; defaults to 50.
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Also encodes the creation time (Snowflake, ADR-0004).
    pub id: Snowflake,
    pub channel_id: Snowflake,
    pub author: PublicUser,
    pub content: String,
    /// May point to a message that has since been deleted ("Original message was deleted").
    pub reply_to_id: Option<Snowflake>,
    pub edited_at_unix_milliseconds: Option<u64>,
    /// Echo of the sender's nonce, only in the response to the sender.
    pub nonce: Option<String>,
}

/// Body of `PUT /api/v1/channels/{channel_id}/read-state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeRequest {
    pub last_read_message_id: Snowflake,
}

/// Entry of `GET /api/v1/users/@me/read-states`, for unread markers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadState {
    pub channel_id: Snowflake,
    pub last_read_message_id: Snowflake,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_query_and_message_wire_format() {
        let query: MessageHistoryQuery =
            serde_json::from_str(r#"{"before":"42","limit":20}"#).unwrap();
        assert_eq!(query.before, Some(Snowflake::from_raw(42)));
        assert_eq!(query.after, None);

        let message = Message {
            id: Snowflake::from_raw(9),
            channel_id: Snowflake::from_raw(3),
            author: PublicUser {
                id: Snowflake::from_raw(1),
                username: "aurore".into(),
                display_name: "Aurore".into(),
            },
            content: "hello".into(),
            reply_to_id: None,
            edited_at_unix_milliseconds: None,
            nonce: Some("abc".into()),
        };
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["id"], "9");
        assert_eq!(json["author"]["username"], "aurore");
        assert_eq!(json["nonce"], "abc");
    }
}
