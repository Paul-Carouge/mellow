use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::canonical_decimal::parse_canonical_decimal_u64;
use crate::{OverwriteKind, PermissionOverwrite, Role, Snowflake};

bitflags::bitflags! {
    /// Permission bitfield of a role, a channel overwrite or a computed member permission (ADR-0007).
    ///
    /// Bit positions are part of the wire protocol: never reorder or reuse them.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Permissions: u64 {
        const ADMINISTRATOR        = 1 << 0;
        const VIEW_CHANNEL         = 1 << 1;
        const SEND_MESSAGES        = 1 << 2;
        const READ_MESSAGE_HISTORY = 1 << 3;
        const ATTACH_FILES         = 1 << 4;
        const MENTION_EVERYONE     = 1 << 5;
        const MANAGE_MESSAGES      = 1 << 6;
        const MANAGE_CHANNELS      = 1 << 7;
        const MANAGE_ROLES         = 1 << 8;
        const MANAGE_GUILD         = 1 << 9;
        const CREATE_INVITE        = 1 << 10;
        const KICK_MEMBERS         = 1 << 11;
        const BAN_MEMBERS          = 1 << 12;
        const CHANGE_NICKNAME      = 1 << 13;
        const MANAGE_NICKNAMES     = 1 << 14;
        const VIEW_AUDIT_LOG       = 1 << 15;
        /// Adding an emoji no one reacted with yet; joining an existing reaction needs none.
        const ADD_REACTIONS        = 1 << 16;
        /// Adding, renaming and deleting the guild's custom emoji (ADR-0028).
        const MANAGE_EMOJIS        = 1 << 17;
        /// Joining a voice channel; `VIEW_CHANNEL` is needed too (ADR-0030).
        const CONNECT              = 1 << 18;
        /// Being heard in a voice channel: without it the server forwards none of the member's
        /// audio (ADR-0030).
        const SPEAK                = 1 << 19;
    }
}

impl Permissions {
    /// What a channel overwrite may allow or deny; the others act on the whole guild only.
    pub const CHANNEL_SCOPED: Self = Self::VIEW_CHANNEL
        .union(Self::SEND_MESSAGES)
        .union(Self::READ_MESSAGE_HISTORY)
        .union(Self::ATTACH_FILES)
        .union(Self::MENTION_EVERYONE)
        .union(Self::MANAGE_MESSAGES)
        .union(Self::MANAGE_CHANNELS)
        .union(Self::MANAGE_ROLES)
        .union(Self::CREATE_INVITE)
        .union(Self::ADD_REACTIONS)
        .union(Self::CONNECT)
        .union(Self::SPEAK);

    /// Permissions granted to `@everyone` in a newly created guild.
    pub const DEFAULT_EVERYONE: Self = Self::VIEW_CHANNEL
        .union(Self::SEND_MESSAGES)
        .union(Self::READ_MESSAGE_HISTORY)
        .union(Self::ATTACH_FILES)
        .union(Self::CREATE_INVITE)
        .union(Self::CHANGE_NICKNAME)
        .union(Self::ADD_REACTIONS)
        .union(Self::CONNECT)
        .union(Self::SPEAK);
}

/// A member's permissions in a guild (ADR-0007 steps 1 to 3): the owner and administrators get
/// everything; everyone else gets the union of `member_roles`, which must include `@everyone`.
///
/// The server decides with this function; clients use it only to adapt their interface (ADR-0014).
#[must_use]
pub fn compute_guild_permissions<'a>(
    member_id: Snowflake,
    owner_id: Snowflake,
    member_roles: impl IntoIterator<Item = &'a Role>,
) -> Permissions {
    if member_id == owner_id {
        return Permissions::all();
    }
    let granted = member_roles
        .into_iter()
        .fold(Permissions::empty(), |granted, role| {
            granted | role.permissions
        });
    if granted.contains(Permissions::ADMINISTRATOR) {
        Permissions::all()
    } else {
        granted
    }
}

