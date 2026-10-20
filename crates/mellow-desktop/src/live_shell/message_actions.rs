//! Reply, edit and delete on the messages of the open channel.
//!
//! The UI names a message by its row index; rows are rebuilt whenever the channel's messages
//! change, so an index is resolved to a message id at once, and the rows being edited or about to
//! be deleted are re-pointed (or closed) by `remap_open_message_rows` when the rows change.

use mellow_client_core::ClientError;
use mellow_client_core::state::StateChange;
use mellow_protocol::{ApiErrorCode, EditMessageRequest, GatewayEvent, Message, Snowflake};
use slint::SharedString;

use super::{
    LiveShell, SharedShell, apply_state_change, authenticated_rest, now_unix_milliseconds,
    report_request_error, run_request,
};
use crate::shell_rows;

const NO_ROW: i32 = -1;

/// The displayed message at a UI row index, with its channel.
fn displayed_message(shell: &LiveShell, row_index: i32) -> Option<(Snowflake, &Message)> {
    let channel_id = shell.selected_channel_id?;
    let message_id = *shell
        .displayed_message_ids
        .get(usize::try_from(row_index).ok()?)?;
    let message = shell
        .state
        .messages(channel_id)
        .iter()
        .find(|message| message.id == message_id)?;
    Some((channel_id, message))
}

fn row_index_of(shell: &LiveShell, message_id: Option<Snowflake>) -> i32 {
    message_id
        .and_then(|message_id| {
            shell
                .displayed_message_ids
                .iter()
                .position(|displayed_id| *displayed_id == message_id)
        })
        .map_or(NO_ROW, shell_rows::saturating_int)
}

/// Called with the ids shown before the rows were rebuilt: the inline editor and the delete
/// dialog follow their message, or close when it is gone; so does the reply.
pub(super) fn remap_open_message_rows(shell: &mut LiveShell, previous_message_ids: &[Snowflake]) {
    let Some(window) = shell.window.upgrade() else {
        return;
    };
    if shell
        .reply_to_message_id
        .is_some_and(|message_id| !shell.displayed_message_ids.contains(&message_id))
    {
        shell.reply_to_message_id = None;
        window.set_replying_to_author(SharedString::default());
    }
    let current_message_ids = &shell.displayed_message_ids;
    window.set_editing_message_index(row_index_after_rebuild(
        previous_message_ids,
        current_message_ids,
        window.get_editing_message_index(),
    ));
    window.set_deleting_message_index(row_index_after_rebuild(
        previous_message_ids,
        current_message_ids,
        window.get_deleting_message_index(),
    ));
}

/// Where the message shown at `previous_row` sits now, or `NO_ROW` when it is gone.
fn row_index_after_rebuild(
    previous_message_ids: &[Snowflake],
    current_message_ids: &[Snowflake],
    previous_row: i32,
) -> i32 {
    usize::try_from(previous_row)
        .ok()
        .and_then(|index| previous_message_ids.get(index))
        .and_then(|message_id| {
            current_message_ids
                .iter()
                .position(|current_id| current_id == message_id)
        })
        .map_or(NO_ROW, shell_rows::saturating_int)
}

/// Back to a plain composer: no reply, no editor, no dialog, no error. On channel change and
/// sign-out.
pub(super) fn reset_message_actions(shell: &mut LiveShell) {
    shell.reply_to_message_id = None;
    if let Some(window) = shell.window.upgrade() {
        window.set_replying_to_author(SharedString::default());
        window.set_editing_message_index(NO_ROW);
        window.set_deleting_message_index(NO_ROW);
        window.set_message_action_error(SharedString::default());
    }
}

pub(super) fn reply_to_message(shell: &SharedShell, row_index: i32) {
    let mut shell = shell.borrow_mut();
    let Some((_, message)) = displayed_message(&shell, row_index) else {
        return;
    };
    let (message_id, author) = (message.id, message.author.display_name.clone());
    shell.reply_to_message_id = Some(message_id);
    if let Some(window) = shell.window.upgrade() {
        window.set_replying_to_author(author.into());
    }
}

/// Taken when a message is sent, so the reply goes with that message only.
pub(super) fn take_reply(shell: &SharedShell) -> Option<Snowflake> {
    let mut shell = shell.borrow_mut();
    let reply_to_message_id = shell.reply_to_message_id.take();
    if let Some(window) = shell.window.upgrade() {
        window.set_replying_to_author(SharedString::default());
    }
    reply_to_message_id
}

