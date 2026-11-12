//! Roles (`RoleSettings.svelte`, DESIGN.md §5.8): the hierarchy on the leading side, the selected
//! role's editor beside it (below it when narrow). Roles above the reader's highest are shown but
//! locked, with the reason. Changes wait in a draft for the unsaved changes bar; a change made
//! meanwhile by someone else is kept under the draft (`rebase_role_edit`). Moving a role and its
//! icon save at once, as on Discord. What the reader may change comes only from client-core's
//! role rules, so nothing is offered that the instance would refuse.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Hsla,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render,
    Role as AccessRole, SharedString, StatefulInteractiveElement as _, Styled, Subscription,
    Toggled, Window, div, prelude::FluentBuilder as _, rgb,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::shared_logic::role_colors::readable_role_colour;
use mellow_client_core::shared_logic::server_settings::{
    MAXIMUM_ROLE_NAME_LENGTH, MoveDirection, NO_ROLE_COLOR, PERMISSION_GROUPS, ROLE_COLOR_PRESETS,
    RoleDraft, RoleEdit, RoleManager, can_manage_role, can_move_any_role, can_toggle_permission,
    draft_of, is_role_edit_pending, rebase_role_edit, role_color_hex, role_ids_after_move,
    role_manager_of, role_member_count, role_update_body, roles_highest_first, toggle_permission,
};
use mellow_protocol::{
    CreateRoleRequest, Guild, Permissions, ReorderRolesRequest, Role, Snowflake,
};

use super::role_icon_editor::RoleIconEditor;
use crate::components::{
    Button, ButtonVariant, ModalDialog, ModalDialogEvent, Switch, Tabs, TextField,
    UnsavedChangesBar, child_id, colour_transition, focus_ring, icon, inset_focus_ring,
    is_focus_visible, next_choice_index, remembered_hover, with_type,
};
use crate::i18n::strings;
use crate::session::Session;
use crate::settings::permission_words::{permission_group_label, permission_words};
use crate::settings::{
    SettingsPage, SettingsPageEvent, error_alert, is_wide, saved_notice, unsaved_changes_labels,
};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_DIALOG_WIDTH, COMPONENT_PERMISSION_BADGE_GAP, COMPONENT_PERMISSION_BADGE_HEIGHT,
    COMPONENT_PERMISSION_BADGE_ICON_SIZE, COMPONENT_PERMISSION_BADGE_PADDING_INLINE,
    COMPONENT_PERMISSION_BADGE_RADIUS, COMPONENT_PROFILE_POPOVER_WIDTH, COMPONENT_ROLES_EDITOR_GAP,
    COMPONENT_ROLES_LIST_GAP, COMPONENT_ROLES_LIST_WIDTH, COMPONENT_ROLES_PERMISSION_GAP,
    COMPONENT_ROLES_PERMISSION_GROUP_GAP, COMPONENT_ROLES_PERMISSION_PADDING_BLOCK,
    COMPONENT_ROLES_ROW_DOT_SIZE, COMPONENT_ROLES_ROW_GAP, COMPONENT_ROLES_ROW_HEIGHT,
    COMPONENT_ROLES_ROW_ICON_GAP, COMPONENT_ROLES_ROW_LOCK_ICON_SIZE,
    COMPONENT_ROLES_ROW_PADDING_INLINE, COMPONENT_ROLES_ROW_RADIUS, COMPONENT_ROLES_SENSITIVE_GAP,
    COMPONENT_ROLES_SWATCH_CHECK_PLATE_SIZE, COMPONENT_ROLES_SWATCH_CHECK_SIZE,
    COMPONENT_ROLES_SWATCH_GAP, COMPONENT_ROLES_SWATCH_RADIUS, COMPONENT_ROLES_SWATCH_RING_OFFSET,
    COMPONENT_ROLES_SWATCH_RING_WIDTH, COMPONENT_ROLES_SWATCH_SIZE,
    COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP, COMPONENT_SETTINGS_ACTION_GAP,
    COMPONENT_SETTINGS_ERROR_ICON_GAP, COMPONENT_SETTINGS_ERROR_ICON_SIZE,
    COMPONENT_SETTINGS_PAGE_TITLE_GAP, COMPONENT_SETTINGS_SECTION_LABEL_GAP, SIZE_HIT_WEB_MIN,
    TYPE_BODY, TYPE_BODY_SM, TYPE_CATEGORY, TYPE_CHANNEL_NAME, TYPE_HEADING_MD, TYPE_MEMBER_NAME,
};
use crate::theme::{color, length, theme};

/// The colour grid is ten hit targets wide, as the web's ten-wide grid.
const SWATCH_GRID_COLUMNS: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoleTab {
    Display,
    Permissions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RolesBusy {
    Create,
    Save,
    Move,
    Delete,
}

/// "No colour" first, then the presets: the colour choices in the order Tab and arrows meet them.
fn colour_choices() -> impl Iterator<Item = u32> {
    std::iter::once(NO_ROLE_COLOR).chain(ROLE_COLOR_PRESETS)
}

/// Ctrl+Shift+Up/Down (Cmd on macOS) on a role row moves it, as the web's list does.
fn move_direction_of(event: &KeyDownEvent) -> Option<MoveDirection> {
    let modifiers = &event.keystroke.modifiers;
    if !modifiers.shift || !(modifiers.control || modifiers.platform) || modifiers.alt {
        return None;
    }
    match event.keystroke.key.as_str() {
        "up" => Some(MoveDirection::Up),
        "down" => Some(MoveDirection::Down),
        _ => None,
    }
}

/// What the page reads of the server each time: its roles, the reader's rank and members.
struct RolesView {
    guild: Guild,
    manager: RoleManager,
    member_count_by_role: HashMap<Snowflake, usize>,
}

pub struct RolesPage {
    session: Entity<Session>,
    guild_id: Snowflake,
    selected_role_id: Option<Snowflake>,
    /// The role whose draft the name field shows, to notice another role taking its place.
    shown_role_id: Option<Snowflake>,
    edit: Option<RoleEdit>,
    /// Saved and waiting for the gateway: not an unsaved change any more.
    is_awaiting_echo: bool,
    /// Bumped by every change, so a save's answer only covers the edit it sent.
    edit_generation: u32,
    /// A role just created: its name field is focused once the gateway brings it, never before
    /// (the fallback role would get the typing).
    role_id_to_focus_when_shown: Option<Snowflake>,
    tab: RoleTab,
    name_input: Entity<InputState>,
    is_setting_name: bool,
    busy: Option<RolesBusy>,
    error_message: Option<SharedString>,
    save_error: Option<SharedString>,
    notice: Option<SharedString>,
    attention: u32,
    local_attention: u32,
    is_reported_changed: bool,
    row_focus: HashMap<Snowflake, FocusHandle>,
    swatch_focus: Vec<FocusHandle>,
    icon_editor: Option<(Snowflake, Entity<RoleIconEditor>)>,
    delete_dialog: Option<(Entity<ModalDialog>, Vec<Subscription>)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsPageEvent> for RolesPage {}

impl SettingsPage for RolesPage {
    fn set_attention(&mut self, attention: u32, cx: &mut Context<Self>) {
        self.attention = attention;
        cx.notify();
    }
}

impl std::fmt::Debug for RolesPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RolesPage")
            .field("guild_id", &self.guild_id)
            .field("selected_role_id", &self.selected_role_id)
            .field("busy", &self.busy)
            .finish_non_exhaustive()
    }
}

