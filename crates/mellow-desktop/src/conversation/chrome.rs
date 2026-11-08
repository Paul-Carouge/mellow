//! What surrounds the rows (`ChatPane.svelte`, `MessageList.svelte`): the list itself with its
//! keys, the unread bar over its top, the jump bar resting on the composer while older messages
//! show, the typing line, the failure of the last action, the composer, and the layers (pins
//! panel, message menu, confirmation).

use gpui::{
    AnyElement, Context, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement,
    ModifiersChangedEvent, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledText, Window, div, list,
    prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::message_rows::{
    UnreadMarker, arrivals_since_leaving_present, unread_summary,
};

use super::ConversationView;
use super::rules::typing_segments;
use crate::components::{
    child_id, colour_transition, focus_ring, icon, is_focus_visible, remembered_focus_handle,
    remembered_hover, with_type,
};
use crate::i18n::strings;
use crate::local_widgets::error_line;
use crate::messages::{local_date_time, message_timestamp_label};
use crate::session::now_unix_milliseconds;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_COMPOSER_ERROR_PADDING_BLOCK, COMPONENT_COMPOSER_MARGIN_INLINE,
    COMPONENT_COMPOSER_TYPING_HEIGHT, COMPONENT_JUMP_BAR_HEIGHT, COMPONENT_JUMP_BAR_ICON_SIZE,
    COMPONENT_JUMP_BAR_RADIUS, COMPONENT_NOTICE_BAR_ICON_SIZE, RADIUS_SM, SPACE_2XS, SPACE_3XS,
    SPACE_MD, SPACE_SM, SPACE_XS, TYPE_BODY, TYPE_BUTTON, TYPE_TYPING,
};
use crate::theme::{color, length, theme};

const LIST_KEY_CONTEXT: &str = "MessageList";

/// Read out politely by screen readers each time `text` changes, as the web's `aria-live="polite"`.
/// GPUI has no live-region setter, so the node is marked live through its AccessKit node; AccessKit
/// announces a live node whose value changes (`NSAccessibilityAnnouncementRequestedNotification`
/// on macOS, a text change event on AT-SPI). The same text twice in a row is read once.
fn polite_live_region(id: &'static str, text: SharedString) -> impl IntoElement {
    div()
        .id(id)
        .role(Role::Status)
        .when(!text.is_empty(), |region| region.aria_value(text))
        .a11y_synthetic_children(|subtree| {
            subtree
                .parent_node()
                .set_live(gpui::accesskit::Live::Polite);
        })
        .size_0()
        .overflow_hidden()
}

impl ConversationView {
    /// "3 new messages since today at 14:32", while the visit's unread messages are not seen.
    pub(super) fn unread_bar_sentence(&self, cx: &gpui::App) -> Option<String> {
        let channel_id = self.open_channel_id()?;
        let marker = self.visit.unread_marker?;
        let state = self.session.read(cx).state();
        let summary = unread_summary(
            state.messages(channel_id),
            UnreadMarker {
                last_read_message_id: marker.last_read_message_id,
                newest_at_open_message_id: marker.newest_at_open_message_id,
            },
            state.current_user().map(|user| user.id),
            !state.has_older_history(channel_id),
        );
        if summary.count == 0 {
            return None;
        }
        let catalogue = strings(cx);
        let since = message_timestamp_label(
            local_date_time(marker.last_read_message_id.unix_milliseconds()),
            local_date_time(now_unix_milliseconds()),
            catalogue,
        );
        // "Today at" becomes "today at" mid-sentence; dates start with a digit and stay.
        let mut letters = since.chars();
        let since_in_sentence = letters
            .next()
            .map(|first| first.to_lowercase().chain(letters).collect::<String>())
            .unwrap_or_default();
        Some(catalogue.chat().new_messages_since(
            i64::try_from(summary.count).unwrap_or(i64::MAX),
            summary.is_partial,
            &since_in_sentence,
        ))
    }

    /// A word that acts inside a bar ("Mark as read", the jump bar's action).
    fn bar_button(
        id: &'static str,
        label: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let tokens = theme(cx);
        let id = gpui::ElementId::from(id);
        let focus_handle = remembered_focus_handle(&id, window, cx);
        let hover = remembered_hover(&id, window, cx);
        let is_hovered = *hover.read(cx);
        let wash = color(tokens.component_jump_bar_background_hover);
        let fill = colour_transition(
            child_id(&id, "background"),
            if is_hovered {
                wash
            } else {
                gpui::Hsla { a: 0.0, ..wash }
            },
            window,
            cx,
        );
        let is_ring_shown = is_focus_visible(&focus_handle, window);
        with_type(div(), TYPE_BUTTON)
            .id(id)
            .role(Role::Button)
            .track_focus(&focus_handle)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .h(length(COMPONENT_JUMP_BAR_HEIGHT - 2.0 * SPACE_3XS))
            .px(length(SPACE_XS))
            .rounded(length(RADIUS_SM))
            .bg(fill)
            .cursor_pointer()
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .on_click(cx.listener(move |view, _, window, cx| on_click(view, window, cx)))
            .child(label.into())
            .when(is_ring_shown, |button| {
                button.child(focus_ring(RADIUS_SM, cx))
            })
    }