pub(super) fn cancel_reply(shell: &SharedShell) {
    let mut shell = shell.borrow_mut();
    shell.reply_to_message_id = None;
    if let Some(window) = shell.window.upgrade() {
        window.set_replying_to_author(SharedString::default());
    }
}

/// Arrow-Up in the empty composer, as on Discord.
pub(super) fn edit_last_own_message(shell: &SharedShell) {
    let shell = shell.borrow();
    let Some(current_user_id) = shell.state.current_user().map(|user| user.id) else {
        return;
    };
    let Some(channel_id) = shell.selected_channel_id else {
        return;
    };
    let last_own_message = shell
        .state
        .messages(channel_id)
        .iter()
        .rev()
        .find(|message| message.author.id == current_user_id);
    if let (Some(window), Some(message)) = (shell.window.upgrade(), last_own_message) {
        window.set_editing_message_draft(message.content.as_str().into());
        window.set_editing_message_index(row_index_of(&shell, Some(message.id)));
    }
}

pub(super) fn save_message_edit(shell: &SharedShell, row_index: i32, text: &SharedString) {
    let content = text.trim().to_owned();
    let Some((channel_id, message_id)) = ({
        let shell = shell.borrow();
        displayed_message(&shell, row_index)
            // An emptied editor opens the delete dialog in the UI; only spaces is left unchanged.
            .filter(|(_, message)| !content.is_empty() && message.content != content)
            .map(|(channel_id, message)| (channel_id, message.id))
    }) else {
        return;
    };
    let Some(rest) = authenticated_rest(shell) else {
        return;
    };
    clear_message_action_error(shell);
    let request = async move {
        rest.edit_message(channel_id, message_id, &EditMessageRequest { content })
            .await
    };
    run_request(shell, request, move |shell, result| match result {
        // The gateway sends the same update; applying it now shows the edit without waiting.
        Ok(message) => apply_message_event(shell, GatewayEvent::MessageUpdate(message)),
        Err(error) => {
            report_message_action_error(shell, &error, "edit", "Message not edited");
            reopen_editor_with_draft(shell, message_id);
        }
    });
}

/// The UI closed the editor on Enter; after a failure the text the user typed is still in the
/// draft, so the editor comes back with it unless another edit started meanwhile.
fn reopen_editor_with_draft(shell: &SharedShell, message_id: Snowflake) {
    let shell = shell.borrow();
    if let Some(window) = shell.window.upgrade()
        && window.get_editing_message_index() == NO_ROW
    {
        window.set_editing_message_index(row_index_of(&shell, Some(message_id)));
    }
}

fn clear_message_action_error(shell: &SharedShell) {
    if let Some(window) = shell.borrow().window.upgrade() {
        window.set_message_action_error(SharedString::default());
    }
}

pub(super) fn delete_message(shell: &SharedShell, row_index: i32) {
    let Some((guild_id, channel_id, message_id)) = ({
        let shell = shell.borrow();
        let guild_id = shell.selected_guild_id;
        displayed_message(&shell, row_index)
            .and_then(|(channel_id, message)| Some((guild_id?, channel_id, message.id)))
    }) else {
        return;
    };
    let Some(rest) = authenticated_rest(shell) else {
        return;
    };
    clear_message_action_error(shell);
    let request = async move { rest.delete_message(channel_id, message_id).await };
    run_request(shell, request, move |shell, result| match result {
        Ok(()) => apply_message_event(
            shell,
            GatewayEvent::MessageDelete {
                guild_id,
                channel_id,
                id: message_id,
            },
        ),
        Err(error) => report_message_action_error(shell, &error, "delete", "Message not deleted"),
    });
}

fn apply_message_event(shell: &SharedShell, event: GatewayEvent) {
    let change: StateChange = shell
        .borrow_mut()
        .state
        .apply_event(event, now_unix_milliseconds());
    apply_state_change(shell, change);
}

fn report_message_action_error(
    shell: &SharedShell,
    error: &ClientError,
    action: &str,
    summary: &str,
) {
    report_request_error(shell, error, action);
    if error.needs_new_login() {
        return;
    }
    if let Some(window) = shell.borrow().window.upgrade() {
        window.set_message_action_error(message_action_error_text(error, summary).into());
    }
}

