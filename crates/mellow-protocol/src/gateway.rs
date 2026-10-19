//! WebSocket gateway frames (ARCHITECTURE.md §4.8), at `/gateway?v=1`.
//!
//! Every frame is a JSON object `{ "op": ..., "d": ... }`; dispatches add a sequence number `s`
//! and carry the event type in `d.t`.

use serde::{Deserialize, Serialize};

use crate::{Channel, CurrentUser, Guild, Member, Message, ReadState, Snowflake};

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
    TypingStart {
        guild_id: Snowflake,
        channel_id: Snowflake,
        user_id: Snowflake,
    },
    PresenceUpdate {
        user_id: Snowflake,
        status: PresenceStatus,
    },
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
}
