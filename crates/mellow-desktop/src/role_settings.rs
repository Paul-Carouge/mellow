//! Pure logic of the Roles section of server settings (DESIGN.md §5.8): rows, drafts, reordering.
//!
//! The server decides every role action (ADR-0007); these rules only adapt the interface, so a
//! locked control matches what the server would refuse (ADR-0014).

use mellow_protocol::{
    CreateRoleRequest, Guild, Member, Permissions, Role, Snowflake, UpdateRoleRequest,
    highest_role_position,
};
use slint::Color;

use crate::generated_ui::{RoleListRow, RolePermissionRow};
use crate::shell_rows::saturating_int;

/// Discord's role colour presets: user-chosen data shown on names, not design tokens.
#[allow(clippy::unreadable_literal, reason = "colours read as #RRGGBB")]
pub const ROLE_COLOR_PRESETS: [u32; 20] = [
    0x1ABC9C, 0x2ECC71, 0x3498DB, 0x9B59B6, 0xE91E63, 0xF1C40F, 0xE67E22, 0xE74C3C, 0x95A5A6,
    0x607D8B, 0x11806A, 0x1F8B4C, 0x206694, 0x71368A, 0xAD1457, 0xC27C0E, 0xA84300, 0x992D22,
    0x979C9F, 0x546E7A,
];
/// The wire value of "no colour", as on Discord.
pub const NO_ROLE_COLOR: u32 = 0;
pub const NEW_ROLE_NAME: &str = "new role";

struct PermissionDescription {
    permission: Permissions,
    group: &'static str,
    label: &'static str,
    description: &'static str,
    is_sensitive: bool,
}

const fn describe(
    permission: Permissions,
    group: &'static str,
    label: &'static str,
    description: &'static str,
    is_sensitive: bool,
) -> PermissionDescription {
    PermissionDescription {
        permission,
        group,
        label,
        description,
        is_sensitive,
    }
}

/// Every permission once, in display order; a group header starts where `group` changes.
const PERMISSION_DESCRIPTIONS: [PermissionDescription; 16] = [
    describe(
        Permissions::VIEW_CHANNEL,
        "General",
        "View channels",
        "See text channels and read what is posted there.",
        false,
    ),
    describe(
        Permissions::MANAGE_CHANNELS,
        "General",
        "Manage channels",
        "Create, edit and delete channels.",
        false,
    ),
    describe(
        Permissions::MANAGE_ROLES,
        "General",
        "Manage roles",
        "Create roles, and edit or delete the roles below their highest role.",
        true,
    ),
    describe(
        Permissions::MANAGE_GUILD,
        "General",
        "Manage server",
        "Change the server's name and settings.",
        false,
    ),
    describe(
        Permissions::VIEW_AUDIT_LOG,
        "General",
        "View moderation log",
        "See who changed what on this server.",
        false,
    ),
    describe(
        Permissions::CREATE_INVITE,
        "Membership",
        "Create invite",
        "Invite new people to this server.",
        false,
    ),
    describe(
        Permissions::CHANGE_NICKNAME,
        "Membership",
        "Change nickname",
        "Change their own nickname on this server.",
        false,
    ),
    describe(
        Permissions::MANAGE_NICKNAMES,
        "Membership",
        "Manage nicknames",
        "Change the nicknames of other members.",
        false,
    ),
    describe(
        Permissions::SEND_MESSAGES,
        "Text channels",
        "Send messages",
        "Write in text channels.",
        false,
    ),
    describe(
        Permissions::READ_MESSAGE_HISTORY,
        "Text channels",
        "Read message history",
        "Read messages sent before they opened the channel.",
        false,
    ),
    describe(
        Permissions::ATTACH_FILES,
        "Text channels",
        "Attach files",
        "Add files and images to their messages.",
        false,
    ),
    describe(
        Permissions::MENTION_EVERYONE,
        "Text channels",
        "Mention @everyone",
        "Notify everyone in a channel at once.",
        false,
    ),
    describe(
        Permissions::MANAGE_MESSAGES,
        "Text channels",
        "Manage messages",
        "Delete messages sent by other members.",
        false,
    ),
    describe(
        Permissions::KICK_MEMBERS,
        "Moderation",
        "Kick members",
        "Remove people; they can come back with a new invite.",
        false,
    ),
    describe(
        Permissions::BAN_MEMBERS,
        "Moderation",
        "Ban members",
        "Remove people and stop them from rejoining.",
        true,
    ),
    describe(
        Permissions::ADMINISTRATOR,
        "Advanced",
        "Administrator",
        "Every permission in every channel, whatever the channel settings. Grant it with care.",
        true,
    ),
];

