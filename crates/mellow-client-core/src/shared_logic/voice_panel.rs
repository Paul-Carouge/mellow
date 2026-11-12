//! Port of `web/src/lib/voice.ts` and `web/src/components/voice-panel.ts`: who joined or left
//! the channel the reader is in, how the latency reads, and what a voice problem offers. Words
//! stay out: the client's catalogue says them.

use mellow_protocol::{Snowflake, VoiceDisconnectReason, VoiceJoinRefusal};

/// At most this round trip reads as good, then medium; above, poor.
pub const GOOD_ROUND_TRIP_MILLISECONDS: u64 = 100;
pub const MEDIUM_ROUND_TRIP_MILLISECONDS: u64 = 250;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatencyQuality {
    Good,
    Medium,
    Poor,
}

#[must_use]
pub fn latency_quality(round_trip_milliseconds: u64) -> LatencyQuality {
    if round_trip_milliseconds <= GOOD_ROUND_TRIP_MILLISECONDS {
        LatencyQuality::Good
    } else if round_trip_milliseconds <= MEDIUM_ROUND_TRIP_MILLISECONDS {
        LatencyQuality::Medium
    } else {
        LatencyQuality::Poor
    }
}

/// Someone came into or went out of the reader's channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticipantChange {
    Joined,
    Left,
}

/// From their channel before and after an event (`None`: not connected anywhere visible).
#[must_use]
pub fn participant_change(
    previous_channel_id: Option<Snowflake>,
    next_channel_id: Option<Snowflake>,
    my_channel_id: Option<Snowflake>,
) -> Option<ParticipantChange> {
    let my_channel_id = my_channel_id?;
    let was_here = previous_channel_id == Some(my_channel_id);
    let is_here = next_channel_id == Some(my_channel_id);
    match (was_here, is_here) {
        (false, true) => Some(ParticipantChange::Joined),
        (true, false) => Some(ParticipantChange::Left),
        _ => None,
    }
}

/// Why voice stopped, or never started, in `channel_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceProblem {
    Refused {
        channel_id: Snowflake,
        reason: VoiceJoinRefusal,
    },
    /// The instance never answered the offer.
    NoAnswer {
        channel_id: Snowflake,
    },
    /// Answered, but audio never flowed: usually a closed UDP port.
    Unreachable {
        channel_id: Snowflake,
    },
    ConnectionLost {
        channel_id: Snowflake,
    },
    Disconnected {
        channel_id: Snowflake,
        reason: VoiceDisconnectReason,
    },
    /// The voice engine could not start (no socket, no audio thread, no address for the
    /// instance): a passing failure as often as not, so it may be retried.
    CouldNotStart {
        channel_id: Snowflake,
    },
}

impl VoiceProblem {
    #[must_use]
    pub fn channel_id(self) -> Snowflake {
        match self {
            Self::Refused { channel_id, .. }
            | Self::NoAnswer { channel_id }
            | Self::Unreachable { channel_id }
            | Self::ConnectionLost { channel_id }
            | Self::Disconnected { channel_id, .. }
            | Self::CouldNotStart { channel_id } => channel_id,
        }
    }

    /// Refusals that may pass on their own, and lost connections, offer a retry.
    #[must_use]
    pub fn can_retry(self) -> bool {
        match self {
            Self::Refused { reason, .. } => matches!(
                reason,
                VoiceJoinRefusal::ChannelFull
                    | VoiceJoinRefusal::InstanceFull
                    | VoiceJoinRefusal::RateLimited
            ),
            Self::NoAnswer { .. }
            | Self::Unreachable { .. }
            | Self::ConnectionLost { .. }
            | Self::CouldNotStart { .. } => true,
            Self::Disconnected { reason, .. } => reason == VoiceDisconnectReason::MediaTimeout,
        }
    }

    /// Voice moved to another device: a notice, not an error.
    #[must_use]
    pub fn is_notice(self) -> bool {
        matches!(
            self,
            Self::Disconnected {
                reason: VoiceDisconnectReason::MovedElsewhere,
                ..
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOUNGE: Snowflake = Snowflake::from_raw(1);

    #[test]
    fn drops_a_bar_as_the_round_trip_grows() {
        assert_eq!(latency_quality(42), LatencyQuality::Good);
        assert_eq!(latency_quality(100), LatencyQuality::Good);
        assert_eq!(latency_quality(180), LatencyQuality::Medium);
        assert_eq!(latency_quality(400), LatencyQuality::Poor);
    }

    #[test]
    fn offers_a_retry_only_where_waiting_may_help() {
        let refused = |reason| VoiceProblem::Refused {
            channel_id: LOUNGE,
            reason,
        };
        assert!(refused(VoiceJoinRefusal::ChannelFull).can_retry());
        assert!(!refused(VoiceJoinRefusal::MissingPermissions).can_retry());
        assert!(VoiceProblem::Unreachable { channel_id: LOUNGE }.can_retry());
        let disconnected = |reason| VoiceProblem::Disconnected {
            channel_id: LOUNGE,
            reason,
        };
        assert!(disconnected(VoiceDisconnectReason::MediaTimeout).can_retry());
        assert!(!disconnected(VoiceDisconnectReason::ChannelDeleted).can_retry());
        assert!(VoiceProblem::CouldNotStart { channel_id: LOUNGE }.can_retry());
    }

    #[test]
    fn tells_a_move_to_another_device_as_a_notice_not_an_error() {
        let moved = VoiceProblem::Disconnected {
            channel_id: LOUNGE,
            reason: VoiceDisconnectReason::MovedElsewhere,
        };
        assert!(moved.is_notice());
        assert!(!moved.can_retry());
        assert!(!VoiceProblem::NoAnswer { channel_id: LOUNGE }.is_notice());
    }

    #[test]
    fn tells_who_joined_or_left_my_channel_only() {
        let other = Snowflake::from_raw(2);
        assert_eq!(
            participant_change(None, Some(LOUNGE), Some(LOUNGE)),
            Some(ParticipantChange::Joined)
        );
        assert_eq!(
            participant_change(Some(LOUNGE), Some(other), Some(LOUNGE)),
            Some(ParticipantChange::Left)
        );
        assert_eq!(
            participant_change(Some(LOUNGE), Some(LOUNGE), Some(LOUNGE)),
            None
        );
        assert_eq!(participant_change(None, Some(other), Some(LOUNGE)), None);
        assert_eq!(participant_change(None, Some(LOUNGE), None), None);
    }
}
