//! Role icons (`web/src/components/role-icons.ts`, ADR-0050): each member shows the icon of their
//! highest role that has one, next to their name in messages and the member list, as on Discord.

use mellow_protocol::{Role, Snowflake};

/// A role with a picture or an emoji.
#[must_use]
pub fn has_role_icon(role: &Role) -> bool {
    role.icon.is_some()
        || role
            .unicode_emoji
            .as_deref()
            .is_some_and(|emoji| !emoji.is_empty())
}

/// The highest of `role_ids` with an icon; `@everyone` never has one.
#[must_use]
pub fn highest_role_with_icon<'r>(roles: &'r [Role], role_ids: &[Snowflake]) -> Option<&'r Role> {
    roles
        .iter()
        .filter(|role| has_role_icon(role) && role_ids.contains(&role.id))
        .max_by_key(|role| role.position)
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{Permissions, ProfileImage};

    use super::*;

    fn role(raw: u64, position: u32, icon: Option<u64>, emoji: Option<&str>) -> Role {
        Role {
            id: Snowflake::from_raw(raw),
            name: format!("role {raw}"),
            color: 0,
            position,
            permissions: Permissions::empty(),
            is_displayed_separately: false,
            is_mentionable: false,
            icon: icon.map(|raw| ProfileImage {
                id: Snowflake::from_raw(raw),
                is_animated: false,
            }),
            unicode_emoji: emoji.map(str::to_owned),
        }
    }

    #[test]
    fn a_picture_or_an_emoji_is_an_icon() {
        assert!(has_role_icon(&role(1, 1, Some(9), None)));
        assert!(has_role_icon(&role(1, 1, None, Some("🔥"))));
        assert!(!has_role_icon(&role(1, 1, None, None)));
        assert!(!has_role_icon(&role(1, 1, None, Some(""))));
    }

    #[test]
    fn the_highest_held_role_with_an_icon_wins() {
        let roles = [
            role(1, 1, None, Some("🌱")),
            role(2, 3, Some(9), None),
            role(3, 5, None, None),
            role(4, 4, None, Some("🔥")),
        ];
        let held = [
            Snowflake::from_raw(1),
            Snowflake::from_raw(2),
            Snowflake::from_raw(3),
        ];
        assert_eq!(
            highest_role_with_icon(&roles, &held).map(|role| role.id),
            Some(Snowflake::from_raw(2))
        );
        assert!(highest_role_with_icon(&roles, &[Snowflake::from_raw(3)]).is_none());
    }
}
