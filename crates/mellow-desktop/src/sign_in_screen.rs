//! The authentication card (DESIGN.md §5.1): the instance address the desktop needs first, then
//! sign in or create an account, then the two-factor step (ADR-0049), as the web's
//! `SignInScreen.svelte` does. Fields are checked before sending and errors are said in words.
//!
//! shortcut: Melli does not stand on the card's edge yet, the instance check has no "Cancel" and
//! the password has no "Show password" toggle; they come with the shared primitives.

use crate::components::LiveRegion as _;
use gpui::{
    AppContext as _, Context, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _,
};
use gpui_base::input::{InputAccessibility, InputEvent, InputState};
use mellow_client_core::{ClientError, InstanceAddress, RestClient, SignIn};
use mellow_protocol::{
    ApiErrorCode, CompleteTwoFactorSignInRequest, LoginRequest, RegisterRequest,
};
use zeroize::{Zeroize as _, Zeroizing};

use crate::local_widgets::{TextField, error_line, link_button, primary_button, text_style};
use crate::preferences::{Preferences, change_preferences};
use crate::session::Session;
use crate::sign_in_form::{
    FieldToCheck, MAXIMUM_ACCOUNT_NAME_LENGTH, MINIMUM_PASSWORD_LENGTH, SignInField,
    describe_error, field_errors_before_sending, is_valid_email_address, judge_instance_health,
};
use crate::strings_pending_i18n::{Strings, strings as active_strings};
use crate::theme::tokens::{
    COMPONENT_AUTH_CARD_ERROR_ICON_SIZE, COMPONENT_AUTH_CARD_MARGIN, COMPONENT_AUTH_CARD_MAX_WIDTH,
    COMPONENT_AUTH_CARD_PADDING, COMPONENT_AUTH_CARD_RADIUS, COMPONENT_AUTH_CARD_SECTION_GAP,
    COMPONENT_AUTH_CARD_SUBTITLE_GAP, COMPONENT_TEXT_FIELD_FIELD_GAP, SPACE_2XS, SPACE_XS,
    TYPE_BODY, TYPE_BODY_LG, TYPE_HEADING_XL,
};
use crate::theme::{SANS_FONT_FAMILY, color, length, theme};
use crate::tokio_bridge::spawn_on_network_runtime;

/// Which card the screen shows.
#[derive(Debug)]
enum Step {
    InstanceAddress,
    Credentials {
        instance: InstanceAddress,
        is_registering: bool,
    },
    /// The password was right; the challenge's token finishes the sign-in with a code.
    TwoFactor {
        instance: InstanceAddress,
        challenge_token: Zeroizing<String>,
        is_using_recovery_code: bool,
    },
}

/// How a sign-in or registration request ended.
enum CredentialsAnswer {
    SignedIn(RestClient),
    TwoFactorRequired(Zeroizing<String>),
}

