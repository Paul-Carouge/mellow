//! My account (`AccountSettings.svelte`, DESIGN.md §5.9): the account card with the name (one
//! per account, unique, ADR-0048), the friend code and the email, the name's form opening in
//! place with a live availability check, then the password section. One form is open at a time,
//! so an error always sits under the form that raised it. Typed passwords live in the form's
//! fields only: the fields are dropped when it closes and the request's copy is wiped once sent.

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::ClientError;
use mellow_client_core::shared_logic::user_settings::{
    NAME_CHECK_PAUSE_MILLISECONDS, NameAvailabilityCheck, NameAvailabilityState, NameCheckAnswer,
    NameCheckRequest, can_change_password, can_save_display_name,
};
use mellow_protocol::{ApiErrorCode, ChangePasswordRequest, UpdateCurrentUserRequest};
use zeroize::{Zeroize as _, Zeroizing};

use super::parts::{
    FormButtons, card, describe_failure, edit_form, error_line, password_field, section_label,
};
use crate::components::{
    Button, FriendCode, FriendCodeCopyState, LiveRegion as _, TextField, with_type,
};
use crate::i18n::strings;
use crate::session::Session;
use crate::settings::{SettingsPage, SettingsPageEvent, saved_notice};
use crate::sign_in_form::{MAXIMUM_ACCOUNT_NAME_LENGTH, MINIMUM_PASSWORD_LENGTH};
use crate::theme::tokens::{
    COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP, COMPONENT_SETTINGS_ACCOUNT_ROW_GAP,
    COMPONENT_SETTINGS_ACCOUNT_VALUE_GAP, COMPONENT_SETTINGS_SECTION_GAP,
    COMPONENT_SETTINGS_SECTION_LABEL_GAP, TYPE_BODY_LG, TYPE_BODY_SM, TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccountForm {
    DisplayName,
    Password,
}

/// The password form's three fields.
struct PasswordFields {
    current: Entity<InputState>,
    new: Entity<InputState>,
    confirmation: Entity<InputState>,
}

pub struct AccountPage {
    session: Entity<Session>,
    open_form: Option<AccountForm>,
    name_input: Option<Entity<InputState>>,
    password_fields: Option<PasswordFields>,
    form_subscriptions: Vec<Subscription>,
    is_busy: bool,
    error_message: Option<SharedString>,
    notice: Option<(AccountForm, SharedString)>,
    name_check: NameAvailabilityCheck,
    /// Waits for typing to pause; dropping it cancels the wait.
    name_check_wait: Option<Task<()>>,
    edit_name_focus: FocusHandle,
    change_password_focus: FocusHandle,
    friend_code_copy: Entity<FriendCodeCopyState>,
    /// Focus to move once drawn: answers arrive without a window.
    focus_on_render: Option<FocusHandle>,
    _session_observation: Subscription,
}

impl EventEmitter<SettingsPageEvent> for AccountPage {}

impl SettingsPage for AccountPage {
    fn set_attention(&mut self, _: u32, _: &mut Context<Self>) {}
}

impl std::fmt::Debug for AccountPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccountPage")
            .field("open_form", &self.open_form)
            .field("is_busy", &self.is_busy)
            .finish_non_exhaustive()
    }
}

/// Reads a field into memory that is wiped when dropped.
fn secret_value(input: &Entity<InputState>, cx: &App) -> Zeroizing<String> {
    Zeroizing::new(input.read(cx).value().to_string())
}

