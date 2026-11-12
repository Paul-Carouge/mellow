//! Channel Permissions (`ChannelPermissionSettings.svelte`, DESIGN.md §5.8): the roles and members
//! with an overwrite here on the leading side (`@everyone` always), the chosen one's Deny / Inherit
//! / Allow per channel permission beside it (below it when narrow). Adding a role or member and
//! removing one save at once, as on Discord; changes to the states wait for the unsaved changes
//! bar, rebased on any change made meanwhile (`rebase_overwrite_edit`). Locked rows and states say
//! why.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, ElementId, Entity, EventEmitter, FocusHandle,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Toggled, Window, div,
    prelude::FluentBuilder as _, px, rgb,
};
use mellow_client_core::shared_logic::channel_settings::{
    OVERWRITE_STATES, OverwriteBits, OverwriteEdit, OverwriteState, OverwriteTarget,
    can_change_channel_permission, can_edit_target, can_remove_overwrite,
    channel_permission_groups_for, overwrite_state_of, overwrite_targets, rebase_overwrite_edit,
    stored_overwrite_of, with_overwrite_state,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::server_settings::{NO_ROLE_COLOR, role_manager_of};
use mellow_protocol::{
    Channel, Guild, OverwriteKind, Permissions, PublicUser, SetPermissionOverwriteRequest,
    Snowflake,
};

use super::add_overwrite_target::{AddOverwriteTarget, AddOverwriteTargetEvent};
use crate::components::{
    Avatar, AvatarPerson, Button, ButtonVariant, UnsavedChangesBar, child_id, focus_ring, icon,
    inset_focus_ring, is_focus_visible, next_choice_index, remembered_focus_handle,
    remembered_hover, with_type,
};
use crate::i18n::{Strings, strings};
use crate::session::Session;
use crate::settings::permission_words::{channel_permission_words, permission_group_label};
use crate::settings::{SettingsPage, SettingsPageEvent, unsaved_changes_labels};
use crate::shell::profile_picture;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CHANNEL_SETTINGS_OVERRIDE_BORDER_WIDTH, COMPONENT_CHANNEL_SETTINGS_OVERRIDE_HEIGHT,
    COMPONENT_CHANNEL_SETTINGS_OVERRIDE_ICON_GAP, COMPONENT_CHANNEL_SETTINGS_OVERRIDE_ICON_SIZE,
    COMPONENT_CHANNEL_SETTINGS_OVERRIDE_PADDING_INLINE, COMPONENT_CHANNEL_SETTINGS_OVERRIDE_RADIUS,
    COMPONENT_CHANNEL_SETTINGS_TARGET_AVATAR_SIZE, COMPONENT_PROFILE_POPOVER_WIDTH,
    COMPONENT_ROLES_EDITOR_GAP, COMPONENT_ROLES_LIST_GAP, COMPONENT_ROLES_LIST_WIDTH,
    COMPONENT_ROLES_PERMISSION_GAP, COMPONENT_ROLES_PERMISSION_GROUP_GAP,
    COMPONENT_ROLES_PERMISSION_PADDING_BLOCK, COMPONENT_ROLES_ROW_DOT_SIZE,
    COMPONENT_ROLES_ROW_GAP, COMPONENT_ROLES_ROW_HEIGHT, COMPONENT_ROLES_ROW_ICON_GAP,
    COMPONENT_ROLES_ROW_LOCK_ICON_SIZE, COMPONENT_ROLES_ROW_PADDING_INLINE,
    COMPONENT_ROLES_ROW_RADIUS, COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP,
    COMPONENT_SETTINGS_ACTION_GAP, COMPONENT_SETTINGS_ERROR_ICON_GAP,
    COMPONENT_SETTINGS_ERROR_ICON_SIZE, COMPONENT_SETTINGS_PAGE_TITLE_GAP,
    COMPONENT_SETTINGS_SECTION_LABEL_GAP, COMPONENT_SWITCH_DISABLED_OPACITY, TYPE_BODY,
    TYPE_BODY_SM, TYPE_CATEGORY, TYPE_CHANNEL_NAME, TYPE_HEADING_MD,
};
use crate::theme::{color, length, theme};

/// The editor column grows three times as fast as the list, as the web's `flex-[3_1_…]`.
const EDITOR_FLEX_GROW: f32 = 3.0;

/// What is being sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Busy {
    Save,
    Remove,
}

/// The dot of a role, the avatar of a member.
#[derive(Debug, Clone)]
pub(super) enum TargetMark {
    /// `None`: the role has no colour.
    Role(Option<u32>),
    /// `None`: the member left or is not loaded.
    Member(Option<PublicUser>),
}

/// A target as the list shows it.
#[derive(Debug, Clone)]
struct TargetRow {
    target: OverwriteTarget,
    name: String,
    mark: TargetMark,
    is_locked: bool,
}

