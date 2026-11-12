//! What the user settings' pages share: the account card's look, a section's label, an error
//! read out as it appears, the edit form's buttons (`AccountEditForm.svelte`) and the words of a
//! failure that may be a wrong current password.

use crate::components::LiveRegion as _;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    Role, SharedString, Styled as _, Window, div, prelude::FluentBuilder as _,
};
use gpui::{Stateful, StatefulInteractiveElement as _};
use gpui_component::input::InputState;
use mellow_client_core::ClientError;
use mellow_protocol::ApiErrorCode;

use crate::components::{Button, ButtonVariant, PasswordToggle, TextField, icon, with_type};
use crate::i18n::strings;
use crate::sign_in_form::describe_error;
use crate::strings_pending_i18n::strings as pending_strings;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP, COMPONENT_SETTINGS_ACCOUNT_CARD_PADDING,
    COMPONENT_SETTINGS_ACCOUNT_CARD_RADIUS, COMPONENT_SETTINGS_ERROR_ICON_GAP,
    COMPONENT_SETTINGS_ERROR_ICON_SIZE, COMPONENT_TEXT_FIELD_FIELD_GAP, TYPE_BODY, TYPE_BODY_LG,
    TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};

/// What a button does when pressed.
pub(super) type Action = Arc<dyn Fn(&mut Window, &mut App) + 'static>;

/// The account card's surface (`--mw-component-settings-account-card-*`).
pub(super) fn card(cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(length(COMPONENT_SETTINGS_ACCOUNT_CARD_RADIUS))
        .bg(color(theme(cx).component_settings_account_card_background))
        .p(length(COMPONENT_SETTINGS_ACCOUNT_CARD_PADDING))
}

/// A section's label, as the web's `<h3 class="type-category">`.
pub(super) fn section_label(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    cx: &App,
) -> Stateful<Div> {
    let text = text.into();
    with_type(div(), TYPE_CATEGORY)
        .id(id)
        .role(Role::Heading)
        .aria_label(text.clone())
        .text_color(color(theme(cx).component_settings_section_label_text))
        .child(text)
}

/// A sentence of the page's body.
pub(super) fn body_text(text: impl Into<SharedString>, cx: &App) -> Div {
    with_type(div(), TYPE_BODY_LG)
        .text_color(color(theme(cx).component_settings_body_text))
        .child(text.into())
}

/// The error a form raised, under it. The alert node is always drawn, empty when nothing is
/// wrong, because screen readers often miss a live region inserted with its text.
pub(super) fn error_line(id: impl Into<ElementId>, message: Option<SharedString>, cx: &App) -> Div {
    let tokens = theme(cx);
    div()
        .child(
            div()
                .id(id)
                .live_assertive(message.clone().unwrap_or_default())
                .h_0()
                .overflow_hidden(),
        )
        .when_some(message, |wrapper, message| {
            wrapper.child(
                with_type(div(), TYPE_BODY)
                    .flex()
                    .gap(length(COMPONENT_SETTINGS_ERROR_ICON_GAP))
                    .text_color(color(tokens.component_settings_error_text))
                    .child(icon(
                        IconName::WarningCircle,
                        COMPONENT_SETTINGS_ERROR_ICON_SIZE,
                        color(tokens.component_settings_error_icon),
                    ))
                    .child(message),
            )
        })
}

/// The labels of an edit form's buttons.
pub(super) struct FormButtons {
    pub id: &'static str,
    pub save_label: SharedString,
    pub busy_label: SharedString,
    pub is_save_enabled: bool,
    pub is_busy: bool,
}

/// Fields, the error they raised, then Cancel and Save, as `AccountEditForm.svelte`.
pub(super) fn edit_form(
    fields: Vec<AnyElement>,
    error: impl IntoElement,
    buttons: FormButtons,
    on_save: Action,
    on_cancel: Action,
    cx: &App,
) -> Div {
    let catalogue = strings(cx);
    div()
        .flex()
        .flex_col()
        .gap(length(COMPONENT_TEXT_FIELD_FIELD_GAP))
        .children(fields)
        .child(error)
        .child(
            div()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap(length(COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP))
                .child(
                    Button::new(
                        SharedString::from(format!("{}-cancel", buttons.id)),
                        catalogue.common().cancel(),
                    )
                    .disabled(buttons.is_busy)
                    .on_click(move |window, cx| on_cancel(window, cx)),
                )
                .child(
                    Button::new(
                        SharedString::from(format!("{}-save", buttons.id)),
                        if buttons.is_busy {
                            buttons.busy_label
                        } else {
                            buttons.save_label
                        },
                    )
                    .variant(ButtonVariant::Primary)
                    .disabled(!buttons.is_save_enabled || buttons.is_busy)
                    .on_click(move |window, cx| on_save(window, cx)),
                ),
        )
}

/// A password field with its show/hide toggle.
pub(super) fn password_field(
    id: &'static str,
    label: &'static str,
    input: &gpui::Entity<InputState>,
    is_read_only: bool,
    cx: &App,
) -> TextField {
    TextField::new(id, label, input)
        .password(PasswordToggle {
            label: strings(cx).sign_in().show_password().into(),
        })
        .read_only(is_read_only)
}

/// A failure in words; no login was typed here, so a refused credential is the current password.
pub(super) fn describe_failure(error: &ClientError, summary: &str, cx: &App) -> String {
    let pending = pending_strings(cx);
    if matches!(
        error,
        ClientError::Api {
            code: ApiErrorCode::InvalidCredentials,
            ..
        }
    ) {
        return (pending.error_with_reason)(
            summary,
            strings(cx).account().wrong_current_password(),
        );
    }
    describe_error(error, summary, pending)
}