/// What the signed-in member may do with roles; mirrors `RoleManager` in the server's domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleManager {
    permissions: Permissions,
    is_owner: bool,
    highest_role_position: u32,
}

impl RoleManager {
    pub fn of_member(guild: &Guild, user_id: Snowflake) -> Self {
        let my_roles = guild
            .roles
            .iter()
            .filter(|role| guild.my_role_ids.contains(&role.id));
        Self {
            permissions: guild.my_permissions,
            is_owner: guild.owner_id == user_id,
            highest_role_position: highest_role_position(my_roles),
        }
    }

    pub fn can_create_roles(&self) -> bool {
        self.permissions.contains(Permissions::MANAGE_ROLES)
    }

    /// `@everyone`'s id is the guild's id.
    pub fn can_manage_role(&self, role: &Role, guild_id: Snowflake) -> bool {
        self.permissions.contains(Permissions::MANAGE_ROLES)
            && (self.is_owner
                || ((role.id == guild_id || role.position < self.highest_role_position)
                    && self.permissions.contains(role.permissions)))
    }

    /// Nobody grants or removes a permission they do not hold; the owner and administrators
    /// hold them all.
    fn can_toggle_permission(&self, permission: Permissions) -> bool {
        self.permissions.contains(permission)
    }
}

/// Roles from highest to lowest, `@everyone` last, as the settings list shows them.
pub fn roles_highest_first(guild: &Guild) -> Vec<&Role> {
    let mut roles: Vec<&Role> = guild.roles.iter().collect();
    roles.sort_by_key(|role| std::cmp::Reverse(role.position));
    roles
}

pub fn role_list_rows(
    guild: &Guild,
    members: &[Member],
    manager: &RoleManager,
) -> Vec<RoleListRow> {
    let roles = roles_highest_first(guild);
    roles
        .iter()
        .enumerate()
        .map(|(index, role)| {
            let is_everyone = role.id == guild.id;
            let member_count = if is_everyone {
                members.len()
            } else {
                members
                    .iter()
                    .filter(|member| member.role_ids.contains(&role.id))
                    .count()
            };
            let can_swap_with = |other_index: Option<usize>| {
                other_index
                    .and_then(|other_index| roles.get(other_index))
                    .is_some_and(|other| {
                        other.id != guild.id
                            && !is_everyone
                            && can_swap(manager, role, other, guild.id)
                    })
            };
            RoleListRow {
                id: role.id.to_string().into(),
                name: role.name.as_str().into(),
                color: rgb_color(role.color),
                has_color: role.color != NO_ROLE_COLOR,
                member_count: saturating_int(member_count),
                is_everyone,
                is_locked: !manager.can_manage_role(role, guild.id),
                can_move_up: can_swap_with(index.checked_sub(1)),
                can_move_down: can_swap_with(index.checked_add(1)),
            }
        })
        .collect()
}

