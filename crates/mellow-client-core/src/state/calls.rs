//! Private calls in conversations (ADR-0062): the calls going on, what each one is for the
//! signed-in user (ringing them, theirs, joinable), the sound to play, and the trace a call's
//! message shows. Answering, joining and hanging up go through the voice session with the
//! conversation's id; starting and declining through `RestClient::start_call` and
//! `RestClient::decline_call`.

use mellow_protocol::{CALL_RING_TIMEOUT_MILLISECONDS, Call, MessageCall, Snowflake};

use super::{ClientState, StateChange};

/// What a call going on is for the signed-in user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallPhase {
    /// It rings them: answer (`voice_join` with the conversation's id) or decline.
    Incoming,
    /// They are in it, no one else is yet, and others are being rung.
    Outgoing,
    /// They are in it, with others or after others left.
    Ongoing,
    /// It goes on without them and does not ring them: they may join it.
    Joinable,
}

/// What a call's message shows to its reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallMessageSummary {
    /// The call goes on.
    Ongoing,
    /// It ended without the reader ever joining: "Missed call".
    Missed,
    /// It ended with the reader as its only participant: their own call nobody answered
    /// ("Unanswered call"), for the caller who rang alone.
    Unanswered,
    /// It ended; the reader took part. Its length, from the start to the end.
    Ended { duration_milliseconds: u64 },
}

/// The sound a client plays for a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallSound {
    /// The ringtone (`crate::ringtones`), looped while the call rings the user.
    Ringtone,
    /// A short discreet cue: the call rings a user already in another call or voice channel.
    CallWaiting,
    /// The caller's waiting tone, while others are rung (ADR-0062: its sound is the owner's to
    /// choose).
    Ringback,
}

/// What `call` is for `user_id`.
#[must_use]
pub fn call_phase(call: &Call, user_id: Snowflake) -> CallPhase {
    let is_in = call
        .participants
        .iter()
        .any(|participant| participant.user_id == user_id);
    if !is_in {
        return if call.ringing_user_ids.contains(&user_id) {
            CallPhase::Incoming
        } else {
            CallPhase::Joinable
        };
    }
    let is_alone = call.participants.len() == 1;
    if is_alone && !call.ringing_user_ids.is_empty() {
        CallPhase::Outgoing
    } else {
        CallPhase::Ongoing
    }
}

/// The sound for a call in `phase`: none under do not disturb (the call still shows), a discreet
/// cue for someone already in a call or voice channel, otherwise the ringtone; the caller hears
/// the waiting tone.
#[must_use]
pub const fn call_sound(
    phase: CallPhase,
    is_in_another_call_or_voice_channel: bool,
    is_do_not_disturb: bool,
) -> Option<CallSound> {
    match phase {
        CallPhase::Incoming if is_do_not_disturb => None,
        CallPhase::Incoming if is_in_another_call_or_voice_channel => Some(CallSound::CallWaiting),
        CallPhase::Incoming => Some(CallSound::Ringtone),
        CallPhase::Outgoing => Some(CallSound::Ringback),
        CallPhase::Ongoing | CallPhase::Joinable => None,
    }
}

/// When a call stops ringing at the latest, as Unix milliseconds: a client stops its ringtone
/// then even if no event came.
#[must_use]
pub const fn ringing_ends_at_unix_milliseconds(call: &Call) -> u64 {
    call.message_id
        .unix_milliseconds()
        .saturating_add(CALL_RING_TIMEOUT_MILLISECONDS)
}

/// What the message `message_id` with this call record shows to `reader_id`.
#[must_use]
pub fn call_message_summary(
    call: &MessageCall,
    message_id: Snowflake,
    reader_id: Snowflake,
) -> CallMessageSummary {
    match call.ended_at_unix_milliseconds {
        None => CallMessageSummary::Ongoing,
        Some(_) if !call.participant_ids.contains(&reader_id) => CallMessageSummary::Missed,
        Some(_) if call.participant_ids.len() == 1 => CallMessageSummary::Unanswered,
        Some(ended_at) => CallMessageSummary::Ended {
            duration_milliseconds: ended_at.saturating_sub(message_id.unix_milliseconds()),
        },
    }
}

impl ClientState {
    /// The calls going on in the user's conversations.
    #[must_use]
    pub fn calls(&self) -> &[Call] {
        &self.calls
    }

    #[must_use]
    pub fn call(&self, channel_id: Snowflake) -> Option<&Call> {
        self.calls.iter().find(|call| call.channel_id == channel_id)
    }

    /// Whether the instance has calls (`Ready.calls_enabled`): hide the call button otherwise.
    #[must_use]
    pub fn is_calls_enabled(&self) -> bool {
        self.capabilities.is_calls_enabled
    }

