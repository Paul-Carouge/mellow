//! Pure rules of the channel settings (`web/src/components/channel-settings.ts`, DESIGN.md §5.8,
//! docs/design/web-channel-settings.md): which sections a member sees, the three-way Deny /
//! Inherit / Allow state of each channel permission, overwrite drafts, and which targets the member
//! may add, change or remove. The instance decides every action (ADR-0007); these rules only hide
//! or lock what it would refuse (ADR-0014).

use mellow_protocol::{
    Channel, ChannelKind, Guild, MAXIMUM_VOICE_CHANNEL_PARTICIPANTS, Member, OverwriteKind,
    Permissions, Role, Snowflake, UpdateChannelRequest, UserLimitChange,
};

use super::server_settings::{
    PERMISSION_GROUPS, PermissionDescription, PermissionGroupKey, RoleManager,
    matches_member_search,
};

/// A page of the channel settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelSettingsSection {
    Overview,
    Permissions,
}

/// Holds every bit of `permission`; administrators hold them all.
fn holds_in_channel(channel_permissions: Option<Permissions>, permission: Permissions) -> bool {
    channel_permissions
        .is_some_and(|held| held.contains(Permissions::ADMINISTRATOR) || held.contains(permission))
}

/// Overview with `MANAGE_CHANNELS`, Permissions with `MANAGE_ROLES`, both as held in this channel
/// (its overwrites included), as the instance checks them. Categories have no settings page yet.
#[must_use]
pub fn visible_channel_sections(
    kind: ChannelKind,
    channel_permissions: Option<Permissions>,
) -> Vec<ChannelSettingsSection> {
    if kind == ChannelKind::Category {
        return Vec::new();
    }
    [
        (
            ChannelSettingsSection::Overview,
            Permissions::MANAGE_CHANNELS,
        ),
        (
            ChannelSettingsSection::Permissions,
            Permissions::MANAGE_ROLES,
        ),
    ]
    .into_iter()
    .filter(|(_, permission)| holds_in_channel(channel_permissions, *permission))
    .map(|(section, _)| section)
    .collect()
}

/// Rename, delete, or move to another category: `MANAGE_CHANNELS` held in the channel itself.
#[must_use]
pub fn can_manage_channel(channel_permissions: Option<Permissions>) -> bool {
    holds_in_channel(channel_permissions, Permissions::MANAGE_CHANNELS)
}

/// The name as the instance stores it: as typed, trimmed (`normalize_channel_name`).
#[must_use]
pub fn channel_name_as_saved(name: &str) -> &str {
    name.trim()
}

/// What the user limit offers: unlimited, or 1 to 25 people.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceUserLimitChoice {
    Unlimited,
    People(u8),
}

/// "Unlimited", then 1 to `MAXIMUM_VOICE_CHANNEL_PARTICIPANTS` people, in the select's order.
#[must_use]
pub fn voice_user_limit_choices() -> Vec<VoiceUserLimitChoice> {
    std::iter::once(VoiceUserLimitChoice::Unlimited)
        .chain((1..=MAXIMUM_VOICE_CHANNEL_PARTICIPANTS).map(VoiceUserLimitChoice::People))
        .collect()
}

/// The choice showing a channel's stored limit.
#[must_use]
pub fn user_limit_choice_of(user_limit: Option<u8>) -> VoiceUserLimitChoice {
    user_limit.map_or(
        VoiceUserLimitChoice::Unlimited,
        VoiceUserLimitChoice::People,
    )
}

/// Overview fields typed by the member; `None` for a field left as stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelOverviewDrafts {
    pub name: Option<String>,
    pub topic: Option<String>,
    /// A voice channel's limit as chosen.
    pub user_limit: Option<VoiceUserLimitChoice>,
}

