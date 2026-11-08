//! Forwarding a message (`web/src/components/forward-messages.ts`, DESIGN.md §5.4, ADR-0039): the
//! places the forward dialog offers, its search, the order its rows keep, where a forward's
//! original was written, and what a dialog remembers between tries. Naming a conversation takes
//! the catalogue's words, so the client passes its names in.

use std::collections::{HashMap, HashSet};

use mellow_protocol::{
    ChannelKind, DirectMessageChannel, Guild, Permissions, PublicUser, Snowflake,
    compute_channel_permissions,
};

use super::channel_order::channel_sections;
use super::format::display_name;
use super::group_conversations::{is_group_conversation, other_participants};
use super::search_text::fold_for_search;

/// As on Discord: one forward reaches at most this many places.
pub const MAXIMUM_FORWARD_DESTINATIONS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardDestinationKind {
    Channel,
    Person,
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardDestination {
    /// The channel the forward is posted in.
    pub channel_id: Snowflake,
    pub kind: ForwardDestinationKind,
    /// A channel's name (shown after its `#`), the person, or the group's name.
    pub name: String,
    /// The second line: the server's name, "@ana", or the client's word for a group.
    pub place: String,
    /// A conversation's other people, for its avatar; empty for a channel.
    pub people: Vec<PublicUser>,
    /// Name, place and usernames, folded once for the search.
    search_text: String,
}

impl ForwardDestination {
    /// How it is named in sentences: "#général", "Ana", or a group's name (`destinationLabel`).
    #[must_use]
    pub fn label(&self) -> String {
        match self.kind {
            ForwardDestinationKind::Channel => format!("#{}", self.name),
            ForwardDestinationKind::Person | ForwardDestinationKind::Group => self.name.clone(),
        }
    }
}

pub struct ForwardDestinationSources<'a> {
    pub guilds: &'a [Guild],
    /// Most recent first, as the navigator lists them.
    pub conversations: &'a [DirectMessageChannel],
    pub current_user_id: Snowflake,
    pub is_blocked: &'a dyn Fn(Snowflake) -> bool,
    /// The message has files: the place needs `ATTACH_FILES` too, or the instance refuses.
    pub has_attachments: bool,
    /// A conversation's name as the client's navigator shows it.
    pub conversation_name: &'a dyn Fn(&DirectMessageChannel) -> String,
    /// The second line of a group ("Group").
    pub group_word: &'a str,
}

impl std::fmt::Debug for ForwardDestinationSources<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ForwardDestinationSources")
            .field("guilds", &self.guilds.len())
            .field("conversations", &self.conversations.len())
            .field("has_attachments", &self.has_attachments)
            .finish_non_exhaustive()
    }
}

/// The conversations first (most recent first), then each server's text channels in the
/// sidebar's order: only places the reader may post the forward in; the instance decides anyway
/// (ADR-0014).
#[must_use]
pub fn forward_destinations(sources: &ForwardDestinationSources<'_>) -> Vec<ForwardDestination> {
    let me = Some(sources.current_user_id);
    let conversations = sources
        .conversations
        .iter()
        // A one-to-one conversation with someone you blocked has no composer: nothing goes there.
        .filter(|conversation| {
            is_group_conversation(conversation)
                || !other_participants(conversation, me)
                    .iter()
                    .any(|person| (sources.is_blocked)(person.id))
        })
        .map(|conversation| {
            let people: Vec<PublicUser> = other_participants(conversation, me)
                .into_iter()
                .cloned()
                .collect();
            let is_group = is_group_conversation(conversation);
            let name = (sources.conversation_name)(conversation);
            let place = if is_group {
                sources.group_word.to_owned()
            } else {
                format!(
                    "@{}",
                    people.first().map_or("", |person| person.username.as_str())
                )
            };
            let mut words = vec![name.clone(), place.clone()];
            for person in &people {
                words.push(display_name(person).to_owned());
                words.push(person.username.clone());
            }
            ForwardDestination {
                channel_id: conversation.id,
                kind: if is_group {
                    ForwardDestinationKind::Group
                } else {
                    ForwardDestinationKind::Person
                },
                name,
                place,
                people,
                search_text: fold_for_search(&words.join(" ")),
            }
        });
    let channels = sources.guilds.iter().flat_map(|guild| {
        channel_sections(&guild.channels)
            .into_iter()
            .flat_map(|section| section.channels)
            .filter(|channel| {
                if channel.kind != ChannelKind::Text {
                    return false;
                }
                let permissions = compute_channel_permissions(
                    guild.my_permissions,
                    guild.id,
                    sources.current_user_id,
                    &guild.my_role_ids,
                    &channel.permission_overwrites,
                );
                let mut needed = Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES;
                if sources.has_attachments {
                    needed |= Permissions::ATTACH_FILES;
                }
                permissions.contains(needed)
            })
            .map(|channel| ForwardDestination {
                channel_id: channel.id,
                kind: ForwardDestinationKind::Channel,
                search_text: fold_for_search(&format!("{} {}", channel.name, guild.name)),
                name: channel.name,
                place: guild.name.clone(),
                people: Vec::new(),
            })
    });
    conversations.chain(channels).collect()
}

