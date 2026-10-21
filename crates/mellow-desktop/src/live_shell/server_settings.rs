//! Server settings → Roles (DESIGN.md §5.8) on the open guild.
//!
//! The UI names roles by row index; rows are rebuilt whenever the guild's roles or members
//! change, so an index is resolved through `displayed_role_ids` at once. The edit lives here,
//! not in Slint, and stays until it is saved, reset, or matches the stored role again.

use mellow_client_core::ClientError;
use mellow_protocol::{ApiErrorCode, GatewayEvent, Guild, Role, Snowflake};
use std::rc::Rc;

use slint::{Model as _, ModelRc, SharedString, VecModel};

use super::{
    LiveShell, SharedShell, apply_state_change, authenticated_rest, now_unix_milliseconds,
    report_request_error, run_request,
};
use crate::generated_ui::{RoleListRow, RolePermissionRow};
use crate::role_settings::{
    self, MoveDirection, RoleDraft, RoleDraftError, RoleEdit, RoleManager, role_color_at_preset,
};
use crate::shell_rows;

const NO_ROW: i32 = -1;

pub(super) struct RoleEditor {
    guild_id: Snowflake,
    /// The role id behind each row of `server-settings-roles`, in the same order.
    displayed_role_ids: Vec<Snowflake>,
    selected_role_id: Option<Snowflake>,
    /// `None` while the selected role has no unsaved change.
    edit: Option<RoleEdit>,
    /// Kept across refreshes and updated row by row: a new model would recreate every row and
    /// drop keyboard focus (development notes: "Slint repeaters").
    role_rows: Rc<VecModel<RoleListRow>>,
    permission_rows: Rc<VecModel<RolePermissionRow>>,
}

/// Whether the signed-in member may open server settings on `guild` (Roles is its only section).
pub(super) fn can_open_server_settings(guild: &Guild) -> bool {
    guild
        .my_permissions
        .contains(mellow_protocol::Permissions::MANAGE_ROLES)
}

pub(super) fn open(shell: &SharedShell) {
    let mut shell = shell.borrow_mut();
    let Some(guild_id) = shell.selected_guild_id else {
        return;
    };
    let role_rows = Rc::new(VecModel::default());
    let permission_rows = Rc::new(VecModel::default());
    shell.role_editor = Some(RoleEditor {
        guild_id,
        displayed_role_ids: Vec::new(),
        selected_role_id: None,
        edit: None,
        role_rows: role_rows.clone(),
        permission_rows: permission_rows.clone(),
    });
    if let Some(window) = shell.window.upgrade() {
        window.set_server_settings_roles(role_rows.into());
        window.set_role_permission_rows(permission_rows.into());
        window.set_role_color_presets(ModelRc::new(VecModel::from(
            role_settings::role_color_presets(),
        )));
        window.set_role_action_error(SharedString::default());
        window.set_is_server_settings_open(true);
    }
    refresh(&mut shell);
}

pub(super) fn close(shell: &mut LiveShell) {
    shell.role_editor = None;
    if let Some(window) = shell.window.upgrade() {
        let was_open = window.get_is_server_settings_open();
        window.set_is_server_settings_open(false);
        // Slint's own close moves the focus itself; this covers a guild that disappeared.
        if was_open && window.get_is_signed_in() {
            window.invoke_restore_focus_after_server_settings();
        }
    }
}