impl ChannelOverviewDrafts {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.topic.is_none() && self.user_limit.is_none()
    }

    /// Forgets each field the stored channel caught up with (our save, echoed by the gateway), so
    /// it shows the stored value again.
    pub fn forget_stored(&mut self, stored: &Channel) {
        if self
            .name
            .as_deref()
            .is_some_and(|name| channel_name_as_saved(name) == stored.name)
        {
            self.name = None;
        }
        if self
            .topic
            .as_deref()
            .is_some_and(|topic| topic.trim() == stored.topic)
        {
            self.topic = None;
        }
        if self.user_limit == Some(user_limit_choice_of(stored.user_limit)) {
            self.user_limit = None;
        }
    }
}

/// The fields to send: only those typed and different from the stored channel, as the instance
/// would store them. A field someone else changed meanwhile is never sent back with its old value.
#[must_use]
pub fn channel_overview_changes(
    stored: &Channel,
    drafts: &ChannelOverviewDrafts,
) -> UpdateChannelRequest {
    let name = drafts
        .name
        .as_deref()
        .map(channel_name_as_saved)
        .filter(|name| *name != stored.name);
    let topic = drafts
        .topic
        .as_deref()
        .map(str::trim)
        .filter(|topic| *topic != stored.topic);
    let user_limit = match drafts.user_limit {
        Some(choice) if choice != user_limit_choice_of(stored.user_limit) => match choice {
            VoiceUserLimitChoice::Unlimited => UserLimitChange::Clear,
            VoiceUserLimitChoice::People(limit) => UserLimitChange::Set(limit),
        },
        _ => UserLimitChange::Keep,
    };
    UpdateChannelRequest {
        name: name.map(str::to_owned),
        topic: topic.map(str::to_owned),
        user_limit,
    }
}

/// Whether a channel update would send nothing.
#[must_use]
pub fn is_channel_update_empty(request: &UpdateChannelRequest) -> bool {
    request.name.is_none() && request.topic.is_none() && request.user_limit.is_keep()
}

// --- Overwrites ----------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteState {
    Deny,
    Inherit,
    Allow,
}

/// Discord's order, so Allow sits where its users look for it.
pub const OVERWRITE_STATES: [OverwriteState; 3] = [
    OverwriteState::Deny,
    OverwriteState::Inherit,
    OverwriteState::Allow,
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OverwriteBits {
    pub allow: Permissions,
    pub deny: Permissions,
}

#[must_use]
pub fn overwrite_state_of(bits: OverwriteBits, permission: Permissions) -> OverwriteState {
    if bits.allow.intersects(permission) {
        OverwriteState::Allow
    } else if bits.deny.intersects(permission) {
        OverwriteState::Deny
    } else {
        OverwriteState::Inherit
    }
}

/// A bit is never allowed and denied at once (the instance refuses it). Unknown bits stay.
#[must_use]
pub fn with_overwrite_state(
    bits: OverwriteBits,
    permission: Permissions,
    state: OverwriteState,
) -> OverwriteBits {
    let allow = bits.allow.bits() & !permission.bits();
    let deny = bits.deny.bits() & !permission.bits();
    OverwriteBits {
        allow: Permissions::from_bits_retain(if state == OverwriteState::Allow {
            allow | permission.bits()
        } else {
            allow
        }),
        deny: Permissions::from_bits_retain(if state == OverwriteState::Deny {
            deny | permission.bits()
        } else {
            deny
        }),
    }
}

/// A group of the channel permission editor: the role editor's group, channel-scoped bits only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelPermissionGroup {
    pub group: PermissionGroupKey,
    pub permissions: Vec<PermissionDescription>,
}

/// The channel-scoped permissions of the role editor's groups, in the same order.
#[must_use]
pub fn channel_permission_groups() -> Vec<ChannelPermissionGroup> {
    PERMISSION_GROUPS
        .iter()
        .map(|group| ChannelPermissionGroup {
            group: group.group,
            permissions: group
                .permissions
                .iter()
                .filter(|permission| Permissions::CHANNEL_SCOPED.intersects(permission.bit))
                .copied()
                .collect(),
        })
        .filter(|group| !group.permissions.is_empty())
        .collect()
}

