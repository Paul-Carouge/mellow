//! The pinned messages of the open channel (`PinnedMessagesPanel.svelte`, DESIGN.md §6.53), from
//! the chat header's pin button: newest pin first, with Jump for the ones loaded in the list and
//! Unpin for those who may; loading, failure with "Try again", and an empty state that says how
//! to pin. It drops under the header's trailing end, as the web's popover does.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, deferred, div,
    prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::state::PinsStatus;

use super::ConversationView;
use crate::components::{
    Avatar, AvatarPerson, Button, ButtonVariant, IconButton, icon, shadow, with_type,
};
use crate::i18n::strings;
use crate::messages::{
    full_timestamp_label, local_date_time, message_summary, message_timestamp_label,
};
use crate::session::now_unix_milliseconds;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CHAT_HEADER_BUTTON_SIZE, COMPONENT_CHAT_HEADER_ICON_SIZE,
    COMPONENT_PINS_BORDER_WIDTH, COMPONENT_PINS_EMPTY_ICON_SIZE, COMPONENT_PINS_HEADER_HEIGHT,
    COMPONENT_PINS_ITEM_AVATAR_SIZE, COMPONENT_PINS_ITEM_GAP, COMPONENT_PINS_ITEM_PADDING,
    COMPONENT_PINS_ITEM_RADIUS, COMPONENT_PINS_MAX_HEIGHT, COMPONENT_PINS_PADDING,
    COMPONENT_PINS_RADIUS, COMPONENT_PINS_WIDTH, SPACE_2XS, SPACE_MD, SPACE_SM, SPACE_XL, SPACE_XS,
    TYPE_BODY, TYPE_HEADING_MD, TYPE_MESSAGE, TYPE_MESSAGE_AUTHOR, TYPE_MESSAGE_TIMESTAMP,
};
use crate::theme::{color, length, theme};