    /// What the call of `channel_id` is for the signed-in user.
    #[must_use]
    pub fn call_phase(&self, channel_id: Snowflake) -> Option<CallPhase> {
        let user_id = self.current_user_id()?;
        self.call(channel_id).map(|call| call_phase(call, user_id))
    }

    /// The calls ringing the signed-in user now, oldest first: the incoming call screen shows
    /// the first, and plays `call_sound` for it.
    pub fn incoming_calls(&self) -> impl Iterator<Item = &Call> {
        let user_id = self.current_user_id();
        let mut incoming: Vec<&Call> = self
            .calls
            .iter()
            .filter(|call| {
                user_id.is_some_and(|user_id| call_phase(call, user_id) == CallPhase::Incoming)
            })
            .collect();
        // Ids grow with time: the call's message id says which started first.
        incoming.sort_unstable_by_key(|call| call.message_id.raw());
        incoming.into_iter()
    }

    /// Whether the signed-in user is connected to a call or a voice channel, from any device:
    /// a new call then plays `CallSound::CallWaiting`.
    #[must_use]
    pub fn is_in_a_call_or_voice_channel(&self) -> bool {
        let Some(user_id) = self.current_user_id() else {
            return false;
        };
        self.voice_state_of(user_id).is_some()
            || self.calls.iter().any(|call| {
                call.participants
                    .iter()
                    .any(|participant| participant.user_id == user_id)
            })
    }

    /// Whether a call starts with `user_id`'s sound off for the user: someone they blocked may
    /// still be in a group's call (ADR-0026, ADR-0062); the user may turn them up.
    #[must_use]
    pub fn is_muted_by_default_in_calls(&self, user_id: Snowflake) -> bool {
        self.is_blocked(user_id)
    }

    /// `CALL_CREATE` and `CALL_UPDATE`: the call as it is now, in a known conversation.
    pub(super) fn apply_call(&mut self, call: Call) -> StateChange {
        let channel_id = call.channel_id;
        if self.direct_message_channel(channel_id).is_none() {
            return StateChange::Nothing;
        }
        let speaking_user_ids = &mut self.speaking_user_ids;
        if let Some(existing) = self
            .calls
            .iter_mut()
            .find(|existing| existing.channel_id == channel_id)
        {
            for left in &existing.participants {
                if !call
                    .participants
                    .iter()
                    .any(|participant| participant.user_id == left.user_id)
                {
                    speaking_user_ids.remove(&left.user_id);
                }
            }
            *existing = call;
        } else {
            self.calls.push(call);
        }
        StateChange::Call { channel_id }
    }

    /// `CALL_DELETE`, or the conversation gone.
    pub(super) fn drop_call(&mut self, channel_id: Snowflake) -> StateChange {
        let Some(index) = self
            .calls
            .iter()
            .position(|call| call.channel_id == channel_id)
        else {
            return StateChange::Nothing;
        };
        let call = self.calls.remove(index);
        for participant in &call.participants {
            self.speaking_user_ids.remove(&participant.user_id);
        }
        StateChange::Call { channel_id }
    }