pub struct SignInScreen {
    session: Entity<Session>,
    step: Step,
    instance_address: Entity<InputState>,
    login: Entity<InputState>,
    password: Entity<InputState>,
    email: Entity<InputState>,
    username: Entity<InputState>,
    new_password: Entity<InputState>,
    code: Entity<InputState>,
    is_busy: bool,
    /// The address being probed, named on the busy button.
    instance_being_checked: Option<InstanceAddress>,
    /// The form's error, under the fields; `None` hides it.
    error_message: Option<String>,
    /// Mellow's own words per field, in the form's order.
    field_errors: Vec<(SignInField, &'static str)>,
    /// The instance ended the last session by itself: the sign-in card says so.
    has_session_ended_by_itself: bool,
    /// The request under way; replacing or dropping it cancels it, so a late answer never lands
    /// on a later step.
    request: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for SignInScreen {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SignInScreen")
            .field("step", &self.step)
            .field("is_busy", &self.is_busy)
            .finish_non_exhaustive()
    }
}

/// `chat.example.org`, or `127.0.0.1:8080`: what the web client shows as `location.host`.
fn instance_display_name(instance: &InstanceAddress) -> &str {
    let address = instance.as_str();
    address
        .strip_prefix("https://")
        .or_else(|| address.strip_prefix("http://"))
        .unwrap_or(address)
        .trim_end_matches('/')
}

/// The address as the instance field shows it: without `https://`, which the field adds back,
/// but with `http://`, which a bare host would lose (a loopback development instance).
fn instance_field_value(instance: &InstanceAddress) -> &str {
    let address = instance.as_str();
    address
        .strip_prefix("https://")
        .unwrap_or(address)
        .trim_end_matches('/')
}

fn device_name() -> String {
    format!("Mellow desktop ({})", std::env::consts::OS)
}

/// What the credentials card sends.
enum Credentials {
    Login {
        login: String,
        password: Zeroizing<String>,
    },
    Register {
        email: String,
        username: String,
        password: Zeroizing<String>,
    },
}

/// Signs in or registers; the request's own copy of the password is wiped once sent.
async fn send_credentials_to_instance(
    instance: InstanceAddress,
    credentials: Credentials,
) -> Result<CredentialsAnswer, ClientError> {
    let mut rest = RestClient::new(instance)?;
    match credentials {
        Credentials::Register {
            email,
            username,
            password,
        } => {
            let mut request = RegisterRequest {
                // The web sends the name as both, as ADR-0048 asks of current clients.
                display_name: Some(username.clone()),
                username,
                email,
                password: password.to_string(),
                device_name: device_name(),
            };
            let answer = rest.register(&request).await;
            request.password.zeroize();
            answer?;
            Ok(CredentialsAnswer::SignedIn(rest))
        }
        Credentials::Login { login, password } => {
            let mut request = LoginRequest {
                login,
                password: password.to_string(),
                device_name: device_name(),
            };
            let answer = rest.login(&request).await;
            request.password.zeroize();
            match answer? {
                SignIn::SignedIn(_) => Ok(CredentialsAnswer::SignedIn(rest)),
                SignIn::TwoFactorRequired(challenge) => Ok(CredentialsAnswer::TwoFactorRequired(
                    Zeroizing::new(challenge.two_factor_token),
                )),
            }
        }
    }
}

impl SignInScreen {
    /// Starts on the sign-in card of the last instance when there is one, otherwise on the
    /// instance address, filled with the last one the preferences remember.
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let strings = active_strings(cx);
        let new_input =
            |label: &'static str, is_masked: bool, window: &mut Window, cx: &mut Context<Self>| {
                cx.new(|cx| {
                    let mut input = InputState::new(window, cx).masked(is_masked);
                    input.set_accessibility(
                        InputAccessibility::new(Some(Role::TextInput))
                            .with_label(Some(label.into())),
                    );
                    input
                })
            };
        let instance_address = cx.new(|cx| {
            let mut input =
                InputState::new(window, cx).placeholder(strings.instance_address_placeholder);
            input.set_accessibility(
                InputAccessibility::new(Some(Role::TextInput))
                    .with_label(Some(strings.instance_address.into())),
            );
            input
        });
        let login = new_input(strings.login, false, window, cx);
        let password = new_input(strings.password, true, window, cx);
        let email = new_input(strings.email, false, window, cx);
        let username = new_input(strings.username, false, window, cx);
        // The web field's `maxlength`; the server checks the rest of the rules.
        username.update(cx, |input, cx| {
            input.set_validator(
                |text, _| text.chars().count() <= MAXIMUM_ACCOUNT_NAME_LENGTH,
                cx,
            );
        });
        let new_password = new_input(strings.password, true, window, cx);
        let code = new_input(strings.two_factor_code, false, window, cx);

