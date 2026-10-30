use serde::{Deserialize, Serialize};

use crate::{Permissions, Snowflake};

/// Body of `POST /api/v1/guilds`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateGuildRequest {
    pub name: String,
    /// The categories and channels the server starts with, in order, so clients can offer
    /// templates in their own language. Absent: one "Text channels" category holding `general`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_categories: Option<Vec<InitialCategory>>,
}

/// A category a new server starts with, and its text channels in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialCategory {
    pub name: String,
    pub channels: Vec<InitialTextChannel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialTextChannel {
    pub name: String,
    #[serde(default)]
    pub topic: String,
}

/// Body of `PATCH /api/v1/guilds/{guild_id}`; absent fields are unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateGuildRequest {
    pub name: Option<String>,
}

/// A guild ("server" in the UI) with its channels, as returned to a member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guild {
    pub id: Snowflake,
    pub name: String,
    pub owner_id: Snowflake,
    /// Absent when the guild has none, and from older servers' payloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<ProfileImage>,
    /// What the requesting member may do here (ADR-0007), so clients can hide unusable actions.
    pub my_permissions: Permissions,
    /// The requesting member's roles, without `@everyone` (every member has it).
    pub my_role_ids: Vec<Snowflake>,
    /// Sorted by `position`, `@everyone` first (its id is the guild's id, position 0).
    pub roles: Vec<Role>,
    /// Sorted by `position`; categories and channels interleaved, children reference `parent_id`.
    pub channels: Vec<Channel>,
    /// The guild's custom emoji, oldest first (ADR-0028). Absent from older servers' payloads.
    #[serde(default)]
    pub emojis: Vec<CustomEmoji>,
}

/// A guild's custom emoji (ADR-0028). Its picture is served like an icon, at
/// `GET /api/v1/icons/{id}` (and `/icons/{id}/still` when animated), to the guild's members.
/// Messages write it `<:name:id>` (`<a:name:id>` when animated); reactions key it `:id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomEmoji {
    pub id: Snowflake,
    /// 2 to 32 ASCII letters, digits and `_`, unique in its guild whatever the case.
    pub name: String,
    #[serde(default)]
    pub is_animated: bool,
}

/// Query of `POST /api/v1/guilds/{guild_id}/emojis`, whose body is the picture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateEmojiQuery {
    pub name: String,
}

/// Body of `PATCH /api/v1/guilds/{guild_id}/emojis/{emoji_id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateEmojiRequest {
    pub name: String,
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

/// Body of `PUT /api/v1/guilds/{guild_id}/channels/order`: every channel the requester can view,
/// once each, in their new order, each with its category. Channels hidden from the requester
/// keep their places. The answer is the guild's channels as the requester sees them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReorderChannelsRequest {
    pub channels: Vec<ChannelPlacement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelPlacement {
    pub id: Snowflake,
    /// `None` for categories and uncategorised channels.
    pub parent_id: Option<Snowflake>,
}

/// Body of `PUT /api/v1/guilds/{guild_id}/bans/{user_id}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBanRequest {
    /// Shown to moderators only.
    #[serde(default)]
    pub reason: String,
}

/// Entry of `GET /api/v1/guilds/{guild_id}/bans`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ban {
    pub user: PublicUser,
    pub reason: String,
    pub created_at_unix_milliseconds: u64,
}

/// What a moderator or administrator did, in the guild's audit log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditAction {
    ChannelCreate,
    ChannelUpdate,
    ChannelDelete,
    /// `target_id` is the role or member, `context_id` the channel.
    OverwriteSet,
    /// `target_id` is the role or member, `context_id` the channel.
    OverwriteDelete,
    RoleCreate,
    RoleUpdate,
    RoleDelete,
    /// No `target_id`: the whole order changed.
    RolesReorder,
    /// No `target_id`: channels moved or changed category.
    ChannelsReorder,
    /// `target_id` is the member, `context_id` the role.
    MemberRoleAdd,
    /// `target_id` is the member, `context_id` the role.
    MemberRoleRemove,
    MemberKick,
    MemberBanAdd,
    MemberBanRemove,
    /// A moderator deleted someone else's message: `target_id` is its author, `context_id` the
    /// channel. Authors deleting their own messages are not recorded.
    MessageDelete,
    /// `target_id` is the message, `context_id` the channel.
    MessagePin,
    /// `target_id` is the message, `context_id` the channel.
    MessageUnpin,
    /// A moderator removed reactions (someone's, one emoji's or all): `target_id` is the message,
    /// `context_id` the channel.
    MessageReactionRemove,
    /// The server's settings changed (its name).
    GuildUpdate,
    /// No `target_id`: the invite's code is the `code` change's new value.
    InviteCreate,
    /// No `target_id`: the invite's code is the `code` change's old value.
    InviteDelete,
    /// `target_id` is the emoji; its name is the `name` change's new value.
    EmojiCreate,
    /// `target_id` is the emoji: it was renamed (`name` change).
    EmojiUpdate,
    /// `target_id` is the emoji; its name is the `name` change's old value.
    EmojiDelete,
    /// An action added by a newer server; never written, only read by older clients.
    #[serde(other)]
    Unknown,
}