/// One permission's row in the editor.
#[derive(Debug, Clone)]
struct PermissionRow {
    bit: Permissions,
    label: &'static str,
    description: &'static str,
    current: OverwriteState,
    is_locked: bool,
}

#[derive(Debug, Clone)]
struct GroupView {
    label: &'static str,
    permissions: Vec<PermissionRow>,
}

/// The chosen target's editor.
#[allow(clippy::struct_excessive_bools)] // Independent facts about one target, as the web derives them.
#[derive(Debug, Clone)]
struct EditorView {
    row: TargetRow,
    is_everyone: bool,
    is_editable: bool,
    can_remove: bool,
    has_locked_permissions: bool,
    groups: Vec<GroupView>,
}

/// Everything the page draws, read from the session at once.
#[derive(Debug, Clone)]
struct PageView {
    channel_name: String,
    rows: Vec<TargetRow>,
    editor: Option<EditorView>,
}

/// A role's colour as drawn: `None` for no colour.
pub(super) fn role_colour(color: u32) -> Option<u32> {
    (color != NO_ROLE_COLOR).then_some(color)
}

pub struct ChannelPermissionsPage {
    session: Entity<Session>,
    channel_id: Snowflake,
    selected_target_id: Option<Snowflake>,
    /// Just added, until the gateway brings its overwrite: nothing else is shown in its place.
    added_target_id: Option<Snowflake>,
    edit: Option<OverwriteEdit>,
    /// Saved; the edit stays until the gateway brings the saved overwrite, which ends it.
    is_awaiting_echo: bool,
    busy: Option<Busy>,
    save_error: Option<String>,
    error_message: Option<String>,
    notice: Option<SharedString>,
    attention: u32,
    local_attention: u32,
    is_reported_changed: bool,
    add_dialog: Option<Entity<AddOverwriteTarget>>,
    add_button_focus: FocusHandle,
    row_focus: HashMap<Snowflake, FocusHandle>,
    /// Focus to give once drawn: answers come without a window.
    pending_focus: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
    /// Replaced by the next dialog, so opening it again and again keeps one.
    add_dialog_subscription: Option<Subscription>,
}

impl EventEmitter<SettingsPageEvent> for ChannelPermissionsPage {}

impl SettingsPage for ChannelPermissionsPage {
    fn set_attention(&mut self, attention: u32, cx: &mut Context<Self>) {
        self.attention = attention;
        cx.notify();
    }
}

impl std::fmt::Debug for ChannelPermissionsPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelPermissionsPage")
            .field("channel_id", &self.channel_id)
            .field("selected_target_id", &self.selected_target_id)
            .field("edit", &self.edit)
            .field("busy", &self.busy)
            .finish_non_exhaustive()
    }
}

impl ChannelPermissionsPage {
    pub fn new(
        session: Entity<Session>,
        channel_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Member targets are named from the member list.
        if let Some(guild_id) = session
            .read(cx)
            .state()
            .guild_of_channel(channel_id)
            .map(|guild| guild.id)
        {
            session.update(cx, |session, cx| session.load_members(guild_id, false, cx));
        }
        let subscriptions = vec![cx.observe_in(&session, window, |page, _, _, cx| {
            page.follow_stored_channel(cx);
        })];
        Self {
            session,
            channel_id,
            selected_target_id: None,
            added_target_id: None,
            edit: None,
            is_awaiting_echo: false,
            busy: None,
            save_error: None,
            error_message: None,
            notice: None,
            attention: 0,
            local_attention: 0,
            is_reported_changed: false,
            add_dialog: None,
            add_button_focus: cx.focus_handle(),
            row_focus: HashMap::new(),
            pending_focus: None,
            _subscriptions: subscriptions,
            add_dialog_subscription: None,
        }
    }

