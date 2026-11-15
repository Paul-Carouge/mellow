//! A call's message (`CallMessage.svelte`, DESIGN.md §5.6.3, ADR-0062): its caller and time as any
//! message, then a disc with the call's sign and one line: "Appel en cours" (apricot, it is
//! happening), "Appel · 12 min" (quiet, it is history), "Appel sans réponse" (your own call nobody
//! took, quiet), "Appel manqué" (red, as phones say it, never by colour alone: its own sign and
//! words). The banner over the conversation is where one joins.

use gpui::{
    App, FontWeight, IntoElement, ParentElement as _, RenderOnce, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use mellow_client_core::state::{CallMessageSummary, call_message_summary};
use mellow_protocol::{MessageCall, Snowflake};

use crate::calls::words::call_message_line;
use crate::components::{icon, with_type};
use crate::i18n::strings;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CALL_MESSAGE_DISC_SIZE, COMPONENT_CALL_MESSAGE_GAP, COMPONENT_CALL_MESSAGE_ICON_SIZE,
    TYPE_MESSAGE,
};
use crate::theme::{color, length, theme};

#[derive(Debug, IntoElement)]
pub struct CallMessage {
    call: MessageCall,
    message_id: Snowflake,
    reader_id: Snowflake,
}

impl CallMessage {
    #[must_use]
    pub fn new(call: MessageCall, message_id: Snowflake, reader_id: Snowflake) -> Self {
        Self {
            call,
            message_id,
            reader_id,
        }
    }
}

impl RenderOnce for CallMessage {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let summary = call_message_summary(&self.call, self.message_id, self.reader_id);
        let (glyph, disc, ink) = match summary {
            CallMessageSummary::Ongoing => (
                IconName::PhoneCall,
                tokens.component_call_message_ongoing_disc,
                tokens.component_call_message_ongoing_icon,
            ),
            CallMessageSummary::Ended { .. } => (
                IconName::Phone,
                tokens.component_call_message_ended_disc,
                tokens.component_call_message_ended_icon,
            ),
            CallMessageSummary::Unanswered => (
                IconName::PhoneX,
                tokens.component_call_message_ended_disc,
                tokens.component_call_message_ended_icon,
            ),
            CallMessageSummary::Missed => (
                IconName::PhoneX,
                tokens.component_call_message_missed_disc,
                tokens.component_call_message_missed_icon,
            ),
        };
        with_type(div(), TYPE_MESSAGE)
            .flex()
            .items_center()
            .min_h(length(COMPONENT_CALL_MESSAGE_DISC_SIZE))
            .gap(length(COMPONENT_CALL_MESSAGE_GAP))
            .text_color(color(tokens.component_call_message_text))
            .child(
                div()
                    .flex_none()
                    .size(length(COMPONENT_CALL_MESSAGE_DISC_SIZE))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(color(disc))
                    .child(icon(glyph, COMPONENT_CALL_MESSAGE_ICON_SIZE, color(ink))),
            )
            .child(
                div()
                    .min_w_0()
                    // "Appel en cours" reads a little heavier: it is happening now.
                    .when(summary == CallMessageSummary::Ongoing, |line| {
                        line.font_weight(FontWeight::MEDIUM)
                    })
                    .child(call_message_line(summary, strings(cx))),
            )
    }
}