impl RolesPage {
    pub fn new(
        session: Entity<Session>,
        guild_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .validate(|text, _| text.chars().count() <= MAXIMUM_ROLE_NAME_LENGTH)
        });
        let subscriptions =
            vec![
                cx.subscribe_in(&name_input, window, |page, _, event: &InputEvent, _, cx| {
                    match event {
                        InputEvent::Change if !page.is_setting_name => page.type_name(cx),
                        InputEvent::PressEnter { .. } => page.save(cx),
                        _ => {}
                    }
                }),
                cx.observe_in(&session, window, |page, _, window, cx| {
                    page.follow_stored_roles(window, cx);
                }),
            ];
        let swatch_focus = colour_choices().map(|_| cx.focus_handle()).collect();
        let mut page = Self {
            session,
            guild_id,
            selected_role_id: None,
            shown_role_id: None,
            edit: None,
            is_awaiting_echo: false,
            edit_generation: 0,
            role_id_to_focus_when_shown: None,
            tab: RoleTab::Display,
            name_input,
            is_setting_name: false,
            busy: None,
            error_message: None,
            save_error: None,
            notice: None,
            attention: 0,
            local_attention: 0,
            is_reported_changed: false,
            row_focus: HashMap::new(),
            swatch_focus,
            icon_editor: None,
            delete_dialog: None,
            _subscriptions: subscriptions,
        };
        page.follow_stored_roles(window, cx);
        // Pinned, so a gateway update that makes another role the highest editable one does not
        // move the selection (and drop its draft).
        page.selected_role_id = page.shown_role_id;
        page
    }

    fn view(&self, cx: &App) -> Option<RolesView> {
        let state = self.session.read(cx).state();
        let guild = state.guild(self.guild_id)?;
        let manager = role_manager_of(guild, state.current_user().map(|user| user.id));
        let members = state.members(guild.id);
        let member_count_by_role = guild
            .roles
            .iter()
            .map(|role| (role.id, role_member_count(guild, role, members)))
            .collect();
        Some(RolesView {
            guild: guild.clone(),
            manager,
            member_count_by_role,
        })
    }

    /// The chosen role; a deleted one gives way to the highest the reader can edit.
    fn selected_role(&self, cx: &App) -> Option<(Role, bool)> {
        let state = self.session.read(cx).state();
        let guild = state.guild(self.guild_id)?;
        let manager = role_manager_of(guild, state.current_user().map(|user| user.id));
        let roles = roles_highest_first(guild);
        let role = roles
            .iter()
            .find(|role| Some(role.id) == self.selected_role_id)
            .or_else(|| {
                roles
                    .iter()
                    .find(|role| can_manage_role(&manager, role, guild.id))
            })
            .or_else(|| roles.first())?;
        Some(((*role).clone(), can_manage_role(&manager, role, guild.id)))
    }

    fn draft(&self, selected: &Role) -> Option<RoleDraft> {
        self.edit
            .as_ref()
            .filter(|edit| edit.role_id == selected.id)
            .and_then(|edit| rebase_role_edit(edit, selected))
            .map(|edit| edit.draft)
    }

    fn has_changes(&self, cx: &App) -> bool {
        !self.is_awaiting_echo
            && self
                .selected_role(cx)
                .is_some_and(|(role, _)| self.draft(&role).is_some())
    }

    /// Drops an edit with nothing left to save and tells the screen when unsaved changes start or
    /// end.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let selected = self.selected_role(cx).map(|(role, _)| role);
        if self
            .edit
            .as_ref()
            .is_some_and(|edit| !is_role_edit_pending(edit, selected.as_ref()))
        {
            self.edit = None;
            self.is_awaiting_echo = false;
        }
        let has_changes = self.has_changes(cx);
        if has_changes != self.is_reported_changed {
            self.is_reported_changed = has_changes;
            cx.emit(SettingsPageEvent::UnsavedChanges(has_changes));
        }
        cx.notify();
    }

    /// The gateway changed the server: an edit that is saved or gone ends, another role taking
    /// the selected one's place shows its name, and a role renamed elsewhere shows its new name
    /// in an untouched field.
    fn follow_stored_roles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
        let selected = self.selected_role(cx).map(|(role, _)| role);
        let selected_id = selected.as_ref().map(|role| role.id);
        let force = selected_id != self.shown_role_id;
        self.shown_role_id = selected_id;
        self.show_name(force, window, cx);
        if let Some(role_id) = selected_id
            && self.role_id_to_focus_when_shown == Some(role_id)
        {
            self.role_id_to_focus_when_shown = None;
            self.name_input.update(cx, |input, cx| {
                input.focus(window, cx);
                let end = input.value().len();
                input.set_selected_range(0..end, cx);
            });
        }
        // The role it would delete is gone (deleted elsewhere, another one now shows) or no
        // longer the reader's to delete. A dialog waiting for its own answer stays
        // (`request_close` is a no-op while busy).
        let is_selected_editable = self
            .selected_role(cx)
            .is_some_and(|(_, is_editable)| is_editable);
        if let Some((dialog, _)) = &self.delete_dialog
            && (force || !is_selected_editable)
        {
            dialog.update(cx, |dialog, cx| dialog.request_close(window, cx));
        }
    }

    /// Puts the shown draft's name in the field; without `force`, only when it differs by more
    /// than trailing spaces, so typing is never undone.
    fn show_name(&mut self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((role, _)) = self.selected_role(cx) else {
            return;
        };
        let name = self.draft(&role).unwrap_or_else(|| draft_of(&role)).name;
        let value = self.name_input.read(cx).value().to_string();
        if force || value.trim() != name.trim() {
            self.is_setting_name = true;
            self.name_input
                .update(cx, |input, cx| input.set_value(name, window, cx));
            self.is_setting_name = false;
        }
    }

    fn change(&mut self, next: RoleDraft, cx: &mut Context<Self>) {
        let Some((role, is_editable)) = self.selected_role(cx) else {
            return;
        };
        if !is_editable {
            return;
        }
        self.edit = Some(RoleEdit {
            role_id: role.id,
            base: draft_of(&role),
            draft: next,
        });
        self.edit_generation = self.edit_generation.wrapping_add(1);
        self.is_awaiting_echo = false;
        self.save_error = None;
        self.notice = None;
        self.refresh(cx);
    }

    fn shown_draft(&self, cx: &App) -> Option<RoleDraft> {
        let (role, _) = self.selected_role(cx)?;
        Some(self.draft(&role).unwrap_or_else(|| draft_of(&role)))
    }

    fn type_name(&mut self, cx: &mut Context<Self>) {
        let Some(shown) = self.shown_draft(cx) else {
            return;
        };
        let name = self.name_input.read(cx).value().to_string();
        if name != shown.name {
            self.change(RoleDraft { name, ..shown }, cx);
        }
    }

    fn select(&mut self, role_id: Snowflake, window: &mut Window, cx: &mut Context<Self>) {
        if self.shown_role_id == Some(role_id) {
            return;
        }
        if self.has_changes(cx) {
            self.local_attention += 1;
            cx.notify();
            return;
        }
        self.selected_role_id = Some(role_id);
        self.edit = None;
        self.is_awaiting_echo = false;
        self.error_message = None;
        self.notice = None;
        self.follow_stored_roles(window, cx);
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit = None;
        self.is_awaiting_echo = false;
        self.save_error = None;
        self.refresh(cx);
        self.show_name(true, window, cx);
    }

    fn create_role(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_changes(cx) {
            self.local_attention += 1;
            cx.notify();
            return;
        }
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(RolesBusy::Create);
        self.error_message = None;
        cx.notify();
        let catalogue = strings(cx);
        let request = CreateRoleRequest {
            name: catalogue
                .server_settings()
                .roles()
                .new_role_name()
                .to_owned(),
            color: NO_ROLE_COLOR,
            permissions: Permissions::empty(),
            is_displayed_separately: false,
            is_mentionable: false,
        };
        let guild_id = self.guild_id;
        let page = cx.entity().downgrade();
        let window_handle = window.window_handle();
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "create role",
                move |rest| async move { rest.create_role(guild_id, &request).await },
                cx,
                move |answer, cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        let _ = page.update(cx, |page, cx| {
                            page.busy = None;
                            match answer {
                                // Selected by id: it shows once the gateway brings it, if not
                                // already there.
                                Ok(role) => {
                                    page.selected_role_id = Some(role.id);
                                    page.role_id_to_focus_when_shown = Some(role.id);
                                    page.edit = None;
                                    page.tab = RoleTab::Display;
                                    page.follow_stored_roles(window, cx);
                                }
                                Err(error) => {
                                    page.error_message = Some(
                                        crate::sign_in_form::describe_error(
                                            &error,
                                            strings(cx).server_settings().roles().create_failed(),
                                            crate::strings_pending_i18n::strings(cx),
                                        )
                                        .into(),
                                    );
                                }
                            }
                            cx.notify();
                        });
                    });
                },
            );
        });
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() || !self.has_changes(cx) {
            return;
        }
        let Some((role, _)) = self.selected_role(cx) else {
            return;
        };
        let Some(draft) = self.draft(&role) else {
            return;
        };
        let words = strings(cx).server_settings().roles();
        let Some(body) = role_update_body(&role, &draft, role.id == self.guild_id) else {
            self.save_error = Some(words.name_required().into());
            cx.notify();
            return;
        };
        self.busy = Some(RolesBusy::Save);
        self.save_error = None;
        cx.notify();
        let (guild_id, role_id) = (self.guild_id, role.id);
        let saved_generation = self.edit_generation;
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                "update role",
                words.save_failed().to_owned(),
                move |rest| async move { rest.update_role(guild_id, role_id, &body).await },
                cx,
                move |outcome, cx| {
                    let _ = page.update(cx, |page, cx| {
                        page.finish_save(saved_generation, outcome, cx);
                    });
                },
            );
        });
    }

    fn finish_save(
        &mut self,
        saved_generation: u32,
        outcome: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        self.busy = None;
        match outcome {
            // The edit stays until the gateway brings the saved role, which ends it (unless the
            // echo came first: `refresh` drops it then). A change made since the save was sent
            // is not saved, so it stays an unsaved change.
            Ok(()) => {
                self.is_awaiting_echo =
                    self.edit.is_some() && self.edit_generation == saved_generation;
                self.notice = Some(strings(cx).server_settings().unsaved().saved().into());
            }
            Err(message) => self.save_error = Some(message.into()),
        }
        self.refresh(cx);
    }

    fn move_role(&mut self, role_id: Snowflake, direction: MoveDirection, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(view) = self.view(cx) else {
            return;
        };
        let Some(role_ids) = role_ids_after_move(&view.guild, &view.manager, role_id, direction)
        else {
            return;
        };
        self.busy = Some(RolesBusy::Move);
        self.error_message = None;
        cx.notify();
        let guild_id = self.guild_id;
        let page = cx.entity().downgrade();
        let failure = strings(cx)
            .server_settings()
            .roles()
            .move_failed()
            .to_owned();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                "move role",
                failure,
                move |rest| async move {
                    rest.reorder_roles(guild_id, &ReorderRolesRequest { role_ids })
                        .await
                },
                cx,
                move |outcome, cx| {
                    let _ = page.update(cx, |page, cx| {
                        page.busy = None;
                        page.error_message = outcome.err().map(SharedString::from);
                        cx.notify();
                    });
                },
            );
        });
    }

    fn open_delete_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((role, true)) = self.selected_role(cx) else {
            return;
        };
        if self.busy.is_some() || role.id == self.guild_id {
            return;
        }
        let words = strings(cx).server_settings().roles();
        let cancel = strings(cx).common().cancel();
        let dialog = cx.new(|cx| {
            ModalDialog::new(words.delete_title(&role.name), cancel, window, cx)
                .subtitle(words.delete_body(&role.name))
                .submit(words.delete_role(), words.deleting(), true)
        });
        let role_id = role.id;
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            move |page, dialog, event: &ModalDialogEvent, window, cx| match event {
                ModalDialogEvent::Closed => {
                    page.delete_dialog = None;
                    cx.notify();
                }
                ModalDialogEvent::Submitted => page.delete_role(role_id, dialog, window, cx),
            },
        );
        self.delete_dialog = Some((dialog, vec![subscription]));
        cx.notify();
    }

    fn delete_role(
        &mut self,
        role_id: Snowflake,
        dialog: &Entity<ModalDialog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        dialog.update(cx, |dialog, cx| {
            dialog.set_error(None, cx);
            dialog.set_busy(true, cx);
        });
        self.busy = Some(RolesBusy::Delete);
        let guild_id = self.guild_id;
        let page = cx.entity().downgrade();
        let dialog = dialog.downgrade();
        let window_handle = window.window_handle();
        let failure = strings(cx)
            .server_settings()
            .roles()
            .delete_failed()
            .to_owned();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                "delete role",
                failure,
                move |rest| async move { rest.delete_role(guild_id, role_id).await },
                cx,
                move |outcome, cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        let _ = dialog.update(cx, |dialog, cx| {
                            dialog.set_busy(false, cx);
                            match &outcome {
                                Ok(()) => dialog.request_close(window, cx),
                                Err(message) => {
                                    dialog.set_error(Some(message.clone().into()), cx);
                                }
                            }
                        });
                        let _ = page.update(cx, |page, cx| {
                            page.busy = None;
                            if outcome.is_ok() {
                                page.edit = None;
                                page.selected_role_id = None;
                                page.follow_stored_roles(window, cx);
                            }
                            cx.notify();
                        });
                    });
                },
            );
        });
    }

    fn row_focus(&mut self, role_id: Snowflake, cx: &mut App) -> FocusHandle {
        self.row_focus
            .entry(role_id)
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone()
    }

    fn icon_editor(
        &mut self,
        role_id: Snowflake,
        cx: &mut Context<Self>,
    ) -> Entity<RoleIconEditor> {
        if let Some((shown_id, editor)) = &self.icon_editor
            && *shown_id == role_id
        {
            return editor.clone();
        }
        let session = self.session.clone();
        let guild_id = self.guild_id;
        let editor = cx.new(|cx| RoleIconEditor::new(session, guild_id, role_id, cx));
        self.icon_editor = Some((role_id, editor.clone()));
        editor
    }
}