/// Swapping two roles changes both slots, so the manager must manage both (the owner manages all).
fn can_swap(manager: &RoleManager, role: &Role, other: &Role, guild_id: Snowflake) -> bool {
    manager.can_manage_role(role, guild_id) && manager.can_manage_role(other, guild_id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDirection {
    Up,
    Down,
}

/// The `ReorderRolesRequest` order (named roles, lowest to highest) after swapping `role_id`
/// with its neighbour; `None` when there is no neighbour to swap with.
pub fn role_ids_after_move(
    guild: &Guild,
    role_id: Snowflake,
    direction: MoveDirection,
) -> Option<Vec<Snowflake>> {
    let mut named_role_ids: Vec<(u32, Snowflake)> = guild
        .roles
        .iter()
        .filter(|role| role.id != guild.id)
        .map(|role| (role.position, role.id))
        .collect();
    named_role_ids.sort_unstable();
    let mut role_ids: Vec<Snowflake> = named_role_ids.into_iter().map(|(_, id)| id).collect();
    let index = role_ids.iter().position(|id| *id == role_id)?;
    let neighbour_index = match direction {
        MoveDirection::Up => index.checked_add(1).filter(|next| *next < role_ids.len())?,
        MoveDirection::Down => index.checked_sub(1)?,
    };
    role_ids.swap(index, neighbour_index);
    Some(role_ids)
}

/// The editable part of a role; compared with the stored role to tell unsaved changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleDraft {
    pub name: String,
    pub color: u32,
    pub permissions: Permissions,
}

impl From<&Role> for RoleDraft {
    fn from(role: &Role) -> Self {
        Self {
            name: role.name.clone(),
            color: role.color,
            permissions: role.permissions,
        }
    }
}

/// What the user changed, on top of the role as it was when they changed it.
///
/// Kept apart from the stored role so that a change arriving meanwhile (another manager
/// revoking a permission) is never undone by saving an older copy of the role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleEdit {
    base: RoleDraft,
    pub draft: RoleDraft,
}

impl RoleEdit {
    pub fn start(role: &Role) -> Self {
        let base = RoleDraft::from(role);
        Self {
            draft: base.clone(),
            base,
        }
    }

