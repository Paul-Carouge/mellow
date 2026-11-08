//! Forwarding a message and its note (ADR-0039, the web's `forwardMessage` and `sendMessageTo`):
//! both go outside the outbox, since the forward dialog shows each place's failure itself, and a
//! retry with the same nonce gets the message already made instead of a copy.

use gpui::{App, Context};
use mellow_client_core::ClientError;
use mellow_protocol::{ForwardOrigin, SendMessageRequest, Snowflake};

use super::{Session, StateChanged};

/// What a forward or its note came to: the instance's refusal is the caller's to word.
type SendDone = Box<dyn FnOnce(Result<(), ClientError>, &mut App)>;

impl Session {
    /// Forwards `origin` to `channel_id`. `open_channel_id` is the channel on screen: a read
    /// destination elsewhere stays read after your own message lands there. `on_done` is deferred,
    /// so it may read the session.
    pub fn forward_message(
        &mut self,
        channel_id: Snowflake,
        origin: ForwardOrigin,
        nonce: String,
        open_channel_id: Option<Snowflake>,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), ClientError>, &mut App) + 'static,
    ) {
        let request = SendMessageRequest {
            content: String::new(),
            reply_to_id: None,
            nonce: Some(nonce),
            attachment_ids: Vec::new(),
            forwarded_from: Some(origin),
            poll: None,
            checklist: None,
            progress_bar: None,
        };
        self.send_outside_outbox(channel_id, request, open_channel_id, cx, Box::new(on_done));
    }

    /// A text message to any channel, outside the outbox: the note sent after a forward.
    pub fn send_message_to(
        &mut self,
        channel_id: Snowflake,
        content: String,
        nonce: String,
        open_channel_id: Option<Snowflake>,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), ClientError>, &mut App) + 'static,
    ) {
        let request = SendMessageRequest {
            content,
            reply_to_id: None,
            nonce: Some(nonce),
            attachment_ids: Vec::new(),
            forwarded_from: None,
            poll: None,
            checklist: None,
            progress_bar: None,
        };
        self.send_outside_outbox(channel_id, request, open_channel_id, cx, Box::new(on_done));
    }

    fn send_outside_outbox(
        &mut self,
        channel_id: Snowflake,
        request: SendMessageRequest,
        open_channel_id: Option<Snowflake>,
        cx: &mut Context<Self>,
        on_done: SendDone,
    ) {
        let Some(rest) = self.rest_client().cloned() else {
            // Later: the caller may be inside its own update.
            App::defer(cx, move |cx| on_done(Err(ClientError::NotSignedIn), cx));
            return;
        };
        // Before sending: the gateway's echo of this message may arrive before the answer.
        let was_read_before = !self.state.has_unread(channel_id);
        self.run_request(
            cx,
            async move { rest.send_message(channel_id, &request).await },
            move |session, answer, cx| {
                let answer = answer.map(|message| {
                    let change = session.state.take_in_message_sent_elsewhere(
                        message,
                        was_read_before,
                        open_channel_id == Some(channel_id),
                    );
                    session.schedule_read_state_send(cx);
                    cx.emit(StateChanged(vec![change]));
                    cx.notify();
                });
                if let Err(error) = &answer {
                    session.report_request_error(error, "forward message", cx);
                }
                // Deferred: the answer comes while the session is leased.
                cx.defer(move |cx| on_done(answer, cx));
            },
        );
    }
}
