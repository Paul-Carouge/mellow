//! Add a server (create or join with an invite), invite people, create a channel and leave a
//! server (DESIGN.md §5.2, §5.3), through the dialogs of `server_dialogs.slint`.
//!
//! Each answer is applied to the state at once as the gateway event it stands for; the gateway
//! sends the same event later, which then changes nothing.

use mellow_client_core::ClientError;
use mellow_protocol::{
    ApiErrorCode, ChannelKind, CreateChannelRequest, CreateInviteRequest, GatewayEvent, Guild,
    Permissions, Snowflake,
};
use slint::SharedString;

use super::{
    LiveShell, SharedShell, apply_state_change, authenticated_rest, now_unix_milliseconds,
    open_channel, report_request_error, run_request, select_guild,
};
use crate::generated_ui::ShellDialog;
use crate::{clipboard, live_rows};

pub(super) fn can_invite_people(guild: &Guild) -> bool {
    guild.my_permissions.contains(Permissions::CREATE_INVITE)
}

pub(super) fn can_create_channels(guild: &Guild) -> bool {
    guild.my_permissions.contains(Permissions::MANAGE_CHANNELS)
}

/// The owner cannot leave their own server, as on Discord.
pub(super) fn can_leave_server(guild: &Guild, current_user_id: Option<Snowflake>) -> bool {
    current_user_id.is_some_and(|user_id| user_id != guild.owner_id)
}

/// Clears what a dialog left behind and drops the answer of any request still in flight, so it
/// cannot land in a later dialog; called when a dialog closes and when the session ends.
pub(super) fn reset_dialog(shell: &mut LiveShell) {
    shell.dialog_request_generation += 1;
    if let Some(window) = shell.window.upgrade() {
        window.set_dialog_error(SharedString::default());
        window.set_invite_code(SharedString::default());
        window.set_is_dialog_busy(false);
    }
}

/// Closes a dialog about the selected server once that server is gone (kicked, deleted, left
/// from another device), before the next selected server could take its place.
pub(super) fn close_dialog_of_gone_guild(shell: &SharedShell) {
    let is_server_dialog_open = shell.borrow().window.upgrade().is_some_and(|window| {
        !matches!(
            window.get_open_dialog(),
            ShellDialog::None | ShellDialog::AddServer
        )
    });
    if is_server_dialog_open {
        close_dialog(shell);
    }
}

pub(super) fn create_server(shell: &SharedShell, name: &str) {
    let name = name.trim().to_owned();
    if name.is_empty() {
        show_error(shell, "Server not created: give it a name.");
        return;
    }
    let Some((rest, generation)) = begin_request(shell) else {
        return;
    };
    let request = async move { rest.create_guild(&name).await };
    run_request(shell, request, move |shell, result| {
        if !finish_request(shell, generation) {
            return;
        }
        match result {
            Ok(guild) => show_new_guild(shell, guild),
            Err(error) => report_error(shell, &error, "create server", "Server not created"),
        }
    });
}

pub(super) fn join_server(shell: &SharedShell, typed_text: &str) {
    let Some(code) = invite_code_from_typed_text(typed_text) else {
        show_error(
            shell,
            "Couldn't join: enter the invite code, letters and digits only.",
        );
        return;
    };
    let Some((rest, generation)) = begin_request(shell) else {
        return;
    };
    let request = async move { rest.join_with_invite(&code).await };
    run_request(shell, request, move |shell, result| {
        if !finish_request(shell, generation) {
            return;
        }
        match result {
            Ok(guild) => show_new_guild(shell, guild),
            Err(error) => report_error(shell, &error, "join server", "Couldn't join"),
        }
    });
}