/// A member's permissions in one channel (ADR-0007 step 4): `guild_permissions` from
/// `compute_guild_permissions`, then the channel's overwrites for `@everyone` (the role whose id
/// is `guild_id`), for the member's roles (all denies, then all allows) and for the member. Any
/// holder of `ADMINISTRATOR` (the owner included) ignores overwrites.
///
/// The server decides with this function; clients use it only to adapt their interface (ADR-0014).
#[must_use]
pub fn compute_channel_permissions(
    guild_permissions: Permissions,
    guild_id: Snowflake,
    member_id: Snowflake,
    member_role_ids: &[Snowflake],
    overwrites: &[PermissionOverwrite],
) -> Permissions {
    if guild_permissions.contains(Permissions::ADMINISTRATOR) {
        return Permissions::all();
    }
    let apply = |permissions: Permissions, overwrite: &PermissionOverwrite| {
        permissions
            .difference(overwrite.deny)
            .union(overwrite.allow)
    };
    let mut permissions = guild_permissions;
    if let Some(everyone) = overwrites
        .iter()
        .find(|overwrite| overwrite.kind == OverwriteKind::Role && overwrite.id == guild_id)
    {
        permissions = apply(permissions, everyone);
    }
    let (role_deny, role_allow) = overwrites
        .iter()
        .filter(|overwrite| {
            overwrite.kind == OverwriteKind::Role
                && overwrite.id != guild_id
                && member_role_ids.contains(&overwrite.id)
        })
        .fold(
            (Permissions::empty(), Permissions::empty()),
            |(deny, allow), overwrite| (deny | overwrite.deny, allow | overwrite.allow),
        );
    permissions = permissions.difference(role_deny).union(role_allow);
    if let Some(member) = overwrites
        .iter()
        .find(|overwrite| overwrite.kind == OverwriteKind::Member && overwrite.id == member_id)
    {
        permissions = apply(permissions, member);
    }
    permissions
}

/// The rank a member acts with in the role hierarchy: the highest position among their roles, or
/// 0 (`@everyone`) without any. The owner outranks everyone and is handled by the caller.
#[must_use]
pub fn highest_role_position<'a>(member_roles: impl IntoIterator<Item = &'a Role>) -> u32 {
    member_roles
        .into_iter()
        .map(|role| role.position)
        .max()
        .unwrap_or(0)
}

/// Serialized as a decimal string of the raw bits, like ids, so JavaScript-style clients keep all 64 bits.
impl Serialize for Permissions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.bits())
    }
}