    /// `CALL_SPEAKING`: only for someone in that call.
    pub(super) fn note_call_speaking(
        &mut self,
        channel_id: Snowflake,
        user_id: Snowflake,
        is_speaking: bool,
    ) -> StateChange {
        let is_in_call = self.call(channel_id).is_some_and(|call| {
            call.participants
                .iter()
                .any(|participant| participant.user_id == user_id)
        });
        let is_changed = if is_speaking {
            is_in_call && self.speaking_user_ids.insert(user_id)
        } else {
            self.speaking_user_ids.remove(&user_id)
        };
        if is_changed {
            StateChange::Speaking {
                channel_id,
                user_id,
            }
        } else {
            StateChange::Nothing
        }
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{CallParticipant, MELLOW_EPOCH_UNIX_MILLISECONDS};

    use super::*;

    const ANA: Snowflake = Snowflake::from_raw(1);
    const BOB: Snowflake = Snowflake::from_raw(2);
    const CLEO: Snowflake = Snowflake::from_raw(3);

    fn connected(user_id: Snowflake) -> CallParticipant {
        CallParticipant {
            user_id,
            self_mute: false,
            self_deaf: false,
            self_video: false,
            self_stream: false,
            video_codec: None,
        }
    }

    fn call(ringing: &[Snowflake], participants: &[Snowflake]) -> Call {
        Call {
            channel_id: Snowflake::from_raw(50),
            message_id: Snowflake::from_parts(60_000, 0).unwrap(),
            caller_id: ANA,
            ringing_user_ids: ringing.to_vec(),
            participants: participants.iter().copied().map(connected).collect(),
        }
    }

    #[test]
    fn a_call_is_incoming_outgoing_ongoing_or_joinable() {
        let ringing_bob = call(&[BOB, CLEO], &[ANA]);
        assert_eq!(call_phase(&ringing_bob, BOB), CallPhase::Incoming);
        assert_eq!(call_phase(&ringing_bob, ANA), CallPhase::Outgoing);
        let answered = call(&[CLEO], &[ANA, BOB]);
        assert_eq!(call_phase(&answered, ANA), CallPhase::Ongoing);
        assert_eq!(call_phase(&answered, CLEO), CallPhase::Incoming);
        let declined_by_cleo = call(&[], &[ANA, BOB]);
        assert_eq!(call_phase(&declined_by_cleo, CLEO), CallPhase::Joinable);
        let alone_after_the_others_left = call(&[], &[BOB]);
        assert_eq!(
            call_phase(&alone_after_the_others_left, BOB),
            CallPhase::Ongoing
        );
    }

    #[test]
    fn do_not_disturb_silences_ringing_and_a_busy_user_hears_a_cue() {
        assert_eq!(
            call_sound(CallPhase::Incoming, false, false),
            Some(CallSound::Ringtone)
        );
        assert_eq!(
            call_sound(CallPhase::Incoming, true, false),
            Some(CallSound::CallWaiting)
        );
        assert_eq!(call_sound(CallPhase::Incoming, true, true), None);
        assert_eq!(
            call_sound(CallPhase::Outgoing, false, true),
            Some(CallSound::Ringback),
            "do not disturb is about being disturbed, not calling"
        );
        assert_eq!(call_sound(CallPhase::Ongoing, false, false), None);
        assert_eq!(call_sound(CallPhase::Joinable, false, false), None);
    }

    #[test]
    fn ringing_ends_thirty_seconds_after_the_start_at_the_latest() {
        let started = call(&[BOB], &[]);
        assert_eq!(
            ringing_ends_at_unix_milliseconds(&started),
            MELLOW_EPOCH_UNIX_MILLISECONDS + 60_000 + 30_000
        );
    }

    #[test]
    fn a_call_message_is_ongoing_missed_or_ended_with_its_length() {
        let message_id = Snowflake::from_parts(60_000, 0).unwrap();
        let ongoing = MessageCall {
            participant_ids: Vec::new(),
            ended_at_unix_milliseconds: None,
        };
        assert_eq!(
            call_message_summary(&ongoing, message_id, BOB),
            CallMessageSummary::Ongoing
        );
        let ended = MessageCall {
            participant_ids: vec![ANA, BOB],
            ended_at_unix_milliseconds: Some(MELLOW_EPOCH_UNIX_MILLISECONDS + 60_000 + 720_000),
        };
        assert_eq!(
            call_message_summary(&ended, message_id, BOB),
            CallMessageSummary::Ended {
                duration_milliseconds: 720_000
            }
        );
        assert_eq!(
            call_message_summary(&ended, message_id, CLEO),
            CallMessageSummary::Missed
        );
    }

    #[test]
    fn the_caller_reads_their_unanswered_call_as_unanswered_and_the_callee_as_missed() {
        let message_id = Snowflake::from_parts(60_000, 0).unwrap();
        let unanswered = MessageCall {
            participant_ids: vec![ANA],
            ended_at_unix_milliseconds: Some(MELLOW_EPOCH_UNIX_MILLISECONDS + 60_000 + 30_000),
        };
        assert_eq!(
            call_message_summary(&unanswered, message_id, ANA),
            CallMessageSummary::Unanswered
        );
        assert_eq!(
            call_message_summary(&unanswered, message_id, BOB),
            CallMessageSummary::Missed
        );
    }
}

#[cfg(test)]
mod blocked_people_tests {
    use mellow_protocol::{GatewayEvent, Relationship, RelationshipKind, Snowflake};

    use crate::state::tests::{NOW, OTHER, ready_state, user};

    #[test]
    fn people_the_user_blocked_start_muted_in_calls() {
        let mut state = ready_state();
        let stranger = Snowflake::from_raw(9);
        state.apply_event(
            GatewayEvent::RelationshipAdd(Relationship {
                user: user(OTHER),
                kind: RelationshipKind::Blocked,
                since_unix_milliseconds: NOW,
            }),
            NOW,
        );
        assert!(state.is_muted_by_default_in_calls(OTHER));
        assert!(!state.is_muted_by_default_in_calls(stranger));
    }
}