    fn render_unread_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let sentence = self.unread_bar_sentence(cx)?;
        let tokens = theme(cx);
        let mark_as_read = strings(cx).chat().mark_as_read();
        Some(
            with_type(div(), TYPE_BODY)
                .id("unread-bar")
                .role(Role::Status)
                .absolute()
                .top(length(SPACE_2XS))
                .left(length(SPACE_MD))
                .right(length(SPACE_MD))
                .min_h(length(COMPONENT_JUMP_BAR_HEIGHT))
                .flex()
                .items_center()
                .justify_between()
                .gap(length(SPACE_XS))
                .pl(length(SPACE_SM))
                .pr(length(SPACE_2XS))
                .rounded(length(COMPONENT_JUMP_BAR_RADIUS))
                .bg(color(tokens.component_jump_bar_background))
                .text_color(color(tokens.component_jump_bar_text))
                .font_weight(FontWeight::SEMIBOLD)
                .child(div().min_w_0().child(sentence))
                .child(Self::bar_button(
                    "mark-as-read",
                    mark_as_read,
                    window,
                    cx,
                    |view, window, cx| {
                        view.mark_as_read(cx);
                        view.focus_composer(window, cx);
                    },
                ))
                .into_any_element(),
        )
    }

    /// Over the list rather than above it: taking height would move the rows being read.
    fn render_jump_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.is_at_latest() || self.entries.len() <= 1 {
            return None;
        }
        let channel_id = self.open_channel_id()?;
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let state = self.session.read(cx).state();
        let arrivals = self
            .visit
            .latest_id_when_leaving_present
            .map_or(0, |latest| {
                arrivals_since_leaving_present(
                    state.messages(channel_id),
                    latest,
                    state.current_user().map(|user| user.id),
                )
            });
        let action = if arrivals > 0 {
            catalogue
                .messages()
                .jump_to_present_with_arrivals(i64::try_from(arrivals).unwrap_or(i64::MAX))
        } else {
            catalogue.messages().jump_to_present().to_owned()
        };
        Some(
            with_type(div(), TYPE_BODY)
                .id("jump-bar")
                .absolute()
                .bottom_0()
                .left(length(COMPONENT_COMPOSER_MARGIN_INLINE))
                .right(length(COMPONENT_COMPOSER_MARGIN_INLINE))
                .min_h(length(COMPONENT_JUMP_BAR_HEIGHT))
                .flex()
                .items_center()
                .gap(length(SPACE_XS))
                .pl(length(SPACE_MD))
                .pr(length(SPACE_2XS))
                .rounded_t(length(COMPONENT_JUMP_BAR_RADIUS))
                .bg(color(tokens.component_jump_bar_background))
                .text_color(color(tokens.component_jump_bar_text))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(catalogue.messages().viewing_older()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(length(SPACE_2XS))
                        .child(icon(
                            IconName::CaretDown,
                            COMPONENT_JUMP_BAR_ICON_SIZE,
                            color(tokens.component_jump_bar_text),
                        ))
                        .child(Self::bar_button(
                            "jump-to-present",
                            action,
                            window,
                            cx,
                            |view, window, cx| {
                                view.visit.has_reader_scrolled = true;
                                view.jump_to_present(cx);
                                view.focus_composer(window, cx);
                            },
                        )),
                )
                .into_any_element(),
        )
    }

    /// "Ana is typing…" above the composer; its row keeps its height, so nothing moves.
    fn render_typing_line(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let state = self.session.read(cx).state();
        let guild_id = self
            .open_conversation
            .as_ref()
            .and_then(|open| open.guild_id);
        let channel_id = self.open_channel_id();
        let names: Vec<String> = self
            .typing_user_ids(cx)
            .into_iter()
            .map(|user_id| {
                let member = guild_id.and_then(|guild_id| {
                    state
                        .members(guild_id)
                        .iter()
                        .find(|member| member.user.id == user_id)
                        .map(|member| display_name(&member.user).to_owned())
                });
                let recipient = || {
                    channel_id
                        .and_then(|channel_id| state.direct_message_channel(channel_id))
                        .and_then(|conversation| {
                            conversation
                                .recipients
                                .iter()
                                .find(|recipient| recipient.id == user_id)
                                .map(|recipient| display_name(recipient).to_owned())
                        })
                };
                member
                    .or_else(recipient)
                    .unwrap_or_else(|| catalogue.common().someone().to_owned())
            })
            .collect();
        let mut sentence = String::new();
        let mut name_ranges = Vec::new();
        for segment in typing_segments(&names, catalogue) {
            let start = sentence.len();
            sentence.push_str(&segment.text);
            if segment.is_name {
                name_ranges.push((
                    start..sentence.len(),
                    HighlightStyle {
                        font_weight: Some(FontWeight::BOLD),
                        ..HighlightStyle::default()
                    },
                ));
            }
        }
        let sentence = SharedString::from(sentence);
        with_type(div(), TYPE_TYPING)
            .id("typing-line")
            .role(Role::Status)
            .aria_label(sentence.clone())
            .flex_none()
            .h(length(COMPONENT_COMPOSER_TYPING_HEIGHT))
            .px(length(SPACE_MD))
            .flex()
            .items_center()
            .overflow_hidden()
            .text_color(color(tokens.component_composer_typing_text))
            .child(StyledText::new(sentence).with_highlights(name_ranges))
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self
            .open_conversation
            .as_ref()
            .map(|open| strings(cx).messages().messages_in(&open.label))
            .unwrap_or_default();
        let view = cx.entity();
        div()
            .id("message-list")
            .role(Role::List)
            .aria_label(SharedString::from(label))
            .key_context(LIST_KEY_CONTEXT)
            .track_focus(&self.list_focus)
            .on_key_down(cx.listener(Self::on_list_key_down))
            .size_full()
            .child(
                list(self.list.clone(), move |index, window, cx| {
                    view.update(cx, |view, cx| view.render_entry(index, window, cx))
                })
                .size_full(),
            )
    }
}

