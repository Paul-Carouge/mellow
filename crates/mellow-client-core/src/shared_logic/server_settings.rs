//! Pure rules of the server settings (`web/src/components/server-settings.ts`, DESIGN.md §5.8):
//! which sections a member sees, the role hierarchy, role drafts and member search. The instance
//! decides every action (ADR-0007); these rules only hide or lock what it would refuse (ADR-0014),
//! and mirror `RoleManager` in the server's domain.
//!
//! The moderation log's sentences are left to the track that ports that page: they are words.

use mellow_protocol::{Guild, Member, Permissions, Role, Snowflake, UpdateRoleRequest};

use super::search_text::fold_for_search;

/// The longest server name the instance takes, in characters (`MAXIMUM_SERVER_NAME_LENGTH`).
pub const MAXIMUM_SERVER_NAME_LENGTH: usize = 100;

/// A page of the server settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerSettingsSection {
    Overview,
    Roles,
    Emoji,
    Soundboard,
    Invites,
    Members,
    Bans,
    ModerationLog,
}

impl ServerSettingsSection {
    /// Members, bans and the moderation log sit under "Moderation" in the navigation.
    #[must_use]
    pub fn is_moderation(self) -> bool {
        matches!(self, Self::Members | Self::Bans | Self::ModerationLog)
    }
}

/// Discord's order: the server's own settings, then moderation.
const SECTION_PERMISSIONS: [(ServerSettingsSection, Permissions); 8] = [
    (ServerSettingsSection::Overview, Permissions::MANAGE_GUILD),
    (ServerSettingsSection::Roles, Permissions::MANAGE_ROLES),
    (
        ServerSettingsSection::Emoji,
        Permissions::MANAGE_EXPRESSIONS,
    ),
    (
        ServerSettingsSection::Soundboard,
        Permissions::MANAGE_EXPRESSIONS,
    ),
    (ServerSettingsSection::Invites, Permissions::MANAGE_GUILD),
    (
        ServerSettingsSection::Members,
        Permissions::KICK_MEMBERS
            .union(Permissions::BAN_MEMBERS)
            .union(Permissions::MANAGE_ROLES),
    ),
    (ServerSettingsSection::Bans, Permissions::BAN_MEMBERS),
    (
        ServerSettingsSection::ModerationLog,
        Permissions::VIEW_AUDIT_LOG,
    ),
];

/// Holds any of `permissions` (the owner and administrators hold them all).
#[must_use]
pub fn holds(guild: &Guild, permissions: Permissions) -> bool {
    guild.my_permissions.contains(Permissions::ADMINISTRATOR)
        || guild.my_permissions.intersects(permissions)
}

#[must_use]
pub fn visible_sections(guild: &Guild) -> Vec<ServerSettingsSection> {
    SECTION_PERMISSIONS
        .iter()
        .filter(|(_, permissions)| holds(guild, *permissions))
        .map(|(section, _)| *section)
        .collect()
}

#[must_use]
pub fn is_owner(guild: &Guild, user_id: Option<Snowflake>) -> bool {
    user_id == Some(guild.owner_id)
}

/// The owner always can: "Delete server" is theirs alone.
#[must_use]
pub fn can_open_server_settings(guild: &Guild, user_id: Option<Snowflake>) -> bool {
    is_owner(guild, user_id) || !visible_sections(guild).is_empty()
}

/// Whether leaving the shown page must wait for its unsaved changes. Only the page that holds them
/// keeps the reader: a page that went away (its permission was lost) never traps them.
#[must_use]
pub fn must_stay_on_page<Section: PartialEq + Copy>(
    unsaved_section: Option<Section>,
    shown_section: Option<Section>,
) -> bool {
    unsaved_section.is_some() && unsaved_section == shown_section
}

/// The row to focus once one is gone: the next row, else the previous one, else none.
#[must_use]
pub fn focus_after_removal<Id: PartialEq + Copy>(row_ids: &[Id], removed_id: Id) -> Option<Id> {
    let index = row_ids.iter().position(|id| *id == removed_id)?;
    row_ids
        .get(index + 1)
        .or_else(|| {
            index
                .checked_sub(1)
                .and_then(|previous| row_ids.get(previous))
        })
        .copied()
}

// --- Roles ---------------------------------------------------------------------------------------

