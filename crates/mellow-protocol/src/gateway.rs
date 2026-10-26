//! WebSocket gateway frames (ARCHITECTURE.md §4.8), at `/gateway?v=1`.
//!
//! Every frame is a JSON object `{ "op": ..., "d": ... }`; dispatches add a sequence number `s`
//! and carry the event type in `d.t`.

use serde::{Deserialize, Serialize};

use crate::{
    Channel, ChannelUnread, CurrentUser, DirectMessageChannel, Guild, GuildSummary, Member,
    Message, PublicUser, ReadState, Role, Snowflake,
};

pub const GATEWAY_VERSION: u32 = 1;

/// Frames the client sends.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "op",
    content = "d",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ClientFrame {
    /// First frame of a new session.
    Identify { token: String },
    /// First frame of a reconnection: replays what was missed since `last_sequence`.
    Resume {
        token: String,
        session_id: String,
        last_sequence: u64,
    },
    /// Keeps the connection alive; due every `heartbeat_interval_milliseconds`.
    Heartbeat,
}

/// The session token is never printed.
impl std::fmt::Debug for ClientFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Identify { .. } => formatter.write_str("Identify { token: <redacted> }"),
            Self::Resume {
                session_id,
                last_sequence,
                ..
            } => formatter
                .debug_struct("Resume")
                .field("token", &"<redacted>")
                .field("session_id", session_id)
                .field("last_sequence", last_sequence)
                .finish(),
            Self::Heartbeat => formatter.write_str("Heartbeat"),
        }
    }
}

/// Frames the server sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ServerFrame {
    /// Sent on connect, before anything else.
    Hello {
        d: Hello,
    },
    /// Answer to `Identify`: everything the client needs to render, in one frame.
    Ready {
        d: Box<Ready>,
    },
    /// Answer to `Resume` once missed dispatches have been replayed.
    Resumed,
    /// The session cannot continue; identify again (and resynchronise through REST).
    InvalidSession,
    HeartbeatAck,
    Dispatch {
        #[serde(rename = "s")]
        sequence: u64,
        #[serde(rename = "d")]
        event: Box<GatewayEvent>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub heartbeat_interval_milliseconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ready {
    /// Needed to `Resume` after a dropped connection.
    pub session_id: String,
    pub user: CurrentUser,
    pub guilds: Vec<Guild>,
    pub read_states: Vec<ReadState>,
    /// Members of the user's guilds who are online now (idle and do not disturb included).
    pub online_user_ids: Vec<Snowflake>,
    /// Added after `v0.2.0`: absent from older servers.
    #[serde(default)]
    pub direct_message_channels: Vec<DirectMessageChannel>,
    /// Added after `v0.10.0`: absent from older servers.
    #[serde(default)]
    pub channel_unreads: Vec<ChannelUnread>,
    /// The status of each user in `online_user_ids`. Added after `v0.11.0`: absent from older
    /// servers, whose online users are all `Online`.
    #[serde(default)]
    pub presences: Vec<Presence>,
}

/// What others see of a user's presence. An invisible user is `Offline`, without custom status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    pub user_id: Snowflake,
    pub status: PresenceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_status: Option<String>,
}

/// Payload of `MESSAGE_REACTION_ADD` and `MESSAGE_REACTION_REMOVE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReaction {
    /// `None` in a direct-message conversation, and then left out of the frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<Snowflake>,
    pub channel_id: Snowflake,
    pub message_id: Snowflake,
    pub user_id: Snowflake,
    pub emoji: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus {
    Online,
    /// Added after `v0.11.0`, as `DoNotDisturb`: older clients skip the frame.
    Idle,
    DoNotDisturb,
    Offline,
}

