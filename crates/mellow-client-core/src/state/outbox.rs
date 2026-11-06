//! Messages we sent that the server has not confirmed yet (the outbox, ARCHITECTURE.md §6).

use mellow_protocol::{Message, SendMessageRequest, Snowflake};

use super::{ClientState, StateChange};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingMessageStatus {
    Sending,
    /// Kept with its text until the user retries or discards it.
    Failed,
}

/// A message we sent that the server has not confirmed yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMessage {
    /// Sent with every attempt, so a retry of a send that did reach the server returns the
    /// stored message instead of a duplicate.
    pub nonce: String,
    pub channel_id: Snowflake,
    pub content: String,
    pub reply_to_id: Option<Snowflake>,
    pub status: PendingMessageStatus,
}

impl PendingMessage {
    fn send_request(&self) -> SendMessageRequest {
        SendMessageRequest {
            content: self.content.clone(),
            reply_to_id: self.reply_to_id,
            nonce: Some(self.nonce.clone()),
            attachment_ids: Vec::new(),
            forwarded_from: None,
            poll: None,
            checklist: None,
            progress_bar: None,
        }
    }
}

impl ClientState {
    /// Our unconfirmed messages in `channel_id`, in send order (shown after the confirmed ones).
    pub fn pending_messages(&self, channel_id: Snowflake) -> impl Iterator<Item = &PendingMessage> {
        self.pending_messages
            .iter()
            .filter(move |pending| pending.channel_id == channel_id)
    }

    /// Adds a message to the outbox and returns the request that sends it. `nonce` must be unique
    /// for this user and channel, forever: the server deduplicates on it (`generate_message_nonce`).
    pub fn queue_message(
        &mut self,
        channel_id: Snowflake,
        content: String,
        reply_to_id: Option<Snowflake>,
        nonce: String,
    ) -> SendMessageRequest {
        let pending = PendingMessage {
            nonce,
            channel_id,
            content,
            reply_to_id,
            status: PendingMessageStatus::Sending,
        };
        let request = pending.send_request();
        self.pending_messages.push(pending);
        request
    }

    /// The server stored the message sent with `nonce`: it replaces its pending copy. `nonce` is
    /// the one we sent, not the server's echo, which an instance could forge.
    /// The message lands in the channel it was sent to, whatever channel the answer names.
    pub fn confirm_sent_message(&mut self, nonce: &str, message: Message) -> StateChange {
        let Some(index) = self
            .pending_messages
            .iter()
            .position(|pending| pending.nonce == nonce)
        else {
            return StateChange::Nothing;
        };
        let channel_id = self.pending_messages.remove(index).channel_id;
        self.insert_messages(channel_id, vec![message])
    }

    pub fn mark_send_failed(&mut self, nonce: &str) -> StateChange {
        match self.pending_message_mut(nonce) {
            Some(pending) => {
                pending.status = PendingMessageStatus::Failed;
                StateChange::Messages {
                    channel_id: pending.channel_id,
                }
            }
            None => StateChange::Nothing,
        }
    }

    /// Only a failed message is sent again, so a double click never sends twice at once. Returns
    /// the channel it was first sent to with the request: a retry never follows the selection.
    pub fn retry_pending_message(
        &mut self,
        nonce: &str,
    ) -> Option<(Snowflake, SendMessageRequest)> {
        let pending = self
            .pending_message_mut(nonce)
            .filter(|pending| pending.status == PendingMessageStatus::Failed)?;
        pending.status = PendingMessageStatus::Sending;
        Some((pending.channel_id, pending.send_request()))
    }

    /// After a reconnect: marks every failed message as sending again and returns its channel,
    /// nonce and request, in send order, for the caller to send one after another.
    pub fn retry_failed_messages(&mut self) -> Vec<(Snowflake, String, SendMessageRequest)> {
        self.pending_messages
            .iter_mut()
            .filter(|pending| pending.status == PendingMessageStatus::Failed)
            .map(|pending| {
                pending.status = PendingMessageStatus::Sending;
                (
                    pending.channel_id,
                    pending.nonce.clone(),
                    pending.send_request(),
                )
            })
            .collect()
    }

    /// The instance refused to store it (`ClientError::refuses_message_for_good`): it leaves the
    /// outbox, and its text is returned so the user can fix it.
    pub fn remove_refused_message(&mut self, nonce: &str) -> Option<PendingMessage> {
        let index = self
            .pending_messages
            .iter()
            .position(|pending| pending.nonce == nonce)?;
        Some(self.pending_messages.remove(index))
    }

    /// Only a failed message is discarded: one still sending may already be stored. A failed one
    /// may be too, when only the answer was lost: discarding then hides it here, not on the server.
    pub fn discard_failed_message(&mut self, nonce: &str) -> StateChange {
        let Some(index) = self.pending_messages.iter().position(|pending| {
            pending.nonce == nonce && pending.status == PendingMessageStatus::Failed
        }) else {
            return StateChange::Nothing;
        };
        let channel_id = self.pending_messages.remove(index).channel_id;
        StateChange::Messages { channel_id }
    }

    fn pending_message_mut(&mut self, nonce: &str) -> Option<&mut PendingMessage> {
        self.pending_messages
            .iter_mut()
            .find(|pending| pending.nonce == nonce)
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::GatewayEvent;

    use super::super::tests::{GENERAL, ME, NOW, message, ready_state};
    use super::*;

    #[test]
    fn the_echo_of_our_own_send_confirms_it_even_before_the_answer() {
        let mut state = ready_state();
        state.queue_message(GENERAL, "hi".into(), None, "n1".into());
        let mut echo = message(11, ME);
        echo.nonce = Some("n1".into());
        state.apply_event(GatewayEvent::MessageCreate(echo.clone()), NOW);
        assert_eq!(state.pending_messages(GENERAL).count(), 0);
        assert_eq!(state.messages(GENERAL).len(), 1);
        assert_eq!(
            state.confirm_sent_message("n1", echo),
            StateChange::Nothing,
            "the late answer changes nothing"
        );
        assert_eq!(state.messages(GENERAL).len(), 1);
    }
}