/// Discord's role colour presets: user-chosen data shown on names, not design tokens.
pub const ROLE_COLOR_PRESETS: [u32; 20] = [
    0x1a_bc9c, 0x2e_cc71, 0x34_98db, 0x9b_59b6, 0xe9_1e63, 0xf1_c40f, 0xe6_7e22, 0xe7_4c3c,
    0x95_a5a6, 0x60_7d8b, 0x11_806a, 0x1f_8b4c, 0x20_6694, 0x71_368a, 0xad_1457, 0xc2_7c0e,
    0xa8_4300, 0x99_2d22, 0x97_9c9f, 0x54_6e7a,
];
/// The wire value of "no colour", as on Discord.
pub const NO_ROLE_COLOR: u32 = 0;

/// `#RRGGBB` of a role colour (`0xRRGGBB`), as picked: swatches and dots, never names.
#[must_use]
pub fn role_color_hex(color: u32) -> String {
    format!("#{color:06X}")
}

/// What the signed-in member may do with roles; mirrors `RoleManager` in the server's domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleManager {
    pub permissions: Permissions,
    pub is_owner: bool,
    pub highest_role_position: u32,
}

#[must_use]
pub fn role_manager_of(guild: &Guild, user_id: Option<Snowflake>) -> RoleManager {
    RoleManager {
        permissions: guild.my_permissions,
        is_owner: is_owner(guild, user_id),
        highest_role_position: highest_position(&guild.roles, &guild.my_role_ids),
    }
}

/// The highest position among `role_ids`, 0 (`@everyone`'s) when none.
fn highest_position(roles: &[Role], role_ids: &[Snowflake]) -> u32 {
    roles
        .iter()
        .filter(|role| role_ids.contains(&role.id))
        .map(|role| role.position)
        .max()
        .unwrap_or(0)
}

/// `@everyone`'s id is the guild's id.
#[must_use]
pub fn can_manage_role(manager: &RoleManager, role: &Role, guild_id: Snowflake) -> bool {
    manager.permissions.contains(Permissions::MANAGE_ROLES)
        && (manager.is_owner
            || ((role.id == guild_id || role.position < manager.highest_role_position)
                && manager.permissions.contains(role.permissions)))
}

/// Nobody grants or removes a permission they do not hold.
#[must_use]
pub fn can_toggle_permission(manager: &RoleManager, permission: Permissions) -> bool {
    manager.permissions.contains(permission)
}

/// Highest first, `@everyone` last, as the role list shows them.
#[must_use]
pub fn roles_highest_first(guild: &Guild) -> Vec<&Role> {
    let mut roles: Vec<&Role> = guild.roles.iter().collect();
    roles.sort_by_key(|role| std::cmp::Reverse(role.position));
    roles
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveDirection {
    Up,
    Down,
}

/// The `PUT /roles/order` body (named roles, lowest to highest) after swapping `role_id` with its
/// neighbour, or `None` when the manager may not: both roles change slots, so both must be theirs.
#[must_use]
pub fn role_ids_after_move(
    guild: &Guild,
    manager: &RoleManager,
    role_id: Snowflake,
    direction: MoveDirection,
) -> Option<Vec<Snowflake>> {
    let mut named: Vec<&Role> = guild
        .roles
        .iter()
        .filter(|role| role.id != guild.id)
        .collect();
    named.sort_by(|left, right| {
        left.position
            .cmp(&right.position)
            .then(left.id.raw().cmp(&right.id.raw()))
    });
    let index = named.iter().position(|role| role.id == role_id)?;
    let neighbour_index = match direction {
        MoveDirection::Up => index + 1,
        MoveDirection::Down => index.checked_sub(1)?,
    };
    let role = named.get(index)?;
    let neighbour = named.get(neighbour_index)?;
    if !can_manage_role(manager, role, guild.id) || !can_manage_role(manager, neighbour, guild.id) {
        return None;
    }
    named.swap(index, neighbour_index);
    Some(named.iter().map(|candidate| candidate.id).collect())
}

/// Whether any two neighbouring roles may swap, so the move shortcuts are worth a visible hint.
#[must_use]
pub fn can_move_any_role(guild: &Guild, manager: &RoleManager) -> bool {
    guild
        .roles
        .iter()
        .any(|role| role_ids_after_move(guild, manager, role.id, MoveDirection::Up).is_some())
}

/// Members holding the role; every member holds `@everyone`.
#[must_use]
pub fn role_member_count(guild: &Guild, role: &Role, members: &[Member]) -> usize {
    if role.id == guild.id {
        members.len()
    } else {
        members
            .iter()
            .filter(|member| member.role_ids.contains(&role.id))
            .count()
    }
}

/// The editable part of a role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleDraft {
    pub name: String,
    pub color: u32,
    pub permissions: Permissions,
    pub is_displayed_separately: bool,
    /// Anyone may `@mention` it (ADR-0040).
    pub is_mentionable: bool,
}