impl AccountPage {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        let observation = cx.observe(&session, |_, _, cx| cx.notify());
        Self {
            session,
            open_form: None,
            name_input: None,
            password_fields: None,
            form_subscriptions: Vec::new(),
            is_busy: false,
            error_message: None,
            notice: None,
            name_check: NameAvailabilityCheck::default(),
            name_check_wait: None,
            edit_name_focus: cx.focus_handle(),
            change_password_focus: cx.focus_handle(),
            friend_code_copy: cx.new(|_| FriendCodeCopyState::default()),
            focus_on_render: None,
            _session_observation: observation,
        }
    }

    fn current_name(&self, cx: &App) -> String {
        self.session
            .read(cx)
            .state()
            .current_user()
            .map(|user| user.display_name.clone())
            .unwrap_or_default()
    }

    fn new_input(
        is_masked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        cx.new(|cx| InputState::new(window, cx).masked(is_masked))
    }

    /// Enter in a field saves once Save is enabled.
    fn save_on_enter(
        &mut self,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subscription =
            cx.subscribe_in(
                input,
                window,
                |page, _, event: &InputEvent, _, cx| match event {
                    InputEvent::PressEnter { .. } => page.save(cx),
                    InputEvent::Change => cx.notify(),
                    _ => {}
                },
            );
        self.form_subscriptions.push(subscription);
    }

    fn open(&mut self, form: AccountForm, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_form();
        self.error_message = None;
        self.notice = None;
        self.open_form = Some(form);
        match form {
            AccountForm::DisplayName => {
                let input = cx.new(|cx| {
                    let mut input = InputState::new(window, cx)
                        .validate(|text, _| text.chars().count() <= MAXIMUM_ACCOUNT_NAME_LENGTH);
                    // Prefilled, as Discord does: the change is usually small.
                    input.set_value(self.current_name(cx), window, cx);
                    input
                });
                self.save_on_enter(&input, window, cx);
                let subscription = cx.subscribe(&input, |page, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        page.follow_name_draft(cx);
                    }
                });
                self.form_subscriptions.push(subscription);
                input.update(cx, |input, cx| input.focus(window, cx));
                self.name_input = Some(input);
            }
            AccountForm::Password => {
                let fields = PasswordFields {
                    current: Self::new_input(true, window, cx),
                    new: Self::new_input(true, window, cx),
                    confirmation: Self::new_input(true, window, cx),
                };
                for input in [&fields.current, &fields.new, &fields.confirmation] {
                    self.save_on_enter(input, window, cx);
                }
                fields
                    .current
                    .update(cx, |input, cx| input.focus(window, cx));
                self.password_fields = Some(fields);
            }
        }
        cx.notify();
    }

    /// Drops the fields, and with them every typed password.
    fn clear_form(&mut self) {
        self.name_input = None;
        self.password_fields = None;
        self.form_subscriptions.clear();
        self.name_check.cancel();
        self.name_check_wait = None;
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        let Some(closed) = self.open_form.take() else {
            return;
        };
        self.error_message = None;
        self.clear_form();
        self.focus_on_render = Some(self.opener_focus(closed).clone());
        cx.notify();
    }

    fn opener_focus(&self, form: AccountForm) -> &FocusHandle {
        match form {
            AccountForm::DisplayName => &self.edit_name_focus,
            AccountForm::Password => &self.change_password_focus,
        }
    }

    /// Esc closes the open form before the settings (true when there was one).
    pub fn close_open_form(&mut self, cx: &mut Context<Self>) -> bool {
        if self.open_form.is_none() {
            return false;
        }
        if !self.is_busy {
            self.close(cx);
        }
        true
    }

    fn follow_name_draft(&mut self, cx: &mut Context<Self>) {
        let Some(input) = &self.name_input else {
            return;
        };
        let draft = input.read(cx).value().to_string();
        let current = self.current_name(cx);
        self.name_check_wait = self.name_check.update(&draft, &current).map(|request| {
            cx.spawn(async move |page, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(NAME_CHECK_PAUSE_MILLISECONDS))
                    .await;
                let _ = page.update(cx, |page, cx| page.ask_name_availability(request, cx));
            })
        });
        cx.notify();
    }

    fn ask_name_availability(&mut self, request: NameCheckRequest, cx: &mut Context<Self>) {
        if !self.name_check.is_current(request.generation) {
            return;
        }
        let page = cx.entity().downgrade();
        let NameCheckRequest { generation, name } = request;
        self.session.update(cx, |session, cx| {
            session.send_account_request(
                "check name availability",
                move |rest| async move { rest.name_availability(&name).await },
                cx,
                move |answer, cx| {
                    let answer = match answer {
                        Ok(availability) if availability.is_available => NameCheckAnswer::Available,
                        Ok(_) => NameCheckAnswer::Taken,
                        Err(ClientError::Api {
                            code: ApiErrorCode::ValidationFailed,
                            ..
                        }) => NameCheckAnswer::Refused,
                        Err(_) => NameCheckAnswer::Failed,
                    };
                    let _ = page.update(cx, |page, cx| {
                        page.name_check.finish(generation, answer);
                        cx.notify();
                    });
                },
            );
        });
    }

    fn can_save(&self, cx: &App) -> bool {
        match self.open_form {
            Some(AccountForm::DisplayName) => self.name_input.as_ref().is_some_and(|input| {
                can_save_display_name(
                    &input.read(cx).value(),
                    &self.current_name(cx),
                    self.name_check.state(),
                )
            }),
            Some(AccountForm::Password) => self.password_fields.as_ref().is_some_and(|fields| {
                can_change_password(
                    &fields.current.read(cx).value(),
                    &fields.new.read(cx).value(),
                    &fields.confirmation.read(cx).value(),
                    MINIMUM_PASSWORD_LENGTH,
                )
            }),
            None => false,
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.is_busy || !self.can_save(cx) {
            return;
        }
        let Some(form) = self.open_form else {
            return;
        };
        self.is_busy = true;
        self.error_message = None;
        cx.notify();
        let page = cx.entity().downgrade();
        let on_done = move |outcome: Result<(), ClientError>, cx: &mut App| {
            let _ = page.update(cx, |page, cx| page.finish(form, outcome, cx));
        };
        match form {
            AccountForm::DisplayName => {
                let name = self
                    .name_input
                    .as_ref()
                    .map(|input| input.read(cx).value().trim().to_owned())
                    .unwrap_or_default();
                self.session.update(cx, |session, cx| {
                    session.send_current_user_change(
                        "rename account",
                        move |rest| async move {
                            rest.update_current_user(&UpdateCurrentUserRequest {
                                display_name: Some(name),
                                ..UpdateCurrentUserRequest::default()
                            })
                            .await
                        },
                        cx,
                        on_done,
                    );
                });
            }
            AccountForm::Password => {
                let Some(fields) = &self.password_fields else {
                    return;
                };
                let current_password = secret_value(&fields.current, cx);
                let new_password = secret_value(&fields.new, cx);
                self.session.update(cx, |session, cx| {
                    session.send_account_request(
                        "change password",
                        move |rest| async move {
                            let mut request = ChangePasswordRequest {
                                current_password: current_password.to_string(),
                                new_password: new_password.to_string(),
                            };
                            let answer = rest.change_password(&request).await;
                            request.current_password.zeroize();
                            request.new_password.zeroize();
                            answer
                        },
                        cx,
                        on_done,
                    );
                });
            }
        }
    }

    fn finish(
        &mut self,
        form: AccountForm,
        outcome: Result<(), ClientError>,
        cx: &mut Context<Self>,
    ) {
        self.is_busy = false;
        let words = strings(cx).account();
        match outcome {
            Ok(()) => {
                let text = match form {
                    AccountForm::DisplayName => words.display_name_saved(),
                    AccountForm::Password => words.password_changed(),
                };
                self.close(cx);
                self.notice = Some((form, text.into()));
            }
            Err(error) => {
                // A name taken meanwhile shows under the field, as the live check does.
                if matches!(
                    error,
                    ClientError::Api {
                        code: ApiErrorCode::UsernameTaken,
                        ..
                    }
                ) {
                    self.name_check.mark_taken();
                }
                let summary = match form {
                    AccountForm::DisplayName => words.display_name_failed(),
                    AccountForm::Password => words.password_failed(),
                };
                self.error_message = Some(describe_failure(&error, summary, cx).into());
            }
        }
        cx.notify();
    }

    fn form_actions(cx: &mut Context<Self>) -> (super::parts::Action, super::parts::Action) {
        let save_page = cx.entity();
        let cancel_page = cx.entity();
        (
            Arc::new(move |_, cx| save_page.update(cx, Self::save)),
            Arc::new(move |_, cx| cancel_page.update(cx, Self::close)),
        )
    }

    fn render_name_form(&self, input: &Entity<InputState>, cx: &mut Context<Self>) -> AnyElement {
        let words = strings(cx).account();
        let problem = match self.name_check.state() {
            NameAvailabilityState::Taken => Some(words.name_taken()),
            NameAvailabilityState::Invalid => Some(words.name_invalid()),
            _ => None,
        };
        let status = match self.name_check.state() {
            NameAvailabilityState::Checking => words.name_checking(),
            NameAvailabilityState::Available => words.name_available(),
            _ => "",
        };
        let mut field = TextField::new("account-display-name", words.display_name(), input)
            .helper(words.display_name_helper())
            .read_only(self.is_busy);
        if let Some(problem) = problem {
            field = field.error(problem);
        }
        let (on_save, on_cancel) = Self::form_actions(cx);
        edit_form(
            vec![
                field.into_any_element(),
                // Polite: "free" is read once the check answers; "taken" is the field's error.
                with_type(div(), TYPE_BODY_SM)
                    .id("account-name-status")
                    .live_polite(status)
                    .text_color(color(theme(cx).color_text_secondary))
                    .child(status)
                    .into_any_element(),
            ],
            error_line("account-name-error", self.error_message.clone(), cx),
            FormButtons {
                id: "account-name",
                save_label: strings(cx).common().save().into(),
                busy_label: strings(cx).common().saving().into(),
                is_save_enabled: self.can_save(cx),
                is_busy: self.is_busy,
            },
            on_save,
            on_cancel,
            cx,
        )
        .into_any_element()
    }

    fn render_password_form(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let fields = self.password_fields.as_ref()?;
        let words = strings(cx).account();
        let confirmation = fields.confirmation.read(cx).value();
        let differs = !confirmation.is_empty() && confirmation != fields.new.read(cx).value();
        let length_helper = strings(cx)
            .sign_in()
            .password_length_helper(i64::try_from(MINIMUM_PASSWORD_LENGTH).unwrap_or(i64::MAX));
        let mut confirmation_field = password_field(
            "account-confirm-password",
            words.confirm_new_password(),
            &fields.confirmation,
            self.is_busy,
            cx,
        );
        // Says why Save stays disabled while the two differ.
        if differs {
            confirmation_field = confirmation_field.helper(words.passwords_differ());
        }
        let (on_save, on_cancel) = Self::form_actions(cx);
        Some(
            card(cx)
                .child(edit_form(
                    vec![
                        password_field(
                            "account-current-password",
                            words.current_password(),
                            &fields.current,
                            self.is_busy,
                            cx,
                        )
                        .into_any_element(),
                        password_field(
                            "account-new-password",
                            words.new_password(),
                            &fields.new,
                            self.is_busy,
                            cx,
                        )
                        .helper(length_helper)
                        .into_any_element(),
                        confirmation_field.into_any_element(),
                    ],
                    error_line("account-password-error", self.error_message.clone(), cx),
                    FormButtons {
                        id: "account-password",
                        save_label: words.change_password().into(),
                        busy_label: words.changing_password().into(),
                        is_save_enabled: self.can_save(cx),
                        is_busy: self.is_busy,
                    },
                    on_save,
                    on_cancel,
                    cx,
                ))
                .into_any_element(),
        )
    }

    fn render_password_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let words = strings(cx).account();
        if self.open_form == Some(AccountForm::Password) {
            self.render_password_form(cx)
                .unwrap_or_else(|| div().into_any_element())
        } else {
            let page = cx.entity();
            div()
                .flex()
                .flex_col()
                .items_start()
                .child(
                    Button::new("account-change-password", words.change_password())
                        .disabled(self.is_busy)
                        .track_focus(&self.change_password_focus)
                        .on_click(move |window, cx| {
                            page.update(cx, |page, cx| {
                                page.open(AccountForm::Password, window, cx);
                            });
                        }),
                )
                .child(saved_notice(
                    "account-password-notice",
                    self.notice_of(AccountForm::Password),
                    cx,
                ))
                .into_any_element()
        }
    }

    fn notice_of(&self, form: AccountForm) -> Option<SharedString> {
        self.notice
            .as_ref()
            .filter(|(notice_form, _)| *notice_form == form)
            .map(|(_, text)| text.clone())
    }
}

