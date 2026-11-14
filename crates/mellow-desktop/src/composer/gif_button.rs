//! The composer's GIF button (`Composer.svelte`, ADR-0042): shown when the instance has GIFs and a
//! message is being written (not edited). It asks the chat pane, which hosts the GIF picker, to
//! open it beside the button; a GIF picked there goes out as its own message through
//! [`Composer::send_gif`], with the reply in progress, and the text being written stays.

use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, FocusHandle, IntoElement,
    ParentElement as _, Pixels, Styled as _, Window, div,
};

use super::{ComposedMessage, Composer, ComposerEvent};
use crate::components::{IconButton, bounds_recorder};
use crate::i18n::strings;
use crate::theme::icons::IconName;
use crate::theme::length;
use crate::theme::tokens::{COMPONENT_COMPOSER_ATTACH_BUTTON_SIZE, COMPONENT_COMPOSER_ICON_SIZE};

#[derive(Debug)]
pub(super) struct GifButton {
    /// The instance has GIFs.
    is_available: bool,
    is_picker_open: bool,
    /// Where the button was drawn: the picker opens beside it.
    bounds: Entity<Option<Bounds<Pixels>>>,
    focus: FocusHandle,
}

impl GifButton {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            is_available: false,
            is_picker_open: false,
            bounds: cx.new(|_| None),
            focus: cx.focus_handle(),
        }
    }
}

impl Composer {
    /// Whether the instance has GIFs: only then does the button show.
    pub fn set_can_choose_gifs(&mut self, can_choose: bool, cx: &mut Context<Self>) {
        if self.gif_button.is_available != can_choose {
            self.gif_button.is_available = can_choose;
            cx.notify();
        }
    }

    /// The picker's state, for the button's expanded state.
    pub fn set_gif_picker_open(&mut self, is_open: bool, cx: &mut Context<Self>) {
        if self.gif_button.is_picker_open != is_open {
            self.gif_button.is_picker_open = is_open;
            cx.notify();
        }
    }

    /// Where the GIF button was last drawn, in window pixels.
    #[must_use]
    pub fn gif_button_bounds(&self, cx: &App) -> Option<Bounds<Pixels>> {
        *self.gif_button.bounds.read(cx)
    }

    /// Where the GIF picker opens from: the button's columns from the composer's top edge, so it
    /// opens above the composer.
    #[must_use]
    pub fn gif_picker_anchor(&self, cx: &App) -> Option<Bounds<Pixels>> {
        self.gif_button_bounds(cx)
            .map(|button| self.picker_anchor(button, cx))
    }

    /// Focus back on the GIF button, after the picker closed with focus inside it.
    pub fn focus_gif_button(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.gif_button.focus, cx);
    }

    /// Sends a GIF picked in the picker as its own message (ADR-0042: its text is Klipy's media
    /// address), through the outbox like any other.
    pub fn send_gif(&mut self, message_url: String, cx: &mut Context<Self>) {
        let Some(channel_id) = self.drafts.open_channel_id() else {
            return;
        };
        if !self.can_send_messages {
            return;
        }
        let message = ComposedMessage {
            channel_id,
            text: message_url,
            reply_to: self
                .reply_target
                .take()
                .map(|reply_target| reply_target.message_id),
            attachments: Vec::new(),
        };
        cx.emit(ComposerEvent::Send(message));
        cx.notify();
    }

    /// The button, centred on the base row by `margin`; `None` while it has no place.
    pub(super) fn render_gif_button(
        &self,
        margin: f32,
        is_editing: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.gif_button.is_available || is_editing {
            return None;
        }
        let composer = cx.entity().downgrade();
        Some(
            div()
                .relative()
                .flex_none()
                .my(length(margin))
                // First, so its static place is the parent's corner.
                .child(bounds_recorder(self.gif_button.bounds.clone()))
                .child(
                    IconButton::new(
                        "composer-gif",
                        IconName::Gif,
                        strings(cx).gifs().choose_gif(),
                    )
                    .sizes(
                        COMPONENT_COMPOSER_ATTACH_BUTTON_SIZE,
                        COMPONENT_COMPOSER_ICON_SIZE,
                    )
                    .expanded(self.gif_button.is_picker_open)
                    .track_focus(&self.gif_button.focus)
                    .on_click(move |_, cx| {
                        let _ =
                            composer.update(cx, |_, cx| cx.emit(ComposerEvent::ToggleGifPicker));
                    }),
                )
                .into_any_element(),
        )
    }
}
