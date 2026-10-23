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
    /// Files uploaded to this channel beforehand by the sender (ADR-0020), at most 10. Left out of
    /// the body when empty, so older servers still accept text-only messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachment_ids: Vec<Snowflake>,
}

/// A file shared in a message (ADR-0020), downloaded at
/// `GET /api/v1/attachments/{id}/{file_name}`. Also the answer to an upload,
/// `POST /api/v1/channels/{channel_id}/attachments?file_name=…`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Snowflake,
    pub file_name: String,
    /// `image/png`, `image/jpeg`, `image/gif` or `image/webp` when the bytes are one, which
    /// clients may preview; `application/octet-stream` for anything else.
    pub content_type: String,
    pub size_bytes: u64,
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
    /// Absent from older servers' payloads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    /// In the order they were first added. Filled in REST answers, where `me` is the requester;
    /// always empty in gateway events (`MESSAGE_UPDATE` included), since `me` differs per
    /// viewer: clients keep the reactions they hold and follow `MESSAGE_REACTION_ADD` and
    /// `MESSAGE_REACTION_REMOVE`. Absent when empty and from older servers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<ReactionSummary>,
    /// Pinned to its channel (`GET /api/v1/channels/{channel_id}/pins`). Absent from older
    /// servers' payloads.
    #[serde(default)]
    pub pinned: bool,
    /// The people the server found mentioned (`@username`, or a one-word display name, outside
    /// code) who can view the channel; never the author. Absent when empty and from older
    /// servers' payloads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mention_ids: Vec<Snowflake>,
}

/// One emoji's reactions on a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionSummary {
    /// A Unicode emoji, as sent (`"👍"`, `"❤️"`).
    pub emoji: String,
    pub count: u32,
    /// Whether the requesting user is among them.
    pub me: bool,
}

/// Query of `GET /api/v1/channels/{channel_id}/messages/{message_id}/reactions/{emoji}`: who
/// reacted, by ascending user id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReactionUsersQuery {
    /// Users whose id is strictly greater than this one.
    pub after: Option<Snowflake>,
    /// 1 to 100; defaults to 25.
    pub limit: Option<u32>,
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

/// Where a channel the user can view stands, in `Ready`: its newest message and how many unread
/// messages mention the user (in a direct-message conversation: how many unread messages from
/// others), counted up to `MAXIMUM_COUNTED_UNREAD_MENTIONS`. Only for channels with a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelUnread {
    pub channel_id: Snowflake,
    pub last_message_id: Snowflake,
    pub mention_count: u32,
}

/// Counting stops here, so a long unread history costs a bounded read; clients show "99+".
pub const MAXIMUM_COUNTED_UNREAD_MENTIONS: u32 = 100;

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
                avatar: None,
                banner: None,
            },
            content: "hello".into(),
            reply_to_id: None,
            edited_at_unix_milliseconds: None,
            nonce: Some("abc".into()),
            attachments: Vec::new(),
            reactions: vec![ReactionSummary {
                emoji: "👍".into(),
                count: 2,
                me: true,
            }],
            pinned: true,
            mention_ids: Vec::new(),
        };
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["id"], "9");
        assert_eq!(json["author"]["username"], "aurore");
        assert_eq!(json["nonce"], "abc");
        assert_eq!(
            json["reactions"],
            serde_json::json!([{"emoji": "👍", "count": 2, "me": true}])
        );
        assert_eq!(json["pinned"], true);
    }

    #[test]
    fn messages_from_older_servers_have_no_reactions_and_are_not_pinned() {
        let message: Message = serde_json::from_str(
            r#"{"id":"9","channel_id":"3","author":{"id":"1","username":"a","display_name":"A"},"content":"hi","reply_to_id":null,"edited_at_unix_milliseconds":null,"nonce":null}"#,
        )
        .unwrap();
        assert_eq!(message.reactions, Vec::new());
        assert!(!message.pinned);
    }
}