/// Rebuilds the role list and the editor; closes settings when the guild is gone.
pub(super) fn refresh(shell: &mut LiveShell) {
    let Some(editor) = shell.role_editor.as_ref() else {
        return;
    };
    let Some(guild) = shell.state.guild(editor.guild_id) else {
        close(shell);
        return;
    };
    let Some(current_user) = shell.state.current_user() else {
        return;
    };
    let manager = RoleManager::of_member(guild, current_user.id);
    let members = shell.state.members(guild.id);
    let rows = role_settings::role_list_rows(guild, members, &manager);
    let displayed_role_ids: Vec<Snowflake> = role_settings::roles_highest_first(guild)
        .iter()
        .map(|role| role.id)
        .collect();
    // The selected role may be gone: fall back to the first one this member manages.
    let selected_role = editor
        .selected_role_id
        .and_then(|id| guild.roles.iter().find(|role| role.id == id))
        .or_else(|| default_selected_role(guild, &manager));
    let selected_role_id = selected_role.map(|role| role.id);
    // Rebased on the stored role, so changes made meanwhile by someone else show and are kept.
    let edit = editor
        .edit
        .as_ref()
        .filter(|_| selected_role_id == editor.selected_role_id)
        .zip(selected_role)
        .and_then(|(edit, role)| edit.rebased_on(role));
    let is_editable = selected_role.is_some_and(|role| manager.can_manage_role(role, guild.id));
    // A role that became locked (moved above the member's rank) cannot be saved any more.
    let edit = edit.filter(|_| is_editable);
    let is_busy = shell.is_role_request_in_flight;

    let Some(window) = shell.window.upgrade() else {
        return;
    };
    update_rows_in_place(&editor.role_rows, rows);
    window.set_can_create_roles(manager.can_create_roles());
    window.set_selected_role_index(
        selected_role_id
            .and_then(|id| displayed_role_ids.iter().position(|shown| *shown == id))
            .map_or(NO_ROW, shell_rows::saturating_int),
    );
    let shown = edit
        .as_ref()
        .map(|edit| edit.draft.clone())
        .or_else(|| selected_role.map(RoleDraft::from));
    if let Some(shown) = &shown {
        // Rewriting the field while it is typed in would move the cursor.
        if window.get_role_draft_name().as_str() != shown.name {
            window.set_role_draft_name(shown.name.as_str().into());
        }
        window.set_role_draft_color(role_settings::rgb_color(shown.color));
        window.set_role_draft_has_color(shown.color != role_settings::NO_ROLE_COLOR);
    }
    update_rows_in_place(
        &editor.permission_rows,
        role_settings::permission_rows(
            shown
                .as_ref()
                .map(|shown| shown.permissions)
                .unwrap_or_default(),
            &manager,
            is_editable,
        ),
    );
    window.set_is_role_draft_editable(is_editable);
    window.set_has_unsaved_role_changes(edit.is_some());
    window.set_is_role_action_busy(is_busy);

    if let Some(editor) = shell.role_editor.as_mut() {
        editor.displayed_role_ids = displayed_role_ids;
        editor.selected_role_id = selected_role_id;
        editor.edit = edit;
    }
}

/// Rewrites only the rows that changed; the model is reset only when the row count changes.
fn update_rows_in_place<Row: Clone + PartialEq + 'static>(model: &VecModel<Row>, rows: Vec<Row>) {
    if model.row_count() != rows.len() {
        model.set_vec(rows);
        return;
    }
    for (index, row) in rows.into_iter().enumerate() {
        if model.row_data(index).as_ref() != Some(&row) {
            model.set_row_data(index, row);
        }
    }
}

fn default_selected_role<'a>(guild: &'a Guild, manager: &RoleManager) -> Option<&'a Role> {
    let roles = role_settings::roles_highest_first(guild);
    roles
        .iter()
        .find(|role| role.id != guild.id && manager.can_manage_role(role, guild.id))
        .or_else(|| roles.last())
        .copied()
}

/// The selected role and its manager, with the pending edit (or a fresh one) on top of it.
fn editable_selection(shell: &LiveShell) -> Option<(Snowflake, Role, RoleManager, RoleEdit)> {
    let editor = shell.role_editor.as_ref()?;
    let guild = shell.state.guild(editor.guild_id)?;
    let manager = RoleManager::of_member(guild, shell.state.current_user()?.id);
    let role = guild
        .roles
        .iter()
        .find(|role| Some(role.id) == editor.selected_role_id)?;
    if shell.is_role_request_in_flight || !manager.can_manage_role(role, guild.id) {
        return None;
    }
    let edit = editor
        .edit
        .as_ref()
        .and_then(|edit| edit.rebased_on(role))
        .unwrap_or_else(|| RoleEdit::start(role));
    Some((guild.id, role.clone(), manager, edit))
}

