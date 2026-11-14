//! Security (`SecuritySettings.svelte`, ADR-0049): two-factor sign-in in the account card's look.
//! Off, a short why and one button; turning it on asks for the password, then shows the QR code
//! (drawn here, the secret never leaves the app) and the code that confirms it, then the recovery
//! codes, once. On, the codes left, new codes and Turn off, each behind the password and a code.
//! Typed passwords and codes live in the fields only, dropped when their form closes. The page's
//! own copies of the secret, the recovery codes and the requests are wiped when it lets them go;
//! the copies drawn on screen and the fields' text are freed, not wiped (GPUI owns them).

use std::sync::Arc;

use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClipboardItem, Context, Entity, EventEmitter,
    FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, canvas, div,
    point, size,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::ClientError;
use mellow_client_core::shared_logic::qr_code::{QR_QUIET_ZONE_MODULES, QrMatrix, encode_qr_code};
use mellow_client_core::shared_logic::user_settings::{
    grouped_secret, is_six_digit_code, is_wrong_code_recent, otpauth_uri, recovery_codes_file_text,
};
use mellow_protocol::{
    EnableTwoFactorRequest, ProveTwoFactorRequest, StartTwoFactorEnrolmentRequest, TwoFactorStatus,
};
use zeroize::{Zeroize as _, Zeroizing};

use super::parts::{
    Action, FormButtons, body_text, card, describe_failure, edit_form, error_line, password_field,
    section_label,
};
use crate::components::{Button, ButtonVariant, TextField, icon, with_type};
use crate::i18n::strings;
use crate::messages::{local_date_time, message_timestamp_label};
use crate::session::{Session, now_unix_milliseconds};
use crate::settings::{SettingsPage, SettingsPageEvent, saved_notice};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP, COMPONENT_SETTINGS_ACTION_GAP,
    COMPONENT_SETTINGS_ERROR_ICON_GAP, COMPONENT_SETTINGS_ERROR_ICON_SIZE,
    COMPONENT_SETTINGS_SECTION_LABEL_GAP, COMPONENT_SETTINGS_TWO_FACTOR_QR_RADIUS,
    COMPONENT_SETTINGS_TWO_FACTOR_QR_SIZE, SPACE_2XS, TYPE_BODY, TYPE_BODY_LG, TYPE_CODE,
    TYPE_HEADING_MD,
};
use crate::theme::{color, length, theme};

#[derive(Debug)]
enum StatusLoad {
    Loading,
    Failed,
    Loaded(TwoFactorStatus),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecurityForm {
    /// The password that starts an enrolment.
    Password,
    /// The QR code and the code confirming it.
    Scan,
    Disable,
    Regenerate,
}

/// What a request of this page may answer.
enum Answer {
    Started(Zeroizing<String>),
    RecoveryCodes {
        codes: Vec<String>,
        is_new_enrolment: bool,
    },
    TurnedOff,
}

pub struct SecurityPage {
    session: Entity<Session>,
    load: StatusLoad,
    open_form: Option<SecurityForm>,
    password_input: Option<Entity<InputState>>,
    code_input: Option<Entity<InputState>>,
    form_subscriptions: Vec<Subscription>,
    is_busy: bool,
    error_message: Option<SharedString>,
    notice: Option<SharedString>,
    secret: Option<Zeroizing<String>>,
    qr: Option<Arc<QrMatrix>>,
    recovery_codes: Option<Vec<String>>,
    copy_notice: Option<SharedString>,
    turn_on_focus: FocusHandle,
    regenerate_focus: FocusHandle,
    disable_focus: FocusHandle,
    codes_heading_focus: FocusHandle,
    focus_on_render: Option<FocusHandle>,
    is_scan_field_pending: bool,
}

impl EventEmitter<SettingsPageEvent> for SecurityPage {}

impl SettingsPage for SecurityPage {
    fn set_attention(&mut self, _: u32, _: &mut Context<Self>) {}
}

impl std::fmt::Debug for SecurityPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecurityPage")
            .field("load", &self.load)
            .field("open_form", &self.open_form)
            .finish_non_exhaustive()
    }
}

impl Drop for SecurityPage {
    fn drop(&mut self) {
        self.forget_recovery_codes();
    }
}