/// A label over its value, read together.
fn field_value(id: &'static str, label: &'static str, value: String, cx: &App) -> impl IntoElement {
    let tokens = theme(cx);
    div()
        .id(id)
        .aria_label(SharedString::from(format!("{label}, {value}")))
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(length(COMPONENT_SETTINGS_ACCOUNT_VALUE_GAP))
        .child(
            with_type(div(), TYPE_CATEGORY)
                .text_color(color(tokens.component_settings_section_label_text))
                .child(label),
        )
        .child(
            with_type(div(), TYPE_BODY_LG)
                .text_color(color(tokens.component_settings_account_value_text))
                .child(value),
        )
}

impl Render for AccountPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(focus) = self.focus_on_render.take() {
            window.focus(&focus, cx);
        }
        let catalogue = strings(cx);
        let words = catalogue.account();
        let user = self.session.read(cx).state().current_user().cloned();
        let display_name = user
            .as_ref()
            .map(|user| user.display_name.clone())
            .unwrap_or_default();
        let email = user
            .as_ref()
            .map(|user| user.email.clone())
            .unwrap_or_default();
        let friend_code = user.and_then(|user| user.friend_code);

        let name_input = self
            .name_input
            .clone()
            .filter(|_| self.open_form == Some(AccountForm::DisplayName));
        let name_row = if let Some(input) = name_input {
            self.render_name_form(&input, cx)
        } else {
            let page = cx.entity();
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(length(COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP))
                        .child(field_value(
                            "account-display-name-value",
                            words.display_name(),
                            display_name,
                            cx,
                        ))
                        .child(
                            Button::new("account-edit-name", catalogue.common().edit())
                                .accessible_label(words.edit_display_name())
                                .disabled(self.is_busy)
                                .track_focus(&self.edit_name_focus)
                                .on_click(move |window, cx| {
                                    page.update(cx, |page, cx| {
                                        page.open(AccountForm::DisplayName, window, cx);
                                    });
                                }),
                        ),
                )
                .child(saved_notice(
                    "account-name-notice",
                    self.notice_of(AccountForm::DisplayName),
                    cx,
                ))
                .into_any_element()
        };

        let friend_code_row = friend_code.map(|code| {
            FriendCode::new("account-friend-code", code, self.friend_code_copy.clone())
        });

        let password_section = self.render_password_section(cx);

        div()
            .flex()
            .flex_col()
            .child(
                card(cx)
                    .gap(length(COMPONENT_SETTINGS_ACCOUNT_ROW_GAP))
                    .child(name_row)
                    .children(friend_code_row)
                    .child(field_value("account-email", words.email(), email, cx)),
            )
            .child(
                div()
                    .mt(length(COMPONENT_SETTINGS_SECTION_GAP))
                    .child(section_label(
                        "account-password-heading",
                        words.password(),
                        cx,
                    )),
            )
            .child(
                div()
                    .mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                    .child(password_section),
            )
    }
}
