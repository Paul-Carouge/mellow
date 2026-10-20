//! The only place permissions are decided (ADR-0007, CLAUDE.md). The computation itself is shared
//! with clients through `mellow-protocol` (ADR-0014).

use mellow_protocol::{
    PermissionOverwrite, Permissions, Role, Snowflake, compute_channel_permissions,
    compute_guild_permissions, highest_role_position,
};

/// A member's permissions in a guild: the owner and administrators get everything; everyone else
/// gets the union of their roles (`member_roles` includes `@everyone`).
/// shortcut: channel overwrites (ADR-0007 step 4) arrive with the next M2 step.
#[must_use]
pub fn guild_permissions(
    member_id: Snowflake,
    owner_id: Snowflake,
    member_roles: &[Role],
) -> Permissions {
    compute_guild_permissions(member_id, owner_id, member_roles)
}

/// A member's permissions in one channel: their guild permissions, then the channel's
/// overwrites (ADR-0007 step 4). `member_roles` includes `@everyone`, whose id is `guild_id`.
#[must_use]
pub fn channel_permissions(
    member_id: Snowflake,
    owner_id: Snowflake,
    guild_id: Snowflake,
    member_roles: &[Role],
    overwrites: &[PermissionOverwrite],
) -> Permissions {
    let member_role_ids: Vec<Snowflake> = member_roles
        .iter()
        .map(|role| role.id)
        .filter(|role_id| *role_id != guild_id)
        .collect();
    compute_channel_permissions(
        guild_permissions(member_id, owner_id, member_roles),
        guild_id,
        member_id,
        &member_role_ids,
        overwrites,
    )
}

/// Whether every member of the guild can view a channel, without looking at members: everyone's
/// permissions are a superset of `@everyone`'s, and overwrites can only take `VIEW_CHANNEL` away
/// through a deny (ADR-0015 fast path).
#[must_use]
pub fn is_visible_to_every_member(
    everyone_permissions: Permissions,
    overwrites: &[PermissionOverwrite],
) -> bool {
    everyone_permissions.contains(Permissions::ADMINISTRATOR)
        || (everyone_permissions.contains(Permissions::VIEW_CHANNEL)
            && !overwrites
                .iter()
                .any(|overwrite| overwrite.deny.contains(Permissions::VIEW_CHANNEL)))
}

/// Someone acting on roles: what the role hierarchy lets them touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleManager {
    pub is_owner: bool,
    pub permissions: Permissions,
    pub highest_role_position: u32,
}

impl RoleManager {
    #[must_use]
    pub fn new(member_id: Snowflake, owner_id: Snowflake, member_roles: &[Role]) -> Self {
        Self {
            is_owner: member_id == owner_id,
            permissions: guild_permissions(member_id, owner_id, member_roles),
            highest_role_position: highest_role_position(member_roles),
        }
    }

    /// `MANAGE_ROLES` is needed for any role. The owner manages every role; anyone else manages
    /// `@everyone` (position 0, every member holds it) and the roles strictly below their highest,
    /// so nobody can edit, give or take away a role at or above their own rank, as on Discord.
    /// Unlike Discord, they must also hold every permission of the role: new roles start at
    /// position 1, so a role holding `ADMINISTRATOR` can sit below a moderator, who must not be
    /// able to give it to themselves.
    #[must_use]
    pub fn can_manage_role(&self, role: &Role, everyone_role_id: Snowflake) -> bool {
        self.permissions.contains(Permissions::MANAGE_ROLES)
            && (self.is_owner
                || ((role.id == everyone_role_id || role.position < self.highest_role_position)
                    && self.permissions.contains(role.permissions)))
    }

    /// Who may set a channel overwrite for `role`: the owner for any role, anyone else for
    /// `@everyone` and roles strictly below their highest. The permission bits are checked
    /// against the channel separately.
    #[must_use]
    pub fn can_target_role_in_overwrites(&self, role: &Role, everyone_role_id: Snowflake) -> bool {
        self.is_owner || role.id == everyone_role_id || role.position < self.highest_role_position
    }