/// `destinations` in the order of `channel_ids` (the list when the dialog opened), places that
/// appeared since at the end: a forward sent moves its conversation to the top of the navigator,
/// which must not reshuffle the rows being picked.
#[must_use]
pub fn in_opening_order(
    mut destinations: Vec<ForwardDestination>,
    channel_ids: &[Snowflake],
) -> Vec<ForwardDestination> {
    let rank_of: HashMap<Snowflake, usize> = channel_ids
        .iter()
        .enumerate()
        .map(|(index, channel_id)| (*channel_id, index))
        .collect();
    // A stable sort: places unknown at opening keep their own order after the others.
    destinations.sort_by_key(|destination| {
        rank_of
            .get(&destination.channel_id)
            .copied()
            .unwrap_or(channel_ids.len())
    });
    destinations
}

/// Every word typed, in any order, ignoring case and accents; a leading `#` or `@` is only a
/// habit (`destinationsMatching`).
#[must_use]
pub fn destinations_matching<'a>(
    destinations: &'a [ForwardDestination],
    query: &str,
) -> Vec<&'a ForwardDestination> {
    let trimmed = query.trim_start();
    let without_sigil = trimmed.strip_prefix(['#', '@']).unwrap_or(trimmed);
    let folded = fold_for_search(without_sigil);
    let words: Vec<&str> = folded.split_whitespace().collect();
    destinations
        .iter()
        .filter(|destination| {
            words
                .iter()
                .all(|word| destination.search_text.contains(word))
        })
        .collect()
}

/// Where a forward's original was written, as its footer names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardOriginPlace {
    /// "#général" in a server, the conversation's name at Home.
    pub label: String,
    pub guild_id: Option<Snowflake>,
}

/// `None` when the reader does not know that channel: the ids reveal nothing else (ADR-0039).
#[must_use]
pub fn forward_origin_place(
    channel_id: Snowflake,
    guilds: &[Guild],
    conversations: &[DirectMessageChannel],
    conversation_name: impl Fn(&DirectMessageChannel) -> String,
) -> Option<ForwardOriginPlace> {
    guilds
        .iter()
        .find_map(|guild| {
            let channel = guild
                .channels
                .iter()
                .find(|channel| channel.id == channel_id)?;
            Some(ForwardOriginPlace {
                label: format!("#{}", channel.name),
                guild_id: Some(guild.id),
            })
        })
        .or_else(|| {
            let conversation = conversations
                .iter()
                .find(|conversation| conversation.id == channel_id)?;
            Some(ForwardOriginPlace {
                label: conversation_name(conversation),
                guild_id: None,
            })
        })
}

/// One nonce for the forward and one for the note in a place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardNonces {
    pub forward: String,
    pub note: String,
}

/// What a forward dialog remembers between tries (`ForwardAttempts`): one nonce pair per place
/// for the dialog's life, so a retry after a lost answer gets the message already made instead
/// of a copy; and the places whose forward landed, which a retry only sends the note to.
#[derive(Debug, Default)]
pub struct ForwardAttempts {
    nonces_by_channel_id: HashMap<Snowflake, ForwardNonces>,
    forwarded_channel_ids: HashSet<Snowflake>,
}

impl ForwardAttempts {
    /// The place's nonces, made with `create_nonce` the first time.
    ///
    /// # Errors
    /// What `create_nonce` returns; nothing is remembered then.
    pub fn nonces_for<Error>(
        &mut self,
        channel_id: Snowflake,
        mut create_nonce: impl FnMut() -> Result<String, Error>,
    ) -> Result<ForwardNonces, Error> {
        if let Some(nonces) = self.nonces_by_channel_id.get(&channel_id) {
            return Ok(nonces.clone());
        }
        let nonces = ForwardNonces {
            forward: create_nonce()?,
            note: create_nonce()?,
        };
        self.nonces_by_channel_id.insert(channel_id, nonces.clone());
        Ok(nonces)
    }

