//! The custom status dialog (`CustomStatusDialog.svelte`, DESIGN.md §5.3), opened from the user
//! card's status menu: one field, Save status, Cancel, and Clear status once there is one. Enter
//! saves, Esc cancels. The status shape stays as it is; only the sentence changes.

use gpui::{
    AnyWindowHandle, AppContext as _, Context, Entity, Focusable as _, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, WeakEntity, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::ClientError;
use mellow_client_core::shared_logic::profiles::custom_status_characters_left;
use mellow_protocol::{ApiErrorCode, ChosenStatus};

use crate::components::{Button, ButtonVariant, ModalDialog, ModalDialogEvent, TextField};
use crate::i18n::strings;
use crate::session::Session;
use crate::sign_in_form::describe_error;
use crate::theme::length;
use crate::theme::tokens::{COMPONENT_BUTTON_PADDING_INLINE, COMPONENT_TEXT_FIELD_HELPER_GAP};

/// The dialog's field and Clear status, drawn inside the `ModalDialog` that owns the footer.
pub(super) struct CustomStatusForm {
    session: Entity<Session>,
    input: Entity<InputState>,
    /// Read once: the field is the person's draft from here on.
    saved: String,
    /// Set once the dialog holding the form exists.
    dialog: Option<WeakEntity<ModalDialog>>,
    window_handle: AnyWindowHandle,
    /// The field's and the dialog's events, for as long as the form lives.
    subscriptions: Vec<Subscription>,
}

impl CustomStatusForm {
    /// Opens the dialog over the window, focus in the field; the returned subscription closes it
    /// through `on_closed`.
    pub(super) fn open<Host: 'static>(
        session: Entity<Session>,
        window: &mut Window,
        cx: &mut Context<Host>,
        on_closed: impl Fn(&mut Host, &mut Context<Host>) + 'static,
    ) -> (Entity<ModalDialog>, Subscription) {
        let catalogue = strings(cx);
        let saved = session
            .read(cx)
            .state()
            .current_user()
            .and_then(|user| user.custom_status.clone())
            .unwrap_or_default();
        let title = if saved.is_empty() {
            catalogue.user_card().set_custom_status()
        } else {
            catalogue.user_card().edit_custom_status()
        };
        let save = catalogue.custom_status().save();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(catalogue.custom_status().placeholder())
                .default_value(saved.clone())
        });
        let window_handle = window.window_handle();
        let form = cx.new(|_| Self {
            session,
            input: input.clone(),
            saved,
            dialog: None,
            window_handle,
            subscriptions: Vec::new(),
        });
        let dialog = cx.new(|cx| {
            ModalDialog::new(title, catalogue.common().cancel(), window, cx)
                .submit(save, save, false)
                .body(form.clone().into())
        });
        form.update(cx, |form, cx| {
            form.subscriptions = vec![
                cx.subscribe(
                    &input,
                    |form: &mut Self, _, event: &InputEvent, cx| match event {
                        InputEvent::Change => form.refresh_submit(cx),
                        InputEvent::PressEnter { .. } if form.can_save(cx) => {
                            form.save(Some(form.draft(cx)), cx);
                        }
                        _ => {}
                    },
                ),
                cx.subscribe(
                    &dialog,
                    |form: &mut Self, _, event: &ModalDialogEvent, cx| {
                        if *event == ModalDialogEvent::Submitted {
                            form.save(Some(form.draft(cx)), cx);
                        }
                    },
                ),
            ];
            form.dialog = Some(dialog.downgrade());
            form.refresh_submit(cx);
        });
        let input_focus = input.read(cx).focus_handle(cx);
        window.focus(&input_focus, cx);
        let subscription = cx.subscribe(&dialog, move |host, _, event: &ModalDialogEvent, cx| {
            if *event == ModalDialogEvent::Closed {
                on_closed(host, cx);
            }
        });
        (dialog, subscription)
    }

    fn dialog(&self) -> Option<Entity<ModalDialog>> {
        self.dialog.as_ref().and_then(WeakEntity::upgrade)
    }

    fn draft(&self, cx: &gpui::App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn can_save(&self, cx: &gpui::App) -> bool {
        let draft = self.draft(cx);
        draft.trim() != self.saved && custom_status_characters_left(&draft) >= 0
    }

    fn refresh_submit(&mut self, cx: &mut Context<Self>) {
        let is_enabled = self.can_save(cx);
        if let Some(dialog) = self.dialog() {
            dialog.update(cx, |dialog, cx| dialog.set_submit_enabled(is_enabled, cx));
        }
        cx.notify();
    }

    /// `None` clears the custom status. Closes on success; a failure stays in the dialog.
    fn save(&mut self, custom_status: Option<String>, cx: &mut Context<Self>) {
        let Some(dialog) = self.dialog() else {
            return;
        };
        if dialog.read(cx).is_busy() {
            return;
        }
        dialog.update(cx, |dialog, cx| {
            dialog.set_busy(true, cx);
            dialog.set_error(None, cx);
        });
        let status = self
            .session
            .read(cx)
            .state()
            .current_user()
            .map_or(ChosenStatus::Online, |user| user.status);
        let window = self.window_handle;
        let catalogue = strings(cx);
        let error_strings = crate::strings_pending_i18n::strings(cx);
        self.session.update(cx, |session, cx| {
            session.update_presence(status, custom_status, cx, move |answer, cx| {
                dialog.update(cx, |dialog, cx| dialog.set_busy(false, cx));
                match answer {
                    Ok(()) => {
                        let _ = window.update(cx, |_, window, cx| {
                            dialog.update(cx, |dialog, cx| dialog.request_close(window, cx));
                        });
                    }
                    Err(error) => {
                        let message = failure_message(&error, catalogue, error_strings);
                        dialog.update(cx, |dialog, cx| dialog.set_error(Some(message.into()), cx));
                    }
                }
            });
        });
    }
}

/// The web's wording: a refusal names what to fix, anything else says why it failed.
fn failure_message(
    error: &ClientError,
    catalogue: crate::i18n::Strings,
    error_strings: &crate::strings_pending_i18n::Strings,
) -> String {
    if matches!(
        error,
        ClientError::Api {
            code: ApiErrorCode::ValidationFailed,
            ..
        }
    ) {
        catalogue.custom_status().refused().to_owned()
    } else {
        describe_error(error, catalogue.custom_status().failed(), error_strings)
    }
}

impl Render for CustomStatusForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalogue = strings(cx);
        let characters_left = custom_status_characters_left(&self.draft(cx));
        let is_busy = self
            .dialog()
            .is_some_and(|dialog| dialog.read(cx).is_busy());
        let entity = cx.entity();
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(length(COMPONENT_TEXT_FIELD_HELPER_GAP))
            .child(
                div().w_full().child(
                    TextField::new(
                        "custom-status",
                        catalogue.custom_status().field(),
                        &self.input,
                    )
                    .helper(catalogue.custom_status().characters_left(characters_left))
                    .read_only(is_busy),
                ),
            )
            .when(!self.saved.is_empty(), |form| {
                // Its label lines up with the field's edge; the hover wash still reaches past it.
                form.child(
                    div().ml(-length(COMPONENT_BUTTON_PADDING_INLINE)).child(
                        Button::new("custom-status-clear", catalogue.custom_status().clear())
                            .variant(ButtonVariant::Ghost)
                            .disabled(is_busy)
                            .on_click(move |_, cx| {
                                entity.update(cx, |form, cx| form.save(None, cx));
                            }),
                    ),
                )
            })
    }
}