    /// The same changes on top of `stored`; `None` once they no longer differ from it.
    pub fn rebased_on(&self, stored: &Role) -> Option<Self> {
        let stored = RoleDraft::from(stored);
        let draft = RoleDraft {
            name: if self.draft.name == self.base.name {
                stored.name.clone()
            } else {
                self.draft.name.clone()
            },
            color: if self.draft.color == self.base.color {
                stored.color
            } else {
                self.draft.color
            },
            // Only the toggled bits; unknown bits of a newer server stay as stored.
            permissions: stored.permissions ^ (self.draft.permissions ^ self.base.permissions),
        };
        // The server trims names, so trailing spaces alone are not a change.
        let draft_as_saved = RoleDraft {
            name: draft.name.trim().to_owned(),
            ..draft.clone()
        };
        (draft_as_saved != stored).then_some(Self {
            base: stored,
            draft,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleDraftError {
    EmptyName,
}

/// Only the fields that changed; `@everyone` takes permissions only.
pub fn update_role_request(
    stored: &Role,
    draft: &RoleDraft,
    is_everyone: bool,
) -> Result<UpdateRoleRequest, RoleDraftError> {
    let name = draft.name.trim();
    if !is_everyone && name.is_empty() {
        return Err(RoleDraftError::EmptyName);
    }
    let is_named_field_editable = !is_everyone;
    Ok(UpdateRoleRequest {
        name: (is_named_field_editable && name != stored.name).then(|| name.to_owned()),
        color: (is_named_field_editable && draft.color != stored.color).then_some(draft.color),
        permissions: (draft.permissions != stored.permissions).then_some(draft.permissions),
    })
}

pub fn new_role_request() -> CreateRoleRequest {
    CreateRoleRequest {
        name: NEW_ROLE_NAME.to_owned(),
        color: NO_ROLE_COLOR,
        permissions: Permissions::empty(),
    }
}

pub fn permission_rows(
    granted: Permissions,
    manager: &RoleManager,
    is_role_editable: bool,
) -> Vec<RolePermissionRow> {
    let mut previous_group = "";
    PERMISSION_DESCRIPTIONS
        .iter()
        .map(|description| {
            let starts_group = description.group != previous_group;
            previous_group = description.group;
            RolePermissionRow {
                group: description.group.into(),
                starts_group,
                label: description.label.into(),
                description: description.description.into(),
                is_sensitive: description.is_sensitive,
                is_granted: granted.contains(description.permission),
                is_locked: !is_role_editable
                    || !manager.can_toggle_permission(description.permission),
            }
        })
        .collect()
}

/// The permission behind a row of `permission_rows`, if the manager may toggle it.
pub fn toggleable_permission_at_row(row_index: i32, manager: &RoleManager) -> Option<Permissions> {
    let description = PERMISSION_DESCRIPTIONS.get(usize::try_from(row_index).ok()?)?;
    manager
        .can_toggle_permission(description.permission)
        .then_some(description.permission)
}

pub fn role_color_presets() -> Vec<Color> {
    ROLE_COLOR_PRESETS.iter().copied().map(rgb_color).collect()
}

/// `-1` (or any index out of range) is "no colour".
pub fn role_color_at_preset(preset_index: i32) -> u32 {
    usize::try_from(preset_index)
        .ok()
        .and_then(|index| ROLE_COLOR_PRESETS.get(index))
        .copied()
        .unwrap_or(NO_ROLE_COLOR)
}

pub fn rgb_color(rgb: u32) -> Color {
    let [_, red, green, blue] = rgb.to_be_bytes();
    Color::from_rgb_u8(red, green, blue)
}

#[cfg(test)]
#[allow(clippy::unreadable_literal, reason = "colours read as #RRGGBB")]
mod tests {
    use mellow_protocol::PublicUser;

    use super::*;

    const GUILD: Snowflake = Snowflake::from_raw(1);
    const OWNER: Snowflake = Snowflake::from_raw(2);
    const ME: Snowflake = Snowflake::from_raw(3);
    const ADMINS: Snowflake = Snowflake::from_raw(10);
    const MODERATORS: Snowflake = Snowflake::from_raw(11);
    const HELPERS: Snowflake = Snowflake::from_raw(12);

    fn role(id: Snowflake, name: &str, position: u32, permissions: Permissions) -> Role {
        Role {
            id,
            name: name.to_owned(),
            color: NO_ROLE_COLOR,
            position,
            permissions,
        }
    }

    /// I hold Moderators (manage roles, kick); Admins rank above, Helpers below.
    fn guild() -> Guild {
        let moderator_permissions = Permissions::MANAGE_ROLES | Permissions::KICK_MEMBERS;
        Guild {
            id: GUILD,
            name: "Club".to_owned(),
            owner_id: OWNER,
            my_permissions: Permissions::DEFAULT_EVERYONE | moderator_permissions,
            my_role_ids: vec![MODERATORS],
            roles: vec![
                role(GUILD, "@everyone", 0, Permissions::DEFAULT_EVERYONE),
                role(HELPERS, "Helpers", 1, Permissions::KICK_MEMBERS),
                role(MODERATORS, "Moderators", 2, moderator_permissions),
                role(ADMINS, "Admins", 3, Permissions::ADMINISTRATOR),
            ],
            channels: Vec::new(),
        }
    }

    fn member(id: u64, role_ids: &[Snowflake]) -> Member {
        Member {
            user: PublicUser {
                id: Snowflake::from_raw(id),
                username: format!("user{id}"),
                display_name: format!("User {id}"),
            },
            role_ids: role_ids.to_vec(),
            joined_at_unix_milliseconds: 0,
        }
    }

    #[test]
    fn rows_list_roles_highest_first_with_member_counts_and_locks() {
        let guild = guild();
        let members = [
            member(20, &[MODERATORS]),
            member(21, &[HELPERS, MODERATORS]),
            member(22, &[]),
        ];
        let rows = role_list_rows(&guild, &members, &RoleManager::of_member(&guild, ME));
        let names: Vec<_> = rows.iter().map(|row| row.name.to_string()).collect();
        assert_eq!(names, ["Admins", "Moderators", "Helpers", "@everyone"]);
        let counts: Vec<_> = rows.iter().map(|row| row.member_count).collect();
        assert_eq!(counts, [0, 2, 1, 3]);
        let locked: Vec<_> = rows.iter().map(|row| row.is_locked).collect();
        assert_eq!(locked, [true, true, false, false]);
        assert!(rows[3].is_everyone && !rows[2].is_everyone);
    }

    #[test]
    fn a_role_moves_only_past_roles_its_manager_can_manage() {
        let guild = guild();
        let rows = role_list_rows(&guild, &[], &RoleManager::of_member(&guild, ME));
        let moves: Vec<_> = rows
            .iter()
            .map(|row| (row.can_move_up, row.can_move_down))
            .collect();
        // Helpers is mine to manage, but both neighbours are not (my own role, @everyone).
        assert_eq!(moves, [(false, false); 4]);

        let rows = role_list_rows(&guild, &[], &RoleManager::of_member(&guild, OWNER));
        let moves: Vec<_> = rows
            .iter()
            .map(|row| (row.can_move_up, row.can_move_down))
            .collect();
        assert_eq!(
            moves,
            [(false, true), (true, true), (true, false), (false, false)]
        );
    }

    #[test]
    fn moving_a_role_swaps_it_with_its_neighbour_in_the_lowest_first_order() {
        let guild = guild();
        assert_eq!(
            role_ids_after_move(&guild, HELPERS, MoveDirection::Up),
            Some(vec![MODERATORS, HELPERS, ADMINS])
        );
        assert_eq!(
            role_ids_after_move(&guild, ADMINS, MoveDirection::Down),
            Some(vec![HELPERS, ADMINS, MODERATORS])
        );
        assert_eq!(role_ids_after_move(&guild, ADMINS, MoveDirection::Up), None);
        assert_eq!(
            role_ids_after_move(&guild, HELPERS, MoveDirection::Down),
            None
        );
        assert_eq!(role_ids_after_move(&guild, GUILD, MoveDirection::Up), None);
    }

    #[test]
    fn permissions_the_manager_lacks_are_locked() {
        let guild = guild();
        let manager = RoleManager::of_member(&guild, ME);
        let rows = permission_rows(Permissions::KICK_MEMBERS, &manager, true);
        let row = |label: &str| {
            rows.iter()
                .find(|row| row.label == label)
                .expect("every permission has a row")
        };
        assert!(row("Kick members").is_granted && !row("Kick members").is_locked);
        assert!(row("Ban members").is_locked && row("Ban members").is_sensitive);
        assert!(row("Administrator").is_locked);
        assert!(
            permission_rows(Permissions::empty(), &manager, false)
                .iter()
                .all(|row| row.is_locked)
        );
        let ban_row_index = rows.iter().position(|row| row.label == "Ban members");
        let kick_row_index = rows.iter().position(|row| row.label == "Kick members");
        let at = |index: Option<usize>| {
            toggleable_permission_at_row(saturating_int(index.unwrap_or(99)), &manager)
        };
        assert_eq!(at(ban_row_index), None);
        assert_eq!(at(kick_row_index), Some(Permissions::KICK_MEMBERS));
        assert_eq!(toggleable_permission_at_row(-1, &manager), None);
    }

    #[test]
    fn every_permission_has_exactly_one_row_and_groups_start_once() {
        let mut covered = Permissions::empty();
        for description in &PERMISSION_DESCRIPTIONS {
            assert!(!covered.intersects(description.permission));
            covered |= description.permission;
        }
        assert_eq!(covered, Permissions::all());
        let manager = RoleManager::of_member(&guild(), OWNER);
        let group_starts = permission_rows(Permissions::empty(), &manager, true)
            .iter()
            .filter(|row| row.starts_group)
            .count();
        assert_eq!(group_starts, 5);
    }

    #[test]
    fn update_requests_carry_only_changed_fields() {
        let stored = role(HELPERS, "Helpers", 1, Permissions::KICK_MEMBERS);
        let mut draft = RoleDraft::from(&stored);
        assert_eq!(
            update_role_request(&stored, &draft, false),
            Ok(UpdateRoleRequest::default())
        );
        draft.name = "  Guides ".to_owned();
        draft.color = 0x3498DB;
        assert_eq!(
            update_role_request(&stored, &draft, false),
            Ok(UpdateRoleRequest {
                name: Some("Guides".to_owned()),
                color: Some(0x3498DB),
                permissions: None,
            })
        );
        draft.name = "   ".to_owned();
        assert_eq!(
            update_role_request(&stored, &draft, false),
            Err(RoleDraftError::EmptyName)
        );
        // @everyone keeps its name and colour whatever the draft says.
        draft.permissions = Permissions::empty();
        assert_eq!(
            update_role_request(&stored, &draft, true),
            Ok(UpdateRoleRequest {
                permissions: Some(Permissions::empty()),
                ..UpdateRoleRequest::default()
            })
        );
    }

    #[test]
    fn an_edit_keeps_changes_made_meanwhile_by_someone_else() {
        let before = role(
            HELPERS,
            "Helpers",
            1,
            Permissions::KICK_MEMBERS | Permissions::BAN_MEMBERS,
        );
        let mut edit = RoleEdit::start(&before);
        edit.draft.name = "Guides".to_owned();
        edit.draft.permissions.toggle(Permissions::MANAGE_MESSAGES);
        // Another manager revokes Ban members before the save.
        let stored = role(HELPERS, "Helpers", 1, Permissions::KICK_MEMBERS);
        let rebased = edit
            .rebased_on(&stored)
            .expect("the rename is still pending");
        assert_eq!(rebased.draft.name, "Guides");
        assert_eq!(
            rebased.draft.permissions,
            Permissions::KICK_MEMBERS | Permissions::MANAGE_MESSAGES
        );
        let request = update_role_request(&stored, &rebased.draft, false);
        assert_eq!(
            request.map(|request| request.permissions),
            Ok(Some(
                Permissions::KICK_MEMBERS | Permissions::MANAGE_MESSAGES
            ))
        );

        let mut rename_only = RoleEdit::start(&before);
        rename_only.draft.name = "Guides".to_owned();
        let rebased = rename_only
            .rebased_on(&stored)
            .expect("the rename is pending");
        assert_eq!(
            update_role_request(&stored, &rebased.draft, false),
            Ok(UpdateRoleRequest {
                name: Some("Guides".to_owned()),
                ..UpdateRoleRequest::default()
            })
        );
        // Once the stored role carries the same change, nothing is left to save.
        let renamed = role(HELPERS, "Guides", 1, Permissions::KICK_MEMBERS);
        assert_eq!(rebased.rebased_on(&renamed), None);
        let mut trailing_space = RoleEdit::start(&renamed);
        trailing_space.draft.name = "Guides ".to_owned();
        assert_eq!(trailing_space.rebased_on(&renamed), None);
    }

    #[test]
    fn preset_indexes_map_to_colours_and_anything_else_to_no_colour() {
        assert_eq!(role_color_at_preset(0), 0x1ABC9C);
        assert_eq!(role_color_at_preset(-1), NO_ROLE_COLOR);
        assert_eq!(role_color_at_preset(999), NO_ROLE_COLOR);
        assert_eq!(rgb_color(0x3498DB), Color::from_rgb_u8(0x34, 0x98, 0xDB));
    }
}