    #[must_use]
    pub fn is_forwarded(&self, channel_id: Snowflake) -> bool {
        self.forwarded_channel_ids.contains(&channel_id)
    }

    pub fn note_forwarded(&mut self, channel_id: Snowflake) {
        self.forwarded_channel_ids.insert(channel_id);
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{
        Channel, OverwriteKind, PermissionOverwrite, Permissions, PublicUser, Snowflake,
    };

    use super::*;

    // Ported from `forward-messages.test.ts`; the sending itself is the desktop session's.

    const GUILD: Snowflake = Snowflake::from_raw(1000);
    const ME: Snowflake = Snowflake::from_raw(1);
    const ANA: Snowflake = Snowflake::from_raw(2);
    const BOB: Snowflake = Snowflake::from_raw(3);
    const WITH_ANA: Snowflake = Snowflake::from_raw(20);
    const WITH_BOB: Snowflake = Snowflake::from_raw(21);
    const WEEKEND: Snowflake = Snowflake::from_raw(22);

    fn person(id: Snowflake, username: &str, display: &str) -> PublicUser {
        serde_json::from_value(serde_json::json!({
            "id": id.to_string(), "username": username, "display_name": display,
        }))
        .expect("a person")
    }

    fn channel(
        id: u64,
        name: &str,
        kind: ChannelKind,
        overwrites: &[PermissionOverwrite],
    ) -> Channel {
        serde_json::from_value(serde_json::json!({
            "id": id.to_string(), "guild_id": GUILD.to_string(), "kind": kind, "name": name,
            "topic": "", "parent_id": null, "position": id, "permission_overwrites": overwrites,
        }))
        .expect("a channel")
    }

    fn everyone(allow: Permissions, deny: Permissions) -> PermissionOverwrite {
        PermissionOverwrite {
            id: GUILD,
            kind: OverwriteKind::Role,
            allow,
            deny,
        }
    }

    fn rust_lyon(my_permissions: Permissions) -> Guild {
        serde_json::from_value(serde_json::json!({
            "id": GUILD.to_string(), "name": "Rust Lyon", "owner_id": "99",
            "my_permissions": my_permissions, "my_role_ids": [], "roles": [],
            "channels": [
                channel(2, "général", ChannelKind::Text, &[]),
                // @everyone (the role with the server's id) may read here, not write.
                channel(1, "annonces", ChannelKind::Text, &[everyone(Permissions::empty(), Permissions::SEND_MESSAGES)]),
                channel(3, "Salon vocal", ChannelKind::Voice, &[]),
                channel(4, "fichiers", ChannelKind::Text, &[everyone(Permissions::ATTACH_FILES, Permissions::empty())]),
            ],
        }))
        .expect("a guild")
    }

    fn conversations() -> Vec<DirectMessageChannel> {
        let ana = person(ANA, "ana", "Ana");
        let bob = person(BOB, "bob", "Bob");
        let me = person(ME, "paul", "Paul");
        let conversation =
            |id: Snowflake, recipients: Vec<PublicUser>, name: &str, owner: Option<Snowflake>| {
                serde_json::from_value::<DirectMessageChannel>(serde_json::json!({
                    "id": id.to_string(), "recipients": recipients, "last_message_id": null,
                    "owner_id": owner.map(|owner| owner.to_string()), "name": name,
                }))
                .expect("a conversation")
            };
        vec![
            conversation(
                WEEKEND,
                vec![me, ana.clone(), bob.clone()],
                "Week-end à Lyon",
                Some(ME),
            ),
            conversation(WITH_ANA, vec![ana], "", None),
            conversation(WITH_BOB, vec![bob], "", None),
        ]
    }

    fn name_of(conversation: &DirectMessageChannel) -> String {
        if conversation.name.is_empty() {
            other_participants(conversation, Some(ME))
                .first()
                .map_or_else(String::new, |person| display_name(person).to_owned())
        } else {
            conversation.name.clone()
        }
    }

    fn destinations(
        guild: &Guild,
        conversations: &[DirectMessageChannel],
        has_attachments: bool,
    ) -> Vec<ForwardDestination> {
        forward_destinations(&ForwardDestinationSources {
            guilds: std::slice::from_ref(guild),
            conversations,
            current_user_id: ME,
            is_blocked: &|user_id| user_id == BOB,
            has_attachments,
            conversation_name: &name_of,
            group_word: "Group",
        })
    }

    fn labels<'a>(destinations: impl IntoIterator<Item = &'a ForwardDestination>) -> Vec<String> {
        destinations
            .into_iter()
            .map(ForwardDestination::label)
            .collect()
    }

    fn read_and_send() -> Guild {
        rust_lyon(Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES)
    }