/// The code alone, or the last path segment of a pasted link ending with it.
fn invite_code_from_typed_text(typed_text: &str) -> Option<String> {
    let code = typed_text.trim().trim_end_matches('/').rsplit('/').next()?;
    (!code.is_empty() && code.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .then(|| code.to_owned())
}

fn show_new_guild(shell: &SharedShell, guild: Guild) {
    let guild_id = guild.id;
    let change = shell.borrow_mut().state.apply_event(
        GatewayEvent::GuildCreate(Box::new(guild)),
        now_unix_milliseconds(),
    );
    apply_state_change(shell, change);
    close_dialog(shell);
    select_guild(shell, Some(guild_id), None);
}

pub(super) fn create_invite(shell: &SharedShell) {
    let Some(guild_id) = shell.borrow().selected_guild_id else {
        return;
    };
    let Some((rest, generation)) = begin_request(shell) else {
        return;
    };
    // Never expires and has no use limit: the dialog says so.
    let request = async move {
        rest.create_invite(
            guild_id,
            &CreateInviteRequest {
                max_uses: None,
                expires_in_seconds: None,
            },
        )
        .await
    };
    run_request(shell, request, move |shell, result| {
        if !finish_request(shell, generation) {
            return;
        }
        match result {
            Ok(invite) => {
                if let Some(window) = shell.borrow().window.upgrade() {
                    window.set_invite_code(invite.code.as_str().into());
                }
            }
            Err(error) => report_error(shell, &error, "create invite", "No invite created"),
        }
    });
}

pub(super) fn copy_invite_code(shell: &SharedShell) {
    let mut shell = shell.borrow_mut();
    let Some(code) = shell
        .window
        .upgrade()
        .map(|window| window.get_invite_code())
    else {
        return;
    };
    if !code.is_empty() {
        clipboard::copy_text(&mut shell.clipboard, &code);
    }
}

pub(super) fn create_channel(shell: &SharedShell, name: &str) {
    let Some(guild_id) = shell.borrow().selected_guild_id else {
        return;
    };
    // From a category's "+", the channel goes in that category, if it is still one of this server's.
    let parent_id = {
        let shell = shell.borrow();
        let category_id = shell.window.upgrade().and_then(|window| {
            window
                .get_create_channel_category_id()
                .parse::<Snowflake>()
                .ok()
        });
        category_id.filter(|id| {
            shell
                .state
                .guild(guild_id)
                .is_some_and(|guild| live_rows::sorted_category_ids(guild).contains(id))
        })
    };
    // The instance lowercases and hyphenates the name, and refuses what it cannot fix.
    let name = name.trim().trim_start_matches('#').trim().to_owned();
    if name.is_empty() {
        show_error(shell, "Channel not created: give it a name.");
        return;
    }
    let Some((rest, generation)) = begin_request(shell) else {
        return;
    };
    let request = async move {
        rest.create_channel(
            guild_id,
            &CreateChannelRequest {
                kind: ChannelKind::Text,
                name,
                topic: String::new(),
                parent_id,
                user_limit: None,
            },
        )
        .await
    };
    run_request(shell, request, move |shell, result| {
        if !finish_request(shell, generation) {
            return;
        }
        match result {
            Ok(channel) => {
                let channel_id = channel.id;
                let change = shell.borrow_mut().state.apply_event(
                    GatewayEvent::ChannelCreate(channel),
                    now_unix_milliseconds(),
                );
                apply_state_change(shell, change);
                close_dialog(shell);
                if shell.borrow().selected_guild_id == Some(guild_id) {
                    open_channel(shell, Some(channel_id));
                }
            }
            Err(error) => report_error(shell, &error, "create channel", "Channel not created"),
        }
    });
}

pub(super) fn leave_server(shell: &SharedShell) {
    let Some(guild_id) = shell.borrow().selected_guild_id else {
        return;
    };
    let Some((rest, generation)) = begin_request(shell) else {
        return;
    };
    let request = async move { rest.leave_guild(guild_id).await };
    run_request(shell, request, move |shell, result| {
        if !finish_request(shell, generation) {
            return;
        }
        match result {
            Ok(()) => {
                // Selects another server and closes this dialog (`close_dialog_of_gone_guild`).
                let change = shell.borrow_mut().state.apply_event(
                    GatewayEvent::GuildDelete { id: guild_id },
                    now_unix_milliseconds(),
                );
                apply_state_change(shell, change);
            }
            Err(error) => report_error(shell, &error, "leave server", "Couldn't leave"),
        }
    });
}

/// Marks the dialog busy and clears its previous error; answers the generation its answer must
/// still match. `None` while signed out or while another dialog request is in flight, so a
/// double Enter sends one request.
fn begin_request(shell: &SharedShell) -> Option<(mellow_client_core::RestClient, u64)> {
    let rest = authenticated_rest(shell)?;
    let shell = shell.borrow();
    let window = shell.window.upgrade()?;
    if window.get_is_dialog_busy() {
        return None;
    }
    window.set_dialog_error(SharedString::default());
    window.set_is_dialog_busy(true);
    Some((rest, shell.dialog_request_generation))
}

/// False when the dialog that sent the request was closed meanwhile: the answer is then dropped.
fn finish_request(shell: &SharedShell, generation: u64) -> bool {
    let shell = shell.borrow();
    if shell.dialog_request_generation != generation {
        return false;
    }
    if let Some(window) = shell.window.upgrade() {
        window.set_is_dialog_busy(false);
    }
    true
}

fn close_dialog(shell: &SharedShell) {
    if let Some(window) = shell.borrow().window.upgrade() {
        window.set_open_dialog(ShellDialog::None);
        window.invoke_restore_focus_after_dialog();
    }
    reset_dialog(&mut shell.borrow_mut());
}

fn show_error(shell: &SharedShell, text: &str) {
    if let Some(window) = shell.borrow().window.upgrade() {
        window.set_dialog_error(text.into());
    }
}

fn report_error(shell: &SharedShell, error: &ClientError, action: &str, summary: &str) {
    report_request_error(shell, error, action);
    if !error.needs_new_login() {
        show_error(shell, &dialog_error_text(error, summary));
    }
}

/// Built from the error code only: the instance's own text never reaches a trusted place.
fn dialog_error_text(error: &ClientError, summary: &str) -> String {
    let reason = match error {
        ClientError::Api {
            code: ApiErrorCode::InvalidInvite,
            ..
        } => "this invite is unknown, expired or used up.",
        ClientError::Api {
            code: ApiErrorCode::Banned,
            ..
        } => "you are banned from this server.",
        ClientError::Api {
            code: ApiErrorCode::OwnerCannotLeave,
            ..
        } => "the owner cannot leave their own server.",
        ClientError::Api {
            code: ApiErrorCode::MissingPermissions,
            ..
        } => "you don't have the permission to do that here.",
        ClientError::Api {
            code: ApiErrorCode::NotFound,
            ..
        } => "this server no longer exists.",
        ClientError::Api {
            code: ApiErrorCode::ValidationFailed,
            ..
        } => {
            "the instance refused the name (letters, digits, - or _ for channels, up to 100 characters)."
        }
        ClientError::Api {
            code: ApiErrorCode::LimitReached,
            ..
        } => "a limit of this instance is reached.",
        ClientError::Api {
            code: ApiErrorCode::RateLimited,
            ..
        } => "too many requests at once. Wait a moment and try again.",
        ClientError::Network(_) => "the instance could not be reached. Try again.",
        _ => "something went wrong. Try again.",
    };
    format!("{summary}: {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_codes_are_taken_alone_or_from_the_end_of_a_link() {
        assert_eq!(invite_code_from_typed_text(" aB3x9 "), Some("aB3x9".into()));
        assert_eq!(
            invite_code_from_typed_text("https://chat.example.org/invite/aB3x9/"),
            Some("aB3x9".into())
        );
        assert_eq!(invite_code_from_typed_text(""), None);
        assert_eq!(invite_code_from_typed_text("ab cd"), None);
        // Only the last segment is ever sent, so path tricks cannot reach another route.
        assert_eq!(
            invite_code_from_typed_text("../admin"),
            Some("admin".into())
        );
        assert_eq!(invite_code_from_typed_text("invite/a.b"), None);
    }

    #[test]
    fn error_text_never_repeats_the_instance_message() {
        let error = ClientError::Api {
            status: 404,
            code: ApiErrorCode::InvalidInvite,
            message: "sign in again at https://evil.example".into(),
        };
        let text = dialog_error_text(&error, "Couldn't join");
        assert_eq!(
            text,
            "Couldn't join: this invite is unknown, expired or used up."
        );
    }
}