/// As on Discord, a channel's editor leaves out the other kind's permissions: no "Speak" in a text
/// channel.
#[must_use]
pub fn channel_permission_groups_for(kind: ChannelKind) -> Vec<ChannelPermissionGroup> {
    let other_kind_group = if kind == ChannelKind::Voice {
        PermissionGroupKey::TextChannels
    } else {
        PermissionGroupKey::VoiceChannels
    };
    channel_permission_groups()
        .into_iter()
        .filter(|group| group.group != other_kind_group)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverwriteTarget {
    pub kind: OverwriteKind,
    pub id: Snowflake,
}

/// The list on the leading side: roles with an overwrite, highest first, then `@everyone`, which is
/// always there (as on Discord), then members with an overwrite in the order they were added.
#[must_use]
pub fn overwrite_targets(guild: &Guild, channel: &Channel) -> Vec<OverwriteTarget> {
    let has_role_overwrite = |role_id: Snowflake| {
        channel
            .permission_overwrites
            .iter()
            .any(|overwrite| overwrite.kind == OverwriteKind::Role && overwrite.id == role_id)
    };
    let mut roles: Vec<&Role> = guild
        .roles
        .iter()
        .filter(|role| role.id != guild.id && has_role_overwrite(role.id))
        .collect();
    roles.sort_by_key(|role| std::cmp::Reverse(role.position));
    let role_target = |id| OverwriteTarget {
        kind: OverwriteKind::Role,
        id,
    };
    roles
        .into_iter()
        .map(|role| role_target(role.id))
        .chain(std::iter::once(role_target(guild.id)))
        .chain(
            channel
                .permission_overwrites
                .iter()
                .filter(|overwrite| overwrite.kind == OverwriteKind::Member)
                .map(|overwrite| OverwriteTarget {
                    kind: OverwriteKind::Member,
                    id: overwrite.id,
                }),
        )
        .collect()
}

/// No overwrite reads as all Inherit.
#[must_use]
pub fn stored_overwrite_of(channel: &Channel, target_id: Snowflake) -> OverwriteBits {
    channel
        .permission_overwrites
        .iter()
        .find(|overwrite| overwrite.id == target_id)
        .map(|overwrite| OverwriteBits {
            allow: overwrite.allow,
            deny: overwrite.deny,
        })
        .unwrap_or_default()
}

/// What the user changed for one target, on top of its overwrite as it was when they started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverwriteEdit {
    pub target: OverwriteTarget,
    pub base: OverwriteBits,
    pub draft: OverwriteBits,
}

/// The same changes on top of `stored`, or `None` once they no longer differ from it. As role
/// drafts do (`rebase_role_edit`), only the bits the user changed take the wanted value, so a
/// change made meanwhile by someone else is kept, and our own save echoed by the gateway ends the
/// edit.
#[must_use]
pub fn rebase_overwrite_edit(edit: &OverwriteEdit, stored: OverwriteBits) -> Option<OverwriteEdit> {
    let changed = (edit.draft.allow.bits() ^ edit.base.allow.bits())
        | (edit.draft.deny.bits() ^ edit.base.deny.bits());
    let take = |wanted: Permissions, current: Permissions| {
        Permissions::from_bits_retain((current.bits() & !changed) | (wanted.bits() & changed))
    };
    let draft = OverwriteBits {
        allow: take(edit.draft.allow, stored.allow),
        deny: take(edit.draft.deny, stored.deny),
    };
    (draft != stored).then_some(OverwriteEdit {
        target: edit.target,
        base: stored,
        draft,
    })
}

/// `@everyone`, or a role below the manager's highest (the owner targets any): the server's
/// `can_target_role_in_overwrites`.
#[must_use]
pub fn can_target_role(manager: &RoleManager, role: &Role, guild_id: Snowflake) -> bool {
    manager.is_owner || role.id == guild_id || role.position < manager.highest_role_position
}

/// Nobody allows, denies or resets a permission they do not hold in this channel.
#[must_use]
pub fn can_change_channel_permission(
    channel_permissions: Option<Permissions>,
    permission: Permissions,
) -> bool {
    holds_in_channel(channel_permissions, permission)
}

