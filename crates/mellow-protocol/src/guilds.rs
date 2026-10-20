use serde::{Deserialize, Serialize};

use crate::{Permissions, Snowflake};

/// Body of `POST /api/v1/guilds`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateGuildRequest {
    pub name: String,
}

/// A guild ("server" in the UI) with its channels, as returned to a member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guild {
    pub id: Snowflake,
    pub name: String,
    pub owner_id: Snowflake,
    /// What the requesting member may do here (ADR-0007), so clients can hide unusable actions.
    pub my_permissions: Permissions,
    /// The requesting member's roles, without `@everyone` (every member has it).
    pub my_role_ids: Vec<Snowflake>,
    /// Sorted by `position`, `@everyone` first (its id is the guild's id, position 0).
    pub roles: Vec<Role>,
    /// Sorted by `position`; categories and channels interleaved, children reference `parent_id`.
    pub channels: Vec<Channel>,
}

/// A guild role. Higher `position` ranks higher: a member manages only roles below their highest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    pub id: Snowflake,
    pub name: String,
    /// `0xRRGGBB`; `0` means no colour, as on Discord.
    pub color: u32,
    pub position: u32,
    pub permissions: Permissions,
}

/// Body of `POST /api/v1/guilds/{guild_id}/roles`. The role is created just above `@everyone`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRoleRequest {
    pub name: String,
    #[serde(default)]
    pub color: u32,
    #[serde(default)]
    pub permissions: Permissions,
}

/// Body of `PATCH /api/v1/guilds/{guild_id}/roles/{role_id}`; absent fields are unchanged.
/// `@everyone` accepts only `permissions`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRoleRequest {
    pub name: Option<String>,
    pub color: Option<u32>,
    pub permissions: Option<Permissions>,
}

/// Body of `PUT /api/v1/guilds/{guild_id}/roles/order`: every role of the guild except
/// `@everyone`, once each, from lowest to highest. The answer is the guild's new role list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReorderRolesRequest {
    pub role_ids: Vec<Snowflake>,
}

/// Entry of `GET /api/v1/users/@me/guilds`, the server rail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildSummary {
    pub id: Snowflake,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Category,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub id: Snowflake,
    pub guild_id: Snowflake,
    pub kind: ChannelKind,
    pub name: String,
    pub topic: String,
    /// The category this channel sits in; always `None` for categories.
    pub parent_id: Option<Snowflake>,
    pub position: u32,
}

/// Body of `POST /api/v1/guilds/{guild_id}/channels`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateChannelRequest {
    pub kind: ChannelKind,
    pub name: String,
    #[serde(default)]
    pub topic: String,
    pub parent_id: Option<Snowflake>,
}

/// Body of `PATCH /api/v1/channels/{channel_id}`; absent fields are unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateChannelRequest {
    pub name: Option<String>,
    pub topic: Option<String>,
}

/// Body of `POST /api/v1/guilds/{guild_id}/invites`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateInviteRequest {
    /// `None` means unlimited.
    pub max_uses: Option<u32>,
    /// `None` means the invite never expires.
    pub expires_in_seconds: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub code: String,
    pub guild_id: Snowflake,
    pub uses: u32,
    pub max_uses: Option<u32>,
    pub expires_at_unix_milliseconds: Option<u64>,
}

/// What other members may see about a user (never the email).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicUser {
    pub id: Snowflake,
    pub username: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub user: PublicUser,
    /// Without `@everyone`.
    pub role_ids: Vec<Snowflake>,
    pub joined_at_unix_milliseconds: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_uses_string_ids_and_snake_case_kinds() {
        let channel = Channel {
            id: Snowflake::from_raw(42),
            guild_id: Snowflake::from_raw(7),
            kind: ChannelKind::Text,
            name: "general".into(),
            topic: String::new(),
            parent_id: None,
            position: 1,
        };
        let json = serde_json::to_value(&channel).unwrap();
        assert_eq!(json["id"], "42");
        assert_eq!(json["kind"], "text");
        assert_eq!(json["parent_id"], serde_json::Value::Null);

        let guild = Guild {
            id: Snowflake::from_raw(7),
            name: "Club".into(),
            owner_id: Snowflake::from_raw(1),
            my_permissions: Permissions::DEFAULT_EVERYONE,
            my_role_ids: Vec::new(),
            roles: Vec::new(),
            channels: vec![channel],
        };
        let json = serde_json::to_value(&guild).unwrap();
        assert_eq!(
            json["my_permissions"],
            Permissions::DEFAULT_EVERYONE.bits().to_string()
        );
        assert!(
            serde_json::from_str::<CreateChannelRequest>(
                r#"{"kind":"voice","name":"x","parent_id":null}"#
            )
            .is_err()
        );
    }
}
