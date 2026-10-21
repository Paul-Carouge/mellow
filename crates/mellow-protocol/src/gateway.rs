//! WebSocket gateway frames (ARCHITECTURE.md §4.8), at `/gateway?v=1`.
//!
//! Every frame is a JSON object `{ "op": ..., "d": ... }`; dispatches add a sequence number `s`
//! and carry the event type in `d.t`.

use serde::{Deserialize, Serialize};

use crate::{Channel, CurrentUser, Guild, Member, Message, PublicUser, ReadState, Role, Snowflake};

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
        event: GatewayEvent,
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
    /// Members of the user's guilds who are online now.
    pub online_user_ids: Vec<Snowflake>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus {
    Online,
    Offline,
}

/// Events pushed to clients. `t` names the event, `data` carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GatewayEvent {
    MessageCreate(Message),
    MessageUpdate(Message),
    MessageDelete {
        guild_id: Snowflake,
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
    /// The user left a guild (or it disappeared for them).
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
    TypingStart {
        guild_id: Snowflake,
        channel_id: Snowflake,
        user_id: Snowflake,
    },
    PresenceUpdate {
        user_id: Snowflake,
        status: PresenceStatus,
    },
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
            event: GatewayEvent::TypingStart {
                guild_id: Snowflake::from_raw(1),
                channel_id: Snowflake::from_raw(2),
                user_id: Snowflake::from_raw(3),
            },
        };
        let json = serde_json::to_value(&dispatch).unwrap();
        assert_eq!(json["op"], "dispatch");
        assert_eq!(json["s"], 7);
        assert_eq!(json["d"]["t"], "TYPING_START");
        assert_eq!(json["d"]["data"]["channel_id"], "2");
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
}
