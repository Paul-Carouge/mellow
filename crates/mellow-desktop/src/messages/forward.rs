//! A forward in the list (ADR-0039, `MessageItem.svelte`, `docs/design/web-forward-messages.md`
//! "A forward in the list"): the copied words and files held by the quote bar, named "Forwarded"
//! above and by their place and date below. The footer is a button to the original when the
//! reader knows its channel, else the date alone: the ids reveal nothing else.

use gpui::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};

use super::time::{full_timestamp_label, local_date_time, message_timestamp_label};
use crate::components::{
    ActivationHandler, Tooltip, child_id, focus_ring, icon, is_focus_visible,
    remembered_focus_handle, remembered_hover, with_type,
};
use crate::i18n;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_MESSAGE_QUOTE_BAR_WIDTH, COMPONENT_MESSAGE_STATUS_ICON_SIZE, RADIUS_XS,
    SIZE_HIT_DESKTOP_MIN, SPACE_2XS, SPACE_3XS, SPACE_SM, TYPE_MESSAGE_TIMESTAMP,
};
use crate::theme::{color, length, theme};

/// Where a forward's original was written, as its footer names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardOrigin {
    /// "#général", a conversation's name; `None` when the reader does not know that channel.
    pub place: Option<SharedString>,
    /// The original's date, from its id.
    pub sent_at_unix_milliseconds: u64,
}

/// The bar, the mark, the copied `content` and the footer.
#[derive(IntoElement)]
pub struct ForwardFrame {
    id: ElementId,
    origin: ForwardOrigin,
    now_unix_milliseconds: u64,
    content: Vec<AnyElement>,
    on_open_origin: Option<ActivationHandler>,
}

impl ForwardFrame {
    #[must_use]
    pub fn new(
        id: ElementId,
        origin: ForwardOrigin,
        now_unix_milliseconds: u64,
        content: Vec<AnyElement>,
        on_open_origin: Option<ActivationHandler>,
    ) -> Self {
        Self {
            id,
            origin,
            now_unix_milliseconds,
            content,
            on_open_origin,
        }
    }
}

impl std::fmt::Debug for ForwardFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ForwardFrame")
            .field("id", &self.id)
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl RenderOnce for ForwardFrame {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let strings = i18n::strings(cx);
        let muted = color(tokens.color_text_muted);
        let sent = local_date_time(self.origin.sent_at_unix_milliseconds);
        let shown_date =
            message_timestamp_label(sent, local_date_time(self.now_unix_milliseconds), strings);
        let full_date = full_timestamp_label(sent, strings);
        let footer = if let (Some(place), Some(open)) = (self.origin.place, self.on_open_origin) {
            let id = child_id(&self.id, "origin");
            let focus = remembered_focus_handle(&id, window, cx);
            let hover = remembered_hover(&id, window, cx);
            let is_hovered = *hover.read(cx);
            with_type(div(), TYPE_MESSAGE_TIMESTAMP)
                .id(id)
                .role(Role::Button)
                .aria_label(strings.forward().go_to_original(&place, &full_date))
                .track_focus(&focus)
                .relative()
                .flex()
                .items_center()
                .self_start()
                .min_h(length(SIZE_HIT_DESKTOP_MIN))
                .rounded(length(RADIUS_XS))
                .cursor_pointer()
                .text_color(if is_hovered {
                    color(tokens.color_text_secondary)
                } else {
                    muted
                })
                .when(is_hovered, gpui::Styled::underline)
                .on_hover(move |is_hovered, _, cx| {
                    hover.update(cx, |remembered, cx| {
                        *remembered = *is_hovered;
                        cx.notify();
                    });
                })
                .on_click(move |_, window, cx| open(window, cx))
                .child(format!("{place} · {shown_date}"))
                .when(is_focus_visible(&focus, window), |button| {
                    button.child(focus_ring(RADIUS_XS, cx))
                })
                .into_any_element()
        } else {
            let full: SharedString = full_date.clone().into();
            with_type(div(), TYPE_MESSAGE_TIMESTAMP)
                .id(child_id(&self.id, "origin"))
                .aria_label(strings.forward().original_from(&full_date))
                .self_start()
                .text_color(muted)
                .tooltip(move |_, cx| Tooltip::view(full.clone(), cx))
                .tooltip_show_delay(Tooltip::show_delay())
                .child(shown_date)
                .into_any_element()
        };
        div()
            .mt(length(SPACE_3XS))
            .flex()
            .flex_col()
            .gap(length(SPACE_2XS))
            .border_l(length(COMPONENT_MESSAGE_QUOTE_BAR_WIDTH))
            .border_color(color(tokens.component_message_quote_bar_color))
            .pl(length(SPACE_SM))
            .child(
                with_type(div(), TYPE_MESSAGE_TIMESTAMP)
                    .flex()
                    .items_center()
                    .gap(length(SPACE_2XS))
                    .text_color(muted)
                    .child(icon(
                        IconName::ArrowBendUpRight,
                        COMPONENT_MESSAGE_STATUS_ICON_SIZE,
                        muted,
                    ))
                    .child(strings.forward().forwarded()),
            )
            .children(self.content)
            .child(footer)
    }
}
