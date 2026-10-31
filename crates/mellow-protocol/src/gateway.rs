//! WebSocket gateway frames (ARCHITECTURE.md §4.8), at `/gateway?v=1`.
//!
//! Every frame is a JSON object `{ "op": ..., "d": ... }`; dispatches add a sequence number `s`
//! and carry the event type in `d.t`.

use serde::{Deserialize, Serialize};

use crate::{
    Channel, ChannelUnread, CurrentUser, CustomEmoji, DirectMessageChannel, Guild, GuildSummary,
    Member, Message, PublicUser, ReadState, Relationship, Role, Snowflake,
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
    /// Joins a voice channel, moves to another one, or reconnects the media of the current one
    /// (an ICE restart after a network change: same channel, no leave or join is announced). The
    /// offer holds one `sendonly` or `sendrecv` audio m-line, then 1 to
    /// `MAXIMUM_VOICE_RECEIVE_SLOTS` `recvonly` ones (ADR-0030). Answered on this session only by
    /// `VOICE_SESSION_DESCRIPTION` or `VOICE_JOIN_REFUSED`.
    VoiceJoin {
        channel_id: Snowflake,
        /// Chosen by the client, opaque to the server and echoed in the answer or refusal, so a
        /// client that sent several offers applies only the answer to its latest.
        offer_id: u32,
        sdp_offer: String,
        self_mute: bool,
        self_deaf: bool,
    },
    /// Leaves the voice channel this session is connected to, if any.
    VoiceLeave,
    /// Changes the user's own mute and deafen while connected.
    VoiceState { self_mute: bool, self_deaf: bool },
}

/// The most voices one participant hears at once: receive m-lines of a voice offer (ADR-0030).
pub const MAXIMUM_VOICE_RECEIVE_SLOTS: usize = 16;

/// The largest SDP offer the server reads, in bytes.
pub const MAXIMUM_VOICE_OFFER_BYTES: usize = 64 * 1024;

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
            // The SDP holds addresses and keys: never printed.
            Self::VoiceJoin {
                channel_id,
                offer_id,
                self_mute,
                self_deaf,
                ..
            } => formatter
                .debug_struct("VoiceJoin")
                .field("channel_id", channel_id)
                .field("offer_id", offer_id)
                .field("sdp_offer", &"<redacted>")
                .field("self_mute", self_mute)
                .field("self_deaf", self_deaf)
                .finish(),
            Self::VoiceLeave => formatter.write_str("VoiceLeave"),
            Self::VoiceState {
                self_mute,
                self_deaf,
            } => formatter
                .debug_struct("VoiceState")
                .field("self_mute", self_mute)
                .field("self_deaf", self_deaf)
                .finish(),
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
    /// The user's friends, requests and blocks (ADR-0025). Added after `v0.12.0`: absent from
    /// older servers.
    #[serde(default)]
    pub relationships: Vec<Relationship>,
    /// Whether the user may create servers: the instance lets everyone or only its
    /// administrators (`guild_creation` in its configuration). Added after `v0.16.0`: absent from
    /// older servers, which let everyone.
    #[serde(default = "may_create_guilds_on_older_servers")]
    pub may_create_guilds: bool,
    /// Who is connected to the voice channels the user can view (ADR-0030). Added after
    /// `v0.22.0`: absent from older servers.
    #[serde(default)]
    pub voice_states: Vec<VoiceState>,
    /// Whether voice works on this instance: enabled, and its UDP port open. Clients hide voice
    /// actions otherwise. Added after `v0.22.0`: older servers have no voice.
    #[serde(default)]
    pub voice_enabled: bool,
}

/// One person connected to a voice channel, as the channel's viewers see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceState {
    pub user_id: Snowflake,
    pub guild_id: Snowflake,
    pub channel_id: Snowflake,
    /// The person muted their own microphone.
    pub self_mute: bool,
    /// The person turned their own sound off (which also mutes them).
    pub self_deaf: bool,
    /// The person lacks `SPEAK` here: the server forwards none of their audio.
    pub suppressed: bool,
}

/// Why the server refused a `voice_join`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceJoinRefusal {
    /// `VIEW_CHANNEL` or `CONNECT` is missing, or the channel is not a voice channel the user
    /// can see.
    MissingPermissions,
    /// The channel holds its `user_limit`, or the hard cap of 25.
    ChannelFull,
    /// The instance holds its configured number of voice participants.
    InstanceFull,
    /// The SDP offer is malformed, too large, or not audio-only with the expected m-lines.
    InvalidOffer,
    /// Voice is disabled on this instance, or its UDP port could not be opened.
    VoiceUnavailable,
    /// Too many joins in a short time.
    RateLimited,
}

/// Why the server ended a session's voice connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceDisconnectReason {
    /// The same user joined voice from another session.
    MovedElsewhere,
    /// The user lost `VIEW_CHANNEL` or `CONNECT` on the channel.
    PermissionsChanged,
    ChannelDeleted,
    /// The user left the guild, or was kicked or banned.
    RemovedFromGuild,
    /// The media connection never came up within 15 s, or carried nothing for 30 s.
    MediaTimeout,
}