/// Unknown bits are kept, so an older client does not erase permissions added by a newer server.
/// The server must reject unknown bits in client requests (`Permissions::from_bits`), otherwise a
/// client could pre-grant permissions that a future version defines.
impl<'de> Deserialize<'de> for Permissions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        parse_canonical_decimal_u64(&text)
            .map(Self::from_bits_retain)
            .ok_or_else(|| {
                serde::de::Error::custom("permissions must be a canonical decimal string")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn owner_gets_everything_whatever_the_roles() {
        assert_eq!(
            compute_guild_permissions(OWNER, OWNER, &[role(7, 0, Permissions::empty())]),
            Permissions::all()
        );
        assert_eq!(
            compute_guild_permissions(OWNER, OWNER, &[]),
            Permissions::all()
        );
    }

    #[test]
    fn members_get_the_union_of_their_roles() {
        let roles = [
            role(7, 0, Permissions::DEFAULT_EVERYONE),
            role(8, 1, Permissions::MANAGE_MESSAGES),
            role(9, 2, Permissions::KICK_MEMBERS),
        ];
        assert_eq!(
            compute_guild_permissions(MEMBER, OWNER, &roles),
            Permissions::DEFAULT_EVERYONE
                | Permissions::MANAGE_MESSAGES
                | Permissions::KICK_MEMBERS
        );
        assert_eq!(
            compute_guild_permissions(MEMBER, OWNER, &roles[..1]),
            Permissions::DEFAULT_EVERYONE
        );
        assert_eq!(
            compute_guild_permissions(MEMBER, OWNER, &[]),
            Permissions::empty()
        );
    }

    #[test]
    fn administrator_from_any_role_grants_everything() {
        let roles = [
            role(7, 0, Permissions::empty()),
            role(8, 1, Permissions::ADMINISTRATOR),
        ];
        assert_eq!(
            compute_guild_permissions(MEMBER, OWNER, &roles),
            Permissions::all()
        );
    }

    const GUILD: Snowflake = Snowflake::from_raw(100);

    fn overwrite(
        id: Snowflake,
        kind: OverwriteKind,
        allow: Permissions,
        deny: Permissions,
    ) -> PermissionOverwrite {
        PermissionOverwrite {
            id,
            kind,
            allow,
            deny,
        }
    }

    #[test]
    fn channel_overwrites_apply_everyone_then_roles_then_the_member() {
        let view = Permissions::VIEW_CHANNEL;
        let send = Permissions::SEND_MESSAGES;
        let base = Permissions::DEFAULT_EVERYONE;
        let role_a = Snowflake::from_raw(8);
        let role_b = Snowflake::from_raw(9);
        let hidden = [overwrite(
            GUILD,
            OverwriteKind::Role,
            Permissions::empty(),
            view,
        )];
        assert!(!compute_channel_permissions(base, GUILD, MEMBER, &[], &hidden).contains(view));

        // A role allow beats the @everyone deny; a role deny and allow on one bit: allow wins.
        let staff_only = [
            hidden[0].clone(),
            overwrite(role_a, OverwriteKind::Role, view, Permissions::empty()),
            overwrite(
                role_b,
                OverwriteKind::Role,
                Permissions::empty(),
                view | send,
            ),
        ];
        let staff =
            compute_channel_permissions(base, GUILD, MEMBER, &[role_a, role_b], &staff_only);
        assert!(staff.contains(view));
        assert!(!staff.contains(send));
        assert!(
            !compute_channel_permissions(base, GUILD, MEMBER, &[role_b], &staff_only)
                .contains(view)
        );

        // The member's own overwrite comes last.
        let muted = [
            overwrite(role_a, OverwriteKind::Role, send, Permissions::empty()),
            overwrite(MEMBER, OverwriteKind::Member, Permissions::empty(), send),
        ];
        assert!(
            !compute_channel_permissions(base, GUILD, MEMBER, &[role_a], &muted).contains(send)
        );
        assert!(compute_channel_permissions(base, GUILD, OWNER, &[role_a], &muted).contains(send));

        // A member overwrite never applies through a role of the same id, nor the reverse.
        let member_kind_on_role_id = [overwrite(
            role_a,
            OverwriteKind::Member,
            Permissions::empty(),
            view,
        )];
        assert!(
            compute_channel_permissions(base, GUILD, MEMBER, &[role_a], &member_kind_on_role_id)
                .contains(view)
        );
    }

    #[test]
    fn administrators_ignore_channel_overwrites() {
        let deny_all = [overwrite(
            GUILD,
            OverwriteKind::Role,
            Permissions::empty(),
            Permissions::all(),
        )];
        assert_eq!(
            compute_channel_permissions(Permissions::all(), GUILD, OWNER, &[], &deny_all),
            Permissions::all()
        );
        assert_eq!(
            compute_channel_permissions(
                Permissions::DEFAULT_EVERYONE,
                GUILD,
                MEMBER,
                &[],
                &deny_all
            ),
            Permissions::empty()
        );
    }

    #[test]
    fn highest_role_position_is_the_maximum_or_zero() {
        assert_eq!(highest_role_position(&[]), 0);
        assert_eq!(
            highest_role_position(&[
                role(7, 0, Permissions::empty()),
                role(9, 4, Permissions::empty()),
                role(8, 2, Permissions::empty()),
            ]),
            4
        );
    }

    #[test]
    fn json_uses_decimal_string_and_keeps_unknown_bits() {
        let permissions = Permissions::SEND_MESSAGES | Permissions::from_bits_retain(1 << 63);
        let json = serde_json::to_string(&permissions).unwrap();
        assert_eq!(json, "\"9223372036854775812\"");
        assert_eq!(
            serde_json::from_str::<Permissions>(&json).unwrap(),
            permissions
        );
    }

    #[test]
    fn non_decimal_strings_are_rejected() {
        for json in [
            "4",
            "\"+4\"",
            "\"-4\"",
            "\"\"",
            "\"04\"",
            "\"SEND_MESSAGES\"",
        ] {
            assert!(serde_json::from_str::<Permissions>(json).is_err(), "{json}");
        }
    }

    #[test]
    fn administrator_and_member_moderation_are_never_channel_scoped() {
        assert!(!Permissions::CHANNEL_SCOPED.intersects(
            Permissions::ADMINISTRATOR
                | Permissions::MANAGE_GUILD
                | Permissions::KICK_MEMBERS
                | Permissions::BAN_MEMBERS
        ));
    }

    #[test]
    fn default_everyone_has_no_moderation_permission() {
        let moderation = Permissions::ADMINISTRATOR
            | Permissions::MANAGE_MESSAGES
            | Permissions::MANAGE_CHANNELS
            | Permissions::MANAGE_ROLES
            | Permissions::MANAGE_GUILD
            | Permissions::KICK_MEMBERS
            | Permissions::BAN_MEMBERS
            | Permissions::MANAGE_NICKNAMES
            | Permissions::VIEW_AUDIT_LOG;
        assert!(!Permissions::DEFAULT_EVERYONE.intersects(moderation));
    }
}