        let subscriptions = [
            (&instance_address, SignInField::InstanceAddress),
            (&login, SignInField::Login),
            (&password, SignInField::Password),
            (&email, SignInField::Email),
            (&username, SignInField::Username),
            (&new_password, SignInField::NewPassword),
            (&code, SignInField::Code),
        ]
        .into_iter()
        .map(|(input, field)| {
            cx.subscribe_in(
                input,
                window,
                move |screen, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => screen.forget_field_error(field, cx),
                    InputEvent::PressEnter { .. } => screen.submit(window, cx),
                    InputEvent::Focus | InputEvent::Blur => cx.notify(),
                },
            )
        })
        .collect();

        let (last_instance, has_session_ended_by_itself) = {
            let session = session.read(cx);
            (
                session.last_instance().cloned(),
                session.has_ended_by_itself(),
            )
        };
        let step = match last_instance {
            Some(instance) => Step::Credentials {
                instance,
                is_registering: false,
            },
            None => Step::InstanceAddress,
        };
        let mut screen = Self {
            session,
            step,
            instance_address,
            login,
            password,
            email,
            username,
            new_password,
            code,
            is_busy: false,
            instance_being_checked: None,
            error_message: None,
            field_errors: Vec::new(),
            has_session_ended_by_itself,
            request: None,
            _subscriptions: subscriptions,
        };
        if matches!(screen.step, Step::InstanceAddress)
            && screen.session.read(cx).is_kept_on_this_device()
        {
            screen.fill_in_the_last_instance_address(window, cx);
        }
        screen.focus_first_field_to_fill(window, cx);
        screen
    }

    /// The preferences are read off the main thread; what the person typed meanwhile stays.
    fn fill_in_the_last_instance_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let load = cx.background_spawn(async { Preferences::load() });
        self.request = Some(cx.spawn_in(window, async move |screen, cx| {
            let preferences = load.await;
            let Some(instance) = preferences.last_instance_address else {
                return;
            };
            let _ = screen.update_in(cx, |screen, window, cx| {
                screen.request = None;
                let input = screen.instance_address.clone();
                if input.read(cx).value().is_empty() {
                    input.update(cx, |input, cx| {
                        input.set_value(instance_field_value(&instance).to_owned(), window, cx);
                    });
                }
            });
        }));
    }

    fn input(&self, field: SignInField) -> &Entity<InputState> {
        match field {
            SignInField::InstanceAddress => &self.instance_address,
            SignInField::Login => &self.login,
            SignInField::Password => &self.password,
            SignInField::Email => &self.email,
            SignInField::Username => &self.username,
            SignInField::NewPassword => &self.new_password,
            SignInField::Code => &self.code,
        }
    }

    /// The fields the current step shows, in order.
    fn visible_fields(&self) -> &'static [SignInField] {
        match self.step {
            Step::InstanceAddress => &[SignInField::InstanceAddress],
            Step::Credentials {
                is_registering: false,
                ..
            } => &[SignInField::Login, SignInField::Password],
            Step::Credentials {
                is_registering: true,
                ..
            } => &[
                SignInField::Email,
                SignInField::Username,
                SignInField::NewPassword,
            ],
            Step::TwoFactor { .. } => &[SignInField::Code],
        }
    }

    fn value(&self, field: SignInField, cx: &gpui::App) -> SharedString {
        self.input(field).read(cx).value()
    }

    fn set_value(
        &self,
        field: SignInField,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input(field).update(cx, |input, cx| {
            input.set_value(value.to_owned(), window, cx);
        });
    }

    fn focus(&self, field: SignInField, window: &mut Window, cx: &mut Context<Self>) {
        self.input(field)
            .update(cx, |input, cx| input.focus(window, cx));
    }

    fn focus_handle_of(&self, field: SignInField, cx: &Context<Self>) -> FocusHandle {
        self.input(field).read(cx).focus_handle(cx)
    }

    /// The first field the person still has to fill, or the first one.
    fn focus_first_field_to_fill(&self, window: &mut Window, cx: &mut Context<Self>) {
        let fields = self.visible_fields();
        let field = fields
            .iter()
            .copied()
            .find(|field| self.value(*field, cx).is_empty())
            .or_else(|| fields.first().copied());
        if let Some(field) = field {
            self.focus(field, window, cx);
        }
    }

    fn forget_field_error(&mut self, field: SignInField, cx: &mut Context<Self>) {
        let count_before = self.field_errors.len();
        self.field_errors.retain(|(errored, _)| *errored != field);
        if self.field_errors.len() != count_before {
            cx.notify();
        }
    }

    /// Shows the field errors; true when there are some (the first field then has the focus).
    fn show_field_errors(
        &mut self,
        errors: Vec<(SignInField, &'static str)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.field_errors = errors;
        let Some((first_field_to_fix, _)) = self.field_errors.first().copied() else {
            return false;
        };
        self.error_message = None;
        self.focus(first_field_to_fix, window, cx);
        cx.notify();
        true
    }

    /// The card's primary action, also Enter in any field.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy {
            return;
        }
        match &self.step {
            Step::InstanceAddress => self.check_instance_address(window, cx),
            Step::Credentials {
                instance,
                is_registering,
            } => {
                let (instance, is_registering) = (instance.clone(), *is_registering);
                self.send_credentials(instance, is_registering, window, cx);
            }
            Step::TwoFactor {
                instance,
                challenge_token,
                is_using_recovery_code,
            } => {
                let (instance, challenge_token, is_using_recovery_code) = (
                    instance.clone(),
                    challenge_token.clone(),
                    *is_using_recovery_code,
                );
                self.verify_code(
                    instance,
                    challenge_token,
                    is_using_recovery_code,
                    window,
                    cx,
                );
            }
        }
    }

    fn check_instance_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let strings = active_strings(cx);
        let typed = self
            .value(SignInField::InstanceAddress, cx)
            .trim()
            .to_owned();
        let instance = InstanceAddress::parse(&typed);
        let errors = field_errors_before_sending(&[FieldToCheck {
            field: SignInField::InstanceAddress,
            value: &typed,
            empty_error: strings.instance_address_missing,
            malformed_error: instance
                .is_err()
                .then_some(strings.instance_address_invalid),
        }]);
        if self.show_field_errors(errors, window, cx) {
            return;
        }
        let Ok(instance) = instance else {
            return;
        };
        self.start_request(cx);
        self.instance_being_checked = Some(instance.clone());
        let probed_instance = instance.clone();
        let probe = spawn_on_network_runtime(cx, async move {
            RestClient::new(probed_instance)?.health().await
        });
        self.request = Some(cx.spawn_in(window, async move |screen, cx| {
            // An error means the network task was cancelled or panicked: nothing to show.
            let Ok(answer) = probe.await else {
                return;
            };
            let _ = screen.update_in(cx, |screen, window, cx| {
                screen.is_busy = false;
                screen.instance_being_checked = None;
                match judge_instance_health(answer) {
                    Ok(()) => {
                        screen.step = Step::Credentials {
                            instance,
                            is_registering: false,
                        };
                        screen.has_session_ended_by_itself = false;
                        screen.focus_first_field_to_fill(window, cx);
                    }
                    Err(refusal) => {
                        let message =
                            refusal.describe(instance_display_name(&instance), active_strings(cx));
                        // Field errors are `'static` copy; this one names the host, so it is the
                        // form's error, still announced and shown under the field.
                        screen.error_message = Some(message);
                        screen.focus(SignInField::InstanceAddress, window, cx);
                    }
                }
                cx.notify();
            });
        }));
    }

    fn start_request(&mut self, cx: &mut Context<Self>) {
        self.is_busy = true;
        self.error_message = None;
        self.field_errors.clear();
        cx.notify();
    }

    fn send_credentials(
        &mut self,
        instance: InstanceAddress,
        is_registering: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let strings = active_strings(cx);
        let login = self.value(SignInField::Login, cx).trim().to_owned();
        let email = self.value(SignInField::Email, cx).trim().to_owned();
        let username = self.value(SignInField::Username, cx).trim().to_owned();
        let password_field = if is_registering {
            SignInField::NewPassword
        } else {
            SignInField::Password
        };
        // As typed: spaces count in a password.
        let password = Zeroizing::new(self.value(password_field, cx).to_string());
        let errors = if is_registering {
            field_errors_before_sending(&[
                FieldToCheck {
                    field: SignInField::Email,
                    value: &email,
                    empty_error: strings.email_missing,
                    malformed_error: (!is_valid_email_address(&email))
                        .then_some(strings.email_malformed),
                },
                FieldToCheck {
                    field: SignInField::Username,
                    value: &username,
                    empty_error: strings.username_missing,
                    malformed_error: None,
                },
                FieldToCheck {
                    field: SignInField::NewPassword,
                    value: &password,
                    empty_error: strings.new_password_missing,
                    malformed_error: None,
                },
            ])
        } else {
            field_errors_before_sending(&[
                FieldToCheck {
                    field: SignInField::Login,
                    value: &login,
                    empty_error: strings.login_missing,
                    malformed_error: None,
                },
                FieldToCheck {
                    field: SignInField::Password,
                    value: &password,
                    empty_error: strings.password_missing,
                    malformed_error: None,
                },
            ])
        };
        if self.show_field_errors(errors, window, cx) {
            return;
        }
        self.start_request(cx);
        let credentials = if is_registering {
            Credentials::Register {
                email,
                username,
                password,
            }
        } else {
            Credentials::Login { login, password }
        };
        let request =
            spawn_on_network_runtime(cx, send_credentials_to_instance(instance, credentials));
        self.request = Some(cx.spawn_in(window, async move |screen, cx| {
            // An error means the network task was cancelled or panicked: nothing to show.
            let Ok(answer) = request.await else {
                return;
            };
            let _ = screen.update_in(cx, |screen, window, cx| {
                screen.is_busy = false;
                match answer {
                    Ok(CredentialsAnswer::SignedIn(rest)) => screen.finish(rest, window, cx),
                    Ok(CredentialsAnswer::TwoFactorRequired(challenge_token)) => {
                        let Step::Credentials { instance, .. } = &screen.step else {
                            return;
                        };
                        screen.step = Step::TwoFactor {
                            instance: instance.clone(),
                            challenge_token,
                            is_using_recovery_code: false,
                        };
                        screen.set_value(SignInField::Password, "", window, cx);
                        screen.set_value(SignInField::Code, "", window, cx);
                        screen.focus_first_field_to_fill(window, cx);
                    }
                    Err(error) => {
                        let strings = active_strings(cx);
                        let summary = if is_registering {
                            strings.register_failed
                        } else {
                            strings.sign_in_failed
                        };
                        screen.error_message = Some(describe_error(&error, summary, strings));
                        screen.focus_first_field_to_fill(window, cx);
                    }
                }
                cx.notify();
            });
        }));
    }

    fn verify_code(
        &mut self,
        instance: InstanceAddress,
        challenge_token: Zeroizing<String>,
        is_using_recovery_code: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let strings = active_strings(cx);
        let code = Zeroizing::new(self.value(SignInField::Code, cx).trim().to_owned());
        let errors = field_errors_before_sending(&[FieldToCheck {
            field: SignInField::Code,
            value: &code,
            empty_error: if is_using_recovery_code {
                strings.two_factor_recovery_code_missing
            } else {
                strings.two_factor_code_missing
            },
            malformed_error: None,
        }]);
        if self.show_field_errors(errors, window, cx) {
            return;
        }
        self.start_request(cx);
        let request = spawn_on_network_runtime(cx, async move {
            let mut rest = RestClient::new(instance)?;
            let mut request = CompleteTwoFactorSignInRequest {
                two_factor_token: challenge_token.to_string(),
                code: code.to_string(),
            };
            let answer = rest.complete_two_factor_sign_in(&request).await;
            request.two_factor_token.zeroize();
            request.code.zeroize();
            answer.map(|_| rest)
        });
        self.request = Some(cx.spawn_in(window, async move |screen, cx| {
            // An error means the network task was cancelled or panicked: nothing to show.
            let Ok(answer) = request.await else {
                return;
            };
            let _ = screen.update_in(cx, |screen, window, cx| {
                screen.is_busy = false;
                match answer {
                    Ok(rest) => screen.finish(rest, window, cx),
                    // The challenge expired, was used up by wrong codes, or the instance restarted.
                    Err(ClientError::Api {
                        code: ApiErrorCode::Unauthorized,
                        ..
                    }) => {
                        let message = active_strings(cx).two_factor_expired.to_owned();
                        screen.back_to_password(Some(message), window, cx);
                    }
                    Err(error) => {
                        let strings = active_strings(cx);
                        let is_rate_limited = matches!(
                            error,
                            ClientError::Api {
                                code: ApiErrorCode::RateLimited,
                                ..
                            }
                        );
                        // Too many wrong codes lock app codes; a recovery code still works.
                        screen.error_message =
                            Some(if is_rate_limited && !is_using_recovery_code {
                                strings.two_factor_codes_locked.to_owned()
                            } else {
                                describe_error(&error, strings.two_factor_verify_failed, strings)
                            });
                        screen.focus(SignInField::Code, window, cx);
                    }
                }
                cx.notify();
            });
        }));
    }

    /// Signed in: the session takes over, and the instance is remembered for the next sign-in.
    fn finish(&mut self, rest: RestClient, window: &mut Window, cx: &mut Context<Self>) {
        for field in [
            SignInField::Password,
            SignInField::NewPassword,
            SignInField::Code,
        ] {
            self.set_value(field, "", window, cx);
        }
        if self.session.read(cx).is_kept_on_this_device() {
            let instance = rest.instance().clone();
            change_preferences(cx, move |preferences| {
                preferences.last_instance_address = Some(instance);
            });
        }
        self.session
            .update(cx, |session, cx| session.begin_signed_in_session(rest, cx));
    }

    /// Back to the password step; the login stays filled in, the code is dropped.
    fn back_to_password(
        &mut self,
        message: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Step::TwoFactor { instance, .. } = &self.step else {
            return;
        };
        self.step = Step::Credentials {
            instance: instance.clone(),
            is_registering: false,
        };
        self.set_value(SignInField::Code, "", window, cx);
        self.set_value(SignInField::Password, "", window, cx);
        self.error_message = message;
        self.field_errors.clear();
        self.focus_first_field_to_fill(window, cx);
        cx.notify();
    }

    /// The link under the button: sign in, create an account, or back from the code step.
    pub fn switch_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_busy {
            return;
        }
        match &mut self.step {
            Step::InstanceAddress => return,
            Step::TwoFactor { .. } => {
                self.back_to_password(None, window, cx);
                return;
            }
            Step::Credentials { is_registering, .. } => *is_registering = !*is_registering,
        }
        self.error_message = None;
        self.field_errors.clear();
        self.focus_first_field_to_fill(window, cx);
        cx.notify();
    }

    fn switch_code_kind(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Step::TwoFactor {
            is_using_recovery_code,
            ..
        } = &mut self.step
        else {
            return;
        };
        *is_using_recovery_code = !*is_using_recovery_code;
        self.set_value(SignInField::Code, "", window, cx);
        self.error_message = None;
        self.field_errors.clear();
        self.focus(SignInField::Code, window, cx);
        cx.notify();
    }

    /// Desktop only: back to the instance address, filled with the current one.
    pub fn use_another_instance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Step::Credentials { instance, .. } = &self.step else {
            return;
        };
        let address = instance_field_value(instance).to_owned();
        // Any request under way belongs to the instance being left.
        self.request = None;
        self.is_busy = false;
        self.step = Step::InstanceAddress;
        self.has_session_ended_by_itself = false;
        self.error_message = None;
        self.field_errors.clear();
        self.set_value(SignInField::InstanceAddress, &address, window, cx);
        self.focus(SignInField::InstanceAddress, window, cx);
        cx.notify();
    }

    fn field_error(&self, field: SignInField) -> Option<&'static str> {
        self.field_errors
            .iter()
            .find_map(|(errored, error)| (*errored == field).then_some(*error))
    }

    fn text_field(
        &self,
        field: SignInField,
        label: &'static str,
        helper: Option<SharedString>,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        TextField {
            label,
            input: self.input(field),
            helper,
            error: self.field_error(field),
            is_focused: self.focus_handle_of(field, cx).is_focused(window),
            is_monospace: field == SignInField::Code,
        }
        .render(theme(cx))
    }

    fn title_and_subtitle(&self, strings: &Strings) -> (String, &'static str) {
        match &self.step {
            Step::InstanceAddress => (strings.instance_title.to_owned(), strings.instance_subtitle),
            Step::Credentials {
                instance,
                is_registering: true,
            } => (
                (strings.register_title)(instance_display_name(instance)),
                strings.register_subtitle,
            ),
            Step::Credentials { instance, .. } => (
                (strings.sign_in_title)(instance_display_name(instance)),
                strings.sign_in_subtitle,
            ),
            Step::TwoFactor {
                is_using_recovery_code,
                ..
            } => (
                strings.two_factor_heading.to_owned(),
                if *is_using_recovery_code {
                    strings.two_factor_recovery_subtitle
                } else {
                    strings.two_factor_subtitle
                },
            ),
        }
    }

    fn primary_label(&self, strings: &Strings) -> SharedString {
        match (&self.step, self.is_busy) {
            (Step::InstanceAddress, _) => match &self.instance_being_checked {
                Some(instance) => {
                    (strings.instance_checking)(instance_display_name(instance)).into()
                }
                None => strings.instance_continue.into(),
            },
            (
                Step::Credentials {
                    is_registering: true,
                    ..
                },
                false,
            ) => strings.create_account.into(),
            (
                Step::Credentials {
                    is_registering: true,
                    ..
                },
                true,
            ) => strings.creating_account.into(),
            (Step::Credentials { .. }, false) => strings.sign_in.into(),
            (Step::Credentials { .. }, true) => strings.signing_in.into(),
            (Step::TwoFactor { .. }, false) => strings.two_factor_verify.into(),
            (Step::TwoFactor { .. }, true) => strings.two_factor_verifying.into(),
        }
    }

    /// The step's fields with their labels and helpers, and the code kind switch.
    fn render_fields(&self, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        let tokens = theme(cx);
        let strings = active_strings(cx);
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_TEXT_FIELD_FIELD_GAP))
            .children(self.visible_fields().iter().map(|field| {
                let (label, helper): (&'static str, Option<SharedString>) =
                    match (field, &self.step) {
                        (SignInField::InstanceAddress, _) => (strings.instance_address, None),
                        (SignInField::Login, _) => (strings.login, None),
                        (SignInField::Password, _) => (strings.password, None),
                        (SignInField::Email, _) => (strings.email, None),
                        (SignInField::Username, _) => {
                            (strings.username, Some(strings.username_helper.into()))
                        }
                        (SignInField::NewPassword, _) => (
                            strings.password,
                            Some((strings.password_length_helper)(MINIMUM_PASSWORD_LENGTH).into()),
                        ),
                        (
                            SignInField::Code,
                            Step::TwoFactor {
                                is_using_recovery_code: true,
                                ..
                            },
                        ) => (strings.two_factor_recovery_code, None),
                        (SignInField::Code, _) => (strings.two_factor_code, None),
                    };
                self.text_field(*field, label, helper, window, cx)
            }))
            .when_some(
                match &self.step {
                    Step::TwoFactor {
                        is_using_recovery_code,
                        ..
                    } => Some(*is_using_recovery_code),
                    _ => None,
                },
                |fields, is_using_recovery_code| {
                    let label = if is_using_recovery_code {
                        strings.two_factor_use_authenticator
                    } else {
                        strings.two_factor_use_recovery_code
                    };
                    fields.child(link_button("switch-code-kind", label, tokens).on_click(
                        cx.listener(|screen, _, window, cx| {
                            screen.switch_code_kind(window, cx);
                        }),
                    ))
                },
            )
    }

    /// The row under the button (sign in, create an account, back) and, on the credentials
    /// card, "Use another instance".
    fn render_links(&self, cx: &mut Context<Self>) -> (Option<gpui::Div>, Option<gpui::Div>) {
        let tokens = theme(cx);
        let strings = active_strings(cx);
        let (hint, link_label) = match &self.step {
            Step::InstanceAddress => (None, None),
            Step::Credentials {
                is_registering: true,
                ..
            } => (Some(strings.has_account), Some(strings.sign_in)),
            Step::Credentials { .. } => (
                Some(strings.has_no_account),
                Some(strings.create_an_account),
            ),
            Step::TwoFactor { .. } => (None, Some(strings.two_factor_back_to_sign_in)),
        };
        let mode_row =
            link_label.map(|label| {
                text_style(div(), TYPE_BODY)
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_center()
                    .gap_x(length(SPACE_2XS))
                    .text_color(color(tokens.component_auth_card_hint_text))
                    .when_some(hint, gpui::ParentElement::child)
                    .child(link_button("switch-mode", label, tokens).on_click(
                        cx.listener(|screen, _, window, cx| screen.switch_mode(window, cx)),
                    ))
            });
        let another_instance = matches!(self.step, Step::Credentials { .. }).then(|| {
            div().flex().justify_center().child(
                link_button("use-another-instance", strings.use_another_instance, tokens).on_click(
                    cx.listener(|screen, _, window, cx| {
                        screen.use_another_instance(window, cx);
                    }),
                ),
            )
        });
        (mode_row, another_instance)
    }
}

