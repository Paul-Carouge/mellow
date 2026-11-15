//! Private calls in conversations (ADR-0062): a voice session of a one-to-one or group
//! conversation, relayed by the voice channels' SFU under the conversation's id.

use serde::{Deserialize, Serialize};

use crate::{Snowflake, VideoCodec};

/// How long a call rings before it stops ringing for everyone who did not answer. Clients stop
/// their ringtone after it even without an event.
pub const CALL_RING_TIMEOUT_MILLISECONDS: u64 = 30_000;

/// A call going on in a conversation, as its participants see it (`CALL_CREATE`, `CALL_UPDATE`,
/// `Ready.calls`, the answer to `POST /api/v1/channels/{channel_id}/call`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    /// The conversation's id; also the id `voice_join` names to answer or join the call.
    pub channel_id: Snowflake,
    /// The call's message in the conversation (`Message::call`); its id gives the start time.
    pub message_id: Snowflake,
    /// Who started it.
    pub caller_id: Snowflake,
    /// Whose devices ring now: everyone in the conversation but the caller and those with a
    /// block either way with them, until each answers, declines, or ringing times out.
    pub ringing_user_ids: Vec<Snowflake>,
    /// Who is connected, in the order they came.
    pub participants: Vec<CallParticipant>,
}

/// One person connected to a call.
// The flags are independent wire fields, as in `VoiceState`.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallParticipant {
    pub user_id: Snowflake,
    pub self_mute: bool,
    pub self_deaf: bool,
    /// Their camera is on and forwarded (ADR-0035).
    pub self_video: bool,
    /// They share a screen (ADR-0037); never with `self_video`.
    pub self_stream: bool,
    /// The codec of their camera or screen (ADR-0052); absent when their offer has no video.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_codec: Option<VideoCodec>,
}

/// A call message's record (`Message::call`): the conversation's trace of a call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageCall {
    /// Everyone who joined at least once, the caller included when they did; written when the
    /// call ends, empty before. A reader not listed once it ended missed it.
    #[serde(default)]
    pub participant_ids: Vec<Snowflake>,
    /// When it ended; absent while it goes on. Its duration counts from the message's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_unix_milliseconds: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GatewayEvent, Message, PublicUser, Ready};

    fn id(raw: u64) -> Snowflake {
        Snowflake::from_raw(raw)
    }

    #[test]
    fn call_events_keep_their_wire_shape() {
        let call = Call {
            channel_id: id(1),
            message_id: id(2),
            caller_id: id(3),
            ringing_user_ids: vec![id(4)],
            participants: vec![CallParticipant {
                user_id: id(3),
                self_mute: true,
                self_deaf: false,
                self_video: false,
                self_stream: true,
                video_codec: Some(VideoCodec::H264),
            }],
        };
        let json = serde_json::json!({"t": "CALL_CREATE", "data": {
            "channel_id": "1", "message_id": "2", "caller_id": "3", "ringing_user_ids": ["4"],
            "participants": [{"user_id": "3", "self_mute": true, "self_deaf": false,
                "self_video": false, "self_stream": true, "video_codec": "h264"}]}});
        assert_eq!(
            serde_json::to_value(GatewayEvent::CallCreate(call.clone())).unwrap(),
            json
        );
        let back: GatewayEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, GatewayEvent::CallCreate(call));
        assert_eq!(
            serde_json::to_value(GatewayEvent::CallDelete { channel_id: id(1) }).unwrap(),
            serde_json::json!({"t": "CALL_DELETE", "data": {"channel_id": "1"}})
        );
        assert_eq!(
            serde_json::to_value(GatewayEvent::CallSpeaking {
                channel_id: id(1),
                user_id: id(3),
                speaking: true,
            })
            .unwrap(),
            serde_json::json!({"t": "CALL_SPEAKING", "data": {
                "channel_id": "1", "user_id": "3", "speaking": true}})
        );
    }

    #[test]
    fn a_call_that_ends_on_its_participants_says_so() {
        assert_eq!(
            serde_json::to_value(crate::VoiceDisconnectReason::CallEnded).unwrap(),
            "call_ended"
        );
    }

    #[test]
    fn a_participant_without_video_leaves_the_codec_out() {
        let participant = CallParticipant {
            user_id: id(3),
            self_mute: false,
            self_deaf: false,
            self_video: false,
            self_stream: false,
            video_codec: None,
        };
        let value = serde_json::to_value(&participant).unwrap();
        assert!(value.get("video_codec").is_none(), "{value}");
    }

    #[test]
    fn a_call_message_reads_with_or_without_its_end() {
        let ongoing: MessageCall = serde_json::from_str("{}").unwrap();
        assert_eq!(
            ongoing,
            MessageCall {
                participant_ids: Vec::new(),
                ended_at_unix_milliseconds: None,
            }
        );
        let ended = MessageCall {
            participant_ids: vec![id(3), id(4)],
            ended_at_unix_milliseconds: Some(720_000),
        };
        assert_eq!(
            serde_json::to_value(&ended).unwrap(),
            serde_json::json!({"participant_ids": ["3", "4"],
                "ended_at_unix_milliseconds": 720_000})
        );
    }

    #[test]
    fn older_payloads_have_no_call_and_newer_ones_are_read() {
        let message = Message {
            id: id(9),
            channel_id: id(1),
            author: PublicUser {
                id: id(3),
                username: "ana".into(),
                display_name: "Ana".into(),
                avatar: None,
                banner: None,
            },
            content: String::new(),
            reply_to_id: None,
            edited_at_unix_milliseconds: None,
            nonce: None,
            attachments: Vec::new(),
            reactions: Vec::new(),
            pinned: false,
            mention_ids: Vec::new(),
            mention_role_ids: Vec::new(),
            link_previews: Vec::new(),
            poll: None,
            checklist: None,
            progress_bar: None,
            forwarded_from: None,
            call: None,
        };
        let value = serde_json::to_value(&message).unwrap();
        assert!(value.get("call").is_none(), "absent when none: {value}");
        let mut with_call = value;
        with_call["call"] = serde_json::json!({"participant_ids": []});
        let read: Message = serde_json::from_value(with_call).unwrap();
        assert_eq!(read.call.map(|call| call.participant_ids), Some(Vec::new()));

        let ready: Ready = serde_json::from_value(serde_json::json!({
            "session_id": "s", "user": {"id": "3", "username": "ana", "display_name": "Ana",
                "email": "ana@example.org", "is_instance_administrator": false},
            "guilds": [], "read_states": [], "online_user_ids": []
        }))
        .unwrap();
        assert_eq!(ready.calls, []);
        assert!(!ready.calls_enabled, "older servers have no calls");
    }
}