/// Whether the target's overwrite can be changed at all: a role must be one the manager may target.
#[must_use]
pub fn can_edit_target(guild: &Guild, manager: &RoleManager, target: OverwriteTarget) -> bool {
    match target.kind {
        OverwriteKind::Member => true,
        OverwriteKind::Role => guild
            .roles
            .iter()
            .find(|role| role.id == target.id)
            .is_some_and(|role| can_target_role(manager, role, guild.id)),
    }
}

/// `@everyone` stays; removing needs every permission the overwrite allows or denies.
#[must_use]
pub fn can_remove_overwrite(
    guild: &Guild,
    manager: &RoleManager,
    channel_permissions: Option<Permissions>,
    channel: &Channel,
    target: OverwriteTarget,
) -> bool {
    channel
        .permission_overwrites
        .iter()
        .find(|overwrite| overwrite.id == target.id)
        .is_some_and(|stored| {
            target.id != guild.id
                && can_edit_target(guild, manager, target)
                && can_change_channel_permission(
                    channel_permissions,
                    Permissions::from_bits_retain(stored.allow.bits() | stored.deny.bits()),
                )
        })
}

fn has_overwrite(channel: &Channel, target_id: Snowflake) -> bool {
    channel
        .permission_overwrites
        .iter()
        .any(|overwrite| overwrite.id == target_id)
}

/// Roles that can be added: not in the list yet, below the manager's highest, highest first.
#[must_use]
pub fn addable_roles<'g>(
    guild: &'g Guild,
    channel: &Channel,
    manager: &RoleManager,
    query: &str,
) -> Vec<&'g Role> {
    let needle = query.trim().to_lowercase();
    let mut roles: Vec<&Role> = guild
        .roles
        .iter()
        .filter(|role| {
            role.id != guild.id
                && !has_overwrite(channel, role.id)
                && can_target_role(manager, role, guild.id)
                && role.name.to_lowercase().contains(&needle)
        })
        .collect();
    roles.sort_by_key(|role| std::cmp::Reverse(role.position));
    roles
}

