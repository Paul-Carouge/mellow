//! Messages we sent that the server has not confirmed yet (the outbox, ARCHITECTURE.md §6).

use mellow_protocol::{
    Attachment, Message, NewChecklist, NewPoll, NewProgressBar, SendMessageRequest, Snowflake,
};

use super::{ClientState, StateChange};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingMessageStatus {
    Sending,
    /// Kept with its text until the user retries or discards it.
    Failed,
}

/// A poll, checklist or progress bar, sent alone in its message (ADR-0043, ADR-0047).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageCard {
    Poll(NewPoll),
    Checklist(NewChecklist),
    ProgressBar(NewProgressBar),
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
    /// Uploaded already (ADR-0020): sent by id with every attempt, shown while it waits.
    pub attachments: Vec<Attachment>,
    /// Sent instead of text and files.
    pub card: Option<MessageCard>,
    pub status: PendingMessageStatus,
}

impl PendingMessage {
    fn send_request(&self) -> SendMessageRequest {
        SendMessageRequest {
            content: self.content.clone(),
            reply_to_id: self.reply_to_id,
            nonce: Some(self.nonce.clone()),
            attachment_ids: self
                .attachments
                .iter()
                .map(|attachment| attachment.id)
                .collect(),
            forwarded_from: None,
            poll: match &self.card {
                Some(MessageCard::Poll(poll)) => Some(poll.clone()),
                _ => None,
            },
            checklist: match &self.card {
                Some(MessageCard::Checklist(checklist)) => Some(checklist.clone()),
                _ => None,
            },
            progress_bar: match &self.card {
                Some(MessageCard::ProgressBar(bar)) => Some(bar.clone()),
                _ => None,
            },
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
        self.queue_message_with_attachments(channel_id, content, reply_to_id, Vec::new(), nonce)
    }

    /// [`Self::queue_message`] with files uploaded for it (ADR-0020).
    pub fn queue_message_with_attachments(
        &mut self,
        channel_id: Snowflake,
        content: String,
        reply_to_id: Option<Snowflake>,
        attachments: Vec<Attachment>,
        nonce: String,
    ) -> SendMessageRequest {
        self.queue(PendingMessage {
            nonce,
            channel_id,
            content,
            reply_to_id,
            attachments,
            card: None,
            status: PendingMessageStatus::Sending,
        })
    }

    /// Adds a poll, checklist or progress bar to the outbox, as [`Self::queue_message`].
    pub fn queue_card_message(
        &mut self,
        channel_id: Snowflake,
        card: MessageCard,
        nonce: String,
    ) -> SendMessageRequest {
        self.queue(PendingMessage {
            nonce,
            channel_id,
            content: String::new(),
            reply_to_id: None,
            attachments: Vec::new(),
            card: Some(card),
            status: PendingMessageStatus::Sending,
        })
    }

    fn queue(&mut self, pending: PendingMessage) -> SendMessageRequest {
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

    #[test]
    fn files_go_with_every_attempt_of_their_message() {
        let mut state = ready_state();
        let file: Attachment = serde_json::from_value(serde_json::json!({
            "id": "77", "file_name": "a.png", "content_type": "image/png", "size_bytes": 1
        }))
        .expect("a valid attachment");
        let request = state.queue_message_with_attachments(
            GENERAL,
            String::new(),
            None,
            vec![file.clone()],
            "n1".into(),
        );
        assert_eq!(request.attachment_ids, vec![file.id]);
        state.mark_send_failed("n1");
        let (_, retry) = state.retry_pending_message("n1").expect("a failed message");
        assert_eq!(retry.attachment_ids, vec![file.id]);
        let pending = state
            .pending_messages(GENERAL)
            .next()
            .expect("still waiting");
        assert_eq!(pending.attachments, vec![file]);
    }

    #[test]
    fn a_poll_goes_alone_with_every_attempt_of_its_message() {
        let mut state = ready_state();
        let poll = NewPoll {
            question: "Lunch?".into(),
            answers: vec!["Pizza".into(), "Soup".into()],
            allows_multiple_answers: false,
            duration_hours: 24,
        };
        let request =
            state.queue_card_message(GENERAL, MessageCard::Poll(poll.clone()), "n1".into());
        assert_eq!(request.poll, Some(poll.clone()));
        assert_eq!((request.content.as_str(), request.checklist), ("", None));
        state.mark_send_failed("n1");
        let (_, retry) = state.retry_pending_message("n1").expect("a failed message");
        assert_eq!(retry.poll, Some(poll));
    }
}