const fn may_create_guilds_on_older_servers() -> bool {
    true
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
    /// The stored key, as in `ReactionSummary::emoji`.
    pub emoji: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus {
    Online,
    /// `Idle` and `DoNotDisturb` were added after `v0.11.0`: older clients cannot parse them and
    /// skip the frame.
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
    /// A custom emoji was added, renamed or deleted: the guild's full list, as in `Guild::emojis`.
    /// Sent to every member. Added after `v0.19.0`: older clients skip it.
    GuildEmojisUpdate {
        guild_id: Snowflake,
        emojis: Vec<CustomEmoji>,
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
    /// A group's name, owner or participants changed; sent to each participant as they see it.
    /// Added after `v0.13.0`: older clients skip it.
    DirectMessageChannelUpdate(DirectMessageChannel),
    /// The user left or was removed from a group, which they can no longer read.
    DirectMessageChannelDelete {
        id: Snowflake,
    },
    /// A relationship of the user was created or changed kind (a request sent or received, a
    /// friendship, a block). Sent to that user only (ADR-0025).
    RelationshipAdd(Relationship),
    /// A relationship of the user ended: removed, refused, cancelled, unblocked, or, for the
    /// person blocked, replaced by a block they are not told about.
    RelationshipRemove {
        user_id: Snowflake,
    },
    /// A user changed their username or display name. Sent to that user and to every member of a
    /// guild they share.
    UserUpdate(PublicUser),
    /// Someone joined a voice channel, moved, or changed their mute, deafen or suppression. Sent
    /// to the members who can view that channel. Added after `v0.22.0`, like every voice event.
    VoiceStateUpdate {
        voice_state: VoiceState,
    },
    /// Someone left a voice channel, or was disconnected. Sent to the members who can view it.
    VoiceStateDelete {
        guild_id: Snowflake,
        channel_id: Snowflake,
        user_id: Snowflake,
    },
    /// The server's SDP answer to this session's `voice_join`. Sent to that session only.
    VoiceSessionDescription {
        channel_id: Snowflake,
        /// The `offer_id` of the `voice_join` answered.
        offer_id: u32,
        sdp_answer: String,
    },
    /// The server refused this session's `voice_join`. Sent to that session only.
    VoiceJoinRefused {
        channel_id: Snowflake,
        /// The `offer_id` of the `voice_join` refused.
        offer_id: u32,
        reason: VoiceJoinRefusal,
    },
    /// The server ended this session's voice connection. Sent to that session only; the others
    /// learn it from `VOICE_STATE_DELETE` (or `VOICE_STATE_UPDATE` after a move).
    VoiceDisconnected {
        channel_id: Snowflake,
        reason: VoiceDisconnectReason,
    },
    /// Someone started or stopped speaking in a voice channel, as the server hears it from the
    /// audio level of their packets. Sent to the members who can view the channel; never for a
    /// muted or suppressed person.
    Speaking {
        guild_id: Snowflake,
        channel_id: Snowflake,
        user_id: Snowflake,
        speaking: bool,
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
    fn emoji_updates_carry_the_whole_list_and_older_guilds_have_none() {
        let json = serde_json::to_value(GatewayEvent::GuildEmojisUpdate {
            guild_id: Snowflake::from_raw(7),
            emojis: vec![CustomEmoji {
                id: Snowflake::from_raw(8),
                name: "party_parrot".into(),
                is_animated: true,
            }],
        })
        .unwrap();
        assert_eq!(json["t"], "GUILD_EMOJIS_UPDATE");
        assert_eq!(json["data"]["guild_id"], "7");
        assert_eq!(json["data"]["emojis"][0]["id"], "8");
        assert_eq!(json["data"]["emojis"][0]["is_animated"], true);
        let older: Guild = serde_json::from_str(
            r#"{"id":"7","name":"Club","owner_id":"1","my_permissions":"0","my_role_ids":[],
               "roles":[],"channels":[]}"#,
        )
        .unwrap();
        assert_eq!(older.emojis, []);
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
        assert!(ready.may_create_guilds);
        assert_eq!(ready.user.custom_status, None);
        assert_eq!(ready.voice_states, Vec::new());
        assert!(!ready.voice_enabled);
    }

    #[test]
    fn voice_frames_from_clients_keep_their_shape_and_hide_the_offer() {
        let join: ClientFrame = serde_json::from_value(serde_json::json!({
            "op": "voice_join",
            "d": {"channel_id": "5", "offer_id": 7, "sdp_offer": "v=0 secret", "self_mute": false, "self_deaf": true}
        }))
        .unwrap();
        assert_eq!(
            join,
            ClientFrame::VoiceJoin {
                channel_id: Snowflake::from_raw(5),
                offer_id: 7,
                sdp_offer: "v=0 secret".into(),
                self_mute: false,
                self_deaf: true,
            }
        );
        assert!(!format!("{join:?}").contains("secret"), "SDP is redacted");
        let json = serde_json::to_value(&join).unwrap();
        assert_eq!(serde_json::from_value::<ClientFrame>(json).unwrap(), join);

        let leave: ClientFrame = serde_json::from_str(r#"{"op":"voice_leave"}"#).unwrap();
        assert_eq!(leave, ClientFrame::VoiceLeave);
        let state: ClientFrame = serde_json::from_str(
            r#"{"op":"voice_state","d":{"self_mute":true,"self_deaf":false}}"#,
        )
        .unwrap();
        assert_eq!(
            state,
            ClientFrame::VoiceState {
                self_mute: true,
                self_deaf: false
            }
        );
        assert!(
            serde_json::from_str::<ClientFrame>(
                r#"{"op":"voice_state","d":{"self_mute":true,"self_deaf":false,"x":1}}"#
            )
            .is_err(),
            "unknown fields are refused"
        );
    }

    #[test]
    fn voice_events_carry_string_ids_and_snake_case_reasons() {
        let voice_state = VoiceState {
            user_id: Snowflake::from_raw(3),
            guild_id: Snowflake::from_raw(1),
            channel_id: Snowflake::from_raw(2),
            self_mute: true,
            self_deaf: false,
            suppressed: false,
        };
        let cases = [
            (
                GatewayEvent::VoiceStateUpdate {
                    voice_state: voice_state.clone(),
                },
                serde_json::json!({"t": "VOICE_STATE_UPDATE", "data": {"voice_state": {
                    "user_id": "3", "guild_id": "1", "channel_id": "2",
                    "self_mute": true, "self_deaf": false, "suppressed": false
                }}}),
            ),
            (
                GatewayEvent::VoiceStateDelete {
                    guild_id: Snowflake::from_raw(1),
                    channel_id: Snowflake::from_raw(2),
                    user_id: Snowflake::from_raw(3),
                },
                serde_json::json!({"t": "VOICE_STATE_DELETE",
                    "data": {"guild_id": "1", "channel_id": "2", "user_id": "3"}}),
            ),
            (
                GatewayEvent::VoiceSessionDescription {
                    channel_id: Snowflake::from_raw(2),
                    offer_id: 7,
                    sdp_answer: "v=0".into(),
                },
                serde_json::json!({"t": "VOICE_SESSION_DESCRIPTION",
                    "data": {"channel_id": "2", "offer_id": 7, "sdp_answer": "v=0"}}),
            ),
            (
                GatewayEvent::VoiceJoinRefused {
                    channel_id: Snowflake::from_raw(2),
                    offer_id: 7,
                    reason: VoiceJoinRefusal::ChannelFull,
                },
                serde_json::json!({"t": "VOICE_JOIN_REFUSED",
                    "data": {"channel_id": "2", "offer_id": 7, "reason": "channel_full"}}),
            ),
            (
                GatewayEvent::VoiceDisconnected {
                    channel_id: Snowflake::from_raw(2),
                    reason: VoiceDisconnectReason::MovedElsewhere,
                },
                serde_json::json!({"t": "VOICE_DISCONNECTED",
                    "data": {"channel_id": "2", "reason": "moved_elsewhere"}}),
            ),
            (
                GatewayEvent::Speaking {
                    guild_id: Snowflake::from_raw(1),
                    channel_id: Snowflake::from_raw(2),
                    user_id: Snowflake::from_raw(3),
                    speaking: true,
                },
                serde_json::json!({"t": "SPEAKING", "data": {
                    "guild_id": "1", "channel_id": "2", "user_id": "3", "speaking": true
                }}),
            ),
        ];
        for (event, expected) in cases {
            let json = serde_json::to_value(&event).unwrap();
            assert_eq!(json, expected);
            assert_eq!(serde_json::from_value::<GatewayEvent>(json).unwrap(), event);
        }
        for (refusal, wire) in [
            (VoiceJoinRefusal::MissingPermissions, "missing_permissions"),
            (VoiceJoinRefusal::ChannelFull, "channel_full"),
            (VoiceJoinRefusal::InstanceFull, "instance_full"),
            (VoiceJoinRefusal::InvalidOffer, "invalid_offer"),
            (VoiceJoinRefusal::VoiceUnavailable, "voice_unavailable"),
            (VoiceJoinRefusal::RateLimited, "rate_limited"),
        ] {
            assert_eq!(serde_json::to_value(refusal).unwrap(), wire);
        }
        for (reason, wire) in [
            (VoiceDisconnectReason::MovedElsewhere, "moved_elsewhere"),
            (
                VoiceDisconnectReason::PermissionsChanged,
                "permissions_changed",
            ),
            (VoiceDisconnectReason::ChannelDeleted, "channel_deleted"),
            (
                VoiceDisconnectReason::RemovedFromGuild,
                "removed_from_guild",
            ),
            (VoiceDisconnectReason::MediaTimeout, "media_timeout"),
        ] {
            assert_eq!(serde_json::to_value(reason).unwrap(), wire);
        }
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