    fn context<'a>(&self, cx: &'a App) -> Option<(&'a Guild, &'a Channel)> {
        let state = self.session.read(cx).state();
        Some((
            state.guild_of_channel(self.channel_id)?,
            state.guild_channel(self.channel_id)?,
        ))
    }

    /// A removed target gives way to the first one, `@everyone` at worst; one just added is
    /// waited for, so the states never edit another target meanwhile.
    fn selected_target(&self, targets: &[OverwriteTarget]) -> Option<OverwriteTarget> {
        targets
            .iter()
            .find(|target| Some(target.id) == self.selected_target_id)
            .copied()
            .or_else(|| {
                let is_waiting_for_added = self.selected_target_id.is_some()
                    && self.selected_target_id == self.added_target_id;
                if is_waiting_for_added {
                    None
                } else {
                    targets.first().copied()
                }
            })
    }

    /// The edit on top of what is stored now, for `selected` only.
    fn draft_for(&self, channel: &Channel, selected: OverwriteTarget) -> Option<OverwriteBits> {
        let edit = self.edit.filter(|edit| edit.target.id == selected.id)?;
        rebase_overwrite_edit(&edit, stored_overwrite_of(channel, selected.id))
            .map(|edit| edit.draft)
    }

    /// The target shown and its unsaved overwrite, if any.
    fn selected_draft(&self, cx: &App) -> Option<(OverwriteTarget, OverwriteBits)> {
        let (guild, channel) = self.context(cx)?;
        let selected = self.selected_target(&overwrite_targets(guild, channel))?;
        Some((selected, self.draft_for(channel, selected)?))
    }

    fn has_changes(&self, cx: &App) -> bool {
        !self.is_awaiting_echo && self.selected_draft(cx).is_some()
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let has_changes = self.has_changes(cx);
        if has_changes != self.is_reported_changed {
            self.is_reported_changed = has_changes;
            cx.emit(SettingsPageEvent::UnsavedChanges(has_changes));
        }
        cx.notify();
    }

    /// Saved (the gateway's echo), undone, or the target left the list: nothing is left to save.
    /// An added target arriving ends the wait for it.
    fn follow_stored_channel(&mut self, cx: &mut Context<Self>) {
        let Some((guild, channel)) = self.context(cx) else {
            return;
        };
        let targets = overwrite_targets(guild, channel);
        if self
            .added_target_id
            .is_some_and(|added| targets.iter().any(|target| target.id == added))
        {
            self.added_target_id = None;
        }
        let is_edit_over = self.edit.is_some_and(|edit| {
            !targets.iter().any(|target| target.id == edit.target.id)
                || rebase_overwrite_edit(&edit, stored_overwrite_of(channel, edit.target.id))
                    .is_none()
        });
        if is_edit_over {
            self.edit = None;
            self.is_awaiting_echo = false;
        }
        self.refresh(cx);
    }

    fn ask_attention(&mut self, cx: &mut Context<Self>) {
        self.local_attention += 1;
        cx.notify();
    }

    fn select(&mut self, target_id: Snowflake, cx: &mut Context<Self>) {
        let Some((guild, channel)) = self.context(cx) else {
            return;
        };
        let shown = self.selected_target(&overwrite_targets(guild, channel));
        if shown.map(|target| target.id) == Some(target_id) {
            return;
        }
        if self.has_changes(cx) {
            self.ask_attention(cx);
            return;
        }
        self.selected_target_id = Some(target_id);
        self.edit = None;
        self.error_message = None;
        self.notice = None;
        self.refresh(cx);
    }

    fn change(&mut self, permission: Permissions, state: OverwriteState, cx: &mut Context<Self>) {
        let Some((guild, channel)) = self.context(cx) else {
            return;
        };
        let Some(selected) = self.selected_target(&overwrite_targets(guild, channel)) else {
            return;
        };
        let manager = role_manager_of(guild, self.current_user_id(cx));
        let channel_permissions = self.channel_permissions(cx);
        if !can_edit_target(guild, &manager, selected)
            || !can_change_channel_permission(channel_permissions, permission)
        {
            return;
        }
        let stored = stored_overwrite_of(channel, selected.id);
        let shown = self.draft_for(channel, selected).unwrap_or(stored);
        self.edit = Some(OverwriteEdit {
            target: selected,
            base: stored,
            draft: with_overwrite_state(shown, permission, state),
        });
        self.is_awaiting_echo = false;
        self.save_error = None;
        self.notice = None;
        self.refresh(cx);
    }

    fn current_user_id(&self, cx: &App) -> Option<Snowflake> {
        self.session
            .read(cx)
            .state()
            .current_user()
            .map(|user| user.id)
    }

    fn channel_permissions(&self, cx: &App) -> Option<Permissions> {
        self.session
            .read(cx)
            .state()
            .my_channel_permissions(self.channel_id)
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        self.edit = None;
        self.is_awaiting_echo = false;
        self.save_error = None;
        self.refresh(cx);
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.busy.is_some() || !self.has_changes(cx) {
            return;
        }
        let Some((target, draft)) = self.selected_draft(cx) else {
            return;
        };
        self.busy = Some(Busy::Save);
        self.save_error = None;
        cx.notify();
        let channel_id = self.channel_id;
        let request = SetPermissionOverwriteRequest {
            kind: target.kind,
            allow: draft.allow,
            deny: draft.deny,
        };
        let failure = strings(cx)
            .channel_settings()
            .permissions()
            .save_failed()
            .to_owned();
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                "set permission overwrite",
                failure,
                move |rest| async move {
                    rest.set_permission_overwrite(channel_id, target.id, &request)
                        .await
                },
                cx,
                move |outcome, cx| {
                    let _ = page.update(cx, |page, cx| {
                        page.busy = None;
                        match outcome {
                            Ok(()) => {
                                // The echo may have come first and ended the edit already.
                                page.is_awaiting_echo = page.edit.is_some();
                                page.notice =
                                    Some(strings(cx).server_settings().unsaved().saved().into());
                            }
                            Err(message) => page.save_error = Some(message),
                        }
                        page.refresh(cx);
                    });
                },
            );
        });
    }

    fn remove(&mut self, target: OverwriteTarget, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let Some(guild_id) = self.context(cx).map(|(guild, _)| guild.id) else {
            return;
        };
        self.busy = Some(Busy::Remove);
        self.error_message = None;
        cx.notify();
        let channel_id = self.channel_id;
        let failure = strings(cx)
            .channel_settings()
            .permissions()
            .remove_failed()
            .to_owned();
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                "delete permission overwrite",
                failure,
                move |rest| async move {
                    rest.delete_permission_overwrite(channel_id, target.id)
                        .await
                },
                cx,
                move |outcome, cx| {
                    let _ = page.update(cx, |page, cx| {
                        page.busy = None;
                        match outcome {
                            Ok(()) => {
                                page.edit = None;
                                // `@everyone` always stays: it is shown and takes focus, whatever
                                // the list's order.
                                page.selected_target_id = Some(guild_id);
                                page.pending_focus = Some(page.row_focus_handle(guild_id, cx));
                            }
                            Err(message) => page.error_message = Some(message),
                        }
                        page.refresh(cx);
                    });
                },
            );
        });
    }

    fn row_focus_handle(&mut self, target_id: Snowflake, cx: &mut App) -> FocusHandle {
        self.row_focus
            .entry(target_id)
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone()
    }

    fn open_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() || self.add_dialog.is_some() {
            return;
        }
        if self.has_changes(cx) {
            self.ask_attention(cx);
            return;
        }
        let dialog = AddOverwriteTarget::open(self.session.clone(), self.channel_id, window, cx);
        let subscription = cx.subscribe(&dialog, |page, _, event: &AddOverwriteTargetEvent, cx| {
            page.add_dialog = None;
            if let AddOverwriteTargetEvent::Added(target) = *event {
                page.selected_target_id = Some(target.id);
                page.added_target_id = Some(target.id);
                page.edit = None;
                page.notice = None;
                page.pending_focus = Some(page.add_button_focus.clone());
            }
            page.refresh(cx);
        });
        self.add_dialog_subscription = Some(subscription);
        self.add_dialog = Some(dialog);
        cx.notify();
    }

    #[allow(clippy::too_many_lines)] // One derivation of what is drawn, in the order it is drawn.
    fn view(&self, cx: &App) -> Option<PageView> {
        let (guild, channel) = self.context(cx)?;
        let catalogue = strings(cx);
        let state = self.session.read(cx).state();
        let members = state.members(guild.id);
        let manager = role_manager_of(guild, state.current_user().map(|user| user.id));
        let channel_permissions = state.my_channel_permissions(self.channel_id);
        let targets = overwrite_targets(guild, channel);
        let row_of = |target: OverwriteTarget| {
            let (name, mark) = match target.kind {
                OverwriteKind::Role => {
                    let role = guild.roles.iter().find(|role| role.id == target.id);
                    (
                        role.map_or_else(
                            || {
                                catalogue
                                    .server_settings()
                                    .moderation_log()
                                    .deleted_role()
                                    .to_owned()
                            },
                            |role| role.name.clone(),
                        ),
                        TargetMark::Role(role.and_then(|role| role_colour(role.color))),
                    )
                }
                OverwriteKind::Member => {
                    let member = members.iter().find(|member| member.user.id == target.id);
                    (
                        member.map_or_else(
                            || {
                                catalogue
                                    .server_settings()
                                    .moderation_log()
                                    .former_member()
                                    .to_owned()
                            },
                            |member| display_name(&member.user).to_owned(),
                        ),
                        TargetMark::Member(member.map(|member| member.user.clone())),
                    )
                }
            };
            TargetRow {
                target,
                name,
                mark,
                is_locked: !can_edit_target(guild, &manager, target),
            }
        };
        let rows: Vec<TargetRow> = targets.iter().copied().map(row_of).collect();
        let editor = self.selected_target(&targets).map(|selected| {
            let stored = stored_overwrite_of(channel, selected.id);
            let shown = self.draft_for(channel, selected).unwrap_or(stored);
            let row = row_of(selected);
            let is_editable = !row.is_locked;
            let groups: Vec<GroupView> = channel_permission_groups_for(channel.kind)
                .into_iter()
                .map(|group| GroupView {
                    label: permission_group_label(group.group, catalogue),
                    permissions: group
                        .permissions
                        .iter()
                        .map(|permission| {
                            let (label, description) =
                                channel_permission_words(permission.key, catalogue);
                            PermissionRow {
                                bit: permission.bit,
                                label,
                                description,
                                current: overwrite_state_of(shown, permission.bit),
                                is_locked: !is_editable
                                    || !can_change_channel_permission(
                                        channel_permissions,
                                        permission.bit,
                                    ),
                            }
                        })
                        .collect(),
                })
                .collect();
            let has_locked_permissions = groups.iter().any(|group| {
                group.permissions.iter().any(|permission| {
                    !can_change_channel_permission(channel_permissions, permission.bit)
                })
            });
            EditorView {
                row,
                is_everyone: selected.id == guild.id,
                is_editable,
                can_remove: can_remove_overwrite(
                    guild,
                    &manager,
                    channel_permissions,
                    channel,
                    selected,
                ),
                has_locked_permissions,
                groups,
            }
        });
        Some(PageView {
            channel_name: channel.name.clone(),
            rows,
            editor,
        })
    }

    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render_target_list(
        &mut self,
        view: &PageView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.channel_settings().permissions();
        let selected_id = view.editor.as_ref().map(|editor| editor.row.target.id);
        let entity = cx.entity();
        let rows: Vec<AnyElement> = view
            .rows
            .iter()
            .map(|row| {
                let target_id = row.target.id;
                let id = ElementId::from(("overwrite-target", target_id.raw()));
                let focus = self.row_focus_handle(target_id, cx);
                let hover = remembered_hover(&id, window, cx);
                let is_selected = selected_id == Some(target_id);
                let is_ring_shown = is_focus_visible(&focus, window);
                let entity = entity.clone();
                let (fill, text) = if is_selected {
                    (
                        Some(tokens.component_roles_row_background_selected),
                        tokens.component_roles_row_text_selected,
                    )
                } else if *hover.read(cx) {
                    (
                        Some(tokens.component_roles_row_background_hover),
                        tokens.component_roles_row_text_hover,
                    )
                } else {
                    (None, tokens.component_roles_row_text)
                };
                with_type(div(), TYPE_CHANNEL_NAME)
                    .id(id)
                    .role(Role::Button)
                    .aria_label(if row.is_locked {
                        words.locked_target_row(&row.name)
                    } else {
                        row.name.clone()
                    })
                    .aria_selected(is_selected)
                    .track_focus(&focus)
                    .relative()
                    .flex()
                    .items_center()
                    .gap(length(COMPONENT_ROLES_ROW_ICON_GAP))
                    .w_full()
                    .min_h(length(COMPONENT_ROLES_ROW_HEIGHT))
                    .px(length(COMPONENT_ROLES_ROW_PADDING_INLINE))
                    .rounded(length(COMPONENT_ROLES_ROW_RADIUS))
                    .when_some(fill, |row, fill| row.bg(color(fill)))
                    .text_color(color(text))
                    .cursor_pointer()
                    .on_hover(move |is_hovered, _, cx| {
                        hover.update(cx, |remembered, cx| {
                            *remembered = *is_hovered;
                            cx.notify();
                        });
                    })
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |page, cx| page.select(target_id, cx));
                    })
                    .child(target_mark(&row.mark, window, cx))
                    .child(div().min_w_0().flex_1().truncate().child(row.name.clone()))
                    .when(row.is_locked, |element| {
                        element.child(icon(
                            IconName::Lock,
                            COMPONENT_ROLES_ROW_LOCK_ICON_SIZE,
                            color(tokens.component_roles_row_lock_icon),
                        ))
                    })
                    .when(is_ring_shown, |element| {
                        element.child(focus_ring(COMPONENT_ROLES_ROW_RADIUS, cx))
                    })
                    .into_any_element()
            })
            .collect();
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .flex_basis(length(COMPONENT_ROLES_LIST_WIDTH))
            .flex_grow(1.0)
            .flex_shrink(1.0)
            .child(
                with_type(div(), TYPE_CATEGORY)
                    .id("overwrite-targets-title")
                    .role(Role::Heading)
                    .aria_level(3)
                    .text_color(color(tokens.component_settings_section_label_text))
                    .child(words.targets()),
            )
            .child(
                div()
                    .id("overwrite-targets")
                    .role(Role::List)
                    .aria_label(words.targets())
                    .mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_ROLES_ROW_GAP))
                    .children(rows),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render_editor(
        &self,
        editor: &EditorView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.channel_settings().permissions();
        let target = editor.row.target;
        let entity = cx.entity();
        let note = |text: &'static str| {
            with_type(div(), TYPE_BODY)
                .flex()
                .gap(length(COMPONENT_SETTINGS_ERROR_ICON_GAP))
                .text_color(color(tokens.component_roles_note_text))
                .child(icon(
                    IconName::Lock,
                    COMPONENT_SETTINGS_ERROR_ICON_SIZE,
                    color(tokens.component_roles_note_text),
                ))
                .child(text)
        };
        let header = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(length(COMPONENT_SETTINGS_ACCOUNT_BUTTON_GAP))
            .child(
                with_type(div(), TYPE_HEADING_MD)
                    .id("overwrite-target-title")
                    .role(Role::Heading)
                    .aria_level(3)
                    .aria_label(editor.row.name.clone())
                    .mr_auto()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(length(COMPONENT_ROLES_ROW_ICON_GAP))
                    .text_color(color(tokens.component_settings_page_title_text))
                    .child(target_mark(&editor.row.mark, window, cx))
                    .child(div().truncate().child(editor.row.name.clone())),
            )
            .when(editor.can_remove, |header| {
                let remove_entity = entity.clone();
                header.child(
                    Button::new(
                        "overwrite-remove",
                        if self.busy == Some(Busy::Remove) {
                            words.removing()
                        } else {
                            words.remove()
                        },
                    )
                    .variant(ButtonVariant::Ghost)
                    .accessible_label(words.remove_label(&editor.row.name))
                    .disabled(self.busy.is_some())
                    .on_click(move |_, cx| {
                        remove_entity.update(cx, |page, cx| page.remove(target, cx));
                    }),
                )
            });
        let locked_permissions_note = words.locked_permissions();
        let groups: Vec<AnyElement> = editor
            .groups
            .iter()
            .enumerate()
            .map(|(group_index, group)| {
                let permissions: Vec<AnyElement> = group
                    .permissions
                    .iter()
                    .map(|permission| {
                        let bit = permission.bit;
                        let entity = entity.clone();
                        let description = if editor.is_editable && permission.is_locked {
                            format!("{} {locked_permissions_note}", permission.description)
                        } else {
                            permission.description.to_owned()
                        };
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_x(length(COMPONENT_ROLES_PERMISSION_GAP))
                            .gap_y(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP))
                            .border_b_1()
                            .border_color(color(tokens.component_roles_permission_divider))
                            .py(length(COMPONENT_ROLES_PERMISSION_PADDING_BLOCK))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_basis(length(COMPONENT_ROLES_LIST_WIDTH))
                                    .flex_grow(1.0)
                                    .flex_shrink(1.0)
                                    .child(
                                        with_type(div(), TYPE_CHANNEL_NAME)
                                            .text_color(color(
                                                tokens.component_roles_permission_label_text,
                                            ))
                                            .child(permission.label),
                                    )
                                    .child(
                                        with_type(div(), TYPE_BODY)
                                            .text_color(color(
                                                tokens.component_roles_permission_description_text,
                                            ))
                                            .child(permission.description),
                                    ),
                            )
                            .child(override_states(
                                ElementId::from(("overwrite-states", bit.bits())),
                                permission,
                                description,
                                words_of_states(catalogue),
                                move |state, cx| {
                                    entity.update(cx, |page, cx| page.change(bit, state, cx));
                                },
                                window,
                                cx,
                            ))
                            .into_any_element()
                    })
                    .collect();
                div()
                    .when(group_index > 0, |section| {
                        section.mt(length(COMPONENT_ROLES_PERMISSION_GROUP_GAP))
                    })
                    .child(
                        with_type(div(), TYPE_CATEGORY)
                            .id(ElementId::from(("overwrite-group", group_index)))
                            .role(Role::Heading)
                            .aria_level(4)
                            .text_color(color(tokens.component_settings_section_label_text))
                            .child(group.label),
                    )
                    .children(permissions)
                    .into_any_element()
            })
            .collect();
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .flex_basis(length(COMPONENT_PROFILE_POPOVER_WIDTH))
            .flex_grow(EDITOR_FLEX_GROW)
            .flex_shrink(1.0)
            .gap(length(COMPONENT_ROLES_EDITOR_GAP))
            .child(header)
            .when(editor.is_everyone, |column| {
                column.child(
                    with_type(div(), TYPE_BODY)
                        .text_color(color(tokens.component_settings_body_text))
                        .child(words.everyone_note()),
                )
            })
            .map(|column| {
                if !editor.is_editable {
                    column.child(note(words.locked_target()))
                } else if editor.has_locked_permissions {
                    column.child(note(locked_permissions_note))
                } else {
                    column
                }
            })
            .children(groups)
            .into_any_element()
    }
}