    /// `named_roles_by_position` holds the roles without `@everyone`, at positions 1, 2, …, and `new_order` the same ids in their
    /// new order. The owner may reorder freely; anyone else may move only roles they manage
    /// (`can_manage_role`), so their own role and everything above it stay where they are.
    #[must_use]
    pub fn can_reorder_roles(
        &self,
        named_roles_by_position: &[Role],
        new_order: &[Snowflake],
        everyone_role_id: Snowflake,
    ) -> bool {
        // Every role that moves leaves a changed slot; if all of those are manageable, so are
        // the roles moving in, and every changed slot lies below the manager's highest role.
        self.permissions.contains(Permissions::MANAGE_ROLES)
            && (self.is_owner
                || named_roles_by_position
                    .iter()
                    .zip(new_order)
                    .all(|(role, new_id)| {
                        role.id == *new_id || self.can_manage_role(role, everyone_role_id)
                    }))
    }

    /// Nobody grants or removes a permission they do not hold themselves: administrators and the
    /// owner hold them all.
    #[must_use]
    pub fn can_change_permissions(&self, from: Permissions, to: Permissions) -> bool {
        self.permissions.contains(from.symmetric_difference(to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUILD: Snowflake = Snowflake::from_raw(100);
    const OWNER: Snowflake = Snowflake::from_raw(1);
    const MEMBER: Snowflake = Snowflake::from_raw(2);

    fn role(id: u64, position: u32, permissions: Permissions) -> Role {
        Role {
            id: Snowflake::from_raw(id),
            name: format!("role {id}"),
            color: 0,
            position,
            permissions,
        }
    }

    fn everyone() -> Role {
        role(GUILD.raw(), 0, Permissions::DEFAULT_EVERYONE)
    }

    #[test]
    fn owner_and_administrators_get_everything_others_get_their_roles() {
        assert_eq!(guild_permissions(OWNER, OWNER, &[]), Permissions::all());
        assert_eq!(
            guild_permissions(
                MEMBER,
                OWNER,
                &[role(GUILD.raw(), 0, Permissions::ADMINISTRATOR)]
            ),
            Permissions::all()
        );
        assert_eq!(
            guild_permissions(MEMBER, OWNER, &[everyone()]),
            Permissions::DEFAULT_EVERYONE
        );
    }

    #[test]
    fn managers_touch_only_everyone_and_roles_below_their_highest() {
        let moderator = role(10, 2, Permissions::MANAGE_ROLES);
        let below = role(11, 1, Permissions::empty());
        let above = role(12, 3, Permissions::empty());
        let manager = RoleManager::new(MEMBER, OWNER, &[everyone(), moderator.clone()]);
        assert!(manager.can_manage_role(&below, GUILD));
        assert!(manager.can_manage_role(&everyone(), GUILD));
        assert!(
            !manager.can_manage_role(&moderator, GUILD),
            "their own role"
        );
        assert!(!manager.can_manage_role(&above, GUILD));

        let owner = RoleManager::new(OWNER, OWNER, &[everyone()]);
        assert!(owner.can_manage_role(&above, GUILD));
    }

    #[test]
    fn roles_holding_permissions_the_manager_lacks_are_out_of_reach() {
        let manager = RoleManager::new(
            MEMBER,
            OWNER,
            &[everyone(), role(10, 3, Permissions::MANAGE_ROLES)],
        );
        assert!(!manager.can_manage_role(&role(11, 1, Permissions::ADMINISTRATOR), GUILD));
        assert!(!manager.can_manage_role(&role(12, 2, Permissions::BAN_MEMBERS), GUILD));
        assert!(manager.can_manage_role(&role(13, 2, Permissions::MANAGE_ROLES), GUILD));
        let owner = RoleManager::new(OWNER, OWNER, &[everyone()]);
        assert!(owner.can_manage_role(&role(11, 1, Permissions::ADMINISTRATOR), GUILD));
    }

    #[test]
    fn managers_reorder_only_roles_below_their_highest_that_they_manage() {
        let low = role(10, 1, Permissions::empty());
        let banners = role(11, 2, Permissions::BAN_MEMBERS);
        let other_low = role(12, 3, Permissions::empty());
        let moderator = role(13, 4, Permissions::MANAGE_ROLES);
        let admin = role(14, 5, Permissions::ADMINISTRATOR);
        let current = [&low, &banners, &other_low, &moderator, &admin].map(Role::clone);
        let order = |roles: [&Role; 5]| roles.map(|role| role.id);
        let manager = RoleManager::new(MEMBER, OWNER, &[everyone(), moderator.clone()]);
        let can_reorder = |roles| manager.can_reorder_roles(&current, &order(roles), GUILD);

        assert!(can_reorder([
            &low, &banners, &other_low, &moderator, &admin
        ]));
        assert!(
            can_reorder([&other_low, &banners, &low, &moderator, &admin]),
            "two manageable roles swap around a fixed one"
        );
        assert!(
            !can_reorder([&other_low, &low, &banners, &moderator, &admin]),
            "a role holding a permission the manager lacks cannot move"
        );
        assert!(
            !can_reorder([&low, &banners, &other_low, &admin, &moderator]),
            "their own role and the ones above stay"
        );
        assert!(!can_reorder([
            &moderator, &banners, &other_low, &low, &admin
        ]));

        let owner = RoleManager::new(OWNER, OWNER, &[everyone()]);
        assert!(owner.can_reorder_roles(
            &current,
            &order([&admin, &moderator, &other_low, &banners, &low]),
            GUILD
        ));
    }

    #[test]
    fn channels_are_visible_to_every_member_only_without_a_view_deny() {
        use mellow_protocol::OverwriteKind;
        let deny_view = |id: u64| PermissionOverwrite {
            id: Snowflake::from_raw(id),
            kind: OverwriteKind::Member,
            allow: Permissions::empty(),
            deny: Permissions::VIEW_CHANNEL,
        };
        let allow_send = PermissionOverwrite {
            deny: Permissions::empty(),
            allow: Permissions::SEND_MESSAGES,
            ..deny_view(5)
        };
        assert!(is_visible_to_every_member(
            Permissions::DEFAULT_EVERYONE,
            &[]
        ));
        assert!(is_visible_to_every_member(
            Permissions::DEFAULT_EVERYONE,
            &[allow_send]
        ));
        assert!(!is_visible_to_every_member(
            Permissions::DEFAULT_EVERYONE,
            &[deny_view(5)]
        ));
        assert!(!is_visible_to_every_member(Permissions::SEND_MESSAGES, &[]));
        assert!(is_visible_to_every_member(
            Permissions::ADMINISTRATOR,
            &[deny_view(5)]
        ));

        let member_roles = [everyone()];
        assert!(
            !channel_permissions(
                MEMBER,
                OWNER,
                GUILD,
                &member_roles,
                &[deny_view(MEMBER.raw())]
            )
            .contains(Permissions::VIEW_CHANNEL)
        );
        assert!(
            channel_permissions(
                OWNER,
                OWNER,
                GUILD,
                &member_roles,
                &[deny_view(OWNER.raw())]
            )
            .contains(Permissions::VIEW_CHANNEL),
            "the owner sees every channel"
        );
    }

    #[test]
    fn without_manage_roles_nothing_is_manageable() {
        let high = role(10, 5, Permissions::KICK_MEMBERS);
        let manager = RoleManager::new(MEMBER, OWNER, &[everyone(), high]);
        assert!(!manager.can_manage_role(&role(11, 1, Permissions::empty()), GUILD));
        assert!(!manager.can_manage_role(&everyone(), GUILD));
    }

    #[test]
    fn only_held_permissions_can_be_granted_or_removed() {
        let manager = RoleManager::new(
            MEMBER,
            OWNER,
            &[
                everyone(),
                role(10, 2, Permissions::MANAGE_ROLES | Permissions::KICK_MEMBERS),
            ],
        );
        let kick = Permissions::KICK_MEMBERS;
        let ban = Permissions::BAN_MEMBERS;
        assert!(manager.can_change_permissions(Permissions::empty(), kick));
        assert!(manager.can_change_permissions(kick, Permissions::empty()));
        assert!(!manager.can_change_permissions(Permissions::empty(), ban));
        assert!(!manager.can_change_permissions(ban, Permissions::empty()));
        assert!(
            manager.can_change_permissions(ban, ban | kick),
            "untouched bits do not count"
        );
        let administrator = RoleManager::new(
            MEMBER,
            OWNER,
            &[everyone(), role(10, 2, Permissions::ADMINISTRATOR)],
        );
        assert!(administrator.can_change_permissions(Permissions::empty(), Permissions::all()));
    }
}