impl SecurityPage {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            session,
            load: StatusLoad::Loading,
            open_form: None,
            password_input: None,
            code_input: None,
            form_subscriptions: Vec::new(),
            is_busy: false,
            error_message: None,
            notice: None,
            secret: None,
            qr: None,
            recovery_codes: None,
            copy_notice: None,
            turn_on_focus: cx.focus_handle(),
            regenerate_focus: cx.focus_handle(),
            disable_focus: cx.focus_handle(),
            codes_heading_focus: cx.focus_handle(),
            focus_on_render: None,
            is_scan_field_pending: false,
        };
        page.load_status(cx);
        page
    }

    fn load_status(&mut self, cx: &mut Context<Self>) {
        self.load = StatusLoad::Loading;
        cx.notify();
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_account_request(
                "load two-factor status",
                |rest| async move { rest.two_factor_status().await },
                cx,
                move |answer, cx| {
                    let _ = page.update(cx, |page, cx| {
                        page.load = answer.map_or(StatusLoad::Failed, StatusLoad::Loaded);
                        cx.notify();
                    });
                },
            );
        });
    }

    fn forget_recovery_codes(&mut self) {
        if let Some(mut codes) = self.recovery_codes.take() {
            for code in &mut codes {
                code.zeroize();
            }
        }
    }

    /// Drops the fields, and with them every typed password and code.
    fn clear_fields(&mut self) {
        self.password_input = None;
        self.code_input = None;
        self.form_subscriptions.clear();
    }

    fn field(
        &mut self,
        is_masked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let input = cx.new(|cx| InputState::new(window, cx).masked(is_masked));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |page, _, event: &InputEvent, _, cx| match event {
                    InputEvent::PressEnter { .. } => page.submit(cx),
                    InputEvent::Change => cx.notify(),
                    _ => {}
                },
            );
        self.form_subscriptions.push(subscription);
        input
    }

    fn open(&mut self, form: SecurityForm, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_fields();
        self.error_message = None;
        self.notice = None;
        self.open_form = Some(form);
        let first = match form {
            SecurityForm::Password => {
                let password = self.field(true, window, cx);
                self.password_input = Some(password.clone());
                password
            }
            SecurityForm::Scan => {
                let code = self.field(false, window, cx);
                self.code_input = Some(code.clone());
                code
            }
            SecurityForm::Disable | SecurityForm::Regenerate => {
                let password = self.field(true, window, cx);
                self.password_input = Some(password.clone());
                self.code_input = Some(self.field(false, window, cx));
                password
            }
        };
        first.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        let closed = self.open_form.take();
        self.error_message = None;
        self.secret = None;
        self.qr = None;
        self.copy_notice = None;
        self.clear_fields();
        self.focus_on_render = Some(match closed {
            Some(SecurityForm::Disable) => self.disable_focus.clone(),
            Some(SecurityForm::Regenerate) => self.regenerate_focus.clone(),
            _ => self.turn_on_focus.clone(),
        });
        cx.notify();
    }

    /// Esc closes the open form before the settings, and never skips the recovery codes.
    pub fn close_open_form(&mut self, cx: &mut Context<Self>) -> bool {
        if self.recovery_codes.is_some() {
            return true;
        }
        if self.open_form.is_none() {
            return false;
        }
        if !self.is_busy {
            self.close(cx);
        }
        true
    }

    fn value(input: Option<&Entity<InputState>>, cx: &App) -> Zeroizing<String> {
        Zeroizing::new(
            input
                .map(|input| input.read(cx).value().to_string())
                .unwrap_or_default(),
        )
    }

    fn can_submit(&self, cx: &App) -> bool {
        let password = Self::value(self.password_input.as_ref(), cx);
        let code = Self::value(self.code_input.as_ref(), cx);
        match self.open_form {
            Some(SecurityForm::Password) => !password.is_empty(),
            Some(SecurityForm::Scan) => is_six_digit_code(&code),
            Some(SecurityForm::Disable | SecurityForm::Regenerate) => {
                !password.is_empty() && !code.trim().is_empty()
            }
            None => false,
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.is_busy || !self.can_submit(cx) {
            return;
        }
        let Some(form) = self.open_form else {
            return;
        };
        let password = Self::value(self.password_input.as_ref(), cx);
        // Spaces from a paste are dropped, as the web's field does.
        let code = Zeroizing::new(Self::value(self.code_input.as_ref(), cx).replace(' ', ""));
        self.is_busy = true;
        self.error_message = None;
        cx.notify();
        let words = strings(cx).security();
        let (action, summary): (&'static str, &'static str) = match form {
            SecurityForm::Password => ("start two-factor enrolment", words.start_failed()),
            SecurityForm::Scan => ("enable two-factor sign-in", words.enable_failed()),
            SecurityForm::Disable => ("disable two-factor sign-in", words.disable_failed()),
            SecurityForm::Regenerate => ("regenerate recovery codes", words.regenerate_failed()),
        };
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_account_request(
                action,
                move |rest| async move {
                    match form {
                        SecurityForm::Password => {
                            let mut request = StartTwoFactorEnrolmentRequest {
                                current_password: password.to_string(),
                            };
                            let answer = rest.start_two_factor_enrolment(&request).await;
                            request.current_password.zeroize();
                            answer
                                .map(|enrolment| Answer::Started(Zeroizing::new(enrolment.secret)))
                        }
                        SecurityForm::Scan => {
                            let mut request = EnableTwoFactorRequest {
                                code: code.to_string(),
                            };
                            let answer = rest.enable_two_factor(&request).await;
                            request.code.zeroize();
                            answer.map(|codes| Answer::RecoveryCodes {
                                codes: codes.recovery_codes,
                                is_new_enrolment: true,
                            })
                        }
                        SecurityForm::Disable | SecurityForm::Regenerate => {
                            let mut request = ProveTwoFactorRequest {
                                current_password: password.to_string(),
                                code: code.to_string(),
                            };
                            let answer = if form == SecurityForm::Disable {
                                rest.disable_two_factor(&request)
                                    .await
                                    .map(|()| Answer::TurnedOff)
                            } else {
                                rest.regenerate_recovery_codes(&request).await.map(|codes| {
                                    Answer::RecoveryCodes {
                                        codes: codes.recovery_codes,
                                        is_new_enrolment: false,
                                    }
                                })
                            };
                            request.current_password.zeroize();
                            request.code.zeroize();
                            answer
                        }
                    }
                },
                cx,
                move |answer, cx| {
                    let _ = page.update(cx, |page, cx| page.finish(answer, summary, cx));
                },
            );
        });
    }

    fn finish(
        &mut self,
        answer: Result<Answer, ClientError>,
        summary: &str,
        cx: &mut Context<Self>,
    ) {
        self.is_busy = false;
        let words = strings(cx).security();
        match answer {
            Err(error) => {
                self.error_message = Some(describe_failure(&error, summary, cx).into());
            }
            Ok(Answer::Started(secret)) => {
                let (username, host) = {
                    let session = self.session.read(cx);
                    (
                        session
                            .state()
                            .current_user()
                            .map(|user| user.username.clone())
                            .unwrap_or_default(),
                        session
                            .rest_client()
                            .map(|rest| rest.instance().host().to_owned())
                            .unwrap_or_default(),
                    )
                };
                let uri = Zeroizing::new(otpauth_uri(&secret, &username, &host));
                self.qr = encode_qr_code(&uri).map(Arc::new);
                self.secret = Some(secret);
                self.password_input = None;
                self.form_subscriptions.clear();
                self.open_form = Some(SecurityForm::Scan);
                // Its field needs the window: made and focused when next drawn.
                self.is_scan_field_pending = true;
            }
            Ok(Answer::RecoveryCodes {
                codes,
                is_new_enrolment,
            }) => {
                let remaining = u32::try_from(codes.len()).unwrap_or(u32::MAX);
                let last_failed = match &self.load {
                    StatusLoad::Loaded(status) if !is_new_enrolment => {
                        status.last_failed_code_at_unix_milliseconds
                    }
                    _ => None,
                };
                self.load = StatusLoad::Loaded(TwoFactorStatus {
                    is_enabled: true,
                    recovery_codes_remaining: remaining,
                    last_failed_code_at_unix_milliseconds: last_failed,
                });
                self.notice = Some(
                    if is_new_enrolment {
                        words.turned_on()
                    } else {
                        words.new_codes_ready()
                    }
                    .into(),
                );
                self.open_form = None;
                self.secret = None;
                self.qr = None;
                self.clear_fields();
                self.forget_recovery_codes();
                self.recovery_codes = Some(codes);
                self.focus_on_render = Some(self.codes_heading_focus.clone());
            }
            Ok(Answer::TurnedOff) => {
                self.load = StatusLoad::Loaded(TwoFactorStatus {
                    is_enabled: false,
                    recovery_codes_remaining: 0,
                    last_failed_code_at_unix_milliseconds: None,
                });
                self.notice = Some(words.turned_off().into());
                self.open_form = None;
                self.clear_fields();
                self.focus_on_render = Some(self.turn_on_focus.clone());
            }
        }
        cx.notify();
    }

    fn copy(&mut self, text: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
        self.copy_notice = Some(strings(cx).security().copied().into());
        cx.notify();
    }

    /// Asks where to keep the codes, then writes them off the UI thread.
    fn save_codes_file(&mut self, cx: &mut Context<Self>) {
        let Some(codes) = &self.recovery_codes else {
            return;
        };
        let words = strings(cx).security();
        let (username, host) = {
            let session = self.session.read(cx);
            (
                session
                    .state()
                    .current_user()
                    .map(|user| user.username.clone())
                    .unwrap_or_default(),
                session
                    .rest_client()
                    .map(|rest| rest.instance().host().to_owned())
                    .unwrap_or_default(),
            )
        };
        let text = Zeroizing::new(recovery_codes_file_text(
            &words.codes_file_heading(&username, &host),
            codes,
        ));
        let directory = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        let picked = cx.prompt_for_new_path(&directory, Some(words.codes_file_name()));
        cx.spawn(async move |page, cx| {
            let Ok(Ok(Some(path))) = picked.await else {
                return;
            };
            let written = cx
                .background_spawn(async move { write_private_file(&path, text.as_bytes()) })
                .await;
            let _ = page.update(cx, |page, cx| {
                let desktop = strings(cx).desktop();
                page.copy_notice = Some(
                    match written {
                        Ok(()) => desktop.codes_saved(),
                        Err(error) => {
                            tracing::warn!(error = %error.kind(), "cannot save the recovery codes");
                            desktop.codes_save_failed()
                        }
                    }
                    .into(),
                );
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_with_codes(&mut self, cx: &mut Context<Self>) {
        self.forget_recovery_codes();
        self.copy_notice = None;
        self.focus_on_render = Some(self.regenerate_focus.clone());
        cx.notify();
    }

    fn action(
        act: fn(&mut Self, &mut Window, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> Action {
        let page = cx.entity();
        Arc::new(move |window, cx| page.update(cx, |page, cx| act(page, window, cx)))
    }

    fn opener(
        id: &'static str,
        label: &'static str,
        variant: ButtonVariant,
        focus: &FocusHandle,
        form: SecurityForm,
        cx: &mut Context<Self>,
    ) -> Button {
        let page = cx.entity();
        Button::new(id, label)
            .variant(variant)
            .track_focus(focus)
            .on_click(move |window, cx| page.update(cx, |page, cx| page.open(form, window, cx)))
    }

    fn form_buttons(
        &self,
        id: &'static str,
        save_label: &'static str,
        busy_label: &'static str,
        cx: &App,
    ) -> FormButtons {
        FormButtons {
            id,
            save_label: save_label.into(),
            busy_label: busy_label.into(),
            is_save_enabled: self.can_submit(cx),
            is_busy: self.is_busy,
        }
    }

    fn code_field(&self, input: &Entity<InputState>, cx: &App) -> TextField {
        let words = strings(cx).security();
        let field = TextField::new("security-code", words.code_from_app(), input)
            .monospace()
            .read_only(self.is_busy);
        if self.open_form == Some(SecurityForm::Scan) {
            field
        } else {
            field.helper(words.code_helper())
        }
    }

    fn render_codes(&self, cx: &mut Context<Self>) -> AnyElement {
        let codes = self.recovery_codes.as_deref().unwrap_or_default();
        let tokens = theme(cx);
        let words = strings(cx).security();
        let page = cx.entity();
        let save_page = cx.entity();
        let done_page = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SETTINGS_ACTION_GAP))
            .child(
                with_type(div(), TYPE_HEADING_MD)
                    .id("security-codes-title")
                    .role(Role::Heading)
                    .aria_label(words.codes_title())
                    .track_focus(&self.codes_heading_focus)
                    .text_color(color(tokens.component_settings_page_title_text))
                    .child(words.codes_title()),
            )
            .child(body_text(words.codes_intro(), cx))
            .child(
                with_type(div(), TYPE_CODE)
                    .id("security-codes")
                    .role(Role::List)
                    .aria_label(words.codes_list_label())
                    .flex()
                    .flex_wrap()
                    .gap(length(SPACE_2XS))
                    .text_color(color(tokens.component_settings_two_factor_code_text))
                    .children(codes.iter().enumerate().map(|(index, code)| {
                        let code = SharedString::from(code.clone());
                        div()
                            .id(("security-code", index))
                            .role(Role::ListItem)
                            .aria_label(code.clone())
                            .min_w(length(COMPONENT_SETTINGS_TWO_FACTOR_QR_SIZE))
                            .child(code)
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(length(COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP))
                    .child(
                        Button::new("security-download", words.download())
                            .on_click(move |_, cx| save_page.update(cx, Self::save_codes_file)),
                    )
                    .child(Button::new("security-copy-all", words.copy_all()).on_click(
                        move |_, cx| {
                            page.update(cx, |page, cx| {
                                // Joined on the click only, so no copy outlives it unwiped.
                                let all = Zeroizing::new(
                                    page.recovery_codes
                                        .as_deref()
                                        .unwrap_or_default()
                                        .join("\n"),
                                );
                                page.copy(&all, cx);
                            });
                        },
                    ))
                    .child(
                        Button::new("security-saved-them", words.saved_them())
                            .variant(ButtonVariant::Primary)
                            .on_click(move |_, cx| done_page.update(cx, Self::finish_with_codes)),
                    ),
            )
            .child(saved_notice(
                "security-copy-notice",
                self.copy_notice.clone(),
                cx,
            ))
            .into_any_element()
    }

    fn render_scan(&self, cx: &mut Context<Self>) -> AnyElement {
        let words = strings(cx).security();
        let secret = self
            .secret
            .as_ref()
            .map(|secret| grouped_secret(secret))
            .unwrap_or_default();
        let raw_secret = Zeroizing::new(
            self.secret
                .as_ref()
                .map(|secret| secret.to_string())
                .unwrap_or_default(),
        );
        let tokens = theme(cx);
        let page = cx.entity();
        let fields = self
            .code_input
            .as_ref()
            .map(|input| vec![self.code_field(input, cx).into_any_element()])
            .unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SETTINGS_ACTION_GAP))
            .child(
                with_type(div(), TYPE_HEADING_MD)
                    .text_color(color(tokens.component_settings_page_title_text))
                    .child(words.scan_title()),
            )
            .child(body_text(words.scan_step(), cx))
            .children(
                self.qr
                    .clone()
                    .map(|matrix| qr_code(matrix, words.qr_code_label(), cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                    .child(section_label(
                        "security-manual-key-label",
                        words.manual_key_label(),
                        cx,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(length(COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP))
                            .child(
                                with_type(div(), TYPE_CODE)
                                    .id("security-manual-key")
                                    .aria_label(SharedString::from(format!(
                                        "{}, {secret}",
                                        words.manual_key_label()
                                    )))
                                    .text_color(color(
                                        tokens.component_settings_two_factor_code_text,
                                    ))
                                    .child(secret),
                            )
                            .child(Button::new("security-copy-key", words.copy_key()).on_click(
                                move |_, cx| page.update(cx, |page, cx| page.copy(&raw_secret, cx)),
                            )),
                    )
                    .child(saved_notice(
                        "security-key-notice",
                        self.copy_notice.clone(),
                        cx,
                    )),
            )
            .child(body_text(words.confirm_step(), cx))
            .child(edit_form(
                fields,
                error_line("security-scan-error", self.error_message.clone(), cx),
                self.form_buttons(
                    "security-scan",
                    words.turn_on_button(),
                    words.turning_on(),
                    cx,
                ),
                Self::action(|page, _, cx| page.submit(cx), cx),
                Self::action(|page, _, cx| page.close(cx), cx),
                cx,
            ))
            .into_any_element()
    }

    fn render_status(&self, status: &TwoFactorStatus, cx: &mut Context<Self>) -> AnyElement {
        let tokens = theme(cx);
        let words = strings(cx).security();
        if !status.is_enabled {
            return div()
                .flex()
                .flex_col()
                .gap(length(COMPONENT_SETTINGS_ACTION_GAP))
                .child(body_text(words.off_intro(), cx))
                .child(div().child(Self::opener(
                    "security-turn-on",
                    words.turn_on(),
                    ButtonVariant::Primary,
                    &self.turn_on_focus,
                    SecurityForm::Password,
                    cx,
                )))
                .into_any_element();
        }
        let has_no_codes = status.recovery_codes_remaining == 0;
        let error_ink = color(tokens.component_settings_error_text);
        let wrong_code = status
            .last_failed_code_at_unix_milliseconds
            .filter(|failed_at| is_wrong_code_recent(Some(*failed_at), now_unix_milliseconds()))
            .map(|failed_at| {
                let when = message_timestamp_label(
                    local_date_time(failed_at),
                    local_date_time(now_unix_milliseconds()),
                    strings(cx),
                );
                words.wrong_code_typed(&when)
            });
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SETTINGS_ACTION_GAP))
            .child(
                with_type(div(), TYPE_BODY_LG)
                    .flex()
                    .gap(length(COMPONENT_SETTINGS_ERROR_ICON_GAP))
                    .text_color(color(tokens.component_settings_body_text))
                    .child(icon(
                        IconName::Lock,
                        COMPONENT_SETTINGS_ERROR_ICON_SIZE,
                        color(tokens.component_settings_notice_text),
                    ))
                    .child(words.on_status()),
            )
            .child(
                with_type(div(), TYPE_BODY)
                    .text_color(if has_no_codes {
                        error_ink
                    } else {
                        color(tokens.component_settings_body_text)
                    })
                    .child(if has_no_codes {
                        words.no_recovery_codes_left().to_owned()
                    } else {
                        words.recovery_codes_left(i64::from(status.recovery_codes_remaining))
                    }),
            )
            .children(wrong_code.map(|text| {
                with_type(div(), TYPE_BODY)
                    .flex()
                    .gap(length(COMPONENT_SETTINGS_ERROR_ICON_GAP))
                    .text_color(error_ink)
                    .child(icon(
                        IconName::WarningCircle,
                        COMPONENT_SETTINGS_ERROR_ICON_SIZE,
                        color(tokens.component_settings_error_icon),
                    ))
                    .child(text)
            }))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(length(COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP))
                    .child(Self::opener(
                        "security-regenerate",
                        words.new_recovery_codes(),
                        ButtonVariant::Secondary,
                        &self.regenerate_focus,
                        SecurityForm::Regenerate,
                        cx,
                    ))
                    .child(Self::opener(
                        "security-turn-off",
                        words.turn_off(),
                        ButtonVariant::Danger,
                        &self.disable_focus,
                        SecurityForm::Disable,
                        cx,
                    )),
            )
            .into_any_element()
    }

    fn render_card_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let words = strings(cx).security();
        if self.recovery_codes.is_some() {
            return self.render_codes(cx);
        }
        let status = match &self.load {
            StatusLoad::Failed => {
                let page = cx.entity();
                return div()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_SETTINGS_ACTION_GAP))
                    .child(error_line(
                        "security-load-error",
                        Some(words.load_failed().into()),
                        cx,
                    ))
                    .child(
                        div().child(
                            Button::new("security-try-again", words.try_again())
                                .on_click(move |_, cx| page.update(cx, Self::load_status)),
                        ),
                    )
                    .into_any_element();
            }
            StatusLoad::Loading => {
                return body_text(words.loading(), cx).into_any_element();
            }
            StatusLoad::Loaded(status) => *status,
        };
        match self.open_form {
            Some(SecurityForm::Scan) => self.render_scan(cx),
            Some(form) => {
                let account = strings(cx).account();
                let mut fields = Vec::new();
                if let Some(input) = &self.password_input {
                    let mut field = password_field(
                        "security-password",
                        account.current_password(),
                        input,
                        self.is_busy,
                        cx,
                    );
                    let helper = match form {
                        SecurityForm::Password => Some(words.password_step_helper()),
                        SecurityForm::Regenerate => Some(words.new_recovery_codes_helper()),
                        _ => None,
                    };
                    if let Some(helper) = helper {
                        field = field.helper(helper);
                    }
                    fields.push(field.into_any_element());
                }
                if let Some(input) = &self.code_input {
                    fields.push(self.code_field(input, cx).into_any_element());
                }
                let buttons = match form {
                    SecurityForm::Password => self.form_buttons(
                        "security-form",
                        words.r#continue(),
                        words.continuing(),
                        cx,
                    ),
                    SecurityForm::Disable => self.form_buttons(
                        "security-form",
                        words.turn_off(),
                        words.turning_off(),
                        cx,
                    ),
                    _ => self.form_buttons(
                        "security-form",
                        words.new_recovery_codes(),
                        words.getting_recovery_codes(),
                        cx,
                    ),
                };
                edit_form(
                    fields,
                    error_line("security-form-error", self.error_message.clone(), cx),
                    buttons,
                    Self::action(|page, _, cx| page.submit(cx), cx),
                    Self::action(|page, _, cx| page.close(cx), cx),
                    cx,
                )
                .into_any_element()
            }
            None => self.render_status(&status, cx),
        }
    }
}

/// Writes the recovery codes readable by their owner only: on Linux the suggested folder is often
/// open to the machine's other accounts. Elsewhere the folder's own permissions apply.
fn write_private_file(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(contents)
}

/// The QR code, one square per dark module inside the quiet zone, on the light plate scanners
/// need whatever the theme.
fn qr_code(matrix: Arc<QrMatrix>, label: &'static str, cx: &App) -> impl IntoElement {
    let tokens = theme(cx);
    let module_ink = color(tokens.component_settings_two_factor_qr_module);
    div()
        .id("security-qr")
        .role(Role::Image)
        .aria_label(label)
        .size(length(COMPONENT_SETTINGS_TWO_FACTOR_QR_SIZE))
        .rounded(length(COMPONENT_SETTINGS_TWO_FACTOR_QR_RADIUS))
        .bg(color(tokens.component_settings_two_factor_qr_background))
        .overflow_hidden()
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, (), window, _| {
                    let modules = matrix.len() + 2 * QR_QUIET_ZONE_MODULES;
                    #[allow(clippy::cast_precision_loss)] // Under 200 modules.
                    let unit = bounds.size.width / modules as f32;
                    for (y, row) in matrix.iter().enumerate() {
                        for (x, is_dark) in row.iter().enumerate() {
                            if !is_dark {
                                continue;
                            }
                            #[allow(clippy::cast_precision_loss)] // Under 200 modules.
                            let origin = point(
                                bounds.origin.x + unit * (x + QR_QUIET_ZONE_MODULES) as f32,
                                bounds.origin.y + unit * (y + QR_QUIET_ZONE_MODULES) as f32,
                            );
                            window.paint_quad(gpui::fill(
                                Bounds::new(origin, size(unit, unit)),
                                module_ink,
                            ));
                        }
                    }
                },
            )
            .size_full(),
        )
}