/// Members that can be added: anyone without an overwrite here, whatever their rank (as on
/// Discord).
#[must_use]
pub fn addable_members<'m>(
    channel: &Channel,
    members: &'m [Member],
    query: &str,
) -> Vec<&'m Member> {
    members
        .iter()
        .filter(|member| {
            !has_overwrite(channel, member.user.id) && matches_member_search(member, query)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use mellow_protocol::PermissionOverwrite;

    use super::super::server_settings::PermissionKey;
    use super::super::server_settings::tests::{GUILD, guild_with, id, member, role};
    use super::*;

    const CHANNEL: u64 = 500;

    fn named_role(raw: u64, position: u32, name: &str) -> Role {
        Role {
            name: name.to_owned(),
            ..role(raw, position, Permissions::empty())
        }
    }

    fn everyone() -> Role {
        named_role(GUILD, 0, "@everyone")
    }
    fn helper() -> Role {
        named_role(201, 1, "Helpers")
    }
    fn moderator() -> Role {
        named_role(202, 2, "Moderators")
    }
    fn admin() -> Role {
        named_role(203, 3, "Admins")
    }

    fn overwrite(
        raw: u64,
        kind: OverwriteKind,
        allow: Permissions,
        deny: Permissions,
    ) -> PermissionOverwrite {
        PermissionOverwrite {
            id: id(raw),
            kind,
            allow,
            deny,
        }
    }

    fn channel(overwrites: Vec<PermissionOverwrite>, kind: ChannelKind) -> Channel {
        Channel {
            id: id(CHANNEL),
            guild_id: id(GUILD),
            kind,
            name: "general".to_owned(),
            topic: String::new(),
            parent_id: None,
            position: 0,
            icon: None,
            permission_overwrites: overwrites,
            user_limit: None,
        }
    }

    fn text(overwrites: Vec<PermissionOverwrite>) -> Channel {
        channel(overwrites, ChannelKind::Text)
    }

    fn guild(channels: Vec<Channel>) -> Guild {
        guild_with(
            Permissions::empty(),
            &[],
            vec![everyone(), helper(), moderator(), admin()],
            channels,
        )
    }

    /// A moderator: may target Helpers and @everyone, not Moderators or Admins.
    const MODERATOR_MANAGER: RoleManager = RoleManager {
        permissions: Permissions::MANAGE_ROLES,
        is_owner: false,
        highest_role_position: 2,
    };
    const OWNER_MANAGER: RoleManager = RoleManager {
        permissions: Permissions::empty(),
        is_owner: true,
        highest_role_position: 0,
    };

    #[test]
    fn shows_overview_with_manage_channels_and_permissions_with_manage_roles_in_the_channel() {
        use ChannelSettingsSection::{Overview, Permissions as PermissionsPage};
        let sections = |held| visible_channel_sections(ChannelKind::Text, Some(held));
        assert_eq!(sections(Permissions::MANAGE_CHANNELS), [Overview]);
        assert_eq!(sections(Permissions::MANAGE_ROLES), [PermissionsPage]);
        assert_eq!(
            sections(Permissions::MANAGE_CHANNELS | Permissions::MANAGE_ROLES),
            [Overview, PermissionsPage]
        );
        assert_eq!(sections(Permissions::SEND_MESSAGES), []);
        assert_eq!(visible_channel_sections(ChannelKind::Text, None), []);
    }

    #[test]
    fn gives_voice_channels_and_administrators_both_pages_and_categories_none_yet() {
        use ChannelSettingsSection::{Overview, Permissions as PermissionsPage};
        let admin = Some(Permissions::ADMINISTRATOR);
        assert_eq!(
            visible_channel_sections(ChannelKind::Voice, admin),
            [Overview, PermissionsPage]
        );
        assert_eq!(
            visible_channel_sections(ChannelKind::Text, admin),
            [Overview, PermissionsPage]
        );
        assert_eq!(visible_channel_sections(ChannelKind::Category, admin), []);
        assert!(can_manage_channel(admin));
        assert!(!can_manage_channel(Some(Permissions::MANAGE_ROLES)));
    }

    #[test]
    fn keeps_capitals_and_inner_spaces_trimming_only_the_ends_as_the_instance_does() {
        assert_eq!(channel_name_as_saved("  Off Topic  "), "Off Topic");
        assert_eq!(channel_name_as_saved("Book   club"), "Book   club");
        assert_eq!(channel_name_as_saved("   "), "");
    }

    #[test]
    fn reads_allow_deny_or_inherit_for_each_bit() {
        let bits = OverwriteBits {
            allow: Permissions::SEND_MESSAGES,
            deny: Permissions::ATTACH_FILES,
        };
        assert_eq!(
            overwrite_state_of(bits, Permissions::SEND_MESSAGES),
            OverwriteState::Allow
        );
        assert_eq!(
            overwrite_state_of(bits, Permissions::ATTACH_FILES),
            OverwriteState::Deny
        );
        assert_eq!(
            overwrite_state_of(bits, Permissions::VIEW_CHANNEL),
            OverwriteState::Inherit
        );
    }

    #[test]
    fn sets_one_bit_without_touching_the_others_never_allowed_and_denied_at_once() {
        let bits = OverwriteBits {
            allow: Permissions::SEND_MESSAGES,
            deny: Permissions::ATTACH_FILES,
        };
        let denied = with_overwrite_state(bits, Permissions::SEND_MESSAGES, OverwriteState::Deny);
        assert_eq!(
            denied,
            OverwriteBits {
                allow: Permissions::empty(),
                deny: Permissions::SEND_MESSAGES | Permissions::ATTACH_FILES
            }
        );
        let inherited =
            with_overwrite_state(denied, Permissions::ATTACH_FILES, OverwriteState::Inherit);
        assert_eq!(
            inherited,
            OverwriteBits {
                allow: Permissions::empty(),
                deny: Permissions::SEND_MESSAGES
            }
        );
        let allowed =
            with_overwrite_state(inherited, Permissions::SEND_MESSAGES, OverwriteState::Allow);
        assert_eq!(
            allowed,
            OverwriteBits {
                allow: Permissions::SEND_MESSAGES,
                deny: Permissions::empty()
            }
        );
    }

    fn keys_of(groups: &[ChannelPermissionGroup]) -> Vec<PermissionKey> {
        groups
            .iter()
            .flat_map(|group| group.permissions.iter().map(|permission| permission.key))
            .collect()
    }

    #[test]
    fn offers_only_the_channel_scoped_permissions() {
        let keys = keys_of(&channel_permission_groups());
        assert!(keys.contains(&PermissionKey::SendMessages));
        assert!(keys.contains(&PermissionKey::ManageRoles));
        assert!(!keys.contains(&PermissionKey::Administrator));
        assert!(!keys.contains(&PermissionKey::KickMembers));
        assert!(!keys.contains(&PermissionKey::ManageServer));
    }

    #[test]
    fn a_voice_channel_offers_connect_and_speak_and_no_message_permission() {
        let voice = keys_of(&channel_permission_groups_for(ChannelKind::Voice));
        for key in [
            PermissionKey::ViewChannels,
            PermissionKey::Connect,
            PermissionKey::Speak,
        ] {
            assert!(voice.contains(&key));
        }
        assert!(!voice.contains(&PermissionKey::SendMessages));
        let text = keys_of(&channel_permission_groups_for(ChannelKind::Text));
        assert!(text.contains(&PermissionKey::SendMessages));
        assert!(!text.contains(&PermissionKey::Speak));
    }

    const HELPER_TARGET: OverwriteTarget = OverwriteTarget {
        kind: OverwriteKind::Role,
        id: Snowflake::from_raw(201),
    };

    #[test]
    fn rebasing_keeps_someone_elses_change_to_another_bit_under_the_draft() {
        let edit = OverwriteEdit {
            target: HELPER_TARGET,
            base: OverwriteBits::default(),
            draft: OverwriteBits {
                allow: Permissions::SEND_MESSAGES,
                deny: Permissions::empty(),
            },
        };
        let stored = OverwriteBits {
            allow: Permissions::empty(),
            deny: Permissions::ATTACH_FILES,
        };
        assert_eq!(
            rebase_overwrite_edit(&edit, stored).map(|rebased| rebased.draft),
            Some(OverwriteBits {
                allow: Permissions::SEND_MESSAGES,
                deny: Permissions::ATTACH_FILES
            })
        );
    }

    #[test]
    fn rebasing_ends_once_stored_holds_the_draft_or_every_bit_is_put_back() {
        let base = OverwriteBits::default();
        let draft = OverwriteBits {
            allow: Permissions::SEND_MESSAGES,
            deny: Permissions::VIEW_CHANNEL,
        };
        let edit = OverwriteEdit {
            target: HELPER_TARGET,
            base,
            draft,
        };
        assert_eq!(rebase_overwrite_edit(&edit, draft), None);
        let put_back = OverwriteEdit {
            draft: base,
            ..edit
        };
        assert_eq!(rebase_overwrite_edit(&put_back, base), None);
    }

    #[test]
    fn rebasing_moves_a_bit_from_allow_to_deny_as_one_change() {
        let allowed = OverwriteBits {
            allow: Permissions::SEND_MESSAGES,
            deny: Permissions::empty(),
        };
        let denied = OverwriteBits {
            allow: Permissions::empty(),
            deny: Permissions::SEND_MESSAGES,
        };
        let edit = OverwriteEdit {
            target: HELPER_TARGET,
            base: allowed,
            draft: denied,
        };
        assert_eq!(
            rebase_overwrite_edit(&edit, allowed).map(|rebased| rebased.draft),
            Some(denied)
        );
    }

    fn target(kind: OverwriteKind, raw: u64) -> OverwriteTarget {
        OverwriteTarget { kind, id: id(raw) }
    }

    #[test]
    fn lists_roles_highest_first_then_everyone_always_then_members() {
        let none = Permissions::empty();
        let with_overwrites = text(vec![
            overwrite(9, OverwriteKind::Member, none, none),
            overwrite(201, OverwriteKind::Role, none, none),
            overwrite(203, OverwriteKind::Role, none, none),
        ]);
        assert_eq!(
            overwrite_targets(&guild(vec![with_overwrites.clone()]), &with_overwrites),
            [
                target(OverwriteKind::Role, 203),
                target(OverwriteKind::Role, 201),
                target(OverwriteKind::Role, GUILD),
                target(OverwriteKind::Member, 9),
            ]
        );
        let plain = text(Vec::new());
        assert_eq!(
            overwrite_targets(&guild(vec![plain.clone()]), &plain),
            [target(OverwriteKind::Role, GUILD)]
        );
        assert_eq!(
            stored_overwrite_of(&plain, id(201)),
            OverwriteBits::default()
        );
    }

    #[test]
    fn targets_everyone_and_roles_below_their_highest_the_owner_any_members_always() {
        let club = guild(vec![text(Vec::new())]);
        assert!(can_edit_target(
            &club,
            &MODERATOR_MANAGER,
            target(OverwriteKind::Role, GUILD)
        ));
        assert!(can_edit_target(
            &club,
            &MODERATOR_MANAGER,
            target(OverwriteKind::Role, 201)
        ));
        assert!(!can_edit_target(
            &club,
            &MODERATOR_MANAGER,
            target(OverwriteKind::Role, 202)
        ));
        assert!(can_edit_target(
            &club,
            &OWNER_MANAGER,
            target(OverwriteKind::Role, 203)
        ));
        assert!(can_edit_target(
            &club,
            &MODERATOR_MANAGER,
            target(OverwriteKind::Member, 1)
        ));
    }

    #[test]
    fn changes_only_permissions_held_in_the_channel_all_of_them_as_an_administrator() {
        let held = Some(Permissions::MANAGE_ROLES | Permissions::SEND_MESSAGES);
        assert!(can_change_channel_permission(
            held,
            Permissions::SEND_MESSAGES
        ));
        assert!(!can_change_channel_permission(
            held,
            Permissions::MANAGE_MESSAGES
        ));
        assert!(can_change_channel_permission(
            Some(Permissions::ADMINISTRATOR),
            Permissions::MANAGE_MESSAGES
        ));
    }

    #[test]
    fn removes_an_overwrite_only_when_it_holds_every_bit_in_it_never_everyones() {
        let none = Permissions::empty();
        let with_overwrites = text(vec![
            overwrite(GUILD, OverwriteKind::Role, none, Permissions::SEND_MESSAGES),
            overwrite(201, OverwriteKind::Role, Permissions::SEND_MESSAGES, none),
            overwrite(9, OverwriteKind::Member, Permissions::MANAGE_MESSAGES, none),
        ]);
        let club = guild(vec![with_overwrites.clone()]);
        let held = Some(Permissions::MANAGE_ROLES | Permissions::SEND_MESSAGES);
        let remove = |target| {
            can_remove_overwrite(&club, &MODERATOR_MANAGER, held, &with_overwrites, target)
        };
        assert!(remove(target(OverwriteKind::Role, 201)));
        assert!(!remove(target(OverwriteKind::Role, GUILD)));
        assert!(!remove(target(OverwriteKind::Member, 9)));
        assert!(!remove(target(OverwriteKind::Role, 202)));
    }

    #[test]
    fn offers_to_add_the_roles_it_may_target_and_the_members_not_listed_yet() {
        let none = Permissions::empty();
        let with_overwrites = text(vec![
            overwrite(201, OverwriteKind::Role, none, none),
            overwrite(7, OverwriteKind::Member, none, none),
        ]);
        let club = guild(vec![with_overwrites.clone()]);
        let role_ids = |manager: &RoleManager, query: &str| -> Vec<u64> {
            addable_roles(&club, &with_overwrites, manager, query)
                .iter()
                .map(|role| role.id.raw())
                .collect()
        };
        assert_eq!(role_ids(&MODERATOR_MANAGER, ""), [] as [u64; 0]);
        assert_eq!(role_ids(&OWNER_MANAGER, ""), [203, 202]);
        assert_eq!(role_ids(&OWNER_MANAGER, "mod"), [202]);
        let members = [
            member(7, "Ana", &[]),
            member(8, "Éloïse", &[]),
            member(9, "Paul", &[]),
        ];
        let member_ids = |query: &str| -> Vec<u64> {
            addable_members(&with_overwrites, &members, query)
                .iter()
                .map(|member| member.user.id.raw())
                .collect()
        };
        assert_eq!(member_ids(""), [8, 9]);
        assert_eq!(member_ids("eloise"), [8]);
    }

    fn stored(name: &str, topic: &str, user_limit: Option<u8>) -> Channel {
        Channel {
            name: name.to_owned(),
            topic: topic.to_owned(),
            user_limit,
            ..text(Vec::new())
        }
    }

    fn drafts(name: Option<&str>, topic: Option<&str>) -> ChannelOverviewDrafts {
        ChannelOverviewDrafts {
            name: name.map(str::to_owned),
            topic: topic.map(str::to_owned),
            user_limit: None,
        }
    }

    #[test]
    fn sends_only_the_typed_fields_so_a_rename_by_someone_else_is_not_undone() {
        // Alice typed a topic; Bob renamed #general to #lobby meanwhile.
        let changes =
            channel_overview_changes(&stored("lobby", "", None), &drafts(None, Some(" News ")));
        assert_eq!(
            changes,
            UpdateChannelRequest {
                topic: Some("News".to_owned()),
                ..UpdateChannelRequest::default()
            }
        );
    }

    #[test]
    fn sends_a_name_as_the_instance_stores_it_and_nothing_once_stored() {
        let general = stored("general", "Hi", None);
        assert_eq!(
            channel_overview_changes(&general, &drafts(Some(" Off Topic "), None)).name,
            Some("Off Topic".to_owned())
        );
        assert_eq!(
            channel_overview_changes(&general, &drafts(Some("General"), None)).name,
            Some("General".to_owned())
        );
        let nothing = channel_overview_changes(&general, &drafts(Some("general "), Some("Hi ")));
        assert!(is_channel_update_empty(&nothing));
        let mut caught_up = drafts(Some("general "), Some("Hi "));
        caught_up.forget_stored(&general);
        assert!(caught_up.is_empty());
    }

    #[test]
    fn sends_a_voice_channels_new_limit_clear_for_unlimited_and_nothing_when_unchanged() {
        use VoiceUserLimitChoice::{People, Unlimited};
        let limit = |stored_limit, choice| {
            let request = channel_overview_changes(
                &stored("lounge", "", stored_limit),
                &ChannelOverviewDrafts {
                    user_limit: Some(choice),
                    ..ChannelOverviewDrafts::default()
                },
            );
            request.user_limit
        };
        assert_eq!(limit(Some(5), Unlimited), UserLimitChange::Clear);
        assert_eq!(limit(Some(5), People(8)), UserLimitChange::Set(8));
        assert_eq!(limit(Some(5), People(5)), UserLimitChange::Keep);
        assert_eq!(limit(None, Unlimited), UserLimitChange::Keep);
        assert_eq!(limit(None, People(1)), UserLimitChange::Set(1));
    }

    #[test]
    fn offers_unlimited_then_one_to_twenty_five_people() {
        let choices = voice_user_limit_choices();
        assert_eq!(choices.len(), 26);
        assert_eq!(choices.first(), Some(&VoiceUserLimitChoice::Unlimited));
        assert_eq!(choices.last(), Some(&VoiceUserLimitChoice::People(25)));
        assert_eq!(user_limit_choice_of(None), VoiceUserLimitChoice::Unlimited);
        assert_eq!(
            user_limit_choice_of(Some(3)),
            VoiceUserLimitChoice::People(3)
        );
    }
}