// --- Drawing -------------------------------------------------------------------------------------

/// A role's colour, or the neutral one for a role without.
fn role_fill(colour: u32, cx: &App) -> Hsla {
    if colour == NO_ROLE_COLOR {
        color(theme(cx).component_roles_neutral_color)
    } else {
        rgb(colour).into()
    }
}

pub(super) fn role_dot(colour: u32, size: f32, cx: &App) -> impl IntoElement {
    div()
        .flex_none()
        .size(length(size))
        .rounded_full()
        .bg(role_fill(colour, cx))
}

impl RolesPage {
    fn role_row(
        &mut self,
        role: &Role,
        view: &RolesView,
        is_selected: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let words = strings(cx).server_settings().roles();
        let is_locked = !can_manage_role(&view.manager, role, view.guild.id);
        let focus = self.row_focus(role.id, cx);
        let row_id = gpui::ElementId::from(("role-row", role.id.raw()));
        let hover = remembered_hover(&row_id, window, cx);
        let is_hovered = *hover.read(cx);
        let (text, background, count) = if is_selected {
            (
                tokens.component_roles_row_text_selected,
                Some(tokens.component_roles_row_background_selected),
                tokens.component_roles_row_count_text_selected,
            )
        } else if is_hovered {
            (
                tokens.component_roles_row_text_hover,
                Some(tokens.component_roles_row_background_hover),
                tokens.component_roles_row_count_text,
            )
        } else {
            (
                tokens.component_roles_row_text,
                None,
                tokens.component_roles_row_count_text,
            )
        };
        let text = colour_transition(child_id(&row_id, "text"), color(text), window, cx);
        let member_count = view
            .member_count_by_role
            .get(&role.id)
            .copied()
            .unwrap_or(0);
        let count_text = words.member_count(i64::try_from(member_count).unwrap_or(i64::MAX));
        let name = if is_locked {
            words.locked_role(&role.name)
        } else {
            role.name.clone()
        };
        let is_ring_shown = is_focus_visible(&focus, window);
        let role_id = role.id;
        let page = cx.entity().downgrade();
        let move_page = page.clone();
        with_type(div(), TYPE_CHANNEL_NAME)
            .id(row_id)
            .role(AccessRole::Button)
            .aria_label(SharedString::from(name))
            .aria_description(SharedString::from(count_text.clone()))
            .when(is_selected, |row| row.aria_selected(true))
            .track_focus(&focus)
            .relative()
            .flex()
            .items_center()
            .gap(length(COMPONENT_ROLES_ROW_ICON_GAP))
            .w_full()
            .min_h(length(COMPONENT_ROLES_ROW_HEIGHT.max(SIZE_HIT_WEB_MIN)))
            .px(length(COMPONENT_ROLES_ROW_PADDING_INLINE))
            .rounded(length(COMPONENT_ROLES_ROW_RADIUS))
            .when_some(background, |row, background| row.bg(color(background)))
            .text_color(text)
            .cursor_pointer()
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .on_click(move |_, window, cx| {
                let _ = page.update(cx, |page, cx| page.select(role_id, window, cx));
            })
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if let Some(direction) = move_direction_of(event) {
                    cx.stop_propagation();
                    let _ = move_page.update(cx, |page, cx| page.move_role(role_id, direction, cx));
                }
            })
            .child(role_dot(role.color, COMPONENT_ROLES_ROW_DOT_SIZE, cx))
            .child(div().min_w_0().flex_1().truncate().child(role.name.clone()))
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .flex_none()
                    .text_color(color(count))
                    .child(count_text),
            )
            .when(is_locked, |row| {
                row.child(icon(
                    IconName::Lock,
                    COMPONENT_ROLES_ROW_LOCK_ICON_SIZE,
                    color(tokens.component_roles_row_lock_icon),
                ))
            })
            .when(is_ring_shown, |row| {
                row.child(inset_focus_ring(COMPONENT_ROLES_ROW_RADIUS, cx))
            })
    }

    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn swatches(
        &self,
        shown: &RoleDraft,
        is_editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let words = strings(cx).server_settings().roles();
        let choices: Vec<u32> = colour_choices().collect();
        // A colour outside the presets (older presets, the API) checks no swatch, as the web does.
        let checked_index = choices.iter().position(|choice| *choice == shown.color);
        let tab_stop_index = checked_index.unwrap_or(0);
        let page = cx.entity().downgrade();
        let swatch = |index: usize, colour: u32, window: &mut Window, cx: &mut Context<Self>| {
            let is_checked = Some(index) == checked_index;
            let focus = self.swatch_focus[index]
                .clone()
                .tab_stop(index == tab_stop_index);
            let label: SharedString = if colour == NO_ROLE_COLOR {
                words.no_color().into()
            } else {
                words.color_option(&role_color_hex(colour)).into()
            };
            let is_ring_shown = is_focus_visible(&focus, window);
            let page = page.clone();
            let shown = shown.clone();
            let fill = role_fill(colour, cx);
            div()
                .id(("role-colour", index))
                .role(AccessRole::RadioButton)
                .aria_label(label)
                .aria_toggled(if is_checked {
                    Toggled::True
                } else {
                    Toggled::False
                })
                .aria_position_in_set(index + 1)
                .aria_size_of_set(choices_len())
                .track_focus(&focus)
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(length(SIZE_HIT_WEB_MIN))
                .rounded(length(COMPONENT_ROLES_SWATCH_RADIUS))
                .when(is_editable, Styled::cursor_pointer)
                .on_click(move |_, _, cx| {
                    if is_editable {
                        let _ = page.update(cx, |page, cx| {
                            page.change(
                                RoleDraft {
                                    color: colour,
                                    ..shown.clone()
                                },
                                cx,
                            );
                        });
                    }
                })
                .child(
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(length(COMPONENT_ROLES_SWATCH_SIZE))
                        .rounded(length(COMPONENT_ROLES_SWATCH_RADIUS))
                        .bg(fill)
                        .when(is_checked, |tile| {
                            let ring_distance = COMPONENT_ROLES_SWATCH_RING_OFFSET
                                + COMPONENT_ROLES_SWATCH_RING_WIDTH;
                            tile.child(
                                div()
                                    .absolute()
                                    .top(-length(ring_distance))
                                    .left(-length(ring_distance))
                                    .right(-length(ring_distance))
                                    .bottom(-length(ring_distance))
                                    .rounded(length(COMPONENT_ROLES_SWATCH_RADIUS + ring_distance))
                                    .border(length(COMPONENT_ROLES_SWATCH_RING_WIDTH))
                                    .border_color(color(tokens.component_roles_swatch_ring)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(length(COMPONENT_ROLES_SWATCH_CHECK_PLATE_SIZE))
                                    .rounded_full()
                                    .bg(color(tokens.component_roles_swatch_check_plate))
                                    .child(icon(
                                        IconName::Check,
                                        COMPONENT_ROLES_SWATCH_CHECK_SIZE,
                                        color(tokens.component_roles_swatch_check),
                                    )),
                            )
                        }),
                )
                .when(is_ring_shown, |swatch| {
                    swatch.child(focus_ring(COMPONENT_ROLES_SWATCH_RADIUS, cx))
                })
        };
        let no_colour = swatch(0, NO_ROLE_COLOR, window, cx);
        let presets: Vec<_> = ROLE_COLOR_PRESETS
            .iter()
            .enumerate()
            .map(|(offset, colour)| swatch(offset + 1, *colour, window, cx))
            .collect();
        let focus_handles = self.swatch_focus.clone();
        let key_page = cx.entity().downgrade();
        let shown = shown.clone();
        div()
            .id("role-colours")
            .role(AccessRole::RadioGroup)
            .aria_label(SharedString::from(words.color()))
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let Some(next) =
                    next_choice_index(&event.keystroke.key, tab_stop_index, choices.len())
                else {
                    return;
                };
                cx.stop_propagation();
                if !is_editable {
                    return;
                }
                if let (Some(colour), Some(focus)) = (choices.get(next), focus_handles.get(next)) {
                    window.focus(focus, cx);
                    let _ = key_page.update(cx, |page, cx| {
                        page.change(
                            RoleDraft {
                                color: *colour,
                                ..shown.clone()
                            },
                            cx,
                        );
                    });
                }
            })
            .child(
                with_type(div(), TYPE_CATEGORY)
                    .text_color(color(tokens.component_settings_section_label_text))
                    .child(words.color()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(length(COMPONENT_ROLES_SWATCH_GAP))
                    .child(no_colour)
                    .child(
                        with_type(div(), TYPE_BODY)
                            .text_color(color(tokens.component_settings_body_text))
                            .child(words.no_color()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .max_w(length(SWATCH_GRID_COLUMNS * SIZE_HIT_WEB_MIN))
                    .children(presets),
            )
    }

    /// A row of the Display tab or of a permission group: words on the leading side, a switch.
    fn switch_row(
        id: impl Into<gpui::ElementId>,
        label: &'static str,
        description: &'static str,
        badge: Option<&'static str>,
        switch: Switch,
        cx: &App,
    ) -> impl IntoElement {
        let tokens = theme(cx);
        let mut title = with_type(div(), TYPE_CHANNEL_NAME)
            .flex()
            .flex_wrap()
            .items_center()
            .gap(length(COMPONENT_ROLES_SENSITIVE_GAP))
            .text_color(color(tokens.component_roles_permission_label_text))
            .child(label);
        if let Some(badge) = badge {
            title = title.child(
                with_type(div(), TYPE_CATEGORY)
                    .flex()
                    .items_center()
                    .gap(length(COMPONENT_PERMISSION_BADGE_GAP))
                    .h(length(COMPONENT_PERMISSION_BADGE_HEIGHT))
                    .px(length(COMPONENT_PERMISSION_BADGE_PADDING_INLINE))
                    .rounded(length(COMPONENT_PERMISSION_BADGE_RADIUS))
                    .bg(color(tokens.component_permission_badge_background))
                    .text_color(color(tokens.component_permission_badge_text))
                    .child(icon(
                        IconName::ShieldWarning,
                        COMPONENT_PERMISSION_BADGE_ICON_SIZE,
                        color(tokens.component_permission_badge_text),
                    ))
                    .child(badge),
            );
        }
        div()
            .id(id.into())
            .flex()
            .items_center()
            .gap(length(COMPONENT_ROLES_PERMISSION_GAP))
            .py(length(COMPONENT_ROLES_PERMISSION_PADDING_BLOCK))
            .border_b_1()
            .border_color(color(tokens.component_roles_permission_divider))
            .child(
                div().min_w_0().flex_1().child(title).child(
                    with_type(div(), TYPE_BODY)
                        .text_color(color(tokens.component_roles_permission_description_text))
                        .child(description),
                ),
            )
            .child(switch)
    }

    fn note(text: &'static str, cx: &App) -> impl IntoElement {
        let note_color = color(theme(cx).component_roles_note_text);
        with_type(div(), TYPE_BODY)
            .flex()
            .gap(length(COMPONENT_SETTINGS_ERROR_ICON_GAP))
            .text_color(note_color)
            .child(icon(
                IconName::Lock,
                COMPONENT_SETTINGS_ERROR_ICON_SIZE,
                note_color,
            ))
            .child(div().min_w_0().child(text))
    }

    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn display_tab(
        &mut self,
        role: &Role,
        shown: &RoleDraft,
        is_editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let tokens = theme(cx);
        let words = strings(cx).server_settings().roles();
        let page = cx.entity().downgrade();
        let preview_name = if shown.name.trim().is_empty() {
            role.name.clone()
        } else {
            shown.name.trim().to_owned()
        };
        let preview_color = if shown.color == NO_ROLE_COLOR {
            color(tokens.color_text_heading)
        } else {
            let readable = readable_role_colour(shown.color, tokens.color_surface_chat >> 8);
            color((readable << 8) | 0xFF)
        };
        let colours = self.swatches(shown, is_editable, window, cx);
        let separately_page = page.clone();
        let separately_shown = shown.clone();
        let mentionable_page = page.clone();
        let mentionable_shown = shown.clone();
        let icon_editor = self.icon_editor(role.id, cx);
        let mut parts = vec![
            div()
                .max_w(length(COMPONENT_DIALOG_WIDTH))
                .child(
                    TextField::new("role-name", words.role_name(), &self.name_input)
                        .read_only(!is_editable),
                )
                .into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                .child(colours)
                .child(
                    with_type(div(), TYPE_BODY)
                        .flex()
                        .flex_wrap()
                        .gap(length(COMPONENT_ROLES_SENSITIVE_GAP))
                        .text_color(color(tokens.component_roles_note_text))
                        .child(words.color_preview())
                        .child(
                            with_type(div(), TYPE_MEMBER_NAME)
                                .text_color(preview_color)
                                .child(preview_name),
                        ),
                )
                .into_any_element(),
            Self::switch_row(
                "role-displayed-separately",
                words.displayed_separately(),
                words.displayed_separately_description(),
                None,
                Switch::new(
                    "role-displayed-separately-switch",
                    shown.is_displayed_separately,
                    words.displayed_separately(),
                )
                .description(words.displayed_separately_description())
                .locked(!is_editable)
                .on_change(move |is_on, _, cx| {
                    let _ = separately_page.update(cx, |page, cx| {
                        page.change(
                            RoleDraft {
                                is_displayed_separately: is_on,
                                ..separately_shown.clone()
                            },
                            cx,
                        );
                    });
                }),
                cx,
            )
            .into_any_element(),
            // ADR-0040: on the Display tab, which @everyone never shows.
            Self::switch_row(
                "role-mentionable",
                words.mentionable(),
                words.mentionable_description(),
                None,
                Switch::new(
                    "role-mentionable-switch",
                    shown.is_mentionable,
                    words.mentionable(),
                )
                .description(words.mentionable_description())
                .locked(!is_editable)
                .on_change(move |is_on, _, cx| {
                    let _ = mentionable_page.update(cx, |page, cx| {
                        page.change(
                            RoleDraft {
                                is_mentionable: is_on,
                                ..mentionable_shown.clone()
                            },
                            cx,
                        );
                    });
                }),
                cx,
            )
            .into_any_element(),
            icon_editor.into_any_element(),
        ];
        if is_editable {
            parts.push(
                div()
                    .child(
                        Button::new("role-delete", words.delete_role())
                            .variant(ButtonVariant::Danger)
                            .disabled(self.busy.is_some())
                            .on_click(move |window, cx| {
                                let _ =
                                    page.update(cx, |page, cx| page.open_delete_dialog(window, cx));
                            }),
                    )
                    .into_any_element(),
            );
        }
        parts
    }

    fn permissions_tab(
        view: &RolesView,
        shown: &RoleDraft,
        is_editable: bool,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.server_settings().roles();
        let has_locked_permissions = PERMISSION_GROUPS
            .iter()
            .flat_map(|group| group.permissions)
            .any(|permission| !can_toggle_permission(&view.manager, permission.bit));
        let mut parts = Vec::new();
        if is_editable && has_locked_permissions {
            parts.push(Self::note(words.permission_locked(), cx).into_any_element());
        }
        for (group_index, group) in PERMISSION_GROUPS.iter().enumerate() {
            let rows = group.permissions.iter().map(|permission| {
                let (label, description) = permission_words(permission.key, catalogue);
                let is_locked =
                    !is_editable || !can_toggle_permission(&view.manager, permission.bit);
                let page = cx.entity().downgrade();
                let shown = shown.clone();
                let bit = permission.bit;
                let switch_description = if is_editable && is_locked {
                    format!("{description} {}", words.permission_locked())
                } else {
                    description.to_owned()
                };
                Self::switch_row(
                    SharedString::from(format!("permission-{:?}", permission.key)),
                    label,
                    description,
                    permission.is_sensitive.then(|| words.sensitive()),
                    Switch::new(
                        SharedString::from(format!("permission-switch-{:?}", permission.key)),
                        shown.permissions.contains(bit),
                        label,
                    )
                    .description(switch_description)
                    .locked(is_locked)
                    .on_change(move |_, _, cx| {
                        let _ = page.update(cx, |page, cx| {
                            page.change(toggle_permission(&shown, bit), cx);
                        });
                    }),
                    cx,
                )
                .into_any_element()
            });
            parts.push(
                div()
                    .when(group_index > 0, |section| {
                        section.mt(length(COMPONENT_ROLES_PERMISSION_GROUP_GAP))
                    })
                    .child(
                        with_type(div(), TYPE_CATEGORY)
                            .id(("permission-group", group_index))
                            .role(AccessRole::Heading)
                            .text_color(color(tokens.component_settings_section_label_text))
                            .child(permission_group_label(group.group, catalogue)),
                    )
                    .children(rows.collect::<Vec<_>>())
                    .into_any_element(),
            );
        }
        parts
    }

    #[allow(clippy::too_many_lines)] // The editor's element tree, in the order it is drawn.
    fn editor(
        &mut self,
        view: &RolesView,
        role: &Role,
        is_editable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let words = strings(cx).server_settings().roles();
        let shown = self.draft(role).unwrap_or_else(|| draft_of(role));
        let is_everyone = role.id == view.guild.id;
        let page = cx.entity().downgrade();
        let mut header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(length(COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP))
            .child(
                with_type(div(), TYPE_HEADING_MD)
                    .id("role-editor-title")
                    .role(AccessRole::Heading)
                    .mr_auto()
                    .flex()
                    .min_w_0()
                    .items_center()
                    .gap(length(COMPONENT_ROLES_ROW_ICON_GAP))
                    .text_color(color(tokens.component_settings_page_title_text))
                    .child(role_dot(shown.color, COMPONENT_ROLES_ROW_DOT_SIZE, cx))
                    .child(div().truncate().child(role.name.clone())),
            );
        if !is_everyone {
            let can_move_up =
                role_ids_after_move(&view.guild, &view.manager, role.id, MoveDirection::Up)
                    .is_some();
            let can_move_down =
                role_ids_after_move(&view.guild, &view.manager, role.id, MoveDirection::Down)
                    .is_some();
            let role_id = role.id;
            let up_page = page.clone();
            let down_page = page.clone();
            header = header
                .child(
                    Button::new("role-move-up", words.move_up())
                        .variant(ButtonVariant::Ghost)
                        .disabled(!can_move_up || self.busy.is_some())
                        .on_click(move |_, cx| {
                            let _ = up_page.update(cx, |page, cx| {
                                page.move_role(role_id, MoveDirection::Up, cx);
                            });
                        }),
                )
                .child(
                    Button::new("role-move-down", words.move_down())
                        .variant(ButtonVariant::Ghost)
                        .disabled(!can_move_down || self.busy.is_some())
                        .on_click(move |_, cx| {
                            let _ = down_page.update(cx, |page, cx| {
                                page.move_role(role_id, MoveDirection::Down, cx);
                            });
                        }),
                );
        }
        let shown_tab = if is_everyone {
            RoleTab::Permissions
        } else {
            self.tab
        };
        let panel = match shown_tab {
            RoleTab::Display => self.display_tab(role, &shown, is_editable, window, cx),
            RoleTab::Permissions => Self::permissions_tab(view, &shown, is_editable, cx),
        };
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .flex_grow(3.0)
            .flex_shrink(1.0)
            .flex_basis(length(COMPONENT_PROFILE_POPOVER_WIDTH))
            .gap(length(COMPONENT_ROLES_EDITOR_GAP))
            .child(header)
            .when(!is_editable, |editor| {
                editor.child(Self::note(words.locked(), cx))
            })
            .map(|editor| {
                if is_everyone {
                    editor.child(
                        with_type(div(), TYPE_BODY)
                            .text_color(color(tokens.component_settings_body_text))
                            .child(words.everyone_note()),
                    )
                } else {
                    let tab_page = page.clone();
                    editor.child(
                        Tabs::new(
                            "role-tabs",
                            role.name.clone(),
                            vec![words.display().into(), words.permissions().into()],
                            usize::from(shown_tab == RoleTab::Permissions),
                        )
                        .on_select(move |index, _, cx| {
                            let _ = tab_page.update(cx, |page, cx| {
                                page.tab = if index == 0 {
                                    RoleTab::Display
                                } else {
                                    RoleTab::Permissions
                                };
                                cx.notify();
                            });
                        }),
                    )
                }
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_ROLES_EDITOR_GAP))
                    .children(panel),
            )
    }
}