#[must_use]
pub fn draft_of(role: &Role) -> RoleDraft {
    RoleDraft {
        name: role.name.clone(),
        color: role.color,
        permissions: role.permissions,
        is_displayed_separately: role.is_displayed_separately,
        is_mentionable: role.is_mentionable,
    }
}

/// What the user changed, on top of the role as it was when they started changing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleEdit {
    pub role_id: Snowflake,
    pub base: RoleDraft,
    pub draft: RoleDraft,
}

/// The instance trims names, so trailing spaces alone are not a change.
fn is_same_as_stored(draft: &RoleDraft, stored: &RoleDraft) -> bool {
    draft.name.trim() == stored.name
        && draft.color == stored.color
        && draft.permissions == stored.permissions
        && draft.is_displayed_separately == stored.is_displayed_separately
        && draft.is_mentionable == stored.is_mentionable
}

/// `wanted` where it differs from `base`, else `current`: a field the user did not touch follows
/// the stored role.
fn rebased<T: PartialEq + Clone>(wanted: &T, base: &T, current: &T) -> T {
    if wanted == base {
        current.clone()
    } else {
        wanted.clone()
    }
}

/// The same changes on top of `stored`, or `None` once they no longer differ from it: a change
/// arriving meanwhile (another manager revoking a permission) is never undone by saving an older
/// copy of the role. Only toggled permission bits move; unknown bits of a newer server stay.
#[must_use]
pub fn rebase_role_edit(edit: &RoleEdit, stored: &Role) -> Option<RoleEdit> {
    let base = draft_of(stored);
    let wanted = edit.draft.permissions.bits();
    let toggled = wanted ^ edit.base.permissions.bits();
    let draft = RoleDraft {
        name: rebased(&edit.draft.name, &edit.base.name, &base.name),
        color: rebased(&edit.draft.color, &edit.base.color, &base.color),
        is_displayed_separately: rebased(
            &edit.draft.is_displayed_separately,
            &edit.base.is_displayed_separately,
            &base.is_displayed_separately,
        ),
        is_mentionable: rebased(
            &edit.draft.is_mentionable,
            &edit.base.is_mentionable,
            &base.is_mentionable,
        ),
        // The toggled bits take the wanted value, so a role that already has it (our own save
        // echoed by the gateway) ends the edit instead of reverting it.
        permissions: Permissions::from_bits_retain(
            (base.permissions.bits() & !toggled) | (wanted & toggled),
        ),
    };
    (!is_same_as_stored(&draft, &base)).then_some(RoleEdit {
        role_id: edit.role_id,
        base,
        draft,
    })
}

/// Whether an edit still has something to save on top of the stored role. Once it has not (saved,
/// undone, or the role is gone) the page drops it, so that a later change of the role (someone
/// undoing it) cannot bring a stale edit back.
#[must_use]
pub fn is_role_edit_pending(edit: &RoleEdit, stored: Option<&Role>) -> bool {
    stored
        .is_some_and(|stored| edit.role_id == stored.id && rebase_role_edit(edit, stored).is_some())
}

#[must_use]
pub fn toggle_permission(draft: &RoleDraft, permission: Permissions) -> RoleDraft {
    RoleDraft {
        permissions: Permissions::from_bits_retain(draft.permissions.bits() ^ permission.bits()),
        ..draft.clone()
    }
}

/// Only the changed fields; `@everyone` takes permissions only. `None` for a blank name.
#[must_use]
pub fn role_update_body(
    stored: &Role,
    draft: &RoleDraft,
    is_everyone: bool,
) -> Option<UpdateRoleRequest> {
    let name = draft.name.trim();
    if !is_everyone && name.is_empty() {
        return None;
    }
    let named = !is_everyone;
    Some(UpdateRoleRequest {
        name: (named && name != stored.name).then(|| name.to_owned()),
        color: (named && draft.color != stored.color).then_some(draft.color),
        is_displayed_separately: (named
            && draft.is_displayed_separately != stored.is_displayed_separately)
            .then_some(draft.is_displayed_separately),
        // Left out when unchanged, so an instance from before ADR-0040 still accepts the request.
        is_mentionable: (named && draft.is_mentionable != stored.is_mentionable)
            .then_some(draft.is_mentionable),
        permissions: (draft.permissions != stored.permissions).then_some(draft.permissions),
    })
}