impl Render for SecurityPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.is_scan_field_pending {
            self.is_scan_field_pending = false;
            let code = self.field(false, window, cx);
            code.update(cx, |input, cx| input.focus(window, cx));
            self.code_input = Some(code);
        }
        if let Some(focus) = self.focus_on_render.take() {
            window.focus(&focus, cx);
        }
        let words = strings(cx).security();
        let content = self.render_card_content(cx);
        div()
            .flex()
            .flex_col()
            .child(div().child(section_label(
                "security-two-factor-heading",
                words.two_factor_heading(),
                cx,
            )))
            .child(
                card(cx)
                    .mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                    .gap(length(COMPONENT_SETTINGS_ACTION_GAP))
                    .child(content),
            )
            .child(saved_notice("security-notice", self.notice.clone(), cx))
    }
}

// The one test reads Unix permission bits; Windows has none to check.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_recovery_codes_file_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory =
            std::env::temp_dir().join(format!("mellow-codes-file-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("a temporary folder");
        let path = directory.join("codes.txt");
        write_private_file(&path, b"AAAA-BBBB").expect("the file is written");
        let mode = std::fs::metadata(&path)
            .expect("the file exists")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read(&path).expect("readable back"), b"AAAA-BBBB");
        std::fs::remove_dir_all(&directory).expect("the folder is removed");
    }
}
