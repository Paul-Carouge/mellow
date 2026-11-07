//! The order of the members side sheet (`web/src/components/member-order.ts`, DESIGN.md §5.7).

use std::cmp::Ordering;
use std::collections::HashSet;

use mellow_protocol::{Member, Role, Snowflake};

use super::format::display_name;
use super::search_text::fold_for_search;

/// shortcut: the web sorts with `Intl.Collator`; without ICU collation data names compare folded
/// (case and accents ignored), then as written, as `mentions.rs` does. Pull in `icu_collator` if
/// people report a different order between the web and the desktop.
fn compare_shown_names(left: &str, right: &str) -> Ordering {
    fold_for_search(left)
        .cmp(&fold_for_search(right))
        .then_with(|| left.cmp(right))
}

/// A new list, by display name in the reader's alphabet.
#[must_use]
pub fn members_sorted_by_display_name(members: &[Member]) -> Vec<Member> {
    let mut sorted = members.to_vec();
    sorted.sort_by(|left, right| {
        compare_shown_names(display_name(&left.user), display_name(&right.user))
    });
    sorted
}

/// A heading of the members side sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberListGroupKind<'r> {
    Role(&'r Role),
    Online,
    Offline,
}

/// A heading and the members under it, in the order given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberListGroup<'m, 'r> {
    pub kind: MemberListGroupKind<'r>,
    pub members: Vec<&'m Member>,
}

/// The side sheet's groups, as on Discord: online members under the highest role displayed
/// separately that they hold (highest role first), the other online members under "Online", and
/// every offline member under "Offline". Empty groups are left out; each group keeps the order
/// of `members`.
#[must_use]
pub fn member_list_groups<'m, 'r>(
    members: &'m [Member],
    roles: &'r [Role],
    is_online: impl Fn(Snowflake) -> bool,
) -> Vec<MemberListGroup<'m, 'r>> {
    let mut displayed_roles: Vec<&Role> = roles
        .iter()
        .filter(|role| role.is_displayed_separately)
        .collect();
    displayed_roles.sort_by_key(|role| std::cmp::Reverse(role.position));
    let mut by_role: Vec<Vec<&Member>> = vec![Vec::new(); displayed_roles.len()];
    let mut online = Vec::new();
    let mut offline = Vec::new();
    for member in members {
        if !is_online(member.user.id) {
            offline.push(member);
            continue;
        }
        let held: HashSet<Snowflake> = member.role_ids.iter().copied().collect();
        match displayed_roles
            .iter()
            .position(|role| held.contains(&role.id))
        {
            Some(index) => by_role[index].push(member),
            None => online.push(member),
        }
    }
    displayed_roles
        .into_iter()
        .zip(by_role)
        .map(|(role, members)| MemberListGroup {
            kind: MemberListGroupKind::Role(role),
            members,
        })
        .chain([
            MemberListGroup {
                kind: MemberListGroupKind::Online,
                members: online,
            },
            MemberListGroup {
                kind: MemberListGroupKind::Offline,
                members: offline,
            },
        ])
        .filter(|group| !group.members.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{Permissions, PublicUser};

    use super::*;

    fn member(id: u64, username: &str, display_name: &str) -> Member {
        Member {
            user: PublicUser {
                id: Snowflake::from_raw(id),
                username: username.to_owned(),
                display_name: display_name.to_owned(),
                avatar: None,
                banner: None,
            },
            role_ids: Vec::new(),
            joined_at_unix_milliseconds: 0,
        }
    }

    #[test]
    fn members_sort_by_display_name_else_username_ignoring_case_and_accents() {
        let members = [
            member(1, "zed", "Zoé"),
            member(2, "bob", ""),
            member(3, "eve", "élodie"),
            member(4, "ana", "Ana"),
            member(5, "al", "anatole"),
            member(6, "max", "Émile"),
        ];
        let sorted = members_sorted_by_display_name(&members);
        let names: Vec<&str> = sorted.iter().map(|each| display_name(&each.user)).collect();
        assert_eq!(names, ["Ana", "anatole", "bob", "élodie", "Émile", "Zoé"]);
        // The given list is left untouched.
        assert_eq!(display_name(&members[0].user), "Zoé");
    }

    fn role(id: u64, position: u32, is_displayed_separately: bool) -> Role {
        Role {
            id: Snowflake::from_raw(id),
            name: format!("role {id}"),
            color: 0,
            position,
            permissions: Permissions::empty(),
            is_displayed_separately,
            is_mentionable: false,
            icon: None,
            unicode_emoji: None,
        }
    }

    fn with_roles(id: u64, role_ids: &[u64]) -> Member {
        Member {
            role_ids: role_ids.iter().copied().map(Snowflake::from_raw).collect(),
            ..member(id, &format!("user{id}"), &format!("User {id}"))
        }
    }

    fn roles() -> Vec<Role> {
        vec![
            role(100, 0, false),
            role(201, 1, true),
            role(202, 2, false),
            role(203, 3, true),
        ]
    }

    fn is_online(user_id: Snowflake) -> bool {
        user_id != Snowflake::from_raw(5)
    }

    fn summary(groups: &[MemberListGroup<'_, '_>]) -> Vec<(String, Vec<u64>)> {
        groups
            .iter()
            .map(|group| {
                let key = match group.kind {
                    MemberListGroupKind::Role(role) => role.id.to_string(),
                    MemberListGroupKind::Online => "online".to_owned(),
                    MemberListGroupKind::Offline => "offline".to_owned(),
                };
                let ids = group
                    .members
                    .iter()
                    .map(|member| member.user.id.raw())
                    .collect();
                (key, ids)
            })
            .collect()
    }

    fn expected(rows: &[(&str, &[u64])]) -> Vec<(String, Vec<u64>)> {
        rows.iter()
            .map(|(key, ids)| ((*key).to_owned(), ids.to_vec()))
            .collect()
    }

    #[test]
    fn online_members_go_under_their_highest_role_displayed_separately() {
        let members = [
            with_roles(1, &[]),
            with_roles(2, &[201]),
            with_roles(3, &[201, 203]),
            with_roles(4, &[202]),
            with_roles(5, &[203]),
        ];
        let roles = roles();
        assert_eq!(
            summary(&member_list_groups(&members, &roles, is_online)),
            expected(&[
                ("203", &[3]),
                ("201", &[2]),
                ("online", &[1, 4]),
                ("offline", &[5])
            ])
        );
    }

    #[test]
    fn groups_keep_the_given_order_and_empty_ones_are_left_out() {
        let roles = roles();
        let members = [with_roles(2, &[201]), with_roles(1, &[201])];
        assert_eq!(
            summary(&member_list_groups(&members, &roles, is_online)),
            expected(&[("201", &[2, 1])])
        );
        assert_eq!(member_list_groups(&[], &roles, is_online), Vec::new());
    }

    #[test]
    fn without_displayed_roles_there_are_only_online_and_offline() {
        let roles = [role(100, 0, false), role(202, 2, false)];
        let members = [with_roles(1, &[202]), with_roles(5, &[])];
        assert_eq!(
            summary(&member_list_groups(&members, &roles, is_online)),
            expected(&[("online", &[1]), ("offline", &[5])])
        );
    }
}
