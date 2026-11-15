//! A call's lifecycle (ADR-0062), as a pure state machine the voice service drives: who rings,
//! who answered, and when the call ends. Who is connected is the voice service's; this only
//! counts them.

use mellow_protocol::Snowflake;

/// What changed a call's ringing, joins or end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallLifecycle {
    caller_id: Snowflake,
    /// Whose devices ring, in the conversation's order.
    ringing_ids: Vec<Snowflake>,
    /// Everyone who joined at least once, in the order they first came.
    joined_ids: Vec<Snowflake>,
}

impl CallLifecycle {
    /// A call `caller_id` just started, ringing `ringing_ids` (decided by
    /// `permissions::rings_in_call`; the caller is never rung).
    pub(crate) fn new(caller_id: Snowflake, ringing_ids: Vec<Snowflake>) -> Self {
        let mut ringing_ids = ringing_ids;
        ringing_ids.retain(|id| *id != caller_id);
        ringing_ids.dedup();
        Self {
            caller_id,
            ringing_ids,
            joined_ids: Vec::new(),
        }
    }

    pub(crate) const fn caller_id(&self) -> Snowflake {
        self.caller_id
    }

    pub(crate) fn ringing_ids(&self) -> &[Snowflake] {
        &self.ringing_ids
    }

    /// Who joined at least once, for the call's message when it ends.
    pub(crate) fn joined_ids(&self) -> &[Snowflake] {
        &self.joined_ids
    }

    /// Someone other than the caller joined at least once.
    pub(crate) fn is_answered(&self) -> bool {
        self.joined_ids.iter().any(|id| *id != self.caller_id)
    }

    /// `user_id` joined (or rejoined): they stop ringing, on all their devices. Whether anything
    /// changed.
    pub(crate) fn join(&mut self, user_id: Snowflake) -> bool {
        let stopped_ringing = self.stop_ringing_for(user_id);
        if self.joined_ids.contains(&user_id) {
            stopped_ringing
        } else {
            self.joined_ids.push(user_id);
            true
        }
    }

    /// `user_id` declined, or lost access: they stop ringing. Whether they were ringing.
    pub(crate) fn stop_ringing_for(&mut self, user_id: Snowflake) -> bool {
        let before = self.ringing_ids.len();
        self.ringing_ids.retain(|id| *id != user_id);
        self.ringing_ids.len() != before
    }

    /// The ring timeout: nobody rings any more. Whether anyone was ringing.
    pub(crate) fn stop_ringing(&mut self) -> bool {
        let was_ringing = !self.ringing_ids.is_empty();
        self.ringing_ids.clear();
        was_ringing
    }

    /// Whether the call is over with `connected` people in it: nobody answered and nobody rings
    /// any more (all declined, timed out, or the caller hung up and ringing ended), or everyone
    /// who joined left. A call no one joined yet waits for its ring timeout.
    pub(crate) fn has_ended(&self, connected: usize) -> bool {
        let unanswered_and_silent = !self.is_answered() && self.ringing_ids.is_empty();
        let everyone_left = connected == 0 && !self.joined_ids.is_empty();
        unanswered_and_silent || everyone_left
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANA: Snowflake = Snowflake::from_raw(1);
    const BOB: Snowflake = Snowflake::from_raw(2);
    const CLEO: Snowflake = Snowflake::from_raw(3);

    fn ringing(ids: &[Snowflake]) -> CallLifecycle {
        CallLifecycle::new(ANA, ids.to_vec())
    }

    #[test]
    fn the_caller_is_never_rung_and_a_new_call_waits_for_its_caller() {
        let call = ringing(&[ANA, BOB]);
        assert_eq!(call.ringing_ids(), [BOB]);
        assert_eq!(call.caller_id(), ANA);
        assert!(!call.has_ended(0), "the caller has not joined yet");
    }

    #[test]
    fn answering_stops_ringing_everywhere_and_keeps_the_call_while_someone_stays() {
        let mut call = ringing(&[BOB, CLEO]);
        assert!(call.join(ANA));
        assert!(!call.is_answered(), "the caller joining is not an answer");
        assert!(call.join(BOB));
        assert_eq!(call.ringing_ids(), [CLEO]);
        assert!(call.is_answered());
        assert!(!call.join(BOB), "a rejoin changes nothing");
        assert!(!call.has_ended(1), "one left: a group call goes on");
        assert!(call.stop_ringing());
        assert!(!call.has_ended(1), "answered calls outlive their ringing");
        assert!(call.has_ended(0), "the last one left");
        assert_eq!(call.joined_ids(), [ANA, BOB]);
    }

    #[test]
    fn an_unanswered_call_ends_when_nobody_rings_any_more() {
        let mut declined = ringing(&[BOB]);
        declined.join(ANA);
        assert!(declined.stop_ringing_for(BOB));
        assert!(
            !declined.stop_ringing_for(BOB),
            "declining twice changes nothing"
        );
        assert!(declined.has_ended(1), "the only callee declined");

        let mut timed_out = ringing(&[BOB, CLEO]);
        timed_out.join(ANA);
        assert!(timed_out.stop_ringing_for(BOB));
        assert!(!timed_out.has_ended(1), "Cleo still rings");
        assert!(timed_out.stop_ringing());
        assert!(timed_out.has_ended(1), "missed by everyone");

        let mut never_joined = ringing(&[BOB]);
        assert!(never_joined.stop_ringing());
        assert!(
            never_joined.has_ended(0),
            "a caller who never joined hung up"
        );
    }

    #[test]
    fn a_caller_hanging_up_before_an_answer_ends_the_call() {
        let mut call = ringing(&[BOB]);
        call.join(ANA);
        assert_eq!(call.ringing_ids(), [BOB]);
        assert!(call.has_ended(0));
    }
}
