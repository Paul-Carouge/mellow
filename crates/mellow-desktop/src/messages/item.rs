//! `MessageItem.svelte` (DESIGN.md §5.4): one message row. A group's first message shows the
//! avatar, name and time; the next ones from the same author align on the gutter and show their
//! time there on hover. A message naming the reader is tinted with a leading bar; one still in the
//! outbox is dimmed, then says it was not sent with Retry and Delete.
//!
//! A forward holds its copied words and files in the quote bar (`ForwardFrame`); link previews
//! follow the files (`MessageLinkPreviews`).
//!
//! shortcut: no polls, checklists or GIFs yet; they come with their own tracks (M3 step 5).

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, Hsla, ImageSource, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Pixels, Point, RenderOnce, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::state::PendingMessageStatus;
use mellow_protocol::{Attachment, PublicUser};

use super::attachments::MessageAttachments;
use super::body::MessageBody;
use super::forward::{ForwardFrame, ForwardOrigin};
use super::link_previews::MessageLinkPreviews;
use super::markdown_view::{EditedMarker, MessageMarkdown};
use super::reactions::{
    MessageReactions, ReactionPill, ReactionToggleHandler, ReactionTooltipBuilder,
};
use super::reply::{ReplyLine, ReplyReference};
use super::time::{
    full_timestamp_label, local_date_time, message_timestamp_label, time_of_day_label,
};
use crate::components::{
    ActivationHandler, Avatar, AvatarPerson, Tooltip, child_id, colour_transition, focus_ring,
    icon, is_focus_visible, remembered_focus_handle, remembered_hover, with_type,
};
use crate::i18n::{self, Strings};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_MENTION_BAR_WIDTH, COMPONENT_MESSAGE_AVATAR_INSET, COMPONENT_MESSAGE_AVATAR_SIZE,
    COMPONENT_MESSAGE_GROUP_GAP, COMPONENT_MESSAGE_GUTTER, COMPONENT_MESSAGE_PADDING_BLOCK,
    COMPONENT_MESSAGE_PADDING_TRAILING, COMPONENT_MESSAGE_REPLY_HEIGHT,
    COMPONENT_MESSAGE_STATUS_ICON_SIZE, COMPONENT_REACTION_PINNED_ICON_SIZE,
    MOTION_DELAY_PENDING_LABEL, RADIUS_XS, SIZE_HIT_DESKTOP_MIN, SPACE_2XS, SPACE_3XS, SPACE_XS,
    TYPE_MESSAGE, TYPE_MESSAGE_AUTHOR, TYPE_MESSAGE_SUBTEXT, TYPE_MESSAGE_TIMESTAMP,
};
use crate::theme::{color, delay, length, theme};

/// The author header is an `<h3>` on the web; Markdown headings sit below it, at 4 to 6.
pub(crate) const MESSAGE_HEADER_HEADING_LEVEL: usize = 3;

/// Who wrote the message, as its header shows them.
#[derive(Clone)]
pub struct MessageAuthor {
    pub user: PublicUser,
    /// Their avatar, when they have one.
    pub picture: Option<ImageSource>,
    /// Their role colour, already readable on the sheet (`readable_role_colour`); `None` keeps
    /// the heading colour.
    pub colour: Option<Hsla>,
}

/// Everything one row shows; the list builds it from client-core's rows and its body cache.
#[derive(Clone)]
pub struct MessageItemContent {
    pub author: MessageAuthor,
    /// When it was sent; `None` while it is in the outbox.
    pub sent_at_unix_milliseconds: Option<u64>,
    /// The list's clock, for "Today at" and "Yesterday at".
    pub now_unix_milliseconds: u64,
    pub body: Rc<MessageBody>,
    pub attachments: Vec<Attachment>,
    pub reactions: Vec<ReactionPill>,
    /// The quoted original and its author's avatar.
    pub reply: Option<(ReplyReference, Option<ImageSource>)>,
    pub edited_at_unix_milliseconds: Option<u64>,
    pub is_pinned: bool,
    pub starts_author_group: bool,
    /// By name, role or `@everyone` (the server's ids and the body's own reading).
    pub is_mentioning_reader: bool,
    /// `Some` while in the outbox.
    pub pending: Option<PendingMessageStatus>,
}