impl ConversationView {
    #[allow(clippy::too_many_lines)] // One panel and its states, as the web's markup.
    pub(super) fn render_pins_panel(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.is_pins_panel_open {
            return None;
        }
        let channel_id = self.open_channel_id()?;
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let pins = catalogue.pins();
        let state = self.session.read(cx).state();
        let rights = self.permissions(channel_id, cx);
        let view = cx.entity().downgrade();
        let now = local_date_time(now_unix_milliseconds());

        let body: AnyElement = match state.pins_status(channel_id) {
            None | Some(PinsStatus::Loading) => with_type(div(), TYPE_BODY)
                .id("pins-loading")
                .role(Role::Status)
                .p(length(SPACE_SM))
                .text_color(color(tokens.component_pins_empty_text))
                .child(pins.loading())
                .into_any_element(),
            Some(PinsStatus::Failed) => {
                let session = self.session.clone();
                div()
                    .id("pins-failed")
                    .role(Role::Alert)
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(length(SPACE_SM))
                    .p(length(SPACE_SM))
                    .child(
                        with_type(div(), TYPE_BODY)
                            .text_color(color(tokens.component_pins_empty_text))
                            .child(pins.load_failed()),
                    )
                    .child(
                        Button::new("pins-try-again", catalogue.common().try_again()).on_click(
                            move |_, cx| {
                                session.update(cx, |session, cx| {
                                    session.load_pins(channel_id, true, cx);
                                });
                            },
                        ),
                    )
                    .into_any_element()
            }
            Some(PinsStatus::Loaded) if state.pinned_messages(channel_id).is_empty() => div()
                .flex()
                .flex_col()
                .items_center()
                .gap(length(SPACE_XS))
                .px(length(SPACE_MD))
                .py(length(SPACE_XL))
                .child(icon(
                    IconName::PushPin,
                    COMPONENT_PINS_EMPTY_ICON_SIZE,
                    color(tokens.component_pins_empty_icon),
                ))
                .child(
                    with_type(div(), TYPE_HEADING_MD)
                        .text_color(color(tokens.color_text_heading))
                        .child(pins.empty_title()),
                )
                .child(
                    with_type(div(), TYPE_BODY)
                        .text_center()
                        .text_color(color(tokens.component_pins_empty_text))
                        .child(pins.empty_body()),
                )
                .into_any_element(),
            Some(PinsStatus::Loaded) => {
                let items = state.pinned_messages(channel_id).iter().map(|message| {
                    let message_id = message.id;
                    let author_name = display_name(&message.author).to_owned();
                    let sent = local_date_time(message.id.unix_milliseconds());
                    let summary = if state.is_blocked(message.author.id) {
                        catalogue.messages().blocked_reply().to_owned()
                    } else {
                        message_summary(message, &self.sources.roles, catalogue)
                    };
                    let picture = message.author.avatar.as_ref().map(|avatar| {
                        crate::messages::picture_for_box(
                            mellow_client_core::DownloadableFile::ProfileImage(avatar.id),
                            COMPONENT_PINS_ITEM_AVATAR_SIZE,
                            COMPONENT_PINS_ITEM_AVATAR_SIZE,
                            crate::messages::picture_animation(cx),
                            window,
                        )
                    });
                    let jump_view = view.clone();
                    let unpin_view = view.clone();
                    let unpinned = message.clone();
                    let full_time: SharedString = full_timestamp_label(sent, catalogue).into();
                    div()
                        .flex()
                        .gap(length(SPACE_SM))
                        .p(length(COMPONENT_PINS_ITEM_PADDING))
                        .rounded(length(COMPONENT_PINS_ITEM_RADIUS))
                        .bg(color(tokens.component_pins_item_background))
                        .child(Avatar::new(
                            AvatarPerson::from_user(&message.author, picture),
                            COMPONENT_PINS_ITEM_AVATAR_SIZE,
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .gap(length(SPACE_2XS))
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .items_baseline()
                                        .gap_x(length(SPACE_XS))
                                        .child(
                                            with_type(div(), TYPE_MESSAGE_AUTHOR)
                                                .text_color(color(
                                                    tokens.component_message_author_text,
                                                ))
                                                .child(author_name.clone()),
                                        )
                                        .child(
                                            with_type(div(), TYPE_MESSAGE_TIMESTAMP)
                                                .id(SharedString::from(format!(
                                                    "pin-time-{}",
                                                    message_id.raw()
                                                )))
                                                .aria_label(full_time)
                                                .text_color(color(
                                                    tokens.component_message_timestamp_text,
                                                ))
                                                .child(message_timestamp_label(
                                                    sent, now, catalogue,
                                                )),
                                        ),
                                )
                                .child(
                                    with_type(div(), TYPE_MESSAGE)
                                        .line_clamp(4)
                                        .text_color(color(tokens.component_message_body_text))
                                        .child(summary),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap(length(SPACE_XS))
                                        .child(
                                            Button::new(
                                                SharedString::from(format!(
                                                    "pin-jump-{}",
                                                    message_id.raw()
                                                )),
                                                pins.jump(),
                                            )
                                            .variant(ButtonVariant::Secondary)
                                            .on_click(
                                                move |window, cx| {
                                                    let _ = jump_view.update(cx, |view, cx| {
                                                        view.jump_to_message(
                                                            message_id, window, cx,
                                                        );
                                                    });
                                                },
                                            ),
                                        )
                                        .when(rights.can_manage_pins, |actions| {
                                            actions.child(
                                                Button::new(
                                                    SharedString::from(format!(
                                                        "pin-unpin-{}",
                                                        message_id.raw()
                                                    )),
                                                    pins.unpin_submit(),
                                                )
                                                .variant(ButtonVariant::Ghost)
                                                .on_click(move |window, cx| {
                                                    let _ = unpin_view.update(cx, |view, cx| {
                                                        view.ask_to_confirm(
                                                            super::Confirmation::Pin(
                                                                unpinned.clone(),
                                                                false,
                                                            ),
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                }),
                                            )
                                        }),
                                ),
                        )
                });
                div()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_PINS_ITEM_GAP))
                    .children(items)
                    .into_any_element()
            }
        };

        let close_view = view.clone();
        Some(
            deferred(
                div()
                    .id("pins-panel")
                    .role(Role::Dialog)
                    .aria_label(pins.pinned_messages())
                    .track_focus(&self.pins_focus)
                    .key_context("PinsPanel")
                    .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified()
                        {
                            cx.stop_propagation();
                            view.close_pins_panel(window, cx);
                        }
                    }))
                    .on_mouse_down_out(cx.listener(
                        |view, event: &gpui::MouseDownEvent, window, cx| {
                            // The header's pin button would open it again on this same click.
                            view.pins_dismissal.remember(event.position, ());
                            view.close_pins_panel(window, cx);
                        },
                    ))
                    .absolute()
                    .top_0()
                    .right(length(SPACE_XS))
                    .w(length(COMPONENT_PINS_WIDTH))
                    .max_h(length(COMPONENT_PINS_MAX_HEIGHT))
                    .flex()
                    .flex_col()
                    .rounded(length(COMPONENT_PINS_RADIUS))
                    .border(length(COMPONENT_PINS_BORDER_WIDTH))
                    .border_color(color(tokens.component_pins_border))
                    .bg(color(tokens.component_pins_background))
                    .shadow(shadow(tokens.component_pins_shadow))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(length(SPACE_XS))
                            .h(length(COMPONENT_PINS_HEADER_HEIGHT))
                            .pl(length(SPACE_SM))
                            .pr(length(SPACE_2XS))
                            .child(icon(
                                IconName::PushPin,
                                COMPONENT_CHAT_HEADER_ICON_SIZE,
                                color(tokens.color_text_muted),
                            ))
                            .child(
                                with_type(div(), TYPE_HEADING_MD)
                                    .id("pins-title")
                                    .role(Role::Heading)
                                    .aria_level(2)
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(color(tokens.component_pins_title_text))
                                    .child(pins.pinned_messages()),
                            )
                            .child(
                                IconButton::new(
                                    "pins-close",
                                    IconName::X,
                                    catalogue.common().close(),
                                )
                                .sizes(
                                    COMPONENT_CHAT_HEADER_BUTTON_SIZE,
                                    COMPONENT_CHAT_HEADER_ICON_SIZE,
                                )
                                .on_click(move |window, cx| {
                                    let _ = close_view.update(cx, |view, cx| {
                                        view.close_pins_panel(window, cx);
                                    });
                                }),
                            ),
                    )
                    .child(
                        div()
                            .id("pins-body")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px(length(COMPONENT_PINS_PADDING))
                            .pb(length(COMPONENT_PINS_PADDING))
                            .child(body),
                    ),
            )
            .into_any_element(),
        )
    }
}