/// Events pushed to clients. `t` names the event, `data` carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GatewayEvent {
    MessageCreate(Message),
    MessageUpdate(Message),
    MessageDelete {
        /// `None` in a direct-message conversation, and then left out of the frame.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guild_id: Option<Snowflake>,
        channel_id: Snowflake,
        id: Snowflake,
    },
    ChannelCreate(Channel),
    ChannelUpdate(Channel),
    ChannelDelete {
        guild_id: Snowflake,
        id: Snowflake,
    },
    /// The user joined or created a guild.
    GuildCreate(Box<Guild>),
    /// The guild was renamed. Sent to every member; older clients skip it.
    GuildUpdate(GuildSummary),
    /// The user left a guild (or it disappeared for them, or its owner deleted it).
    GuildDelete {
        id: Snowflake,
    },
    GuildMemberAdd {
        guild_id: Snowflake,
        member: Member,
    },
    GuildMemberRemove {
        guild_id: Snowflake,
        user_id: Snowflake,
    },
    /// A member's roles changed; `role_ids` is the full new list, without `@everyone`.
    GuildMemberUpdate {
        guild_id: Snowflake,
        user_id: Snowflake,
        role_ids: Vec<Snowflake>,
    },
    /// Any role was created, edited or deleted: the guild's full role list, as in `Guild::roles`.
    /// Creating a role shifts the positions of the others, so they are always sent together. A
    /// deleted role is dropped from members without a `GUILD_MEMBER_UPDATE`.
    GuildRolesUpdate {
        guild_id: Snowflake,
        roles: Vec<Role>,
    },
    /// Someone reacted to a message. Sent to the channel's viewers, the reacting user included.
    MessageReactionAdd(MessageReaction),
    /// Someone took their reaction back, or a moderator removed it.
    MessageReactionRemove(MessageReaction),
    /// A moderator removed every reaction of a message.
    MessageReactionRemoveAll {
        /// `None` in a direct-message conversation, and then left out of the frame.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guild_id: Option<Snowflake>,
        channel_id: Snowflake,
        message_id: Snowflake,
    },
    /// A moderator removed every reaction with one emoji from a message.
    MessageReactionRemoveEmoji {
        /// `None` in a direct-message conversation, and then left out of the frame.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guild_id: Option<Snowflake>,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: String,
    },
    TypingStart {
        /// `None` in a direct-message conversation, and then left out of the frame.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guild_id: Option<Snowflake>,
        channel_id: Snowflake,
        user_id: Snowflake,
    },
    /// A user's status or custom status changed, or they connected or left. Sent to the members
    /// of their guilds.
    PresenceUpdate(Presence),
    /// A conversation was opened with the user, by them or by someone else (before its first
    /// message reaches them).
    DirectMessageChannelCreate(DirectMessageChannel),
    /// A user changed their username or display name. Sent to that user and to every member of a
    /// guild they share.
    UserUpdate(PublicUser),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_use_op_s_d_on_the_wire() {
        let identify: ClientFrame =
            serde_json::from_str(r#"{"op":"identify","d":{"token":"mlw_x"}}"#).unwrap();
        assert!(matches!(identify, ClientFrame::Identify { .. }));
        assert!(
            !format!("{identify:?}").contains("mlw_x"),
            "tokens are redacted"
        );
        let heartbeat: ClientFrame = serde_json::from_str(r#"{"op":"heartbeat"}"#).unwrap();
        assert_eq!(heartbeat, ClientFrame::Heartbeat);

        let dispatch = ServerFrame::Dispatch {
            sequence: 7,
            event: Box::new(GatewayEvent::TypingStart {
                guild_id: Some(Snowflake::from_raw(1)),
                channel_id: Snowflake::from_raw(2),
                user_id: Snowflake::from_raw(3),
            }),
        };
        let json = serde_json::to_value(&dispatch).unwrap();
        assert_eq!(json["op"], "dispatch");
        assert_eq!(json["s"], 7);
        assert_eq!(json["d"]["t"], "TYPING_START");
        assert_eq!(json["d"]["data"]["channel_id"], "2");
        assert_eq!(json["d"]["data"]["guild_id"], "1");
        assert_eq!(
            serde_json::to_value(ServerFrame::HeartbeatAck).unwrap()["op"],
            "heartbeat_ack"
        );
    }

    #[test]
    fn role_events_carry_string_ids_and_decimal_permissions() {
        let event = GatewayEvent::GuildRolesUpdate {
            guild_id: Snowflake::from_raw(7),
            roles: vec![Role {
                id: Snowflake::from_raw(7),
                name: "@everyone".into(),
                color: 0,
                position: 0,
                permissions: crate::Permissions::SEND_MESSAGES,
            }],
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["t"], "GUILD_ROLES_UPDATE");
        assert_eq!(json["data"]["roles"][0]["id"], "7");
        assert_eq!(json["data"]["roles"][0]["permissions"], "4");
        let member_update = serde_json::to_value(GatewayEvent::GuildMemberUpdate {
            guild_id: Snowflake::from_raw(7),
            user_id: Snowflake::from_raw(8),
            role_ids: vec![Snowflake::from_raw(9)],
        })
        .unwrap();
        assert_eq!(member_update["t"], "GUILD_MEMBER_UPDATE");
        assert_eq!(member_update["data"]["role_ids"][0], "9");
    }

    #[test]
    fn guild_updates_carry_the_new_name() {
        let json = serde_json::to_value(GatewayEvent::GuildUpdate(GuildSummary {
            id: Snowflake::from_raw(7),
            name: "Club".into(),
            icon: None,
        }))
        .unwrap();
        assert_eq!(json["t"], "GUILD_UPDATE");
        assert_eq!(json["data"]["id"], "7");
        assert_eq!(json["data"]["name"], "Club");
    }

    #[test]
    fn direct_message_events_leave_the_guild_out() {
        let event = GatewayEvent::MessageDelete {
            guild_id: None,
            channel_id: Snowflake::from_raw(2),
            id: Snowflake::from_raw(3),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert!(json["data"].get("guild_id").is_none());
        assert_eq!(serde_json::from_value::<GatewayEvent>(json).unwrap(), event);
    }

    #[test]
    fn reaction_events_carry_the_emoji_and_string_ids() {
        let event = GatewayEvent::MessageReactionAdd(MessageReaction {
            guild_id: None,
            channel_id: Snowflake::from_raw(2),
            message_id: Snowflake::from_raw(3),
            user_id: Snowflake::from_raw(4),
            emoji: "👍".into(),
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "t": "MESSAGE_REACTION_ADD",
                "data": {"channel_id": "2", "message_id": "3", "user_id": "4", "emoji": "👍"}
            })
        );
        assert_eq!(serde_json::from_value::<GatewayEvent>(json).unwrap(), event);
        let removed_emoji = GatewayEvent::MessageReactionRemoveEmoji {
            guild_id: Some(Snowflake::from_raw(1)),
            channel_id: Snowflake::from_raw(2),
            message_id: Snowflake::from_raw(3),
            emoji: "👍".into(),
        };
        assert_eq!(
            serde_json::to_value(&removed_emoji).unwrap(),
            serde_json::json!({
                "t": "MESSAGE_REACTION_REMOVE_EMOJI",
                "data": {"guild_id": "1", "channel_id": "2", "message_id": "3", "emoji": "👍"}
            })
        );
        let removed_all = GatewayEvent::MessageReactionRemoveAll {
            guild_id: None,
            channel_id: Snowflake::from_raw(2),
            message_id: Snowflake::from_raw(3),
        };
        assert_eq!(
            serde_json::to_value(&removed_all).unwrap(),
            serde_json::json!({
                "t": "MESSAGE_REACTION_REMOVE_ALL",
                "data": {"channel_id": "2", "message_id": "3"}
            })
        );
    }

    #[test]
    fn ready_from_an_older_server_has_no_conversations() {
        let ready: Ready = serde_json::from_str(
            r#"{"session_id":"s","user":{"id":"1","username":"a","display_name":"A","email":"a@example.org","is_instance_administrator":false},"guilds":[],"read_states":[],"online_user_ids":[]}"#,
        )
        .unwrap();
        assert_eq!(ready.direct_message_channels, Vec::new());
        assert_eq!(ready.presences, Vec::new());
        assert_eq!(ready.user.status, crate::ChosenStatus::Online);
        assert_eq!(ready.user.custom_status, None);
    }

    #[test]
    fn presence_updates_keep_their_v1_shape() {
        let event = GatewayEvent::PresenceUpdate(Presence {
            user_id: Snowflake::from_raw(4),
            status: PresenceStatus::DoNotDisturb,
            custom_status: Some("Focus".into()),
        });
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "t": "PRESENCE_UPDATE",
                "data": {"user_id": "4", "status": "do_not_disturb", "custom_status": "Focus"}
            })
        );
        assert_eq!(serde_json::from_value::<GatewayEvent>(json).unwrap(), event);
        let from_an_older_server: GatewayEvent = serde_json::from_value(serde_json::json!({
            "t": "PRESENCE_UPDATE",
            "data": {"user_id": "4", "status": "offline"}
        }))
        .unwrap();
        assert_eq!(
            from_an_older_server,
            GatewayEvent::PresenceUpdate(Presence {
                user_id: Snowflake::from_raw(4),
                status: PresenceStatus::Offline,
                custom_status: None,
            })
        );
    }
}