/// One row of the message list.
#[derive(IntoElement)]
pub struct MessageItem {
    id: ElementId,
    content: MessageItemContent,
    can_join_reactions: bool,
    on_toggle_reaction: Option<ReactionToggleHandler>,
    reaction_tooltip: Option<ReactionTooltipBuilder>,
    selection_order: u64,
    on_retry: Option<ActivationHandler>,
    on_discard: Option<ActivationHandler>,
    on_open_author_menu: Option<AuthorMenuHandler>,
    link_previews: Option<MessageLinkPreviews>,
    forward: Option<(ForwardOrigin, Option<ActivationHandler>)>,
}

/// Opens the author's menu at a window point.
type AuthorMenuHandler = Arc<dyn Fn(Point<Pixels>, &mut Window, &mut App) + 'static>;

impl MessageItem {
    /// `id` is the row's key: the message's id, or the pending message's nonce.
    #[must_use]
    pub fn new(id: impl Into<ElementId>, content: MessageItemContent) -> Self {
        Self {
            id: id.into(),
            content,
            can_join_reactions: true,
            on_toggle_reaction: None,
            reaction_tooltip: None,
            selection_order: 0,
            on_retry: None,
            on_discard: None,
            on_open_author_menu: None,
            link_previews: None,
            forward: None,
        }
    }

    /// The previews of the message's links, under its files.
    #[must_use]
    pub fn link_previews(mut self, previews: MessageLinkPreviews) -> Self {
        self.link_previews = Some(previews);
        self
    }

    /// A forward (ADR-0039): its words and files go in the quote bar; `on_open_origin` jumps to
    /// the original when the reader knows its channel.
    #[must_use]
    pub fn forward(
        mut self,
        origin: ForwardOrigin,
        on_open_origin: Option<impl Fn(&mut Window, &mut App) + 'static>,
    ) -> Self {
        let on_open_origin =
            on_open_origin.map(|handler| -> ActivationHandler { Arc::new(handler) });
        self.forward = Some((origin, on_open_origin));
        self
    }

    #[must_use]
    pub fn can_join_reactions(mut self, can_join: bool) -> Self {
        self.can_join_reactions = can_join;
        self
    }

