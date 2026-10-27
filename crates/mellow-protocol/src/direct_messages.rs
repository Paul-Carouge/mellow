//! Direct messages (ARCHITECTURE.md §4.5): a conversation is a channel outside any guild, whose
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
    /// Present for a group (ADR-0026): who may remove people from it. Absent for a one-to-one
    /// conversation and from older servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<Snowflake>,
    /// A group's name; empty for none and for one-to-one conversations.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

/// At most this many people in a group, its creator included.
pub const MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS: usize = 10;
/// A group's name is at most this many characters.
pub const MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH: usize = 100;

/// Body of `POST /api/v1/group-conversations`: a new group with these friends of the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateGroupConversationRequest {
    pub recipient_ids: Vec<Snowflake>,
    #[serde(default)]
    pub name: String,
}

/// Body of `PATCH /api/v1/group-conversations/{channel_id}`; an empty name clears it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateGroupConversationRequest {
    pub name: String,
}

/// Body of `POST /api/v1/users/@me/channels`: answers the existing one-to-one conversation with
/// that user, or opens it. Only with someone the caller may reach (ADR-0025).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenDirectMessageRequest {
    pub recipient_id: Snowflake,
}