fn set_edit(shell: &SharedShell, edit: RoleEdit) {
    let mut shell = shell.borrow_mut();
    if let Some(editor) = shell.role_editor.as_mut() {
        editor.edit = Some(edit);
    }
    if let Some(window) = shell.window.upgrade() {
        window.set_role_action_error(SharedString::default());
    }
    refresh(&mut shell);
}

pub(super) fn select_role(shell: &SharedShell, row_index: i32) {
    let mut shell = shell.borrow_mut();
    let is_busy = shell.is_role_request_in_flight;
    let Some(editor) = shell.role_editor.as_mut() else {
        return;
    };
    let Some(role_id) = role_id_at_row(&editor.displayed_role_ids, row_index) else {
        return;
    };
    if is_busy || editor.selected_role_id == Some(role_id) {
        return;
    }
    // Slint refuses to switch while there are unsaved changes; this only drops a stale edit.
    editor.selected_role_id = Some(role_id);
    editor.edit = None;
    if let Some(window) = shell.window.upgrade() {
        window.set_role_action_error(SharedString::default());
    }
    refresh(&mut shell);
}

pub(super) fn edit_draft_name(shell: &SharedShell) {
    let name = {
        let shell = shell.borrow();
        let Some(window) = shell.window.upgrade() else {
            return;
        };
        window.get_role_draft_name().to_string()
    };
    let Some((_, role, _, mut edit)) = editable_selection(&shell.borrow()) else {
        return;
    };
    if is_everyone_role(&shell.borrow(), role.id) {
        return;
    }
    edit.draft.name = name;
    set_edit(shell, edit);
}

pub(super) fn choose_color(shell: &SharedShell, preset_index: i32) {
    let Some((_, role, _, mut edit)) = editable_selection(&shell.borrow()) else {
        return;
    };
    if is_everyone_role(&shell.borrow(), role.id) {
        return;
    }
    edit.draft.color = role_color_at_preset(preset_index);
    set_edit(shell, edit);
}

pub(super) fn toggle_permission(shell: &SharedShell, row_index: i32) {
    let Some((_, _, manager, mut edit)) = editable_selection(&shell.borrow()) else {
        return;
    };
    let Some(permission) = role_settings::toggleable_permission_at_row(row_index, &manager) else {
        return;
    };
    edit.draft.permissions.toggle(permission);
    set_edit(shell, edit);
}

pub(super) fn reset_changes(shell: &SharedShell) {
    let mut shell = shell.borrow_mut();
    if let Some(editor) = shell.role_editor.as_mut() {
        editor.edit = None;
    }
    if let Some(window) = shell.window.upgrade() {
        window.set_role_action_error(SharedString::default());
    }
    refresh(&mut shell);
}

pub(super) fn save_changes(shell: &SharedShell) {
    let Some((guild_id, role, _, edit)) = editable_selection(&shell.borrow()) else {
        return;
    };
    let is_everyone = role.id == guild_id;
    let request = match role_settings::update_role_request(&role, &edit.draft, is_everyone) {
        Ok(request) => request,
        Err(RoleDraftError::EmptyName) => {
            show_error(shell, "Role not saved: give it a name.");
            return;
        }
    };
    let Some((rest, roles_before)) = begin_request(shell, guild_id) else {
        return;
    };
    let role_id = role.id;
    let request = async move { rest.update_role(guild_id, role_id, &request).await };
    run_request(shell, request, move |shell, result| {
        finish_request(shell);
        match result {
            Ok(updated) => {
                // Rebasing the saved edit on the updated role would toggle its bits back.
                if let Some(editor) = shell.borrow_mut().role_editor.as_mut() {
                    editor.edit = None;
                }
                apply_roles(shell, guild_id, &roles_before, |roles| {
                    if let Some(stored) = roles.iter_mut().find(|role| role.id == updated.id) {
                        *stored = updated;
                    }
                });
            }
            Err(error) => report_error(shell, &error, "save role", "Role not saved"),
        }
    });
}