const fn choices_len() -> usize {
    ROLE_COLOR_PRESETS.len() + 1
}

impl Render for RolesPage {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(view) = self.view(cx) else {
            return div().into_any_element();
        };
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.server_settings().roles();
        let selected = self.selected_role(cx);
        let has_changes = self.has_changes(cx);
        let roles: Vec<Role> = roles_highest_first(&view.guild)
            .into_iter()
            .cloned()
            .collect();
        let page = cx.entity().downgrade();
        let rows: Vec<_> = roles
            .iter()
            .map(|role| {
                let is_selected = selected
                    .as_ref()
                    .is_some_and(|(shown, _)| shown.id == role.id);
                self.role_row(role, &view, is_selected, window, cx)
                    .into_any_element()
            })
            .collect();
        let is_move_hint_shown = can_move_any_role(&view.guild, &view.manager);
        let create_label = if self.busy == Some(RolesBusy::Create) {
            words.creating()
        } else {
            words.create_role()
        };
        let create_page = page.clone();
        let list = div()
            .flex()
            .flex_col()
            .min_w_0()
            .flex_grow(1.0)
            .flex_shrink(1.0)
            .flex_basis(length(COMPONENT_ROLES_LIST_WIDTH))
            .gap(length(COMPONENT_ROLES_ROW_GAP))
            .child(
                div()
                    .id("role-list")
                    .role(AccessRole::List)
                    .aria_label(SharedString::from(words.role_list()))
                    .when(is_move_hint_shown, |list| {
                        list.aria_description(SharedString::from(words.move_hint()))
                    })
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_ROLES_ROW_GAP))
                    .children(rows),
            )
            .when(is_move_hint_shown, |list| {
                list.child(
                    with_type(div(), TYPE_BODY_SM)
                        .mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                        .text_color(color(tokens.component_roles_note_text))
                        .child(words.move_hint()),
                )
            });
        let editor = selected
            .as_ref()
            .map(|(role, is_editable)| self.editor(&view, role, *is_editable, window, cx));
        let reset_page = page.clone();
        let save_page = page;
        div()
            .flex()
            .flex_col()
            .flex_1()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(length(COMPONENT_SETTINGS_ACTION_GAP))
                    .child(
                        Button::new("role-create", create_label)
                            .disabled(self.busy.is_some())
                            .on_click(move |window, cx| {
                                let _ =
                                    create_page.update(cx, |page, cx| page.create_role(window, cx));
                            }),
                    )
                    .when(roles.len() == 1, |row| {
                        row.child(
                            with_type(div(), TYPE_BODY)
                                .text_color(color(tokens.component_settings_body_text))
                                .child(words.empty()),
                        )
                    }),
            )
            .child(
                div()
                    .when(self.error_message.is_some(), |alert| {
                        alert.mt(length(COMPONENT_SETTINGS_ACTION_GAP))
                    })
                    .child(error_alert("roles-error", self.error_message.clone(), cx)),
            )
            .child(
                div()
                    .mt(length(COMPONENT_SETTINGS_PAGE_TITLE_GAP))
                    .flex()
                    .when(is_wide(window), |body| body.flex_row().items_start())
                    .when(!is_wide(window), Styled::flex_col)
                    .gap(length(COMPONENT_ROLES_LIST_GAP))
                    .child(list)
                    .children(editor),
            )
            .child(saved_notice(
                "roles-notice",
                self.notice.clone().filter(|_| !has_changes),
                cx,
            ))
            .child(div().flex_1())
            .when(has_changes, |column| {
                let mut bar = UnsavedChangesBar::new(
                    "roles-unsaved",
                    unsaved_changes_labels(catalogue),
                    Arc::new(move |window, cx| {
                        let _ = reset_page.update(cx, |page, cx| page.reset(window, cx));
                    }),
                    Arc::new(move |_, cx| {
                        let _ = save_page.update(cx, Self::save);
                    }),
                )
                .attention(self.attention + self.local_attention)
                .saving(self.busy == Some(RolesBusy::Save));
                if let Some(error) = self.save_error.clone() {
                    bar = bar.error_message(error);
                }
                column.child(bar)
            })
            .when_some(self.delete_dialog.as_ref(), |column, (dialog, _)| {
                column.child(dialog.clone())
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use gpui::Keystroke;

    use super::*;

    fn key_down(keystroke: &'static str) -> KeyDownEvent {
        KeyDownEvent {
            keystroke: Keystroke::parse(keystroke).expect("a valid keystroke in a test"),
            is_held: false,
            prefer_character_input: false,
        }
    }

    #[test]
    fn ctrl_or_cmd_with_shift_and_an_arrow_moves_a_role() {
        assert_eq!(
            move_direction_of(&key_down("ctrl-shift-up")),
            Some(MoveDirection::Up)
        );
        assert_eq!(
            move_direction_of(&key_down("cmd-shift-down")),
            Some(MoveDirection::Down)
        );
        assert_eq!(move_direction_of(&key_down("shift-up")), None);
        assert_eq!(move_direction_of(&key_down("ctrl-up")), None);
    }

    #[test]
    fn every_colour_choice_has_a_place_in_the_group() {
        assert_eq!(colour_choices().count(), choices_len());
        assert_eq!(colour_choices().next(), Some(NO_ROLE_COLOR));
    }

    mod headless {
        use gpui::{Entity, TestAppContext, VisualTestContext};
        use mellow_client_core::shared_logic::server_settings::ServerSettingsSection;
        use mellow_protocol::GatewayEvent;

        use super::super::*;
        use crate::settings::server::test_support::{
            ADMINS, open_server_settings, receive, shown_page,
        };
        use crate::shell_fixtures::{GUILD_RUST, MODERATORS};

        fn selected_id(page: &Entity<RolesPage>, cx: &mut VisualTestContext) -> Option<Snowflake> {
            page.read_with(cx, |page, cx| {
                page.selected_role(cx).map(|(role, _)| role.id)
            })
        }

        fn toggle_send_messages(page: &Entity<RolesPage>, cx: &mut VisualTestContext) {
            page.update(cx, |page, cx| {
                let shown = page.shown_draft(cx).expect("a role is shown");
                page.change(toggle_permission(&shown, Permissions::SEND_MESSAGES), cx);
            });
            cx.run_until_parked();
        }

        fn roles_with_moderators_sending(
            cx: &mut VisualTestContext,
            session: &Entity<Session>,
        ) -> Vec<Role> {
            session.read_with(cx, |session, _| {
                let mut roles = session
                    .state()
                    .guild(GUILD_RUST)
                    .expect("Rust Lyon")
                    .roles
                    .clone();
                for role in &mut roles {
                    if role.id == MODERATORS {
                        role.permissions |= Permissions::SEND_MESSAGES;
                    }
                }
                roles
            })
        }

        #[gpui::test]
        fn opens_on_the_highest_role_the_reader_may_edit(cx: &mut TestAppContext) {
            let (_, settings, cx) = open_server_settings(cx, ServerSettingsSection::Roles);
            let page = shown_page::<RolesPage>(&settings, cx);
            assert_eq!(selected_id(&page, cx), Some(MODERATORS));
            let name = page.read_with(cx, |page, cx| page.name_input.read(cx).value().to_string());
            assert_eq!(name, "Moderators");
        }

        #[gpui::test]
        fn a_draft_waits_for_the_gateway_to_bring_the_saved_role(cx: &mut TestAppContext) {
            let (session, settings, cx) = open_server_settings(cx, ServerSettingsSection::Roles);
            let page = shown_page::<RolesPage>(&settings, cx);
            toggle_send_messages(&page, cx);
            assert!(page.read_with(cx, RolesPage::has_changes));
            // Another role cannot be chosen meanwhile: the bar asks to save or reset first.
            page.update_in(cx, |page, window, cx| page.select(GUILD_RUST, window, cx));
            assert_eq!(selected_id(&page, cx), Some(MODERATORS));
            assert_eq!(page.read_with(cx, |page, _| page.local_attention), 1);

            let roles = roles_with_moderators_sending(cx, &session);
            receive(
                &session,
                GatewayEvent::GuildRolesUpdate {
                    guild_id: GUILD_RUST,
                    roles,
                },
                cx,
            );
            assert!(
                page.read_with(cx, |page, cx| !page.has_changes(cx) && page.edit.is_none()),
                "the stored role now matches the draft"
            );
        }

        #[gpui::test]
        fn roles_at_or_above_the_reader_are_shown_but_never_changed(cx: &mut TestAppContext) {
            let (_, settings, cx) = open_server_settings(cx, ServerSettingsSection::Roles);
            let page = shown_page::<RolesPage>(&settings, cx);
            page.update_in(cx, |page, window, cx| page.select(ADMINS, window, cx));
            let is_editable = page.read_with(cx, |page, cx| {
                page.selected_role(cx).map(|(_, editable)| editable)
            });
            assert_eq!(is_editable, Some(false));
            toggle_send_messages(&page, cx);
            assert!(page.read_with(cx, |page, cx| !page.has_changes(cx)));
        }

        #[gpui::test]
        fn a_failed_save_keeps_the_draft_and_says_why(cx: &mut TestAppContext) {
            let (_, settings, cx) = open_server_settings(cx, ServerSettingsSection::Roles);
            let page = shown_page::<RolesPage>(&settings, cx);
            toggle_send_messages(&page, cx);
            // The test session has no connection: the instance never answers yes.
            page.update(cx, RolesPage::save);
            cx.run_until_parked();
            page.read_with(cx, |page, cx| {
                assert!(page.busy.is_none());
                assert!(page.save_error.is_some());
                assert!(page.has_changes(cx));
            });
        }

        #[gpui::test]
        fn a_role_deleted_elsewhere_gives_way_to_the_next_editable_one(cx: &mut TestAppContext) {
            let (session, settings, cx) = open_server_settings(cx, ServerSettingsSection::Roles);
            let page = shown_page::<RolesPage>(&settings, cx);
            toggle_send_messages(&page, cx);
            let roles = session.read_with(cx, |session, _| {
                session
                    .state()
                    .guild(GUILD_RUST)
                    .expect("Rust Lyon")
                    .roles
                    .iter()
                    .filter(|role| role.id != MODERATORS)
                    .cloned()
                    .collect()
            });
            receive(
                &session,
                GatewayEvent::GuildRolesUpdate {
                    guild_id: GUILD_RUST,
                    roles,
                },
                cx,
            );
            assert_eq!(selected_id(&page, cx), Some(GUILD_RUST));
            assert!(page.read_with(cx, |page, cx| !page.has_changes(cx) && page.edit.is_none()));
        }

        #[gpui::test]
        fn a_delete_dialog_closes_when_its_role_is_deleted_elsewhere(cx: &mut TestAppContext) {
            let (session, settings, cx) = open_server_settings(cx, ServerSettingsSection::Roles);
            let page = shown_page::<RolesPage>(&settings, cx);
            page.update_in(cx, RolesPage::open_delete_dialog);
            cx.run_until_parked();
            assert!(page.read_with(cx, |page, _| page.delete_dialog.is_some()));
            let roles = session.read_with(cx, |session, _| {
                session
                    .state()
                    .guild(GUILD_RUST)
                    .expect("Rust Lyon")
                    .roles
                    .iter()
                    .filter(|role| role.id != MODERATORS)
                    .cloned()
                    .collect()
            });
            receive(
                &session,
                GatewayEvent::GuildRolesUpdate {
                    guild_id: GUILD_RUST,
                    roles,
                },
                cx,
            );
            assert!(page.read_with(cx, |page, _| page.delete_dialog.is_none()));
        }

        #[gpui::test]
        fn a_change_made_while_saving_stays_unsaved(cx: &mut TestAppContext) {
            let (_, settings, cx) = open_server_settings(cx, ServerSettingsSection::Roles);
            let page = shown_page::<RolesPage>(&settings, cx);
            toggle_send_messages(&page, cx);
            let sent_generation = page.read_with(cx, |page, _| page.edit_generation);
            // Another change while the save is in flight, then the save's answer.
            page.update(cx, |page, cx| {
                let shown = page.shown_draft(cx).expect("a role is shown");
                page.change(toggle_permission(&shown, Permissions::ADD_REACTIONS), cx);
                page.finish_save(sent_generation, Ok(()), cx);
            });
            assert!(page.read_with(cx, RolesPage::has_changes));

            // An answer covering the latest edit waits for the gateway instead.
            page.update(cx, |page, cx| {
                let latest = page.edit_generation;
                page.finish_save(latest, Ok(()), cx);
            });
            assert!(page.read_with(cx, |page, cx| !page.has_changes(cx)));
        }
    }
}