    /// A reaction pill was activated: add or take back the reader's reaction with that key.
    #[must_use]
    pub fn on_toggle_reaction(
        mut self,
        handler: impl Fn(&str, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle_reaction = Some(Arc::new(handler));
        self
    }

    /// Who reacted, named in a pill's tooltip.
    #[must_use]
    pub fn reaction_tooltip(
        mut self,
        builder: impl Fn(&ReactionPill, &mut App) -> gpui::AnyView + 'static,
    ) -> Self {
        self.reaction_tooltip = Some(Arc::new(builder));
        self
    }

    /// Where the message's text comes in the window's reading order, for a selection dragged
    /// across messages.
    #[must_use]
    pub fn selection_order(mut self, order: u64) -> Self {
        self.selection_order = order;
        self
    }

    /// Send a failed message again.
    #[must_use]
    pub fn on_retry(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_retry = Some(Arc::new(handler));
        self
    }

    /// A right-click on the author's name or avatar: their menu (`onopenauthormenu`).
    #[must_use]
    pub fn on_open_author_menu(
        mut self,
        handler: impl Fn(Point<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_author_menu = Some(Arc::new(handler));
        self
    }

    /// Drop a failed message from the outbox.
    #[must_use]
    pub fn on_discard(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_discard = Some(Arc::new(handler));
        self
    }
}

/// A word that acts, as the web's inline text buttons ("Retry", "Delete").
fn text_button(
    id: ElementId,
    label: &'static str,
    accessible_label: &'static str,
    on_click: Option<ActivationHandler>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let focus_handle = remembered_focus_handle(&id, window, cx);
    let hover = remembered_hover(&id, window, cx);
    let is_hovered = *hover.read(cx);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(accessible_label)
        .track_focus(&focus_handle)
        .relative()
        .flex()
        .items_center()
        .min_h(length(SIZE_HIT_DESKTOP_MIN))
        .rounded(length(RADIUS_XS))
        .cursor_pointer()
        .text_color(color(tokens.component_message_editor_hint_link))
        .when(is_hovered, gpui::Styled::underline)
        .on_hover(move |is_hovered, _, cx| {
            hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, window, cx| {
            if let Some(on_click) = &on_click {
                on_click(window, cx);
            }
        })
        .child(label)
        .when(is_focus_visible(&focus_handle, window), |button| {
            button.child(focus_ring(RADIUS_XS, cx))
        })
        .into_any_element()
}

fn pinned_mark(id: ElementId, strings: Strings, tint: Hsla) -> impl IntoElement {
    let label = SharedString::from(strings.pins().pinned());
    div()
        .id(id)
        .role(Role::Image)
        .aria_label(label.clone())
        .flex()
        .items_center()
        .tooltip(move |_, cx| Tooltip::view(label.clone(), cx))
        .tooltip_show_delay(Tooltip::show_delay())
        .child(icon(
            IconName::PushPin,
            COMPONENT_REACTION_PINNED_ICON_SIZE,
            tint,
        ))
}

impl RenderOnce for MessageItem {
    #[allow(clippy::too_many_lines)] // One row and its states, as the web's markup.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let strings = i18n::strings(cx);
        let content = self.content;
        let id = self.id;
        let hover = remembered_hover(&id, window, cx);
        let is_hovered = *hover.read(cx);
        let is_sending = content.pending == Some(PendingMessageStatus::Sending);
        let is_failed = content.pending == Some(PendingMessageStatus::Failed);
        // "Sending…" appears only after a delay, so fast sends do not flicker (DESIGN.md §5.4).
        let is_sending_label_shown = is_sending
            && *window
                .use_keyed_state(child_id(&id, "sending-label"), cx, |_, cx| {
                    cx.spawn(async move |shown, cx| {
                        cx.background_executor()
                            .timer(delay(MOTION_DELAY_PENDING_LABEL))
                            .await;
                        let _ = shown.update(cx, |shown, cx| {
                            *shown = true;
                            cx.notify();
                        });
                    })
                    .detach();
                    false
                })
                .read(cx);

        let background = colour_transition(
            child_id(&id, "background"),
            match (content.is_mentioning_reader, is_hovered) {
                (true, false) => color(tokens.component_mention_background),
                (true, true) => color(tokens.component_mention_background_hover),
                (false, true) => color(tokens.component_message_background_hover),
                (false, false) => Hsla {
                    a: 0.0,
                    ..color(tokens.component_message_background_hover)
                },
            },
            window,
            cx,
        );
        let author_name = display_name(&content.author.user).to_owned();
        // The author's menu instead of the message's, which the row opens.
        let with_author_menu = |element: gpui::Div| match self.on_open_author_menu.clone() {
            Some(open) => element.on_mouse_down(MouseButton::Right, move |event, window, cx| {
                cx.stop_propagation();
                open(event.position, window, cx);
            }),
            None => element,
        };
        let sent = content.sent_at_unix_milliseconds.map(local_date_time);
        let now = local_date_time(content.now_unix_milliseconds);

        let header: Option<AnyElement> = content.starts_author_group.then(|| {
            let time = sent.map(|sent| {
                let full: SharedString = full_timestamp_label(sent, strings).into();
                with_type(div(), TYPE_MESSAGE_TIMESTAMP)
                    .id(child_id(&id, "time"))
                    .aria_label(full.clone())
                    .text_color(color(tokens.component_message_timestamp_text))
                    .tooltip(move |_, cx| Tooltip::view(full.clone(), cx))
                    .tooltip_show_delay(Tooltip::show_delay())
                    .child(message_timestamp_label(sent, now, strings))
            });
            div()
                .id(child_id(&id, "header"))
                .role(Role::Heading)
                .aria_level(MESSAGE_HEADER_HEADING_LEVEL)
                .flex()
                .flex_wrap()
                .items_baseline()
                .gap_x(length(SPACE_XS))
                .child(
                    with_author_menu(with_type(div(), TYPE_MESSAGE_AUTHOR))
                        .text_color(
                            content
                                .author
                                .colour
                                .unwrap_or_else(|| color(tokens.component_message_author_text)),
                        )
                        .child(author_name.clone()),
                )
                .children(time)
                .when(content.is_pinned, |header| {
                    header.child(pinned_mark(
                        child_id(&id, "pinned"),
                        strings,
                        color(tokens.component_reaction_pinned_icon),
                    ))
                })
                .into_any_element()
        });
        let avatar_top = if content.reply.is_some() {
            COMPONENT_MESSAGE_PADDING_BLOCK + SPACE_2XS + COMPONENT_MESSAGE_REPLY_HEIGHT
        } else {
            COMPONENT_MESSAGE_PADDING_BLOCK + SPACE_3XS
        };
        let avatar = content.starts_author_group.then(|| {
            with_author_menu(div())
                .absolute()
                .left(length(COMPONENT_MESSAGE_AVATAR_INSET))
                .top(length(avatar_top))
                .child(Avatar::new(
                    AvatarPerson::from_user(&content.author.user, content.author.picture.clone()),
                    COMPONENT_MESSAGE_AVATAR_SIZE,
                ))
        });
        // A continued message shows its time in the gutter while hovered.
        let gutter_time = (!content.starts_author_group && is_hovered)
            .then_some(sent)
            .flatten()
            .map(|sent| {
                with_type(div(), TYPE_MESSAGE_TIMESTAMP)
                    .absolute()
                    .left_0()
                    .top(length(COMPONENT_MESSAGE_PADDING_BLOCK))
                    .w(length(COMPONENT_MESSAGE_GUTTER))
                    .flex()
                    .justify_center()
                    .text_color(color(tokens.component_message_timestamp_text))
                    .child(time_of_day_label(sent, strings))
            });
        let gutter_pin = (!content.starts_author_group && content.is_pinned).then(|| {
            // In the gutter's trailing corner, clear of the hover time.
            div()
                .absolute()
                .top(length(COMPONENT_MESSAGE_PADDING_BLOCK))
                .left(length(
                    COMPONENT_MESSAGE_GUTTER - COMPONENT_REACTION_PINNED_ICON_SIZE - SPACE_XS,
                ))
                .child(pinned_mark(
                    child_id(&id, "pinned"),
                    strings,
                    color(tokens.component_reaction_pinned_icon),
                ))
        });
        let edited = content
            .edited_at_unix_milliseconds
            .map(|edited_at| EditedMarker {
                label: strings.messages().edited().into(),
                tooltip: strings
                    .messages()
                    .edited_at(&full_timestamp_label(local_date_time(edited_at), strings))
                    .into(),
            });
        let body = with_type(div(), TYPE_MESSAGE)
            .min_w_0()
            .text_color(color(if is_sending {
                tokens.component_message_pending_text
            } else {
                tokens.component_message_body_text
            }))
            .child(
                MessageMarkdown::new(id.clone(), content.body.clone())
                    .edited(edited)
                    .selection_order(self.selection_order),
            );
        let attachments = (!content.attachments.is_empty()).then(|| {
            MessageAttachments::new(child_id(&id, "attachments"), content.attachments.clone())
                .pending(content.pending.is_some())
        });
        let reactions = (!content.reactions.is_empty()).then(|| {
            let reactions = MessageReactions::new(child_id(&id, "reactions"), content.reactions)
                .can_join(self.can_join_reactions)
                .tooltip(self.reaction_tooltip);
            match self.on_toggle_reaction {
                Some(on_toggle) => {
                    reactions.on_toggle(move |key, window, cx| on_toggle(key, window, cx))
                }
                None => reactions,
            }
        });
        let sending_label = is_sending_label_shown.then(|| {
            with_type(div(), TYPE_MESSAGE_SUBTEXT)
                .text_color(color(tokens.component_message_pending_text))
                .child(strings.messages().sending())
        });
        let failure = is_failed.then(|| {
            let failed_text = color(tokens.component_message_failed_status_text);
            with_type(div(), TYPE_MESSAGE_SUBTEXT)
                .id(child_id(&id, "failure"))
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(length(SPACE_XS))
                .text_color(failed_text)
                .child(icon(
                    IconName::WarningCircle,
                    COMPONENT_MESSAGE_STATUS_ICON_SIZE,
                    failed_text,
                ))
                .child(strings.messages().not_sent())
                .child(text_button(
                    child_id(&id, "retry"),
                    strings.messages().retry(),
                    strings.messages().retry_label(),
                    self.on_retry,
                    window,
                    cx,
                ))
                .child(text_button(
                    child_id(&id, "discard"),
                    strings.common().delete(),
                    strings.messages().delete_unsent_label(),
                    self.on_discard,
                    window,
                    cx,
                ))
        });
        let mention_bar = content.is_mentioning_reader.then(|| {
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(length(COMPONENT_MENTION_BAR_WIDTH))
                .bg(color(tokens.component_mention_bar_color))
        });
        let mut description = Vec::new();
        if content.is_mentioning_reader {
            description.push(strings.messages().mentions_you().to_owned());
        }
        if !content.starts_author_group {
            description.push(
                strings
                    .messages()
                    .author_said(&author_name, "")
                    .trim()
                    .to_owned(),
            );
        }

        div()
            .id(id.clone())
            .role(Role::ListItem)
            .when(!description.is_empty(), |row| {
                row.aria_description(description.join(". "))
            })
            .relative()
            .flex()
            .flex_col()
            .min_w_0()
            .pl(length(COMPONENT_MESSAGE_GUTTER))
            .pr(length(COMPONENT_MESSAGE_PADDING_TRAILING))
            .py(length(COMPONENT_MESSAGE_PADDING_BLOCK))
            .when(content.starts_author_group, |row| {
                row.mt(length(COMPONENT_MESSAGE_GROUP_GAP))
            })
            .bg(background)
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .children(mention_bar)
            .children(
                content
                    .reply
                    .map(|(reference, picture)| ReplyLine::new(reference, picture)),
            )
            .children(avatar)
            .children(header)
            .children(gutter_time)
            .children(gutter_pin)
            .map(|row| match self.forward {
                // A forward copies no preview (ADR-0039).
                Some((origin, on_open_origin)) => {
                    let mut copied = vec![body.into_any_element()];
                    copied.extend(attachments.map(IntoElement::into_any_element));
                    row.child(ForwardFrame::new(
                        child_id(&id, "forward"),
                        origin,
                        content.now_unix_milliseconds,
                        copied,
                        on_open_origin,
                    ))
                }
                None => row
                    .child(body)
                    .children(attachments)
                    .children(self.link_previews),
            })
            .children(reactions)
            .children(sending_label)
            .children(failure)
    }
}

impl std::fmt::Debug for MessageItem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MessageItem")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for MessageItemContent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MessageItemContent")
            .field("author", &self.author.user.id)
            .field("sent_at_unix_milliseconds", &self.sent_at_unix_milliseconds)
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for MessageAuthor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MessageAuthor")
            .field("user", &self.user.id)
            .finish_non_exhaustive()
    }
}
