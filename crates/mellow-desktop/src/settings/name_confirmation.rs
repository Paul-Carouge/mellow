//! "Delete server" and "Delete channel" (`DeleteServerDialog.svelte`, `DeleteChannelDialog.svelte`,
//! DESIGN.md §5.8, §8): irreversible for everyone, so the name must be typed before the danger
//! button wakes up. Cancel takes focus; the name is compared trimmed, as the instance stores names.

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Subscription, WeakEntity, Window, div,
};
use gpui_component::input::{InputEvent, InputState};

use crate::components::{ModalDialog, TextField};
use crate::i18n::strings;

/// The dialog's words; the host runs the deletion on `ModalDialogEvent::Submitted`.
pub struct NameConfirmationWords {
    pub title: String,
    pub body: String,
    pub field_label: String,
    pub submit: &'static str,
    pub busy_submit: &'static str,
}

/// Whether what was typed confirms `name`.
#[must_use]
pub fn is_name_confirmed(typed: &str, name: &str) -> bool {
    typed.trim() == name
}

/// The dialog's field: it wakes the danger button up once the name is typed.
pub struct NameConfirmation {
    input: Entity<InputState>,
    /// The dialog the field sits in, known once both exist.
    dialog: WeakEntity<ModalDialog>,
    label: SharedString,
    _subscription: Subscription,
}

impl NameConfirmation {
    fn new(name: String, label: SharedString, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |field: &mut Self, input, event: &InputEvent, _, cx| match event {
                InputEvent::Change => {
                    let is_confirmed = is_name_confirmed(&input.read(cx).value(), &name);
                    let _ = field.dialog.update(cx, |dialog, cx| {
                        dialog.set_submit_enabled(is_confirmed, cx);
                    });
                }
                // Enter in the field is the dialog's action, once the name is right.
                InputEvent::PressEnter { .. } => {
                    let _ = field.dialog.update(cx, ModalDialog::submit_now);
                }
                _ => {}
            },
        );
        Self {
            input,
            dialog: WeakEntity::new_invalid(),
            label,
            _subscription: subscription,
        }
    }
}

impl Render for NameConfirmation {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child(TextField::new(
            "name-confirmation",
            self.label.clone(),
            &self.input,
        ))
    }
}

/// Opens the dialog over the window, its danger button asleep until `name` is typed.
pub fn open_name_confirmation_dialog<Host: 'static>(
    words: NameConfirmationWords,
    name: String,
    window: &mut Window,
    cx: &mut Context<Host>,
) -> Entity<ModalDialog> {
    let cancel = strings(cx).common().cancel();
    let label = SharedString::from(words.field_label);
    let field = cx.new(|cx| NameConfirmation::new(name, label, window, cx));
    let dialog = cx.new(|cx| {
        let mut dialog = ModalDialog::new(words.title, cancel, window, cx)
            .subtitle(words.body)
            .body(field.clone().into())
            .submit(words.submit, words.busy_submit, true);
        dialog.set_submit_enabled(false, cx);
        dialog
    });
    let weak_dialog = dialog.downgrade();
    field.update(cx, |field, _| field.dialog = weak_dialog);
    dialog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_must_be_typed_whole_but_spaces_around_it_do_not_count() {
        assert!(is_name_confirmed("  Test T5 ", "Test T5"));
        assert!(!is_name_confirmed("Test", "Test T5"));
        assert!(!is_name_confirmed("test t5", "Test T5"));
    }
}
