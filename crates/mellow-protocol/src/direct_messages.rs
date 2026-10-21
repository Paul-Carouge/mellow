//! Direct messages (ARCHITECTURE.md §4.3): a conversation is a channel outside any guild, whose
//! messages go through the same `/channels/{id}/messages` routes as guild channels.

use serde::{Deserialize, Serialize};

use crate::{PublicUser, Snowflake};

/// A conversation as its participant sees it: everyone in it but themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectMessageChannel {
    pub id: Snowflake,
    pub recipients: Vec<PublicUser>,
    /// Orders conversations by last activity; `None` until the first message.
    pub last_message_id: Option<Snowflake>,
}

/// Body of `POST /api/v1/users/@me/channels`: answers the existing one-to-one conversation with
/// that user, or opens it. Only with someone who shares a guild with the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenDirectMessageRequest {
    pub recipient_id: Snowflake,
}