pub(super) fn create_role(shell: &SharedShell) {
    let guild_id = {
        let shell = shell.borrow();
        let Some(editor) = shell.role_editor.as_ref() else {
            return;
        };
        if shell.is_role_request_in_flight || editor.edit.is_some() {
            return;
        }
        editor.guild_id
    };
    let Some((rest, roles_before)) = begin_request(shell, guild_id) else {
        return;
    };
    let request = async move {
        rest.create_role(guild_id, &role_settings::new_role_request())
            .await
    };
    run_request(shell, request, move |shell, result| {
        finish_request(shell);
        match result {
            Ok(created) => {
                let created_id = created.id;
                if let Some(editor) = shell.borrow_mut().role_editor.as_mut() {
                    editor.selected_role_id = Some(created_id);
                    editor.edit = None;
                }
                apply_roles(shell, guild_id, &roles_before, |roles| {
                    if !roles.iter().any(|role| role.id == created_id) {
                        roles.push(created);
                    }
                });
            }
            Err(error) => report_error(shell, &error, "create role", "Role not created"),
        }
    });
}

pub(super) fn delete_role(shell: &SharedShell, row_index: i32) {
    let (guild_id, role_id) = {
        let shell = shell.borrow();
        let Some(editor) = shell.role_editor.as_ref() else {
            return;
        };
        let Some(role_id) = role_id_at_row(&editor.displayed_role_ids, row_index) else {
            return;
        };
        if shell.is_role_request_in_flight || role_id == editor.guild_id {
            return;
        }
        (editor.guild_id, role_id)
    };
    let Some((rest, roles_before)) = begin_request(shell, guild_id) else {
        return;
    };
    let request = async move { rest.delete_role(guild_id, role_id).await };
    run_request(shell, request, move |shell, result| {
        finish_request(shell);
        match result {
            Ok(()) => apply_roles(shell, guild_id, &roles_before, |roles| {
                roles.retain(|role| role.id != role_id);
            }),
            Err(error) => report_error(shell, &error, "delete role", "Role not deleted"),
        }
    });
}

pub(super) fn move_role(shell: &SharedShell, row_index: i32, direction: MoveDirection) {
    let (guild_id, role_ids) = {
        let shell = shell.borrow();
        let Some(editor) = shell.role_editor.as_ref() else {
            return;
        };
        if shell.is_role_request_in_flight {
            return;
        }
        let Some(guild) = shell.state.guild(editor.guild_id) else {
            return;
        };
        let Some(role_ids) = role_id_at_row(&editor.displayed_role_ids, row_index)
            .and_then(|role_id| role_settings::role_ids_after_move(guild, role_id, direction))
        else {
            return;
        };
        (guild.id, role_ids)
    };
    let Some((rest, roles_before)) = begin_request(shell, guild_id) else {
        return;
    };
    let request = async move {
        rest.reorder_roles(guild_id, &mellow_protocol::ReorderRolesRequest { role_ids })
            .await
    };
    run_request(shell, request, move |shell, result| {
        finish_request(shell);
        match result {
            Ok(roles) => apply_roles(shell, guild_id, &roles_before, |current| *current = roles),
            Err(error) => report_error(shell, &error, "reorder roles", "Roles not reordered"),
        }
    });
}

fn role_id_at_row(displayed_role_ids: &[Snowflake], row_index: i32) -> Option<Snowflake> {
    displayed_role_ids
        .get(usize::try_from(row_index).ok()?)
        .copied()
}

fn is_everyone_role(shell: &LiveShell, role_id: Snowflake) -> bool {
    shell
        .role_editor
        .as_ref()
        .is_some_and(|editor| editor.guild_id == role_id)
}