impl Render for ConversationView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Read here: modifier events only reach the focus path, so a Shift pressed or released
        // with focus outside the view would otherwise leave a stale value.
        self.is_shift_held = window.modifiers().shift;
        self.apply_requested_place(window, cx);
        if let Some((message_id, text, can_reopen)) = self.refused_edit.take() {
            self.composer.update(cx, |composer, cx| {
                composer.give_back_refused_edit(message_id, text, can_reopen, window, cx);
            });
        }
        if self.are_rows_stale {
            self.rebuild_rows(cx);
            // New rows move the list in its layout (opening, the divider, a page): what follows
            // from where it lands is read once that layout is done.
            cx.defer_in(window, |view, window, cx| view.follow_reading(window, cx));
        }
        self.follow_reading(window, cx);
        let tokens = theme(cx);
        let unread_bar = self.render_unread_bar(window, cx);
        let jump_bar = self.render_jump_bar(window, cx);
        let pins_panel = self.render_pins_panel(window, cx);
        let action_error = self.action_error.clone().map(|error| {
            div()
                .id("chat-action-error")
                .role(Role::Alert)
                .flex_none()
                .px(length(COMPONENT_COMPOSER_MARGIN_INLINE))
                .py(length(COMPONENT_COMPOSER_ERROR_PADDING_BLOCK))
                .child(error_line(
                    error,
                    tokens.component_composer_error_text,
                    COMPONENT_NOTICE_BAR_ICON_SIZE,
                ))
        });
        div()
            .debug_selector(|| "conversation-slot".to_owned())
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .on_modifiers_changed(cx.listener(|view, event: &ModifiersChangedEvent, _, cx| {
                if view.is_shift_held != event.modifiers.shift {
                    view.is_shift_held = event.modifiers.shift;
                    cx.notify();
                }
            }))
            .child(
                div()
                    .debug_selector(|| "message-list".to_owned())
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_list(cx))
                    .children(unread_bar)
                    .children(jump_bar),
            )
            .child(self.render_typing_line(cx))
            .children(action_error)
            .child(div().flex_none().child(self.composer.clone()))
            // Copies, new messages and other quiet results, for screen readers only.
            .child(polite_live_region(
                "conversation-announcement",
                self.announcement.clone(),
            ))
            .children(pins_panel)
            .when_some(self.menu.as_ref(), |slot, (menu, _, _)| {
                slot.child(menu.clone())
            })
            .when_some(self.confirmation.as_ref(), |slot, (dialog, _)| {
                slot.child(dialog.clone())
            })
    }
}