/// The three states' words, in `OVERWRITE_STATES`' order.
fn words_of_states(catalogue: Strings) -> [&'static str; 3] {
    let states = catalogue.channel_settings().permissions().states();
    OVERWRITE_STATES.map(|state| match state {
        OverwriteState::Deny => states.deny(),
        OverwriteState::Inherit => states.inherit(),
        OverwriteState::Allow => states.allow(),
    })
}

fn state_icon(state: OverwriteState) -> IconName {
    match state {
        OverwriteState::Deny => IconName::X,
        OverwriteState::Inherit => IconName::Minus,
        OverwriteState::Allow => IconName::Check,
    }
}

/// A role's dot or a member's avatar, at the target avatar size.
pub(super) fn target_mark(mark: &TargetMark, window: &Window, cx: &App) -> AnyElement {
    let tokens = theme(cx);
    match mark {
        TargetMark::Role(colour) => div()
            .flex_none()
            .size(length(COMPONENT_CHANNEL_SETTINGS_TARGET_AVATAR_SIZE))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(length(COMPONENT_ROLES_ROW_DOT_SIZE))
                    .rounded_full()
                    .bg(colour.map_or_else(
                        || color(tokens.component_roles_neutral_color),
                        |colour| rgb(colour).into(),
                    )),
            )
            .into_any_element(),
        TargetMark::Member(Some(user)) => {
            let picture = user.avatar.map(|image| {
                profile_picture(image, COMPONENT_CHANNEL_SETTINGS_TARGET_AVATAR_SIZE, window)
            });
            Avatar::new(
                AvatarPerson::from_user(user, picture),
                COMPONENT_CHANNEL_SETTINGS_TARGET_AVATAR_SIZE,
            )
            .into_any_element()
        }
        TargetMark::Member(None) => div()
            .flex_none()
            .size(length(COMPONENT_CHANNEL_SETTINGS_TARGET_AVATAR_SIZE))
            .rounded_full()
            .bg(color(tokens.component_roles_neutral_color))
            .into_any_element(),
    }
}

