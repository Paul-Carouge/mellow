//! The only place permissions are decided (ADR-0007, CLAUDE.md).

use mellow_protocol::{Permissions, Snowflake};

/// A member's permissions in a guild: the owner and administrators get everything; everyone
/// else gets the @everyone role's permissions.
/// shortcut: named roles and channel overwrites (ADR-0007 steps 2 and 4) arrive with M2.
#[must_use]
pub fn guild_permissions(
    member_id: Snowflake,
    owner_id: Snowflake,
    everyone_permissions: Permissions,
) -> Permissions {
    if member_id == owner_id || everyone_permissions.contains(Permissions::ADMINISTRATOR) {
        Permissions::all()
    } else {
        everyone_permissions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: Snowflake = Snowflake::from_raw(1);
    const MEMBER: Snowflake = Snowflake::from_raw(2);

    #[test]
    fn owner_and_administrators_get_everything_others_get_everyone_permissions() {
        assert_eq!(
            guild_permissions(OWNER, OWNER, Permissions::empty()),
            Permissions::all()
        );
        assert_eq!(
            guild_permissions(MEMBER, OWNER, Permissions::ADMINISTRATOR),
            Permissions::all()
        );
        assert_eq!(
            guild_permissions(MEMBER, OWNER, Permissions::DEFAULT_EVERYONE),
            Permissions::DEFAULT_EVERYONE
        );
        assert!(
            !guild_permissions(MEMBER, OWNER, Permissions::DEFAULT_EVERYONE)
                .contains(Permissions::MANAGE_CHANNELS)
        );
    }
}