/// The outbox nonce behind a UI row: outbox rows follow the `displayed_message_ids` rows.
pub(super) fn pending_nonce_at_row<'nonces>(
    displayed_message_ids: &[Snowflake],
    displayed_pending_nonces: &'nonces [String],
    row_index: i32,
) -> Option<&'nonces str> {
    usize::try_from(row_index)
        .ok()
        .and_then(|index| index.checked_sub(displayed_message_ids.len()))
        .and_then(|index| displayed_pending_nonces.get(index))
        .map(String::as_str)
}

/// Why the instance refused a sent message (`ClientError::refuses_message_for_good`), in the
/// client's words like `message_action_error_text`.
pub(super) fn send_refusal_text(error: &ClientError) -> String {
    let reason = match error {
        ClientError::Api {
            code: ApiErrorCode::ValidationFailed,
            ..
        } => "the text is not valid (too long?).",
        ClientError::Api {
            code: ApiErrorCode::NotFound,
            ..
        } => "the channel or the message it replies to no longer exists.",
        ClientError::Api {
            code: ApiErrorCode::MissingPermissions,
            ..
        } => "you are not allowed to send messages here.",
        _ => "the instance refused it.",
    };
    format!("Message not sent: {reason}")
}

/// Written by the client from the error code: the instance's own text could say anything, at any
/// length, in a place the user trusts.
fn message_action_error_text(error: &ClientError, summary: &str) -> String {
    let reason = match error {
        ClientError::Api {
            code: ApiErrorCode::MissingPermissions,
            ..
        } => "you are not allowed to change this message.",
        ClientError::Api {
            code: ApiErrorCode::NotFound,
            ..
        } => "it no longer exists.",
        ClientError::Api {
            code: ApiErrorCode::RateLimited,
            ..
        } => "too many changes at once. Wait a moment and try again.",
        ClientError::Api {
            code: ApiErrorCode::ValidationFailed,
            ..
        } => "the text is not valid (too long?).",
        ClientError::Network(_) => "the instance could not be reached. Try again.",
        _ => "something went wrong. Try again.",
    };
    format!("{summary}: {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_rows_follow_their_message_when_rows_are_rebuilt() {
        let ids = |raw: &[u64]| {
            raw.iter()
                .map(|id| Snowflake::from_raw(*id))
                .collect::<Vec<_>>()
        };
        let shown = ids(&[10, 20, 30]);
        // A message arrives after the row, an earlier one is deleted, older history is inserted.
        assert_eq!(
            row_index_after_rebuild(&shown, &ids(&[10, 20, 30, 40]), 1),
            1
        );
        assert_eq!(row_index_after_rebuild(&shown, &ids(&[20, 30]), 1), 0);
        assert_eq!(
            row_index_after_rebuild(&shown, &ids(&[1, 2, 10, 20, 30]), 1),
            3
        );
        // The message itself is deleted, nothing was open, or the row was out of range.
        assert_eq!(row_index_after_rebuild(&shown, &ids(&[10, 30]), 1), NO_ROW);
        assert_eq!(row_index_after_rebuild(&shown, &shown, NO_ROW), NO_ROW);
        assert_eq!(row_index_after_rebuild(&shown, &shown, 7), NO_ROW);
    }

    #[test]
    fn outbox_rows_follow_the_confirmed_rows() {
        let shown = [Snowflake::from_raw(10), Snowflake::from_raw(20)];
        let nonces = ["a".to_owned(), "b".to_owned()];
        let at = |row_index| pending_nonce_at_row(&shown, &nonces, row_index);
        assert_eq!([at(-1), at(0), at(1)], [None, None, None]);
        assert_eq!([at(2), at(3), at(4)], [Some("a"), Some("b"), None]);
    }

    #[test]
    fn error_text_never_repeats_the_instance_message() {
        let error = ClientError::Api {
            status: 403,
            code: ApiErrorCode::MissingPermissions,
            message: "sign in again at https://evil.example".into(),
        };
        let text = message_action_error_text(&error, "Message not edited");
        assert_eq!(
            text,
            "Message not edited: you are not allowed to change this message."
        );
        let unknown = ClientError::Api {
            status: 500,
            code: ApiErrorCode::Internal,
            message: "evil".into(),
        };
        assert!(!message_action_error_text(&unknown, "Message not deleted").contains("evil"));
        assert_eq!(
            send_refusal_text(&error),
            "Message not sent: you are not allowed to send messages here."
        );
    }
}
