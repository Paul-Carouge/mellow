//! The emoji picker in the conversation (`ChatPane.svelte`'s `picker`): opened by the composer's
//! emoji button or Ctrl+E (an emoji goes in at the caret), and by Add reaction in a message's
//! hover bar, menu or reactions (the emoji reacts). The quick reactions are the account's latest
//! picks, then the defaults.

use std::sync::Arc;

use gpui::{
    AnyElement, AppContext as _, Bounds, Context, ElementId, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Toggled, Window, div,
    prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::emoji_picker::{PickerEmoji, quick_reactions};
use mellow_client_core::state::reaction_key;
use mellow_protocol::{Message, Snowflake};

use super::ConversationView;
use crate::components::{
    IconButton, Tooltip, bounds_recorder, child_id, colour_transition, focus_ring,
    is_focus_visible, remembered_bounds, remembered_focus_handle, remembered_hover,
};
use crate::emoji_picker::{
    EmojiPicker, EmojiPickerEvent, EmojiPickerServer, note_emoji_used, recent_emoji,
};
use crate::i18n::strings;
use crate::messages::AddReactionButton;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_ACTION_BAR_BUTTON_SIZE, COMPONENT_ACTION_BAR_ICON_SIZE, RADIUS_SM,
};
use crate::theme::{color, length, theme};

/// What a pick does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EmojiTarget {
    Composer,
    /// Reacts to this message.
    Message(Snowflake),
}

/// The open picker, what it is for, and where focus goes back after Esc or a reaction.
pub(super) struct OpenEmojiPicker {
    pub(super) picker: Entity<EmojiPicker>,
    pub(super) target: EmojiTarget,
    /// The button (or row) it opened beside: a click there dismissed it on purpose.
    anchor: Bounds<Pixels>,
    opener: Option<FocusHandle>,
    _subscription: Subscription,
}

impl ConversationView {
    fn reader_id(&self, cx: &gpui::App) -> Option<Snowflake> {
        self.session
            .read(cx)
            .state()
            .current_user()
            .map(|user| user.id)
    }

    /// The hover bar's and the menu's three quick reactions.
    pub(super) fn quick_reactions(&self, cx: &gpui::App) -> Vec<String> {
        quick_reactions(&recent_emoji(self.reader_id(cx), cx))
    }

    /// A quick reaction: remembered as used, then added or taken back.
    pub(super) fn react_with_quick_reaction(
        &mut self,
        message_id: Snowflake,
        emoji: &str,
        cx: &mut Context<Self>,
    ) {
        note_emoji_used(self.reader_id(cx), emoji, cx);
        self.toggle_reaction(message_id, emoji, cx);
    }

    /// The message the open picker reacts to: its hover bar and add button stay for focus to
    /// come back to.
    pub(super) fn message_holding_the_picker(&self) -> Option<Snowflake> {
        match self.emoji_picker.as_ref()?.target {
            EmojiTarget::Message(message_id) => Some(message_id),
            EmojiTarget::Composer => None,
        }
    }

    /// Opens the picker beside `anchor`, or closes it when the same button opened it (a second
    /// click on the opener, whose mouse-down already dismissed it: `createPopoverToggle`).
    pub(super) fn toggle_emoji_picker(
        &mut self,
        target: EmojiTarget,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_same_click_as_dismissal =
            self.emoji_picker_dismissed_at
                .take()
                .is_some_and(|(position, dismissed)| {
                    dismissed == target && position == window.mouse_position()
                });
        if is_same_click_as_dismissal {
            return;
        }
        if self
            .emoji_picker
            .as_ref()
            .is_some_and(|open| open.target == target)
        {
            self.close_emoji_picker(true, window, cx);
            return;
        }
        let state = self.session.read(cx).state();
        // The open server's emoji only, and none in a conversation (ADR-0028).
        let server = self
            .open_conversation
            .as_ref()
            .and_then(|open| open.guild_id)
            .and_then(|guild_id| state.guild(guild_id))
            .map(|guild| EmojiPickerServer {
                name: guild.name.clone().into(),
                emojis: guild.emojis.clone(),
            });
        let recent = recent_emoji(self.reader_id(cx), cx);
        let opener = window.focused(cx);
        let picker = cx.new(|cx| EmojiPicker::open(anchor, recent, server, window, cx));
        let subscription = cx.subscribe_in(
            &picker,
            window,
            |view, _, event: &EmojiPickerEvent, window, cx| match event {
                EmojiPickerEvent::Picked(emoji) => view.use_picked_emoji(emoji, window, cx),
                EmojiPickerEvent::Dismissed {
                    should_return_focus,
                } => {
                    let position = window.mouse_position();
                    // Only a click on the opener is a toggle; a click anywhere else, then a
                    // shortcut or another message's button, must still open a picker.
                    view.emoji_picker_dismissed_at = view
                        .emoji_picker
                        .as_ref()
                        .filter(|open| !should_return_focus && open.anchor.contains(&position))
                        .map(|open| (position, open.target));
                    view.close_emoji_picker(*should_return_focus, window, cx);
                }
            },
        );
        self.emoji_picker = Some(OpenEmojiPicker {
            picker,
            target,
            anchor,
            opener,
            _subscription: subscription,
        });
        self.show_composer_picker_state(cx);
        cx.notify();
    }

