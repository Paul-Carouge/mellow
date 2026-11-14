//! "Delete server" and "Delete channel" (`DeleteServerDialog.svelte`, `DeleteChannelDialog.svelte`,
//! DESIGN.md §5.8, §8): irreversible for everyone, so the name must be typed before the danger
//! button wakes up. Cancel takes focus; the name is compared trimmed, as the instance stores names.

use gpui::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Subscription, WeakEntity, Window, div,
};
use gpui_component::input::{InputEvent, InputState};

use crate::components::{ModalDialog, TextField};
use crate::i18n::strings;
use crate::session::Session;

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

/// Reads the server's or channel's name as it is now; `None` once it is gone.
type CurrentName = Box<dyn Fn(&App) -> Option<String>>;

/// The dialog's field: it wakes the danger button up once the name is typed. The name is the
/// current one: renamed while the dialog is open, the old name no longer confirms.
pub struct NameConfirmation {
    input: Entity<InputState>,
    current_name: CurrentName,
    /// The dialog the field sits in, known once both exist.
    dialog: WeakEntity<ModalDialog>,
    label: SharedString,
    _subscriptions: Vec<Subscription>,
}

impl NameConfirmation {
    fn new(
        current_name: CurrentName,
        session: &Entity<Session>,
        label: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(
                &input,
                window,
                |field: &mut Self, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => field.refresh(cx),
                    // Enter in the field is the dialog's action, once the name is right.
                    InputEvent::PressEnter { .. } if field.is_confirmed(cx) => {
                        let _ = field.dialog.update(cx, ModalDialog::submit_now);
                    }
                    _ => {}
                },
            ),
            cx.observe(session, |field, _, cx| field.refresh(cx)),
        ];
        Self {
            input,
            current_name,
            dialog: WeakEntity::new_invalid(),
            label,
            _subscriptions: subscriptions,
        }
    }

    /// The field, for tests that type into it.
    #[cfg(test)]
    pub fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    /// Whether what is typed is the current name.
    pub fn is_confirmed(&self, cx: &App) -> bool {
        (self.current_name)(cx)
            .is_some_and(|name| is_name_confirmed(&self.input.read(cx).value(), &name))
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let is_confirmed = self.is_confirmed(cx);
        let _ = self.dialog.update(cx, |dialog, cx| {
            if dialog.is_submit_enabled() != is_confirmed {
                dialog.set_submit_enabled(is_confirmed, cx);
            }
        });
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

/// Opens the dialog over the window, its danger button asleep until the current name (read by
/// `current_name`, followed through `session`) is typed. The host checks
/// [`NameConfirmation::is_confirmed`] again before deleting.
pub fn open_name_confirmation_dialog<Host: 'static>(
    words: NameConfirmationWords,
    current_name: impl Fn(&App) -> Option<String> + 'static,
    session: &Entity<Session>,
    window: &mut Window,
    cx: &mut Context<Host>,
) -> (Entity<ModalDialog>, Entity<NameConfirmation>) {
    let cancel = strings(cx).common().cancel();
    let label = SharedString::from(words.field_label);
    let field =
        cx.new(|cx| NameConfirmation::new(Box::new(current_name), session, label, window, cx));
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
    (dialog, field)
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
