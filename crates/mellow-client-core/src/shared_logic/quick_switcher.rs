//! The quick switcher's results (`web/src/components/quick-switcher.ts`, DESIGN.md §5.10):
//! channels, people and servers whose name contains what was typed, ignoring case and accents;
//! "#", "@" and "*" narrow to one kind. Words stay out: the client names the kinds and places.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use mellow_protocol::{ChannelKind, DirectMessageChannel, Guild, PublicUser, Snowflake};

use super::channel_order::channel_sections;
use super::format::display_name;
use super::search_text::fold_for_search;

/// Before anything is typed, a short list per kind: a large instance would otherwise list every
/// channel and member at once.
pub const MAXIMUM_ITEMS_PER_KIND_BEFORE_TYPING: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickSwitcherTarget {
    Channel {
        guild_id: Snowflake,
        channel_id: Snowflake,
    },
    /// A person leads to their one-to-one conversation, opened first when there is none yet.
    Person {
        user_id: Snowflake,
        conversation_id: Option<Snowflake>,
    },
    Server {
        guild_id: Snowflake,
    },
    /// A group conversation.
    Conversation {
        conversation_id: Snowflake,
    },
}

/// The icon beside a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickSwitcherIcon {
    Hash,
    /// A channel the reader cannot send in.
    Lock,
    At,
    Users,
}

/// Where a result leads, said after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickSwitcherPlace {
    /// The server's name.
    InServer(String),
    DirectMessage,
    Group,
    Server,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickSwitcherItem {
    pub label: String,
    pub place: QuickSwitcherPlace,
    pub icon: QuickSwitcherIcon,
    pub has_unread: bool,
    pub target: QuickSwitcherTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickSwitcherGroupKind {
    Channels,
    /// People, then groups.
    People,
    Servers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickSwitcherGroup {
    pub kind: QuickSwitcherGroupKind,
    pub items: Vec<QuickSwitcherItem>,
}

/// What the switcher searches, and what it asks the client state.
pub struct QuickSwitcherSources<'s> {
    pub guilds: &'s [Guild],
    pub conversations: &'s [DirectMessageChannel],
    /// Members of the servers loaded so far, for people without a conversation yet.
    pub known_people: Vec<&'s PublicUser>,
    pub current_user_id: Option<Snowflake>,
    /// Friends can be messaged even when they share no loaded server (ADR-0025).
    pub is_friend: &'s dyn Fn(Snowflake) -> bool,
    pub has_unread: &'s dyn Fn(Snowflake) -> bool,
    pub can_send_in: &'s dyn Fn(Snowflake) -> bool,
    /// A group's own name, else its people's names, in the client's words.
    pub conversation_name: &'s dyn Fn(&DirectMessageChannel) -> String,
}

impl std::fmt::Debug for QuickSwitcherSources<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QuickSwitcherSources")
            .field("guild_count", &self.guilds.len())
            .field("conversation_count", &self.conversations.len())
            .finish_non_exhaustive()
    }
}

fn kind_of_prefix(prefix: Option<char>) -> Option<QuickSwitcherGroupKind> {
    match prefix {
        Some('#') => Some(QuickSwitcherGroupKind::Channels),
        Some('@') => Some(QuickSwitcherGroupKind::People),
        Some('*') => Some(QuickSwitcherGroupKind::Servers),
        _ => None,
    }
}

/// shortcut: the web sorts people with `localeCompare`; names compare folded, then as written
/// (see `member_order.rs`).
fn compare_shown_names(left: &str, right: &str) -> Ordering {
    fold_for_search(left)
        .cmp(&fold_for_search(right))
        .then_with(|| left.cmp(right))
}