/// Deny / Inherit / Allow as radios: one Tab stop on the chosen state, arrows move and choose,
/// read with the permission's name. A chosen Deny or Allow takes its meaning's wash, so a column
/// reads at a glance; the icon and the word still say it without colour.
#[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
fn override_states(
    id: ElementId,
    permission: &PermissionRow,
    description: String,
    state_words: [&'static str; 3],
    on_change: impl Fn(OverwriteState, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let is_locked = permission.is_locked;
    let current = permission.current;
    let checked_index = OVERWRITE_STATES
        .iter()
        .position(|state| *state == current)
        .unwrap_or(1);
    let on_change = Arc::new(on_change);
    let focus_handles: Vec<FocusHandle> = OVERWRITE_STATES
        .iter()
        .enumerate()
        .map(|(index, _)| {
            remembered_focus_handle(&child_id(&id, state_name(index)), window, cx)
                .tab_stop(!is_locked && index == checked_index)
        })
        .collect();
    let options: Vec<AnyElement> = OVERWRITE_STATES
        .iter()
        .zip(state_words)
        .zip(focus_handles.iter().cloned())
        .enumerate()
        .map(|(index, ((state, word), focus))| {
            let state = *state;
            let option_id = child_id(&id, state_name(index));
            let is_chosen = state == current;
            let hover = remembered_hover(&option_id, window, cx);
            let is_hovered = *hover.read(cx) && !is_chosen && !is_locked;
            let (fill, text) = match (is_chosen, state) {
                (true, OverwriteState::Deny) => (
                    Some(tokens.component_channel_settings_override_deny_background_selected),
                    tokens.component_channel_settings_override_deny_text_selected,
                ),
                (true, OverwriteState::Inherit) => (
                    Some(tokens.component_channel_settings_override_background_selected),
                    tokens.component_channel_settings_override_text_selected,
                ),
                (true, OverwriteState::Allow) => (
                    Some(tokens.component_channel_settings_override_allow_background_selected),
                    tokens.component_channel_settings_override_allow_text_selected,
                ),
                (false, _) if is_hovered => (
                    Some(tokens.component_channel_settings_override_background_hover),
                    tokens.component_channel_settings_override_text_hover,
                ),
                (false, _) => (None, tokens.component_channel_settings_override_text),
            };
            let is_ring_shown = !is_locked && is_focus_visible(&focus, window);
            let on_change = on_change.clone();
            with_type(div(), TYPE_BODY_SM)
                .id(option_id)
                .role(Role::RadioButton)
                .aria_label(word)
                .aria_toggled(if is_chosen {
                    Toggled::True
                } else {
                    Toggled::False
                })
                .aria_position_in_set(index + 1)
                .aria_size_of_set(OVERWRITE_STATES.len())
                .when(!is_locked, |option| option.track_focus(&focus))
                .relative()
                .flex()
                .items_center()
                .gap(length(COMPONENT_CHANNEL_SETTINGS_OVERRIDE_ICON_GAP))
                .h(length(COMPONENT_CHANNEL_SETTINGS_OVERRIDE_HEIGHT))
                .px(length(COMPONENT_CHANNEL_SETTINGS_OVERRIDE_PADDING_INLINE))
                .when(is_chosen, |option| {
                    option.font_weight(gpui::FontWeight::SEMIBOLD)
                })
                .when_some(fill, |option, fill| option.bg(color(fill)))
                .text_color(color(text))
                .when(!is_locked && !is_chosen, |option| {
                    option
                        .cursor_pointer()
                        .on_hover(move |is_hovered, _, cx| {
                            hover.update(cx, |remembered, cx| {
                                *remembered = *is_hovered;
                                cx.notify();
                            });
                        })
                        .on_click(move |_, _, cx| on_change(state, cx))
                })
                .child(icon(
                    state_icon(state),
                    COMPONENT_CHANNEL_SETTINGS_OVERRIDE_ICON_SIZE,
                    color(text),
                ))
                .child(word)
                .when(is_ring_shown, |option| {
                    option.child(inset_focus_ring(0.0, cx))
                })
                .into_any_element()
        })
        .collect();
    let key_on_change = on_change;
    div()
        .id(id)
        .role(Role::RadioGroup)
        .aria_label(permission.label)
        .aria_description(description)
        .flex()
        .flex_none()
        .overflow_hidden()
        .rounded(length(COMPONENT_CHANNEL_SETTINGS_OVERRIDE_RADIUS))
        .border(px(COMPONENT_CHANNEL_SETTINGS_OVERRIDE_BORDER_WIDTH))
        .border_color(color(tokens.component_channel_settings_override_border))
        .when(is_locked, |group| {
            group.opacity(COMPONENT_SWITCH_DISABLED_OPACITY)
        })
        .when(!is_locked, |group| {
            group.on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                let Some(next) =
                    next_choice_index(&event.keystroke.key, checked_index, OVERWRITE_STATES.len())
                else {
                    return;
                };
                cx.stop_propagation();
                if let Some(handle) = focus_handles.get(next) {
                    window.focus(handle, cx);
                }
                if let Some(state) = OVERWRITE_STATES.get(next) {
                    key_on_change(*state, cx);
                }
            })
        })
        .children(options)
        .into_any_element()
}

