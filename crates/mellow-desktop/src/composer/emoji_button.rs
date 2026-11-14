//! The composer's emoji button and Discord's Ctrl+E (Cmd+E on macOS), which ask the chat to open
//! the emoji picker beside the button (DESIGN.md §5.5), and the picked emoji written at the caret.

use gpui::{
    AnyElement, App, Bounds, Context, IntoElement, KeyBinding, ParentElement as _, Pixels,
    Styled as _, Window, actions, div,
};
use mellow_client_core::shared_logic::emoji_suggestions::replace_in_draft;

use super::{Composer, ComposerEvent};
use crate::components::{IconButton, bounds_recorder};
use crate::i18n::strings;
use crate::theme::icons::IconName;
use crate::theme::length;
use crate::theme::tokens::{
    COMPONENT_COMPOSER_ATTACH_BUTTON_SIZE, COMPONENT_COMPOSER_ICON_SIZE, SIZE_DOCK_HEIGHT,
};

#[cfg(target_os = "macos")]
const OPEN_EMOJI_PICKER_KEYSTROKE: &str = "cmd-e";
#[cfg(not(target_os = "macos"))]
const OPEN_EMOJI_PICKER_KEYSTROKE: &str = "ctrl-e";
/// The shortcut as screen readers name it (`aria-keyshortcuts`).
#[cfg(target_os = "macos")]
const OPEN_EMOJI_PICKER_KEY_SHORTCUTS: &str = "Meta+E";
#[cfg(not(target_os = "macos"))]
const OPEN_EMOJI_PICKER_KEY_SHORTCUTS: &str = "Control+E";

actions!(
    composer,
    [
        /// Ctrl+E (Cmd+E on macOS) in the field: the emoji picker.
        OpenEmojiPickerInComposer
    ]
);

pub(super) fn bind_emoji_button_keys(cx: &mut App, context: Option<&str>) {
    cx.bind_keys([KeyBinding::new(
        OPEN_EMOJI_PICKER_KEYSTROKE,
        OpenEmojiPickerInComposer,
        context,
    )]);
}

impl Composer {
    /// Whether the chat shows the picker this button opened: the button says so.
    pub fn set_emoji_picker_open(&mut self, is_open: bool, cx: &mut Context<Self>) {
        if self.is_emoji_picker_open != is_open {
            self.is_emoji_picker_open = is_open;
            cx.notify();
        }
    }

    /// Writes `text` and a space at the caret, replacing what is selected (`insert`), and puts
    /// focus back in the field after them.
    pub fn insert_at_caret(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_send_messages {
            return;
        }
        let selection = self.input.read(cx).selected_range();
        let (draft, caret) = replace_in_draft(&self.text(cx), selection.start, selection.end, text);
        self.set_text(draft, window, cx);
        self.input.update(cx, |input, cx| {
            input.set_selected_range(caret..caret, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn open_emoji_picker(
        &mut self,
        _: &OpenEmojiPickerInComposer,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ask_for_emoji_picker(cx);
    }

    fn ask_for_emoji_picker(&mut self, cx: &mut Context<Self>) {
        // Drawn already: the shortcut works only where the button shows.
        if let Some(button) = *self.emoji_button_bounds.read(cx) {
            let anchor = self.picker_anchor(button, cx);
            cx.emit(ComposerEvent::OpenEmojiPicker { anchor });
        }
    }

    /// The button at the end of the base row, centred on it as the attach button is.
    pub(super) fn emoji_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let label = strings(cx).chat().choose_emoji();
        let button_margin = (SIZE_DOCK_HEIGHT - COMPONENT_COMPOSER_ATTACH_BUTTON_SIZE) / 2.0;
        let composer = cx.entity().downgrade();
        div()
            .relative()
            .my(length(button_margin))
            .child(
                IconButton::new("composer-emoji", IconName::Smiley, label)
                    .sizes(
                        COMPONENT_COMPOSER_ATTACH_BUTTON_SIZE,
                        COMPONENT_COMPOSER_ICON_SIZE,
                    )
                    .expanded(self.is_emoji_picker_open)
                    .key_shortcuts(OPEN_EMOJI_PICKER_KEY_SHORTCUTS)
                    .on_click(move |_, cx| {
                        let _ = composer.update(cx, Self::ask_for_emoji_picker);
                    }),
            )
            .child(bounds_recorder(self.emoji_button_bounds.clone()))
            .into_any_element()
    }
}

/// Where the emoji button was drawn last, for the picker to open beside it.
pub(super) type EmojiButtonBounds = gpui::Entity<Option<Bounds<Pixels>>>;