/// The results for `query`, grouped as the mockup shows them; groups without results are left out.
#[must_use]
pub fn quick_switcher_results(
    query: &str,
    sources: &QuickSwitcherSources<'_>,
) -> Vec<QuickSwitcherGroup> {
    let trimmed = query.trim().to_lowercase();
    let narrowed_to = kind_of_prefix(trimmed.chars().next());
    let term = fold_for_search(if narrowed_to.is_some() {
        trimmed[1..].trim()
    } else {
        &trimmed
    });
    let matches = |name: &str| fold_for_search(name).contains(&term);

    let channels: Vec<QuickSwitcherItem> = sources
        .guilds
        .iter()
        .flat_map(|guild| {
            channel_sections(&guild.channels)
                .into_iter()
                .flat_map(|section| section.channels)
                // shortcut: voice channels are left out (joining from here comes with voice).
                .filter(|channel| channel.kind == ChannelKind::Text && matches(&channel.name))
                .map(|channel| QuickSwitcherItem {
                    icon: if (sources.can_send_in)(channel.id) {
                        QuickSwitcherIcon::Hash
                    } else {
                        QuickSwitcherIcon::Lock
                    },
                    has_unread: (sources.has_unread)(channel.id),
                    place: QuickSwitcherPlace::InServer(guild.name.clone()),
                    target: QuickSwitcherTarget::Channel {
                        guild_id: guild.id,
                        channel_id: channel.id,
                    },
                    label: channel.name,
                })
                .collect::<Vec<_>>()
        })
        .collect();

    let people_and_groups = people_and_group_items(sources, &matches);

    let servers: Vec<QuickSwitcherItem> = sources
        .guilds
        .iter()
        .filter(|guild| matches(&guild.name))
        .map(|guild| QuickSwitcherItem {
            label: guild.name.clone(),
            place: QuickSwitcherPlace::Server,
            icon: QuickSwitcherIcon::Users,
            has_unread: guild
                .channels
                .iter()
                .any(|channel| (sources.has_unread)(channel.id)),
            target: QuickSwitcherTarget::Server { guild_id: guild.id },
        })
        .collect();

    let shown = |mut items: Vec<QuickSwitcherItem>| {
        if term.is_empty() {
            items.truncate(MAXIMUM_ITEMS_PER_KIND_BEFORE_TYPING);
        }
        items
    };
    [
        (QuickSwitcherGroupKind::Channels, channels),
        (QuickSwitcherGroupKind::People, people_and_groups),
        (QuickSwitcherGroupKind::Servers, servers),
    ]
    .into_iter()
    .filter(|(kind, _)| narrowed_to.is_none_or(|narrowed| narrowed == *kind))
    .map(|(kind, items)| QuickSwitcherGroup {
        kind,
        items: shown(items),
    })
    .filter(|group| !group.items.is_empty())
    .collect()
}