/// A permission the editors show; the client's catalogue names it (`serverSettings.permissions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionKey {
    ViewChannels,
    ManageChannels,
    ManageRoles,
    ManageServer,
    ManageExpressions,
    ViewModerationLog,
    CreateInvite,
    SendMessages,
    ReadMessageHistory,
    AttachFiles,
    AddReactions,
    SendPolls,
    SendChecklists,
    MentionEveryone,
    ManageMessages,
    Connect,
    Speak,
    UseSoundboard,
    Stream,
    KickMembers,
    BanMembers,
    Administrator,
}

/// A group of the editors (`serverSettings.roles.groups`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionGroupKey {
    General,
    Membership,
    TextChannels,
    VoiceChannels,
    Moderation,
    Advanced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionDescription {
    pub key: PermissionKey,
    pub bit: Permissions,
    pub is_sensitive: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionGroup {
    pub group: PermissionGroupKey,
    pub permissions: &'static [PermissionDescription],
}

const fn permission(key: PermissionKey, bit: Permissions) -> PermissionDescription {
    PermissionDescription {
        key,
        bit,
        is_sensitive: false,
    }
}

const fn sensitive(key: PermissionKey, bit: Permissions) -> PermissionDescription {
    PermissionDescription {
        key,
        bit,
        is_sensitive: true,
    }
}

/// Every permission the editor shows, by group, in Discord's order. Nicknames have no feature yet,
/// so their bits are not offered; like any bit not listed, they stay as stored.
pub const PERMISSION_GROUPS: [PermissionGroup; 6] = [
    PermissionGroup {
        group: PermissionGroupKey::General,
        permissions: &[
            permission(PermissionKey::ViewChannels, Permissions::VIEW_CHANNEL),
            permission(PermissionKey::ManageChannels, Permissions::MANAGE_CHANNELS),
            sensitive(PermissionKey::ManageRoles, Permissions::MANAGE_ROLES),
            permission(PermissionKey::ManageServer, Permissions::MANAGE_GUILD),
            permission(
                PermissionKey::ManageExpressions,
                Permissions::MANAGE_EXPRESSIONS,
            ),
            permission(
                PermissionKey::ViewModerationLog,
                Permissions::VIEW_AUDIT_LOG,
            ),
        ],
    },
    PermissionGroup {
        group: PermissionGroupKey::Membership,
        permissions: &[permission(
            PermissionKey::CreateInvite,
            Permissions::CREATE_INVITE,
        )],
    },
    PermissionGroup {
        group: PermissionGroupKey::TextChannels,
        permissions: &[
            permission(PermissionKey::SendMessages, Permissions::SEND_MESSAGES),
            permission(
                PermissionKey::ReadMessageHistory,
                Permissions::READ_MESSAGE_HISTORY,
            ),
            permission(PermissionKey::AttachFiles, Permissions::ATTACH_FILES),
            permission(PermissionKey::AddReactions, Permissions::ADD_REACTIONS),
            permission(PermissionKey::SendPolls, Permissions::SEND_POLLS),
            permission(PermissionKey::SendChecklists, Permissions::SEND_CHECKLISTS),
            permission(
                PermissionKey::MentionEveryone,
                Permissions::MENTION_EVERYONE,
            ),
            permission(PermissionKey::ManageMessages, Permissions::MANAGE_MESSAGES),
        ],
    },
    PermissionGroup {
        group: PermissionGroupKey::VoiceChannels,
        permissions: &[
            permission(PermissionKey::Connect, Permissions::CONNECT),
            permission(PermissionKey::Speak, Permissions::SPEAK),
            permission(PermissionKey::UseSoundboard, Permissions::USE_SOUNDBOARD),
            permission(PermissionKey::Stream, Permissions::STREAM),
        ],
    },
    PermissionGroup {
        group: PermissionGroupKey::Moderation,
        permissions: &[
            permission(PermissionKey::KickMembers, Permissions::KICK_MEMBERS),
            sensitive(PermissionKey::BanMembers, Permissions::BAN_MEMBERS),
        ],
    },
    PermissionGroup {
        group: PermissionGroupKey::Advanced,
        permissions: &[sensitive(
            PermissionKey::Administrator,
            Permissions::ADMINISTRATOR,
        )],
    },
];

// --- Members -------------------------------------------------------------------------------------

/// Whether the manager may kick, ban or (with `MANAGE_ROLES`) act on this member: strictly below
/// their highest role, never the owner or themselves (the server's `outranks`).
#[must_use]
pub fn outranks_member(
    guild: &Guild,
    manager: &RoleManager,
    member: &Member,
    user_id: Option<Snowflake>,
) -> bool {
    if Some(member.user.id) == user_id || member.user.id == guild.owner_id {
        return false;
    }
    manager.is_owner
        || manager.highest_role_position > highest_position(&guild.roles, &member.role_ids)
}

/// Case- and accent-insensitive, on the display name and the username.
#[must_use]
pub fn matches_member_search(member: &Member, query: &str) -> bool {
    let needle = fold_for_search(query.trim());
    needle.is_empty()
        || fold_for_search(&member.user.display_name).contains(&needle)
        || fold_for_search(&member.user.username).contains(&needle)
}

#[cfg(test)]
pub(crate) mod tests {
    use mellow_protocol::{Channel, ChannelKind, PublicUser};

    use super::*;

    pub(crate) const GUILD: u64 = 100;
    const OWNER: u64 = 1;
    const ME: u64 = 2;
    const OTHER: u64 = 3;

    pub(crate) fn id(raw: u64) -> Snowflake {
        Snowflake::from_raw(raw)
    }

    pub(crate) fn role(raw: u64, position: u32, permissions: Permissions) -> Role {
        Role {
            id: id(raw),
            name: format!("role {raw}"),
            color: 0,
            position,
            permissions,
            is_displayed_separately: false,
            is_mentionable: false,
            icon: None,
            unicode_emoji: None,
        }
    }

    fn everyone() -> Role {
        role(
            GUILD,
            0,
            Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
        )
    }
    fn helper() -> Role {
        role(201, 1, Permissions::empty())
    }
    fn moderator() -> Role {
        role(
            202,
            2,
            Permissions::MANAGE_ROLES | Permissions::KICK_MEMBERS,
        )
    }
    fn admin() -> Role {
        role(203, 3, Permissions::ADMINISTRATOR)
    }

    pub(crate) fn guild_with(
        my_permissions: Permissions,
        my_role_ids: &[u64],
        roles: Vec<Role>,
        channels: Vec<Channel>,
    ) -> Guild {
        Guild {
            id: id(GUILD),
            name: "Club".to_owned(),
            owner_id: id(OWNER),
            icon: None,
            my_permissions,
            my_role_ids: my_role_ids.iter().copied().map(id).collect(),
            roles,
            channels,
            emojis: Vec::new(),
            sounds: Vec::new(),
            my_join_sound_id: None,
        }
    }

    fn guild(my_permissions: Permissions, my_role_ids: &[u64]) -> Guild {
        let general = Channel {
            id: id(300),
            guild_id: id(GUILD),
            kind: ChannelKind::Text,
            name: "general".to_owned(),
            topic: String::new(),
            parent_id: None,
            position: 0,
            icon: None,
            permission_overwrites: Vec::new(),
            user_limit: None,
        };
        guild_with(
            my_permissions,
            my_role_ids,
            vec![everyone(), helper(), moderator(), admin()],
            vec![general],
        )
    }

    pub(crate) fn member(raw: u64, display_name: &str, role_ids: &[u64]) -> Member {
        Member {
            user: PublicUser {
                id: id(raw),
                username: display_name.to_lowercase(),
                display_name: display_name.to_owned(),
                avatar: None,
                banner: None,
            },
            role_ids: role_ids.iter().copied().map(id).collect(),
            joined_at_unix_milliseconds: 0,
        }
    }

    #[test]
    fn shows_each_section_only_with_its_permission_in_discords_order() {
        use ServerSettingsSection::{
            Bans, Emoji, Invites, Members, ModerationLog, Overview, Roles, Soundboard,
        };
        assert_eq!(visible_sections(&guild(Permissions::VIEW_CHANNEL, &[])), []);
        assert_eq!(
            visible_sections(&guild(Permissions::KICK_MEMBERS, &[])),
            [Members]
        );
        assert_eq!(
            visible_sections(&guild(Permissions::MANAGE_EXPRESSIONS, &[])),
            [Emoji, Soundboard]
        );
        assert_eq!(
            visible_sections(&guild(
                Permissions::MANAGE_GUILD | Permissions::BAN_MEMBERS,
                &[]
            )),
            [Overview, Invites, Members, Bans]
        );
        assert_eq!(
            visible_sections(&guild(Permissions::ADMINISTRATOR, &[])),
            [
                Overview,
                Roles,
                Emoji,
                Soundboard,
                Invites,
                Members,
                Bans,
                ModerationLog
            ]
        );
    }

    #[test]
    fn opens_for_the_owner_even_without_a_section_so_they_can_delete_the_server() {
        let nothing = guild(Permissions::empty(), &[]);
        assert!(can_open_server_settings(&nothing, Some(id(OWNER))));
        assert!(!can_open_server_settings(&nothing, Some(id(ME))));
        assert!(!can_open_server_settings(&nothing, None));
        assert!(can_open_server_settings(
            &guild(Permissions::VIEW_AUDIT_LOG, &[]),
            Some(id(ME))
        ));
    }

    #[test]
    fn unsaved_changes_keep_the_reader_only_on_the_page_that_holds_them() {
        use ServerSettingsSection::{Members, Overview, Roles};
        assert!(must_stay_on_page(Some(Roles), Some(Roles)));
        // Roles went away with MANAGE_ROLES: the page shown now holds nothing.
        assert!(!must_stay_on_page(Some(Roles), Some(Members)));
        assert!(!must_stay_on_page(None, Some(Overview)));
        assert!(!must_stay_on_page::<ServerSettingsSection>(None, None));
    }

    #[test]
    fn focus_moves_to_the_next_row_else_the_previous_one_else_nowhere() {
        assert_eq!(focus_after_removal(&["a", "b", "c"], "b"), Some("c"));
        assert_eq!(focus_after_removal(&["a", "b", "c"], "c"), Some("b"));
        assert_eq!(focus_after_removal(&["a"], "a"), None);
        assert_eq!(focus_after_removal(&["a"], "z"), None);
    }

    #[test]
    fn role_colours_read_as_upper_case_hex() {
        assert_eq!(role_color_hex(0x0034_98db), "#3498DB");
        assert_eq!(role_color_hex(NO_ROLE_COLOR), "#000000");
        assert_eq!(role_color_hex(0x00ff), "#0000FF");
    }

    fn moderator_guild() -> Guild {
        guild(
            Permissions::MANAGE_ROLES | Permissions::KICK_MEMBERS,
            &[202],
        )
    }

    fn all_permissions() -> Permissions {
        Permissions::from_bits_retain(u64::MAX)
    }

    #[test]
    fn lists_roles_highest_first_with_everyone_last() {
        let club = moderator_guild();
        let ids: Vec<u64> = roles_highest_first(&club)
            .iter()
            .map(|role| role.id.raw())
            .collect();
        assert_eq!(ids, [203, 202, 201, GUILD]);
    }

    #[test]
    fn managers_edit_only_everyone_and_roles_below_their_highest_whose_permissions_they_hold() {
        let moderator_manager = role_manager_of(&moderator_guild(), Some(id(ME)));
        let everyone_id = id(GUILD);
        assert!(!can_manage_role(
            &moderator_manager,
            &everyone(),
            everyone_id
        ));
        assert!(can_manage_role(&moderator_manager, &helper(), everyone_id));
        assert!(!can_manage_role(
            &moderator_manager,
            &moderator(),
            everyone_id
        ));
        assert!(!can_manage_role(&moderator_manager, &admin(), everyone_id));
        let owner = role_manager_of(
            &guild(Permissions::ADMINISTRATOR | Permissions::MANAGE_ROLES, &[]),
            Some(id(OWNER)),
        );
        assert!(can_manage_role(&owner, &admin(), everyone_id));
        let with_everyones_permissions = role_manager_of(
            &guild(
                Permissions::MANAGE_ROLES | Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
                &[202],
            ),
            Some(id(ME)),
        );
        assert!(can_manage_role(
            &with_everyones_permissions,
            &everyone(),
            everyone_id
        ));
        assert!(can_toggle_permission(
            &with_everyones_permissions,
            Permissions::SEND_MESSAGES
        ));
        assert!(!can_toggle_permission(
            &with_everyones_permissions,
            Permissions::BAN_MEMBERS
        ));
    }

    #[test]
    fn moves_a_role_by_swapping_it_with_its_neighbour_only_when_both_are_manageable() {
        let owner = role_manager_of(&guild(all_permissions(), &[]), Some(id(OWNER)));
        let plain = guild(Permissions::empty(), &[]);
        assert_eq!(
            role_ids_after_move(&plain, &owner, id(201), MoveDirection::Up),
            Some(vec![id(202), id(201), id(203)])
        );
        assert_eq!(
            role_ids_after_move(&plain, &owner, id(203), MoveDirection::Up),
            None
        );
        assert_eq!(
            role_ids_after_move(&plain, &owner, id(201), MoveDirection::Down),
            None
        );
        let club = moderator_guild();
        let moderator_manager = role_manager_of(&club, Some(id(ME)));
        assert_eq!(
            role_ids_after_move(&club, &moderator_manager, id(201), MoveDirection::Up),
            None
        );
    }

    #[test]
    fn offers_the_move_hint_only_when_two_neighbouring_roles_may_swap() {
        let owner = role_manager_of(&guild(all_permissions(), &[]), Some(id(OWNER)));
        assert!(can_move_any_role(&guild(Permissions::empty(), &[]), &owner));
        let club = moderator_guild();
        assert!(!can_move_any_role(
            &club,
            &role_manager_of(&club, Some(id(ME)))
        ));
        let mut only_one_named_role = guild(Permissions::empty(), &[]);
        only_one_named_role
            .roles
            .retain(|role| role.id.raw() == GUILD || role.id.raw() == 201);
        assert!(!can_move_any_role(&only_one_named_role, &owner));
    }

    #[test]
    fn counts_a_roles_members_and_everyone_holds_everyone() {
        let club = guild(Permissions::empty(), &[]);
        let members = [member(5, "Ana", &[201]), member(6, "Paul", &[])];
        assert_eq!(role_member_count(&club, &everyone(), &members), 2);
        assert_eq!(role_member_count(&club, &helper(), &members), 1);
    }

    #[test]
    fn keeps_a_drafts_own_changes_on_top_of_a_role_changed_meanwhile() {
        let stored = role(
            201,
            1,
            Permissions::SEND_MESSAGES | Permissions::ATTACH_FILES,
        );
        let edit = RoleEdit {
            role_id: stored.id,
            base: draft_of(&stored),
            draft: RoleDraft {
                name: "Helpers".to_owned(),
                color: 0,
                permissions: Permissions::SEND_MESSAGES
                    | Permissions::ATTACH_FILES
                    | Permissions::KICK_MEMBERS,
                is_displayed_separately: true,
                is_mentionable: true,
            },
        };
        // Someone else removed ATTACH_FILES and set a colour.
        let changed = Role {
            color: 0x0034_98db,
            permissions: Permissions::SEND_MESSAGES,
            ..stored.clone()
        };
        assert_eq!(
            rebase_role_edit(&edit, &changed).map(|rebased| rebased.draft),
            Some(RoleDraft {
                name: "Helpers".to_owned(),
                color: 0x0034_98db,
                permissions: Permissions::SEND_MESSAGES | Permissions::KICK_MEMBERS,
                is_displayed_separately: true,
                is_mentionable: true,
            })
        );
        let saved = Role {
            name: "Helpers".to_owned(),
            permissions: edit.draft.permissions,
            is_displayed_separately: true,
            is_mentionable: true,
            ..stored.clone()
        };
        assert_eq!(rebase_role_edit(&edit, &saved), None);
        let trailing_space = RoleEdit {
            draft: RoleDraft {
                name: format!("{} ", stored.name),
                ..edit.base.clone()
            },
            ..edit.clone()
        };
        assert_eq!(rebase_role_edit(&trailing_space, &stored), None);
    }

    #[test]
    fn rebasing_keeps_unknown_bits_of_a_newer_server() {
        let unknown = Permissions::from_bits_retain(1 << 60);
        let stored = role(201, 1, unknown);
        let edit = RoleEdit {
            role_id: stored.id,
            base: draft_of(&stored),
            draft: toggle_permission(&draft_of(&stored), Permissions::SEND_MESSAGES),
        };
        assert_eq!(
            rebase_role_edit(&edit, &stored).map(|rebased| rebased.draft.permissions),
            Some(Permissions::from_bits_retain(
                unknown.bits() | Permissions::SEND_MESSAGES.bits()
            ))
        );
    }

    #[test]
    fn drops_an_edit_once_the_stored_role_holds_it_so_a_later_undo_cannot_revive_it() {
        let stored = role(201, 1, Permissions::empty());
        let edit = RoleEdit {
            role_id: stored.id,
            base: draft_of(&stored),
            draft: RoleDraft {
                name: "Helpers".to_owned(),
                ..draft_of(&stored)
            },
        };
        assert!(is_role_edit_pending(&edit, Some(&stored)));
        // Our save, echoed by the gateway: nothing left to save, the page drops the edit.
        let renamed = Role {
            name: "Helpers".to_owned(),
            ..stored.clone()
        };
        assert!(!is_role_edit_pending(&edit, Some(&renamed)));
        // Kept, the same edit would come back once someone renames the role back.
        assert!(rebase_role_edit(&edit, &stored).is_some());
        assert!(!is_role_edit_pending(&edit, None));
        assert!(!is_role_edit_pending(
            &edit,
            Some(&role(202, 2, Permissions::empty()))
        ));
    }

    #[test]
    fn sends_only_the_changed_fields_and_only_permissions_for_everyone() {
        let stored = role(201, 1, Permissions::empty());
        let unchanged = draft_of(&stored);
        let with = |change: fn(&mut RoleDraft)| {
            let mut draft = unchanged.clone();
            change(&mut draft);
            draft
        };
        assert_eq!(
            role_update_body(
                &stored,
                &with(|draft| draft.name = " Helpers ".to_owned()),
                false
            ),
            Some(UpdateRoleRequest {
                name: Some("Helpers".to_owned()),
                ..UpdateRoleRequest::default()
            })
        );
        assert_eq!(
            role_update_body(&stored, &with(|draft| draft.name = "  ".to_owned()), false),
            None
        );
        assert_eq!(
            role_update_body(
                &stored,
                &with(|draft| draft.is_displayed_separately = true),
                false
            ),
            Some(UpdateRoleRequest {
                is_displayed_separately: Some(true),
                ..UpdateRoleRequest::default()
            })
        );
        assert_eq!(
            role_update_body(&stored, &with(|draft| draft.is_mentionable = true), false),
            Some(UpdateRoleRequest {
                is_mentionable: Some(true),
                ..UpdateRoleRequest::default()
            })
        );
        let mentionable = Role {
            is_mentionable: true,
            ..stored.clone()
        };
        assert_eq!(
            role_update_body(&mentionable, &unchanged, false),
            Some(UpdateRoleRequest {
                is_mentionable: Some(false),
                ..UpdateRoleRequest::default()
            })
        );
        let everything = RoleDraft {
            name: "x".to_owned(),
            color: 5,
            permissions: Permissions::SEND_MESSAGES,
            is_displayed_separately: true,
            is_mentionable: true,
        };
        assert_eq!(
            role_update_body(&everyone(), &everything, true),
            Some(UpdateRoleRequest {
                permissions: Some(Permissions::SEND_MESSAGES),
                ..UpdateRoleRequest::default()
            })
        );
    }

    #[test]
    fn acts_only_on_members_ranked_strictly_below_never_the_owner_or_oneself() {
        let club = guild(Permissions::KICK_MEMBERS, &[202]);
        let manager = role_manager_of(&club, Some(id(ME)));
        let me = Some(id(ME));
        assert!(outranks_member(
            &club,
            &manager,
            &member(OTHER, "Ana", &[201]),
            me
        ));
        assert!(!outranks_member(
            &club,
            &manager,
            &member(OTHER, "Ana", &[202]),
            me
        ));
        assert!(!outranks_member(
            &club,
            &manager,
            &member(OWNER, "Owner", &[]),
            me
        ));
        assert!(!outranks_member(
            &club,
            &manager,
            &member(ME, "Me", &[]),
            me
        ));
    }

    #[test]
    fn searches_display_names_and_usernames_without_case_or_accents() {
        let zoe = member(OTHER, "Zoé", &[]);
        assert!(matches_member_search(&zoe, "zoe"));
        assert!(matches_member_search(&zoe, " ZO "));
        assert!(!matches_member_search(&zoe, "ana"));
        assert!(matches_member_search(&zoe, ""));
    }

    #[test]
    fn the_editor_offers_each_permission_once() {
        let mut bits = Permissions::empty();
        for described in PERMISSION_GROUPS.iter().flat_map(|group| group.permissions) {
            assert!(!bits.intersects(described.bit), "{:?} twice", described.key);
            bits |= described.bit;
        }
        // Nicknames have no feature yet.
        assert!(!bits.intersects(Permissions::CHANGE_NICKNAME | Permissions::MANAGE_NICKNAMES));
    }
}
