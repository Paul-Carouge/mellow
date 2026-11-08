//! Forwards in the conversation (ADR-0039): the dialog opened from Forward, the status line once
//! every place got the message, a forward's frame and its jump to the original, and the link
//! previews under a message.

use gpui::{App, Context, ElementId, WeakEntity, Window};
use mellow_client_core::shared_logic::forward_messages::forward_origin_place;
use mellow_protocol::{Message, Snowflake};

use super::ConversationView;
use super::forward_dialog::{ForwardDialog, ForwardDialogEvent};
use crate::home::conversation_display_name;
use crate::i18n::strings;
use crate::messages::{ForwardOrigin, MessageItem, MessageLinkPreviews};

/// A forward's footer: open the original's channel at it. The shell opens the channel, then
/// tells the conversation to reach the message ([`ConversationView::reach_after_opening`]).
#[derive(Debug, Clone, PartialEq, gpui::Action)]
#[action(namespace = mellow_conversation, no_json)]
pub struct OpenForwardOrigin {
    pub channel_id: Snowflake,
    pub message_id: Snowflake,
}

impl ConversationView {
    /// A forward's frame and footer, or the message's link previews.
    pub(super) fn with_forward_and_previews(
        &self,
        item: MessageItem,
        message: &Message,
        view: &WeakEntity<Self>,
        cx: &App,
    ) -> MessageItem {
        let Some(origin) = message.forwarded_from else {
            if message.link_previews.is_empty() {
                return item;
            }
            return item.link_previews(MessageLinkPreviews::new(
                ElementId::from(("link-previews", message.id.raw())),
                message.channel_id,
                message.id,
                message.link_previews.clone(),
            ));
        };
        let state = self.session.read(cx).state();
        let reader_id = state.current_user().map(|user| user.id);
        let catalogue = strings(cx);
        // Only a channel the reader knows is named; the ids reveal nothing else (ADR-0039).
        let place = forward_origin_place(
            origin.channel_id,
            state.guilds(),
            state.direct_message_channels(),
            |conversation| conversation_display_name(conversation, reader_id, catalogue),
        );
        let view = view.clone();
        let on_open_origin = place
            .is_some()
            .then_some(move |window: &mut Window, cx: &mut App| {
                let _ = view.update(cx, |view, cx| {
                    view.open_forward_origin(origin.channel_id, origin.message_id, window, cx);
                });
            });
        item.forward(
            ForwardOrigin {
                place: place.map(|place| place.label.into()),
                sent_at_unix_milliseconds: origin.message_id.unix_milliseconds(),
            },
            on_open_origin,
        )
    }

    /// Forward, from the hover bar or the menu.
    pub(super) fn open_forward_dialog(
        &mut self,
        message_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(message) = self.cached_message(message_id, cx) else {
            return;
        };
        let (dialog, body) = ForwardDialog::open(
            self.session.clone(),
            message,
            self.open_channel_id(),
            self.sources.roles.clone(),
            window,
            cx,
        );
        let subscription = cx.subscribe_in(
            &body,
            window,
            |view, _, event: &ForwardDialogEvent, window, cx| {
                if let ForwardDialogEvent::Forwarded(sentence) = event {
                    view.notice = Some(sentence.clone().into());
                    view.announcement = sentence.clone().into();
                    // Focus goes to the composer, or the message list where you cannot write.
                    let can_write = view
                        .open_channel_id()
                        .is_some_and(|channel_id| view.permissions(channel_id, cx).can_send);
                    if can_write {
                        view.focus_composer(window, cx);
                    } else {
                        view.focus_message_list(window, cx);
                    }
                }
                view.confirmation = None;
                cx.notify();
            },
        );
        self.confirmation = Some((dialog, subscription));
        cx.notify();
    }

    /// The footer of a forward: the original in its own channel, reached as a pin is.
    fn open_forward_origin(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_channel_id() == Some(channel_id) {
            self.jump_to_message(message_id, window, cx);
            return;
        }
        let action = OpenForwardOrigin {
            channel_id,
            message_id,
        };
        self.list_focus.dispatch_action(&action, window, cx);
    }

    /// The shell opened `channel_id` for a forward's original: once it is the open channel, the
    /// next render reaches `message_id`.
    pub fn reach_after_opening(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        cx: &mut Context<Self>,
    ) {
        self.pending_origin_jump = Some((channel_id, message_id));
        cx.notify();
    }

    /// Called by the render once the requested place is applied.
    pub(super) fn reach_pending_origin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((channel_id, message_id)) = self.pending_origin_jump else {
            return;
        };
        if self.open_channel_id() == Some(channel_id) {
            self.pending_origin_jump = None;
            self.jump_to_message(message_id, window, cx);
        } else if self.requested_place.as_ref().map(|place| place.channel_id) != Some(channel_id) {
            // The shell could not open it (gone meanwhile): nothing to reach.
            self.pending_origin_jump = None;
        }
    }
}