impl Render for SignInScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let strings = active_strings(cx);
        let (title, subtitle) = self.title_and_subtitle(strings);
        let subtitle = match &self.step {
            Step::Credentials {
                instance,
                is_registering: false,
            } if self.has_session_ended_by_itself => {
                (strings.session_ended)(instance_display_name(instance))
            }
            _ => subtitle.to_owned(),
        };
        let fields = self.render_fields(window, cx);

        let submit = primary_button("submit", self.primary_label(strings), self.is_busy, tokens)
            .on_click(cx.listener(|screen, _, window, cx| screen.submit(window, cx)));
        let form_error = div()
            .id("sign-in-error")
            .live_assertive(self.error_message.clone().unwrap_or_default())
            .when_some(self.error_message.clone(), |region, message| {
                region.child(error_line(
                    message,
                    tokens.component_auth_card_error_text,
                    COMPONENT_AUTH_CARD_ERROR_ICON_SIZE,
                ))
            });

        let (mode_row, another_instance) = self.render_links(cx);

        let card = div()
            .id("sign-in-card")
            .role(Role::Form)
            .w_full()
            .max_w(length(COMPONENT_AUTH_CARD_MAX_WIDTH))
            .flex()
            .flex_col()
            .gap(length(COMPONENT_AUTH_CARD_SECTION_GAP))
            .p(length(COMPONENT_AUTH_CARD_PADDING))
            .rounded(length(COMPONENT_AUTH_CARD_RADIUS))
            .bg(color(tokens.component_auth_card_background))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(length(COMPONENT_AUTH_CARD_SUBTITLE_GAP))
                    .text_center()
                    .child(
                        text_style(div().id("sign-in-title"), TYPE_HEADING_XL)
                            .role(Role::Heading)
                            .text_color(color(tokens.component_auth_card_title_text))
                            .child(title),
                    )
                    .child(
                        text_style(div(), TYPE_BODY_LG)
                            .text_color(color(tokens.component_auth_card_subtitle_text))
                            .child(subtitle),
                    ),
            )
            .child(fields)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(length(SPACE_XS))
                    .child(form_error)
                    .child(submit),
            )
            .children(mode_row)
            .children(another_instance);

        div()
            .id("sign-in-screen")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .items_center()
            .justify_center()
            .p(length(COMPONENT_AUTH_CARD_MARGIN))
            .bg(color(tokens.component_auth_card_backdrop))
            .font_family(SANS_FONT_FAMILY)
            .child(card)
    }
}

#[cfg(test)]
#[path = "sign_in_screen_tests.rs"]
mod tests;