    pub(super) fn close_emoji_picker(
        &mut self,
        should_return_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(open) = self.emoji_picker.take()
            && should_return_focus
            && let Some(opener) = open.opener
        {
            window.focus(&opener, cx);
        }
        self.show_composer_picker_state(cx);
        cx.notify();
    }

    /// The composer's emoji button says whether its picker is open.
    pub(super) fn show_composer_picker_state(&self, cx: &mut Context<Self>) {
        let is_open = self
            .emoji_picker
            .as_ref()
            .is_some_and(|open| open.target == EmojiTarget::Composer);
        self.composer.update(cx, |composer, cx| {
            composer.set_emoji_picker_open(is_open, cx);
        });
    }

    /// A reaction goes back to the button that opened the picker; into the composer, focus goes
    /// to the field, at the caret it had.
    fn use_picked_emoji(
        &mut self,
        emoji: &PickerEmoji,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.emoji_picker.as_ref().map(|open| open.target) else {
            return;
        };
        if let PickerEmoji::Unicode(emoji) = emoji {
            note_emoji_used(self.reader_id(cx), emoji, cx);
        }
        match target {
            EmojiTarget::Message(message_id) => {
                self.close_emoji_picker(true, window, cx);
                self.toggle_reaction(message_id, &emoji.reaction_emoji(), cx);
            }
            EmojiTarget::Composer => {
                self.close_emoji_picker(false, window, cx);
                let text = emoji.composer_text();
                self.composer.update(cx, |composer, cx| {
                    composer.insert_at_caret(&text, window, cx);
                });
            }
        }
    }

    /// Add reaction from the menu: beside the message's row, or the list when it is not drawn.
    /// Never called while the list draws, which holds its state.
    pub(super) fn open_reaction_picker_beside_row(
        &mut self,
        message_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = message_id.raw().to_string();
        let anchor = self
            .entries
            .iter()
            .position(|entry| entry.key() == key)
            .and_then(|index| self.list.bounds_for_item(index))
            .unwrap_or_else(|| self.list.viewport_bounds());
        self.toggle_emoji_picker(EmojiTarget::Message(message_id), anchor, window, cx);
    }