/// A field an audited update changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditChangeKey {
    Name,
    Topic,
    Color,
    Permissions,
    /// Of a channel overwrite.
    Allow,
    /// Of a channel overwrite.
    Deny,
    /// Of an invite.
    Code,
    /// Of a guild or channel: the icon's id, empty for none.
    Icon,
    /// Of a channel: its category's id, empty for none.
    Parent,
    /// A field added by a newer server.
    #[serde(other)]
    Unknown,
}

/// One changed field, with its values as text: permissions and colours in decimal, as on the
/// wire elsewhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditChange {
    pub key: AuditChangeKey,
    pub old_value: String,
    pub new_value: String,
}

/// Entry of `GET /api/v1/guilds/{guild_id}/audit-log`, newest first. Its id encodes when it
/// happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditLogEntry {
    pub id: Snowflake,
    /// `None` once that account no longer exists.
    pub actor_id: Option<Snowflake>,
    pub action: AuditAction,
    pub target_id: Option<Snowflake>,
    pub context_id: Option<Snowflake>,
    /// The ban reason, empty otherwise.
    pub reason: String,
    /// What an update changed (channel, role or overwrite updates), empty otherwise.
    pub changes: Vec<AuditChange>,
}

/// Query of `GET /api/v1/guilds/{guild_id}/audit-log`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditLogQuery {
    /// Entries strictly older than this id.
    pub before: Option<Snowflake>,
    /// 1 to 100; defaults to 50.
    pub limit: Option<u32>,
}

/// Entry of `GET /api/v1/users/@me/guilds`, the server rail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildSummary {
    pub id: Snowflake,
    pub name: String,
    /// The current icon: absent means none, also in `GUILD_UPDATE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<ProfileImage>,
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
    /// Never animated. Absent when the channel has none, and from older servers' payloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<ProfileImage>,
    /// Applied on top of guild permissions in this channel only (ADR-0007 step 4).
    pub permission_overwrites: Vec<PermissionOverwrite>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverwriteKind {
    /// `id` is a role of the guild; the guild's id targets `@everyone`.
    Role,
    /// `id` is a member's user id.
    Member,
}

/// Permissions `allow`ed and `deny`ed to one role or member in one channel; never both for a bit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOverwrite {
    pub id: Snowflake,
    pub kind: OverwriteKind,
    pub allow: Permissions,
    pub deny: Permissions,
}

/// Body of `PUT /api/v1/channels/{channel_id}/permissions/{target_id}`: creates or replaces the
/// overwrite of that role or member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetPermissionOverwriteRequest {
    pub kind: OverwriteKind,
    #[serde(default)]
    pub allow: Permissions,
    #[serde(default)]
    pub deny: Permissions,
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

/// Query of `PUT /api/v1/guilds/{guild_id}/icon` and `PUT /api/v1/channels/{channel_id}/icon`,
/// whose body is the picture: the square to keep, in pixels of the picture as browsers show it
/// (EXIF orientation applied). All three or none; none keeps the centred square, a square
/// reaching past the picture is cut at its edges, and one starting outside it (the picture was
/// smaller than the client thought) keeps the centred square too.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IconCropQuery {
    pub crop_x: Option<u32>,
    pub crop_y: Option<u32>,
    pub crop_size: Option<u32>,
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

/// Entry of `GET /api/v1/guilds/{guild_id}/invites`: a usable invite, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildInvite {
    pub code: String,
    pub creator: PublicUser,
    pub uses: u32,
    pub max_uses: Option<u32>,
    pub expires_at_unix_milliseconds: Option<u64>,
    pub created_at_unix_milliseconds: u64,
}

/// What other members may see about a user (never the email).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicUser {
    pub id: Snowflake,
    pub username: String,
    pub display_name: String,
    /// Absent when the user has none, and from older servers' payloads (ADR-0021).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<ProfileImage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner: Option<ProfileImage>,
}

/// An avatar or a profile banner, served at `GET /api/v1/profile-images/{id}` (and
/// `/profile-images/{id}/still`, its first frame, when animated); a guild or channel icon, at
/// `GET /api/v1/icons/{id}` (and `/icons/{id}/still`). Each version has its own id, so clients
/// cache it forever (ADR-0021).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileImage {
    pub id: Snowflake,
    #[serde(default)]
    pub is_animated: bool,
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
    fn unknown_audit_actions_and_change_keys_still_parse() {
        let entry: AuditLogEntry = serde_json::from_str(
            r#"{"id":"1","actor_id":null,"action":"guild_rename","target_id":null,
               "context_id":null,"reason":"",
               "changes":[{"key":"banner","old_value":"a","new_value":"b"}]}"#,
        )
        .unwrap();
        assert_eq!(entry.action, AuditAction::Unknown);
        assert_eq!(entry.changes[0].key, AuditChangeKey::Unknown);
    }

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
            icon: None,
            permission_overwrites: Vec::new(),
        };
        let json = serde_json::to_value(&channel).unwrap();
        assert_eq!(json["id"], "42");
        assert_eq!(json["kind"], "text");
        assert_eq!(json["parent_id"], serde_json::Value::Null);

        let guild = Guild {
            id: Snowflake::from_raw(7),
            name: "Club".into(),
            owner_id: Snowflake::from_raw(1),
            icon: None,
            my_permissions: Permissions::DEFAULT_EVERYONE,
            my_role_ids: Vec::new(),
            roles: Vec::new(),
            channels: vec![channel],
            emojis: Vec::new(),
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
