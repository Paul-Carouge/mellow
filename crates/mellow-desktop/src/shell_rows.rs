//! Pure conversions from workspace data to the rows the Slint shell displays.

use std::collections::BTreeSet;

use crate::fake_workspace::{FakeCategory, FakePresence, FakeRoleGroup, FakeServer};
use slint::SharedString;

use crate::generated_ui::{ChannelListRow, MemberListRow, PresenceStatus, ServerItem};

const MAXIMUM_SERVER_INITIALS: usize = 3;

/// First letter of each word, as Discord shows servers without an icon ("Board Games Night" → "BGN").
pub fn server_initials(server_name: &str) -> String {
    server_name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(MAXIMUM_SERVER_INITIALS)
        .collect()
}

pub fn first_letter(name: &str) -> String {
    name.chars().next().map(String::from).unwrap_or_default()
}

pub fn server_items(servers: &[FakeServer]) -> Vec<ServerItem> {
    servers
        .iter()
        .map(|server| ServerItem {
            name: server.name.into(),
            initials: server_initials(server.name).into(),
            has_unread: server.has_unread_messages,
            mention_count: saturating_int(server.unread_mention_count),
        })
        .collect()
}

/// A collapsed category still lists its selected and unread channels, as on Discord.
pub fn channel_rows(
    categories: &[FakeCategory],
    collapsed_category_indexes: &BTreeSet<usize>,
    selected_channel_name: &str,
) -> Vec<ChannelListRow> {
    categories
        .iter()
        .enumerate()
        .flat_map(|(category_index, category)| {
            let is_collapsed = collapsed_category_indexes.contains(&category_index);
            let header = ChannelListRow {
                id: SharedString::default(),
                is_category: true,
                is_collapsed,
                category_index: saturating_int(category_index),
                name: category.name.into(),
                has_unread: false,
                mention_count: 0,
                is_read_only: false,
            };
            let visible_channels = category
                .channels
                .iter()
                .filter(move |channel| {
                    !is_collapsed
                        || channel.has_unread_messages
                        || channel.name == selected_channel_name
                })
                .map(move |channel| ChannelListRow {
                    // Demo channel names are unique, so they double as ids.
                    id: channel.name.into(),
                    is_category: false,
                    is_collapsed: false,
                    category_index: saturating_int(category_index),
                    name: channel.name.into(),
                    has_unread: channel.has_unread_messages,
                    mention_count: saturating_int(channel.unread_mention_count),
                    is_read_only: false,
                });
            std::iter::once(header).chain(visible_channels)
        })
        .collect()
}

pub fn member_rows(role_groups: &[FakeRoleGroup]) -> Vec<MemberListRow> {
    role_groups
        .iter()
        .filter(|group| !group.members.is_empty())
        .flat_map(|group| {
            let header = MemberListRow {
                is_group_header: true,
                label: group.role_name.into(),
                member_count: saturating_int(group.members.len()),
                initial: "".into(),
                activity: "".into(),
                status: PresenceStatus::Offline,
                user_id: "".into(),
                can_message: false,
            };
            let members = group.members.iter().map(|member| MemberListRow {
                is_group_header: false,
                label: member.display_name.into(),
                member_count: 0,
                initial: first_letter(member.display_name).into(),
                activity: member.activity.into(),
                status: presence_status(member.presence),
                // The demo has no accounts: the name stands in for an id.
                user_id: member.display_name.into(),
                can_message: true,
            });
            std::iter::once(header).chain(members)
        })
        .collect()
}

fn presence_status(presence: FakePresence) -> PresenceStatus {
    match presence {
        FakePresence::Online => PresenceStatus::Online,
        FakePresence::Idle => PresenceStatus::Idle,
        FakePresence::DoNotDisturb => PresenceStatus::DoNotDisturb,
        FakePresence::Offline => PresenceStatus::Offline,
    }
}

/// Slint models use `i32`; counts beyond it are displayed as the maximum ("99+" anyway).
pub fn saturating_int(count: impl TryInto<i32>) -> i32 {
    count.try_into().unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_workspace::{FakeChannel, FakeMember};

    fn sample_categories() -> Vec<FakeCategory> {
        let channel = |name, has_unread_messages| FakeChannel {
            name,
            topic: "",
            has_unread_messages,
            unread_mention_count: 0,
        };
        vec![FakeCategory {
            name: "Text channels",
            channels: vec![
                channel("general", false),
                channel("dev", true),
                channel("design", false),
            ],
        }]
    }

    #[test]
    fn initials_take_the_first_letter_of_up_to_three_words() {
        assert_eq!(server_initials("Board Games Night Out"), "BGN");
        assert_eq!(server_initials("Élan"), "É");
        assert_eq!(server_initials(""), "");
    }

    #[test]
    fn expanded_category_lists_all_channels_under_its_sentence_case_name() {
        let rows = channel_rows(&sample_categories(), &BTreeSet::new(), "general");
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].name, "Text channels");
    }

    #[test]
    fn collapsed_category_keeps_only_selected_and_unread_channels() {
        let rows = channel_rows(&sample_categories(), &BTreeSet::from([0]), "general");
        let channel_names: Vec<_> = rows
            .iter()
            .filter(|row| !row.is_category)
            .map(|row| row.name.to_string())
            .collect();
        assert_eq!(channel_names, ["general", "dev"]);
        assert!(rows[0].is_collapsed);
    }

    #[test]
    fn member_rows_start_each_group_with_a_counted_header_and_skip_empty_groups() {
        let member = |display_name, presence| FakeMember {
            display_name,
            activity: "",
            presence,
        };
        let role_groups = vec![
            FakeRoleGroup {
                role_name: "Moderators",
                members: vec![member("Aurore", FakePresence::Online)],
            },
            FakeRoleGroup {
                role_name: "Empty role",
                members: Vec::new(),
            },
            FakeRoleGroup {
                role_name: "Offline",
                members: vec![member("Basile", FakePresence::Offline)],
            },
        ];

        let rows = member_rows(&role_groups);

        let labels: Vec<_> = rows.iter().map(|row| row.label.to_string()).collect();
        assert_eq!(labels, ["Moderators", "Aurore", "Offline", "Basile"]);
        assert!(rows[0].is_group_header);
        assert_eq!(rows[0].member_count, 1);
        assert_eq!(rows[1].initial, "A");
        assert_eq!(rows[3].status, PresenceStatus::Offline);
    }
}