    /// The hover bar's first keys, where the reader may start a reaction (`actions.canAddReactions`):
    /// the quick reactions, pressed when already given, then Add reaction.
    pub(super) fn hover_bar_reaction_buttons(
        &self,
        message: &Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let message_id = message.id;
        let catalogue = strings(cx);
        let view = cx.entity().downgrade();
        let mut buttons: Vec<AnyElement> = self
            .quick_reactions(cx)
            .into_iter()
            .map(|emoji| {
                let key = reaction_key(&emoji);
                let is_given = is_reaction_given(message, &emoji);
                // Keyed by emoji, so focus follows it when a pick reorders the quick reactions.
                let id = ElementId::Name(
                    format!("hover-quick-reaction-{key}-{}", message_id.raw()).into(),
                );
                let view = view.clone();
                let label: SharedString = catalogue.reactions().react_with(&emoji).into();
                quick_reaction_button(id, emoji.clone(), label, is_given, window, cx, move |cx| {
                    let _ = view.update(cx, |view, cx| {
                        view.react_with_quick_reaction(message_id, &emoji, cx);
                    });
                })
            })
            .collect();
        let id = ElementId::Name(format!("hover-add-reaction-{}", message_id.raw()).into());
        let bounds = remembered_bounds(&id, window, cx);
        let anchor = bounds.clone();
        buttons.push(
            div()
                .relative()
                .child(
                    IconButton::new(id, IconName::Smiley, catalogue.reactions().add_reaction())
                        .sizes(
                            COMPONENT_ACTION_BAR_BUTTON_SIZE,
                            COMPONENT_ACTION_BAR_ICON_SIZE,
                        )
                        .expanded(self.message_holding_the_picker() == Some(message_id))
                        .on_click(move |window, cx| {
                            let Some(drawn) = *anchor.read(cx) else {
                                return;
                            };
                            let _ = view.update(cx, |view, cx| {
                                view.toggle_emoji_picker(
                                    EmojiTarget::Message(message_id),
                                    drawn,
                                    window,
                                    cx,
                                );
                            });
                        }),
                )
                .child(bounds_recorder(bounds))
                .into_any_element(),
        );
        buttons
    }

    /// The "+" after a message's reaction pills, shown with its hover bar (`canAddNew`).
    pub(super) fn reactions_add_button(
        &self,
        message_id: Snowflake,
        can_add_reactions: bool,
        is_row_active: bool,
        cx: &Context<Self>,
    ) -> Option<AddReactionButton> {
        if !can_add_reactions {
            return None;
        }
        let view = cx.entity().downgrade();
        Some(AddReactionButton {
            on_add: Arc::new(move |anchor, window, cx| {
                let _ = view.update(cx, |view, cx| {
                    view.toggle_emoji_picker(EmojiTarget::Message(message_id), anchor, window, cx);
                });
            }),
            is_shown: is_row_active || self.message_holding_the_picker() == Some(message_id),
        })
    }
}

/// Whether the reader already reacted to `message` with `emoji`.
pub(super) fn is_reaction_given(message: &Message, emoji: &str) -> bool {
    let key = reaction_key(emoji);
    message
        .reactions
        .iter()
        .any(|reaction| reaction.me && reaction.emoji == key)
}

/// A quick reaction in the hover bar: the emoji at the icons' size, pressed when the reader gave
/// it already (taking it back then).
fn quick_reaction_button(
    id: ElementId,
    emoji: String,
    label: SharedString,
    is_given: bool,
    window: &mut Window,
    cx: &mut gpui::App,
    on_click: impl Fn(&mut gpui::App) + 'static,
) -> AnyElement {
    let tokens = theme(cx);
    let focus_handle = remembered_focus_handle(&id, window, cx);
    let hover = remembered_hover(&id, window, cx);
    let wash = color(tokens.component_action_bar_button_background_hover);
    let fill = colour_transition(
        child_id(&id, "background"),
        if *hover.read(cx) {
            wash
        } else {
            gpui::Hsla { a: 0.0, ..wash }
        },
        window,
        cx,
    );
    let is_ring_shown = is_focus_visible(&focus_handle, window);
    let tooltip = label.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .aria_toggled(if is_given {
            Toggled::True
        } else {
            Toggled::False
        })
        .track_focus(&focus_handle)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(length(COMPONENT_ACTION_BAR_BUTTON_SIZE))
        .rounded(length(RADIUS_SM))
        .bg(fill)
        .text_size(length(COMPONENT_ACTION_BAR_ICON_SIZE))
        .line_height(length(COMPONENT_ACTION_BAR_ICON_SIZE))
        .cursor_pointer()
        .tooltip(move |_, cx| Tooltip::view(tooltip.clone(), cx))
        .tooltip_show_delay(Tooltip::show_delay())
        .on_hover(move |is_hovered, _, cx| {
            hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, _, cx| on_click(cx))
        .child(emoji)
        .when(is_ring_shown, |button| {
            button.child(focus_ring(RADIUS_SM, cx))
        })
        .into_any_element()
}