    #[test]
    fn lists_conversations_first_then_the_text_channels_you_may_write_in() {
        let found = destinations(&read_and_send(), &conversations(), false);
        assert_eq!(
            labels(&found),
            ["Week-end à Lyon", "Ana", "#général", "#fichiers"]
        );
        let kinds: Vec<(ForwardDestinationKind, &str)> = found
            .iter()
            .map(|destination| (destination.kind, destination.place.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                (ForwardDestinationKind::Group, "Group"),
                (ForwardDestinationKind::Person, "@ana"),
                (ForwardDestinationKind::Channel, "Rust Lyon"),
                (ForwardDestinationKind::Channel, "Rust Lyon"),
            ]
        );
    }

    #[test]
    fn asks_for_files_permission_too_when_the_message_carries_files() {
        let found = destinations(&read_and_send(), &conversations(), true);
        let ids: Vec<Snowflake> = found
            .iter()
            .map(|destination| destination.channel_id)
            .collect();
        // Conversations always allow files; of the server, only the channel granting them is left.
        assert_eq!(ids, [WEEKEND, WITH_ANA, Snowflake::from_raw(4)]);
    }

    #[test]
    fn offers_everything_to_an_administrator_whatever_the_overwrites() {
        let found = destinations(&rust_lyon(Permissions::ADMINISTRATOR), &[], false);
        assert_eq!(labels(&found), ["#annonces", "#général", "#fichiers"]);
    }

    #[test]
    fn search_ignores_case_accents_and_a_leading_sigil_and_counts_every_word() {
        let found = destinations(&read_and_send(), &conversations(), false);
        let matching = |query: &str| labels(destinations_matching(&found, query));
        assert_eq!(matching("  ").len(), found.len());
        assert_eq!(matching("#GENERAL"), ["#général"]);
        assert_eq!(matching("@ana"), ["Week-end à Lyon", "Ana"]);
        assert_eq!(matching("lyon fich"), ["#fichiers"]);
        assert_eq!(matching("bob"), ["Week-end à Lyon"]);
        assert_eq!(matching("zed"), Vec::<String>::new());
    }

    #[test]
    fn names_the_origin_by_its_channel_or_conversation_and_knows_nothing_else() {
        let guild = read_and_send();
        let conversations = conversations();
        assert_eq!(
            forward_origin_place(
                Snowflake::from_raw(2),
                std::slice::from_ref(&guild),
                &conversations,
                name_of
            ),
            Some(ForwardOriginPlace {
                label: "#général".to_owned(),
                guild_id: Some(GUILD)
            })
        );
        assert_eq!(
            forward_origin_place(
                WITH_ANA,
                std::slice::from_ref(&guild),
                &conversations,
                name_of
            ),
            Some(ForwardOriginPlace {
                label: "Ana".to_owned(),
                guild_id: None
            })
        );
        assert_eq!(
            forward_origin_place(
                Snowflake::from_raw(404),
                std::slice::from_ref(&guild),
                &conversations,
                name_of
            ),
            None
        );
    }

    #[test]
    fn rows_keep_their_opening_order_with_newcomers_last() {
        let found = destinations(&read_and_send(), &conversations(), false);
        // Ana's conversation moved to the top after a forward; #général went away meanwhile.
        let now = vec![found[1].clone(), found[0].clone(), found[3].clone()];
        let opening = [WEEKEND, Snowflake::from_raw(2), Snowflake::from_raw(4)];
        assert_eq!(
            labels(&in_opening_order(now, &opening)),
            ["Week-end à Lyon", "#fichiers", "Ana"]
        );
    }

    #[test]
    fn a_place_keeps_its_nonces_for_the_dialogs_life() {
        let mut attempts = ForwardAttempts::default();
        let mut count = 0;
        let mut create = || -> Result<String, ()> {
            count += 1;
            Ok(format!("nonce{count}"))
        };
        let first = attempts.nonces_for(WITH_ANA, &mut create);
        let again = attempts.nonces_for(WITH_ANA, &mut create);
        let other = attempts.nonces_for(WEEKEND, &mut create);
        assert_eq!(
            first,
            Ok(ForwardNonces {
                forward: "nonce1".to_owned(),
                note: "nonce2".to_owned()
            })
        );
        assert_eq!(again, first);
        assert_eq!(other.map(|nonces| nonces.forward), Ok("nonce3".to_owned()));
        assert!(!attempts.is_forwarded(WITH_ANA));
        attempts.note_forwarded(WITH_ANA);
        assert!(attempts.is_forwarded(WITH_ANA));
    }
}