/// Marks a role request in flight and clears the previous error; answers the guild's roles as
/// they are now, for `apply_roles`. `None` when signed out.
fn begin_request(
    shell: &SharedShell,
    guild_id: Snowflake,
) -> Option<(mellow_client_core::RestClient, Vec<Role>)> {
    let rest = authenticated_rest(shell)?;
    let mut shell = shell.borrow_mut();
    let roles_before = shell
        .state
        .guild(guild_id)
        .map(|guild| guild.roles.clone())
        .unwrap_or_default();
    // On the shell, not the editor: closing and reopening settings must not start a second one.
    shell.is_role_request_in_flight = true;
    if let Some(window) = shell.window.upgrade() {
        window.set_role_action_error(SharedString::default());
        window.set_is_role_action_busy(true);
    }
    Some((rest, roles_before))
}

fn finish_request(shell: &SharedShell) {
    let mut shell = shell.borrow_mut();
    shell.is_role_request_in_flight = false;
    if let Some(window) = shell.window.upgrade() {
        window.set_is_role_action_busy(false);
    }
}

/// The gateway sends the same `GUILD_ROLES_UPDATE`; applying it now shows the change at once.
/// Skipped when the roles changed since the request started: the gateway was then faster, and
/// the answer would overwrite a newer state with no later event to correct it.
fn apply_roles(
    shell: &SharedShell,
    guild_id: Snowflake,
    roles_before: &[Role],
    change: impl FnOnce(&mut Vec<Role>),
) {
    let Some(mut roles) = shell
        .borrow()
        .state
        .guild(guild_id)
        .map(|guild| guild.roles.clone())
        .filter(|current| current == roles_before)
    else {
        return;
    };
    change(&mut roles);
    roles.sort_by_key(|role| role.position);
    let state_change = shell.borrow_mut().state.apply_event(
        GatewayEvent::GuildRolesUpdate { guild_id, roles },
        now_unix_milliseconds(),
    );
    apply_state_change(shell, state_change);
}

fn show_error(shell: &SharedShell, text: &str) {
    if let Some(window) = shell.borrow().window.upgrade() {
        window.set_role_action_error(text.into());
    }
}

fn report_error(shell: &SharedShell, error: &ClientError, action: &str, summary: &str) {
    report_request_error(shell, error, action);
    if !error.needs_new_login() {
        show_error(shell, &role_action_error_text(error, summary));
    }
    refresh(&mut shell.borrow_mut());
}

/// Built from the error code only: the instance's own text never reaches a trusted place.
fn role_action_error_text(error: &ClientError, summary: &str) -> String {
    let reason = match error {
        ClientError::Api {
            code: ApiErrorCode::MissingPermissions,
            ..
        } => "you can only change roles below your highest role, with permissions you have.",
        ClientError::Api {
            code: ApiErrorCode::NotFound,
            ..
        } => "the role no longer exists.",
        ClientError::Api {
            code: ApiErrorCode::ValidationFailed,
            ..
        } => "the instance refused it (a name empty or longer than 100 characters?).",
        ClientError::Api {
            code: ApiErrorCode::Conflict,
            ..
        } => "the roles changed meanwhile. Try again.",
        ClientError::Api {
            code: ApiErrorCode::LimitReached,
            ..
        } => "this server has reached its role limit.",
        ClientError::Api {
            code: ApiErrorCode::RateLimited,
            ..
        } => "too many changes at once. Wait a moment and try again.",
        ClientError::Network(_) => "the instance could not be reached. Try again.",
        _ => "something went wrong. Try again.",
    };
    format!("{summary}: {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_resolve_to_role_ids_and_out_of_range_rows_to_nothing() {
        let ids = [Snowflake::from_raw(3), Snowflake::from_raw(1)];
        assert_eq!(role_id_at_row(&ids, 1), Some(Snowflake::from_raw(1)));
        assert_eq!(role_id_at_row(&ids, 2), None);
        assert_eq!(role_id_at_row(&ids, NO_ROW), None);
    }

    #[test]
    fn error_text_never_repeats_the_instance_message() {
        let error = ClientError::Api {
            status: 403,
            code: ApiErrorCode::MissingPermissions,
            message: "sign in again at https://evil.example".into(),
        };
        let text = role_action_error_text(&error, "Role not saved");
        assert!(text.starts_with("Role not saved: you can only change roles"));
        assert!(!text.contains("evil"));
    }
}