fn state_name(index: usize) -> &'static str {
    match index {
        0 => "deny",
        1 => "inherit",
        _ => "allow",
    }
}

impl Render for ChannelPermissionsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(focus) = self.pending_focus.take() {
            window.focus(&focus, cx);
        }
        let Some(view) = self.view(cx) else {
            return div().into_any_element();
        };
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.channel_settings().permissions();
        let has_changes = self.has_changes(cx);
        let entity = cx.entity();
        let add_entity = entity.clone();
        let target_list = self.render_target_list(&view, window, cx);
        let editor = view
            .editor
            .as_ref()
            .map(|editor| self.render_editor(editor, window, cx));
        let reset_entity = entity.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .child(
                with_type(div(), TYPE_BODY)
                    .text_color(color(tokens.component_settings_body_text))
                    .child(words.intro(&view.channel_name)),
            )
            .child(
                div().mt(length(COMPONENT_SETTINGS_ACTION_GAP)).child(
                    Button::new("overwrite-add", words.add())
                        .disabled(self.busy.is_some())
                        .track_focus(&self.add_button_focus)
                        .on_click(move |window, cx| {
                            add_entity.update(cx, |page, cx| page.open_add_dialog(window, cx));
                        }),
                ),
            )
            .child(
                div()
                    .id("channel-permissions-alert")
                    .role(Role::Alert)
                    .aria_label(SharedString::from(
                        self.error_message.clone().unwrap_or_default(),
                    ))
                    .when_some(self.error_message.clone(), |alert, message| {
                        alert.child(
                            with_type(div(), TYPE_BODY)
                                .mt(length(COMPONENT_SETTINGS_ACTION_GAP))
                                .flex()
                                .gap(length(COMPONENT_SETTINGS_ERROR_ICON_GAP))
                                .text_color(color(tokens.component_roles_error_text))
                                .child(icon(
                                    IconName::WarningCircle,
                                    COMPONENT_SETTINGS_ERROR_ICON_SIZE,
                                    color(tokens.component_roles_error_text),
                                ))
                                .child(message),
                        )
                    }),
            )
            .child(
                div()
                    .mt(length(COMPONENT_SETTINGS_PAGE_TITLE_GAP))
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .gap(length(COMPONENT_ROLES_LIST_GAP))
                    .child(target_list)
                    .children(editor),
            )
            .child(
                // Said, not shown: the bar going away already shows it.
                div()
                    .id("channel-permissions-notice")
                    .role(Role::Status)
                    .aria_label(self.notice.clone().unwrap_or_default())
                    .h_0()
                    .overflow_hidden(),
            )
            .child(div().flex_1())
            .when(has_changes, |column| {
                let mut bar = UnsavedChangesBar::new(
                    "channel-permissions-unsaved",
                    unsaved_changes_labels(catalogue),
                    Arc::new(move |_, cx| reset_entity.update(cx, Self::reset)),
                    Arc::new(move |_, cx| entity.update(cx, Self::save)),
                )
                .attention(self.attention + self.local_attention)
                .saving(self.busy == Some(Busy::Save));
                if let Some(error) = self.save_error.clone() {
                    bar = bar.error_message(error);
                }
                column.child(bar)
            })
            .when_some(self.add_dialog.as_ref(), |column, dialog| {
                column.children(dialog.read(cx).dialog().cloned())
            })
            .into_any_element()
    }
}

#[cfg(test)]
#[path = "permissions_tests.rs"]
mod tests;
