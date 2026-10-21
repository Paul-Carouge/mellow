//! Server settings → Roles (DESIGN.md §5.8) on the open guild.
//!
//! The UI names roles by row index; rows are rebuilt whenever the guild's roles or members
//! change, so an index is resolved through `displayed_role_ids` at once. The draft lives here,
//! not in Slint, and stays until it is saved, reset, or matches the stored role again.

use mellow_client_core::ClientError;
use mellow_protocol::{ApiErrorCode, GatewayEvent, Guild, Role, Snowflake};
use slint::{ModelRc, SharedString, VecModel};

use super::{
    LiveShell, SharedShell, apply_state_change, authenticated_rest, now_unix_milliseconds,
    report_request_error, run_request,
};
use crate::role_settings::{
    self, MoveDirection, RoleDraft, RoleDraftError, RoleManager, role_color_at_preset,
};
use crate::shell_rows;

const NO_ROW: i32 = -1;

pub(super) struct RoleEditor {
    guild_id: Snowflake,
    /// The role id behind each row of `server-settings-roles`, in the same order.
    displayed_role_ids: Vec<Snowflake>,
    selected_role_id: Option<Snowflake>,
    /// `None` while the selected role has no unsaved change.
    draft: Option<RoleDraft>,
    is_busy: bool,
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
    shell.role_editor = Some(RoleEditor {
        guild_id,
        displayed_role_ids: Vec::new(),
        selected_role_id: None,
        draft: None,
        is_busy: false,
    });
    if let Some(window) = shell.window.upgrade() {
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
        window.set_is_server_settings_open(false);
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
    let draft = editor
        .draft
        .clone()
        .filter(|_| selected_role_id == editor.selected_role_id)
        .filter(|draft| selected_role.is_some_and(|role| *draft != RoleDraft::from(role)));
    let is_busy = editor.is_busy;

    let Some(window) = shell.window.upgrade() else {
        return;
    };
    window.set_server_settings_roles(ModelRc::new(VecModel::from(rows)));
    window.set_can_create_roles(manager.can_create_roles());
    window.set_selected_role_index(
        selected_role_id
            .and_then(|id| displayed_role_ids.iter().position(|shown| *shown == id))
            .map_or(NO_ROW, shell_rows::saturating_int),
    );
    let is_editable = selected_role.is_some_and(|role| manager.can_manage_role(role, guild.id));
    let shown = draft.clone().or_else(|| selected_role.map(RoleDraft::from));
    if let Some(shown) = &shown {
        // Rewriting the field while it is typed in would move the cursor.
        if window.get_role_draft_name().as_str() != shown.name {
            window.set_role_draft_name(shown.name.as_str().into());
        }
        window.set_role_draft_color(role_settings::rgb_color(shown.color));
        window.set_role_draft_has_color(shown.color != role_settings::NO_ROLE_COLOR);
    }
    window.set_role_permission_rows(ModelRc::new(VecModel::from(
        role_settings::permission_rows(
            shown
                .as_ref()
                .map(|shown| shown.permissions)
                .unwrap_or_default(),
            &manager,
            is_editable,
        ),
    )));
    window.set_is_role_draft_editable(is_editable);
    window.set_has_unsaved_role_changes(draft.is_some());
    window.set_is_role_action_busy(is_busy);

    if let Some(editor) = shell.role_editor.as_mut() {
        editor.displayed_role_ids = displayed_role_ids;
        editor.selected_role_id = selected_role_id;
        editor.draft = draft;
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

/// The selected role and its manager, with the draft (or the stored values) to edit.
fn editable_selection(shell: &LiveShell) -> Option<(Snowflake, Role, RoleManager, RoleDraft)> {
    let editor = shell.role_editor.as_ref()?;
    let guild = shell.state.guild(editor.guild_id)?;
    let manager = RoleManager::of_member(guild, shell.state.current_user()?.id);
    let role = guild
        .roles
        .iter()
        .find(|role| Some(role.id) == editor.selected_role_id)?;
    if editor.is_busy || !manager.can_manage_role(role, guild.id) {
        return None;
    }
    let draft = editor
        .draft
        .clone()
        .unwrap_or_else(|| RoleDraft::from(role));
    Some((guild.id, role.clone(), manager, draft))
}

fn set_draft(shell: &SharedShell, draft: RoleDraft) {
    let mut shell = shell.borrow_mut();
    if let Some(editor) = shell.role_editor.as_mut() {
        editor.draft = Some(draft);
    }
    if let Some(window) = shell.window.upgrade() {
        window.set_role_action_error(SharedString::default());
    }
    refresh(&mut shell);
}

pub(super) fn select_role(shell: &SharedShell, row_index: i32) {
    let mut shell = shell.borrow_mut();
    let Some(editor) = shell.role_editor.as_mut() else {
        return;
    };
    let Some(role_id) = role_id_at_row(&editor.displayed_role_ids, row_index) else {
        return;
    };
    if editor.is_busy || editor.selected_role_id == Some(role_id) {
        return;
    }
    // Slint refuses to switch while there are unsaved changes; this only drops a stale draft.
    editor.selected_role_id = Some(role_id);
    editor.draft = None;
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
    let Some((_, role, _, mut draft)) = editable_selection(&shell.borrow()) else {
        return;
    };
    if is_everyone_role(&shell.borrow(), role.id) {
        return;
    }
    draft.name = name;
    set_draft(shell, draft);
}

pub(super) fn choose_color(shell: &SharedShell, preset_index: i32) {
    let Some((_, role, _, mut draft)) = editable_selection(&shell.borrow()) else {
        return;
    };
    if is_everyone_role(&shell.borrow(), role.id) {
        return;
    }
    draft.color = role_color_at_preset(preset_index);
    set_draft(shell, draft);
}

pub(super) fn toggle_permission(shell: &SharedShell, row_index: i32) {
    let Some((_, _, manager, mut draft)) = editable_selection(&shell.borrow()) else {
        return;
    };
    let Some(permission) = role_settings::toggleable_permission_at_row(row_index, &manager) else {
        return;
    };
    draft.permissions.toggle(permission);
    set_draft(shell, draft);
}

pub(super) fn reset_changes(shell: &SharedShell) {
    let mut shell = shell.borrow_mut();
    if let Some(editor) = shell.role_editor.as_mut() {
        editor.draft = None;
    }
    if let Some(window) = shell.window.upgrade() {
        window.set_role_action_error(SharedString::default());
    }
    refresh(&mut shell);
}

pub(super) fn save_changes(shell: &SharedShell) {
    let Some((guild_id, role, _, draft)) = editable_selection(&shell.borrow()) else {
        return;
    };
    let is_everyone = role.id == guild_id;
    let request = match role_settings::update_role_request(&role, &draft, is_everyone) {
        Ok(request) => request,
        Err(RoleDraftError::EmptyName) => {
            show_error(shell, "Role not saved: give it a name.");
            return;
        }
    };
    let Some(rest) = begin_request(shell) else {
        return;
    };
    let role_id = role.id;
    let request = async move { rest.update_role(guild_id, role_id, &request).await };
    run_request(shell, request, move |shell, result| {
        finish_request(shell);
        match result {
            Ok(updated) => {
                // The draft now matches the stored role, so `refresh` drops it.
                apply_roles(shell, guild_id, |roles| {
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
        if editor.is_busy || editor.draft.is_some() {
            return;
        }
        editor.guild_id
    };
    let Some(rest) = begin_request(shell) else {
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
                    editor.draft = None;
                }
                apply_roles(shell, guild_id, |roles| {
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
        if editor.is_busy || role_id == editor.guild_id {
            return;
        }
        (editor.guild_id, role_id)
    };
    let Some(rest) = begin_request(shell) else {
        return;
    };
    let request = async move { rest.delete_role(guild_id, role_id).await };
    run_request(shell, request, move |shell, result| {
        finish_request(shell);
        match result {
            Ok(()) => apply_roles(shell, guild_id, |roles| {
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
        if editor.is_busy {
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
    let Some(rest) = begin_request(shell) else {
        return;
    };
    let request = async move {
        rest.reorder_roles(guild_id, &mellow_protocol::ReorderRolesRequest { role_ids })
            .await
    };
    run_request(shell, request, move |shell, result| {
        finish_request(shell);
        match result {
            Ok(roles) => apply_roles(shell, guild_id, |current| *current = roles),
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

/// Marks the editor busy and clears the previous error; `None` when signed out.
fn begin_request(shell: &SharedShell) -> Option<mellow_client_core::RestClient> {
    let rest = authenticated_rest(shell)?;
    let mut shell = shell.borrow_mut();
    if let Some(editor) = shell.role_editor.as_mut() {
        editor.is_busy = true;
    }
    if let Some(window) = shell.window.upgrade() {
        window.set_role_action_error(SharedString::default());
        window.set_is_role_action_busy(true);
    }
    Some(rest)
}

fn finish_request(shell: &SharedShell) {
    let mut shell = shell.borrow_mut();
    if let Some(editor) = shell.role_editor.as_mut() {
        editor.is_busy = false;
    }
    if let Some(window) = shell.window.upgrade() {
        window.set_is_role_action_busy(false);
    }
}

/// The gateway sends the same `GUILD_ROLES_UPDATE`; applying it now shows the change at once.
fn apply_roles(shell: &SharedShell, guild_id: Snowflake, change: impl FnOnce(&mut Vec<Role>)) {
    let Some(mut roles) = shell
        .borrow()
        .state
        .guild(guild_id)
        .map(|guild| guild.roles.clone())
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
        } => "the name is not valid (empty or longer than 100 characters).",
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
