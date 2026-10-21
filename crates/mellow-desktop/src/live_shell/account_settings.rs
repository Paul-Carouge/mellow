//! User settings → My Account (DESIGN.md §5.9): display name, username and password, through the
//! account API (`PATCH /api/v1/users/@me`, `POST /api/v1/users/@me/password`).

use mellow_client_core::ClientError;
use mellow_protocol::{
    ApiErrorCode, ChangePasswordRequest, CurrentUser, GatewayEvent, PublicUser,
    UpdateCurrentUserRequest,
};
use slint::SharedString;
use zeroize::Zeroize as _;

use super::{
    LiveShell, SharedShell, apply_state_change, authenticated_rest, now_unix_milliseconds,
    report_request_error, run_request,
};

const PASSWORD_CHANGED_NOTICE: &str = "Password changed. Your other devices are signed out.";

/// The account card's values, from the signed-in user.
pub(super) fn refresh(shell: &LiveShell) {
    if let (Some(window), Some(user)) = (shell.window.upgrade(), shell.state.current_user()) {
        window.set_account_display_name(user.display_name.as_str().into());
        window.set_account_username(user.username.as_str().into());
        window.set_account_email(user.email.as_str().into());
    }
}

pub(super) fn clear(window: &crate::generated_ui::AppWindow) {
    window.set_account_display_name(SharedString::default());
    window.set_account_username(SharedString::default());
    window.set_account_email(SharedString::default());
    window.set_account_error(SharedString::default());
    window.set_account_notice(SharedString::default());
    window.set_is_account_busy(false);
}

pub(super) fn save_display_name(shell: &SharedShell, display_name: &str) {
    let request = UpdateCurrentUserRequest {
        display_name: Some(display_name.trim().to_owned()),
        ..UpdateCurrentUserRequest::default()
    };
    update_current_user(
        shell,
        request,
        "Display name not saved",
        "Display name saved.",
    );
}

pub(super) fn save_username(shell: &SharedShell, username: &str, current_password: &str) {
    let request = UpdateCurrentUserRequest {
        username: Some(username.trim().to_owned()),
        current_password: Some(current_password.to_owned()),
        ..UpdateCurrentUserRequest::default()
    };
    update_current_user(shell, request, "Username not saved", "Username saved.");
}

fn update_current_user(
    shell: &SharedShell,
    request: UpdateCurrentUserRequest,
    summary: &'static str,
    notice: &'static str,
) {
    let Some(rest) = begin_request(shell) else {
        return;
    };
    let mut request = request;
    // The password is wiped once sent, as on sign-in.
    let request = async move {
        let result = rest.update_current_user(&request).await;
        if let Some(password) = request.current_password.as_mut() {
            password.zeroize();
        }
        result
    };
    run_request(shell, request, move |shell, result| {
        finish_request(shell);
        match result {
            Ok(user) => {
                apply_current_user(shell, user);
                succeed(shell, notice);
            }
            Err(error) => report_error(shell, &error, "update account", summary),
        }
    });
}

pub(super) fn change_password(shell: &SharedShell, current_password: &str, new_password: &str) {
    let Some(rest) = begin_request(shell) else {
        return;
    };
    let mut request = ChangePasswordRequest {
        current_password: current_password.to_owned(),
        new_password: new_password.to_owned(),
    };
    // Both passwords are wiped once sent, as on sign-in.
    let request = async move {
        let result = rest.change_password(&request).await;
        request.current_password.zeroize();
        request.new_password.zeroize();
        result
    };
    run_request(shell, request, |shell, result| {
        finish_request(shell);
        match result {
            Ok(()) => succeed(shell, PASSWORD_CHANGED_NOTICE),
            Err(error) => report_error(shell, &error, "change password", "Password not changed"),
        }
    });
}

/// The gateway sends the same `USER_UPDATE`; applying the answer shows the change at once.
fn apply_current_user(shell: &SharedShell, user: CurrentUser) {
    let change = shell.borrow_mut().state.apply_event(
        GatewayEvent::UserUpdate(PublicUser {
            id: user.id,
            username: user.username,
            display_name: user.display_name,
        }),
        now_unix_milliseconds(),
    );
    apply_state_change(shell, change);
    refresh(&shell.borrow());
}

/// `None` while signed out or while another account request runs.
fn begin_request(shell: &SharedShell) -> Option<mellow_client_core::RestClient> {
    let rest = authenticated_rest(shell)?;
    let window = shell.borrow().window.upgrade()?;
    if window.get_is_account_busy() {
        return None;
    }
    window.set_account_error(SharedString::default());
    window.set_account_notice(SharedString::default());
    window.set_is_account_busy(true);
    Some(rest)
}

fn finish_request(shell: &SharedShell) {
    if let Some(window) = shell.borrow().window.upgrade() {
        window.set_is_account_busy(false);
    }
}

fn succeed(shell: &SharedShell, notice: &str) {
    if let Some(window) = shell.borrow().window.upgrade() {
        window.set_account_notice(notice.into());
        window.set_account_saved_count(window.get_account_saved_count().wrapping_add(1));
    }
}

fn report_error(shell: &SharedShell, error: &ClientError, action: &str, summary: &str) {
    report_request_error(shell, error, action);
    if !error.needs_new_login()
        && let Some(window) = shell.borrow().window.upgrade()
    {
        window.set_account_error(account_error_text(error, summary).into());
    }
}

/// Built from the error code only: the instance's own text never reaches a trusted place.
fn account_error_text(error: &ClientError, summary: &str) -> String {
    let reason = match error {
        ClientError::Api {
            code: ApiErrorCode::InvalidCredentials,
            ..
        } => "the current password is wrong.",
        ClientError::Api {
            code: ApiErrorCode::UsernameTaken,
            ..
        } => "this username is taken.",
        ClientError::Api {
            code: ApiErrorCode::ValidationFailed,
            ..
        } => "the instance refused it (check the length and the characters).",
        ClientError::Api {
            code: ApiErrorCode::RateLimited,
            ..
        } => "too many attempts. Wait a few minutes and try again.",
        ClientError::Network(_) => "the instance could not be reached. Try again.",
        _ => "something went wrong. Try again.",
    };
    format!("{summary}: {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_text_never_repeats_the_instance_message() {
        let error = ClientError::Api {
            status: 401,
            code: ApiErrorCode::InvalidCredentials,
            message: "visit https://evil.example".into(),
        };
        assert_eq!(
            account_error_text(&error, "Password not changed"),
            "Password not changed: the current password is wrong."
        );
    }
}