/// People by name (never the reader), then groups by their name or by anyone in them (ADR-0026).
fn people_and_group_items(
    sources: &QuickSwitcherSources<'_>,
    matches: &dyn Fn(&str) -> bool,
) -> Vec<QuickSwitcherItem> {
    let mut conversation_by_user_id: HashMap<Snowflake, Snowflake> = HashMap::new();
    let mut people: Vec<&PublicUser> = Vec::new();
    let mut listed_user_ids: HashSet<Snowflake> = HashSet::new();
    for conversation in sources.conversations {
        let is_group = conversation.owner_id.is_some();
        for recipient in &conversation.recipients {
            // Someone met only in a group is listed when the instance would open a conversation
            // with them: a friend here, or a member of a loaded server below.
            if !is_group {
                conversation_by_user_id.insert(recipient.id, conversation.id);
            }
            if (!is_group || (sources.is_friend)(recipient.id))
                && listed_user_ids.insert(recipient.id)
            {
                people.push(recipient);
            }
        }
    }
    for person in &sources.known_people {
        if listed_user_ids.insert(person.id) {
            people.push(person);
        }
    }
    people.retain(|person| {
        Some(person.id) != sources.current_user_id
            && (matches(display_name(person)) || matches(&person.username))
    });
    people.sort_by(|left, right| compare_shown_names(display_name(left), display_name(right)));
    let people_items = people.into_iter().map(|person| {
        let conversation_id = conversation_by_user_id.get(&person.id).copied();
        QuickSwitcherItem {
            label: display_name(person).to_owned(),
            place: QuickSwitcherPlace::DirectMessage,
            icon: QuickSwitcherIcon::At,
            has_unread: conversation_id.is_some_and(|id| (sources.has_unread)(id)),
            target: QuickSwitcherTarget::Person {
                user_id: person.id,
                conversation_id,
            },
        }
    });
    // Groups by their name or by anyone in them (ADR-0026), after the people.
    let group_items = sources
        .conversations
        .iter()
        .filter(|conversation| conversation.owner_id.is_some())
        .filter_map(|conversation| {
            let label = (sources.conversation_name)(conversation);
            let is_match = matches(&label)
                || conversation.recipients.iter().any(|person| {
                    Some(person.id) != sources.current_user_id
                        && (matches(display_name(person)) || matches(&person.username))
                });
            is_match.then(|| QuickSwitcherItem {
                label,
                place: QuickSwitcherPlace::Group,
                icon: QuickSwitcherIcon::Users,
                has_unread: (sources.has_unread)(conversation.id),
                target: QuickSwitcherTarget::Conversation {
                    conversation_id: conversation.id,
                },
            })
        });
    people_items.chain(group_items).collect()
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{Channel, Permissions};

    use super::*;

    const ME: Snowflake = Snowflake::from_raw(1000);

    fn id(raw: u64) -> Snowflake {
        Snowflake::from_raw(raw)
    }

    fn person(raw: u64, username: &str, display_name: &str) -> PublicUser {
        PublicUser {
            id: id(raw),
            username: username.to_owned(),
            display_name: display_name.to_owned(),
            avatar: None,
            banner: None,
        }
    }

    fn channel(raw: u64, name: &str) -> Channel {
        Channel {
            id: id(raw),
            guild_id: id(1),
            kind: ChannelKind::Text,
            name: name.to_owned(),
            topic: String::new(),
            parent_id: None,
            position: u32::try_from(raw).unwrap(),
            icon: None,
            permission_overwrites: Vec::new(),
            user_limit: None,
        }
    }

    fn rust_lyon() -> Guild {
        Guild {
            id: id(1),
            name: "Rust Lyon".to_owned(),
            owner_id: id(999),
            icon: None,
            my_permissions: Permissions::empty(),
            my_role_ids: Vec::new(),
            roles: Vec::new(),
            channels: vec![
                channel(11, "general"),
                channel(12, "off-topic"),
                channel(13, "announcements"),
            ],
            emojis: Vec::new(),
            sounds: Vec::new(),
            my_join_sound_id: None,
        }
    }

    fn conversation(raw: u64, recipients: Vec<PublicUser>) -> DirectMessageChannel {
        DirectMessageChannel {
            id: id(raw),
            recipients,
            last_message_id: None,
            owner_id: None,
            name: String::new(),
        }
    }

    fn group(raw: u64, recipients: Vec<PublicUser>, name: &str) -> DirectMessageChannel {
        DirectMessageChannel {
            owner_id: Some(ME),
            name: name.to_owned(),
            ..conversation(raw, recipients)
        }
    }

    fn ana() -> PublicUser {
        person(2, "ana", "Ana")
    }

    fn zoe() -> PublicUser {
        person(3, "zoe", "Zoé")
    }

    fn names_of(conversation: &DirectMessageChannel) -> String {
        if !conversation.name.is_empty() {
            return conversation.name.clone();
        }
        conversation
            .recipients
            .iter()
            .filter(|recipient| recipient.id != ME)
            .map(|recipient| display_name(recipient).to_owned())
            .collect::<Vec<_>>()
            .join(" and ")
    }

    fn results(
        query: &str,
        conversations: &[DirectMessageChannel],
        known_people: &[PublicUser],
        friend: Option<Snowflake>,
    ) -> Vec<QuickSwitcherGroup> {
        let guilds = [rust_lyon()];
        let is_friend = move |user_id| Some(user_id) == friend;
        let has_unread = |channel_id| channel_id == id(12);
        let can_send_in = |channel_id| channel_id != id(13);
        let sources = QuickSwitcherSources {
            guilds: &guilds,
            conversations,
            known_people: known_people.iter().collect(),
            current_user_id: Some(ME),
            is_friend: &is_friend,
            has_unread: &has_unread,
            can_send_in: &can_send_in,
            conversation_name: &names_of,
        };
        quick_switcher_results(query, &sources)
    }

    fn default_results(query: &str) -> Vec<QuickSwitcherGroup> {
        results(
            query,
            &[conversation(50, vec![ana()])],
            &[person(1000, "paul", "Paul"), zoe()],
            None,
        )
    }

    fn labels(groups: &[QuickSwitcherGroup]) -> Vec<Vec<&str>> {
        groups
            .iter()
            .map(|group| group.items.iter().map(|item| item.label.as_str()).collect())
            .collect()
    }

    #[test]
    fn groups_channels_people_never_yourself_and_servers() {
        let groups = default_results("");
        assert_eq!(
            groups.iter().map(|group| group.kind).collect::<Vec<_>>(),
            [
                QuickSwitcherGroupKind::Channels,
                QuickSwitcherGroupKind::People,
                QuickSwitcherGroupKind::Servers
            ]
        );
        assert_eq!(
            labels(&groups),
            [
                vec!["general", "off-topic", "announcements"],
                vec!["Ana", "Zoé"],
                vec!["Rust Lyon"]
            ]
        );
        let off_topic = &groups[0].items[1];
        assert!(off_topic.has_unread);
        assert_eq!(off_topic.icon, QuickSwitcherIcon::Hash);
        assert_eq!(
            off_topic.place,
            QuickSwitcherPlace::InServer("Rust Lyon".to_owned())
        );
        assert_eq!(groups[0].items[2].icon, QuickSwitcherIcon::Lock);
        assert_eq!(
            groups[1].items[0].target,
            QuickSwitcherTarget::Person {
                user_id: id(2),
                conversation_id: Some(id(50))
            }
        );
    }

    #[test]
    fn at_most_twenty_of_a_kind_before_anything_is_typed() {
        let crowd: Vec<PublicUser> = (0..300)
            .map(|index| person(2000 + index, &format!("p{index}"), ""))
            .collect();
        let people_count = |query: &str| {
            results(query, &[], &crowd, None)
                .into_iter()
                .find(|group| group.kind == QuickSwitcherGroupKind::People)
                .map_or(0, |group| group.items.len())
        };
        assert_eq!(people_count(""), MAXIMUM_ITEMS_PER_KIND_BEFORE_TYPING);
        assert_eq!(people_count("@p"), 300);
    }

    #[test]
    fn a_prefix_narrows_and_names_match_anywhere() {
        assert_eq!(labels(&default_results("#TOP")), [vec!["off-topic"]]);
        let kinds = |query: &str| {
            default_results(query)
                .iter()
                .map(|group| group.kind)
                .collect::<Vec<_>>()
        };
        assert_eq!(kinds("@zo"), [QuickSwitcherGroupKind::People]);
        assert_eq!(kinds("*lyon"), [QuickSwitcherGroupKind::Servers]);
        assert_eq!(default_results("nothing like it"), Vec::new());
    }

    #[test]
    fn accents_are_ignored_both_ways() {
        assert_eq!(labels(&default_results("#GÉNÉ")), [vec!["general"]]);
        assert_eq!(labels(&default_results("@zoe")), [vec!["Zoé"]]);
        assert_eq!(labels(&default_results("@ZOÉ")), [vec!["Zoé"]]);
    }

    #[test]
    fn groups_match_by_name_or_anyone_in_them_and_people_keep_their_own_conversation() {
        let ines = person(4, "ines", "Inès");
        let yann = person(5, "yann", "Yann");
        let conversations = [
            group(60, vec![ana(), zoe(), ines.clone(), yann], "Trip"),
            conversation(50, vec![ana()]),
            DirectMessageChannel {
                owner_id: Some(id(3)),
                ..conversation(61, vec![zoe(), ana()])
            },
        ];
        let known = [person(1000, "paul", "Paul"), zoe()];
        let groups = results("@", &conversations, &known, Some(ines.id));
        let people = &groups[0].items;
        // Yann is met only in a group and is no friend: the instance would refuse a conversation.
        assert_eq!(
            labels(&groups),
            [vec!["Ana", "Inès", "Zoé", "Trip", "Zoé and Ana"]]
        );
        assert_eq!(
            people[1].target,
            QuickSwitcherTarget::Person {
                user_id: ines.id,
                conversation_id: None
            }
        );
        // Zoé shares a loaded server: she can be messaged without a conversation yet.
        assert_eq!(
            people[2].target,
            QuickSwitcherTarget::Person {
                user_id: id(3),
                conversation_id: None
            }
        );
        assert_eq!(people[3].place, QuickSwitcherPlace::Group);
        assert_eq!(
            people[3].target,
            QuickSwitcherTarget::Conversation {
                conversation_id: id(60)
            }
        );
        let trip = results("trip", &conversations, &known, None);
        assert_eq!(labels(&trip), [vec!["Trip"]]);
        let by_member = results("yann", &conversations, &known, None);
        assert_eq!(labels(&by_member), [vec!["Trip"]]);
    }
}
