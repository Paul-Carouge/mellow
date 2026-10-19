//! What the client knows, updated by `Ready`, gateway events and REST answers. Pure: no I/O,
//! so every UI behaves the same and the logic is tested without a server.

use std::collections::{BTreeMap, HashMap, HashSet};

use mellow_protocol::{
    Channel, CurrentUser, GatewayEvent, Guild, Member, Message, PresenceStatus, Ready, Snowflake,
};

/// Messages kept in memory per channel; older ones are fetched again when scrolled to.
pub const MAXIMUM_CACHED_MESSAGES_PER_CHANNEL: usize = 500;
/// A typing indicator disappears after this long without a new `TYPING_START`.
pub const TYPING_INDICATOR_LIFETIME_MILLISECONDS: u64 = 8_000;

/// What an update touched, so a UI refreshes only that part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateChange {
    /// Guild list, a guild's channels, or the signed-in user.
    Guilds,
    Messages {
        channel_id: Snowflake,
    },
    Members {
        guild_id: Snowflake,
    },
    Presence,
    Typing {
        channel_id: Snowflake,
    },
    ReadStates,
    Nothing,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ClientState {
    current_user: Option<CurrentUser>,
    /// In server-rail order: `Ready` order, then joined guilds appended.
    guilds: Vec<Guild>,
    /// Sorted by id (chronological), at most `MAXIMUM_CACHED_MESSAGES_PER_CHANNEL`.
    messages_by_channel: HashMap<Snowflake, Vec<Message>>,
    members_by_guild: HashMap<Snowflake, Vec<Member>>,
    last_read_by_channel: HashMap<Snowflake, Snowflake>,
    online_user_ids: HashSet<Snowflake>,
    /// (channel, user) → when the indicator expires.
    typing_until: BTreeMap<(Snowflake, Snowflake), u64>,
}

impl ClientState {
    #[must_use]
    pub fn current_user(&self) -> Option<&CurrentUser> {
        self.current_user.as_ref()
    }

    #[must_use]
    pub fn guilds(&self) -> &[Guild] {
        &self.guilds
    }

    #[must_use]
    pub fn guild(&self, guild_id: Snowflake) -> Option<&Guild> {
        self.guilds.iter().find(|guild| guild.id == guild_id)
    }

    #[must_use]
    pub fn messages(&self, channel_id: Snowflake) -> &[Message] {
        self.messages_by_channel
            .get(&channel_id)
            .map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn members(&self, guild_id: Snowflake) -> &[Member] {
        self.members_by_guild
            .get(&guild_id)
            .map_or(&[], Vec::as_slice)
    }

    #[must_use]
    pub fn is_online(&self, user_id: Snowflake) -> bool {
        self.online_user_ids.contains(&user_id)
    }

    /// True when the newest known message is after the user's read position.
    #[must_use]
    pub fn has_unread(&self, channel_id: Snowflake) -> bool {
        let newest = self.messages(channel_id).last().map(|message| message.id);
        match (newest, self.last_read_by_channel.get(&channel_id)) {
            (Some(newest), Some(last_read)) => newest > *last_read,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// Users typing in `channel_id` at `now_unix_milliseconds`, excluding the signed-in user.
    #[must_use]
    pub fn typing_user_ids(
        &self,
        channel_id: Snowflake,
        now_unix_milliseconds: u64,
    ) -> Vec<Snowflake> {
        let me = self.current_user.as_ref().map(|user| user.id);
        self.typing_until
            .range(
                (channel_id, Snowflake::from_raw(0))..=(channel_id, Snowflake::from_raw(u64::MAX)),
            )
            .filter(|((_, user_id), until)| **until > now_unix_milliseconds && Some(*user_id) != me)
            .map(|((_, user_id), _)| *user_id)
            .collect()
    }

    /// Replaces everything with a fresh `Ready` (identify after a lost session).
    pub fn apply_ready(&mut self, ready: Ready) -> StateChange {
        *self = Self {
            current_user: Some(ready.user),
            guilds: ready.guilds,
            last_read_by_channel: ready
                .read_states
                .into_iter()
                .map(|state| (state.channel_id, state.last_read_message_id))
                .collect(),
            online_user_ids: ready.online_user_ids.into_iter().collect(),
            ..Self::default()
        };
        StateChange::Guilds
    }

    pub fn apply_event(&mut self, event: GatewayEvent, now_unix_milliseconds: u64) -> StateChange {
        match event {
            GatewayEvent::MessageCreate(message) => {
                let channel_id = message.channel_id;
                self.typing_until.remove(&(channel_id, message.author.id));
                self.insert_messages(channel_id, vec![message])
            }
            GatewayEvent::MessageUpdate(message) => self.replace_cached_message(message),
            GatewayEvent::MessageDelete { channel_id, id, .. } => {
                if let Some(messages) = self.messages_by_channel.get_mut(&channel_id) {
                    messages.retain(|message| message.id != id);
                }
                StateChange::Messages { channel_id }
            }
            GatewayEvent::ChannelCreate(channel) | GatewayEvent::ChannelUpdate(channel) => {
                self.upsert_channel(channel);
                StateChange::Guilds
            }
            GatewayEvent::ChannelDelete { guild_id, id } => {
                if let Some(guild) = self.guilds.iter_mut().find(|guild| guild.id == guild_id) {
                    guild.channels.retain(|channel| channel.id != id);
                    // Children of a deleted category become uncategorised, as on the server.
                    for channel in &mut guild.channels {
                        if channel.parent_id == Some(id) {
                            channel.parent_id = None;
                        }
                    }
                }
                self.messages_by_channel.remove(&id);
                StateChange::Guilds
            }
            GatewayEvent::GuildCreate(guild) => {
                let guild = *guild;
                match self
                    .guilds
                    .iter_mut()
                    .find(|existing| existing.id == guild.id)
                {
                    Some(existing) => *existing = guild,
                    None => self.guilds.push(guild),
                }
                StateChange::Guilds
            }
            GatewayEvent::GuildDelete { id } => {
                if let Some(guild) = self.guild(id) {
                    let channel_ids: Vec<Snowflake> =
                        guild.channels.iter().map(|channel| channel.id).collect();
                    for channel_id in channel_ids {
                        self.messages_by_channel.remove(&channel_id);
                    }
                }
                self.guilds.retain(|guild| guild.id != id);
                self.members_by_guild.remove(&id);
                StateChange::Guilds
            }
            GatewayEvent::GuildMemberAdd { guild_id, member } => {
                if let Some(members) = self.members_by_guild.get_mut(&guild_id)
                    && !members
                        .iter()
                        .any(|existing| existing.user.id == member.user.id)
                {
                    members.push(member);
                    members.sort_by(|left, right| left.user.username.cmp(&right.user.username));
                }
                StateChange::Members { guild_id }
            }
            GatewayEvent::GuildMemberRemove { guild_id, user_id } => {
                if let Some(members) = self.members_by_guild.get_mut(&guild_id) {
                    members.retain(|member| member.user.id != user_id);
                }
                StateChange::Members { guild_id }
            }
            GatewayEvent::TypingStart {
                channel_id,
                user_id,
                ..
            } => {
                // Expired entries are dropped here, so the map never outgrows who types now.
                self.typing_until
                    .retain(|_, until| *until > now_unix_milliseconds);
                self.typing_until.insert(
                    (channel_id, user_id),
                    now_unix_milliseconds.saturating_add(TYPING_INDICATOR_LIFETIME_MILLISECONDS),
                );
                StateChange::Typing { channel_id }
            }
            GatewayEvent::PresenceUpdate { user_id, status } => {
                match status {
                    PresenceStatus::Online => self.online_user_ids.insert(user_id),
                    PresenceStatus::Offline => self.online_user_ids.remove(&user_id),
                };
                StateChange::Presence
            }
        }
    }

    /// Only cached messages change: an edit of older history must not appear out of context, nor
    /// mark a never-opened channel as unread. An older edit never replaces a newer one: our own
    /// REST answer can arrive after the gateway already delivered a later edit.
    fn replace_cached_message(&mut self, message: Message) -> StateChange {
        let channel_id = message.channel_id;
        let Some(cached) = self
            .messages_by_channel
            .get_mut(&channel_id)
            .and_then(|messages| {
                let index = messages
                    .binary_search_by_key(&message.id, |existing| existing.id)
                    .ok()?;
                messages.get_mut(index)
            })
        else {
            return StateChange::Nothing;
        };
        // `None` (never edited) orders before any edit time.
        if message.edited_at_unix_milliseconds < cached.edited_at_unix_milliseconds {
            return StateChange::Nothing;
        }
        *cached = message;
        StateChange::Messages { channel_id }
    }

    /// Merges messages from history or from our own send (deduplicated by id, kept sorted and
    /// capped, keeping the newest).
    pub fn insert_messages(
        &mut self,
        channel_id: Snowflake,
        incoming: Vec<Message>,
    ) -> StateChange {
        let messages = self.messages_by_channel.entry(channel_id).or_default();
        // One sort for the whole batch, whatever order a server sends: never O(n²) on the UI thread.
        let mut incoming = incoming;
        incoming.sort_by_key(|message| message.id);
        let newest_incoming = incoming.split_off(
            incoming
                .len()
                .saturating_sub(MAXIMUM_CACHED_MESSAGES_PER_CHANNEL),
        );
        // On equal ids the incoming copy (listed first) wins, so edits replace cached versions.
        let mut merged: Vec<Message> = newest_incoming
            .into_iter()
            .chain(std::mem::take(messages))
            .collect();
        merged.sort_by_key(|message| message.id);
        merged.dedup_by_key(|message| message.id);
        let excess = merged
            .len()
            .saturating_sub(MAXIMUM_CACHED_MESSAGES_PER_CHANNEL);
        merged.drain(..excess);
        *messages = merged;
        StateChange::Messages { channel_id }
    }

    pub fn set_members(&mut self, guild_id: Snowflake, members: Vec<Member>) -> StateChange {
        self.members_by_guild.insert(guild_id, members);
        StateChange::Members { guild_id }
    }

    /// Our own acknowledgement; like the server, never moves backwards.
    pub fn mark_read(&mut self, channel_id: Snowflake, message_id: Snowflake) -> StateChange {
        let last_read = self
            .last_read_by_channel
            .entry(channel_id)
            .or_insert(message_id);
        if message_id > *last_read {
            *last_read = message_id;
        }
        StateChange::ReadStates
    }

    fn upsert_channel(&mut self, channel: Channel) {
        if let Some(guild) = self
            .guilds
            .iter_mut()
            .find(|guild| guild.id == channel.guild_id)
        {
            match guild
                .channels
                .iter_mut()
                .find(|existing| existing.id == channel.id)
            {
                Some(existing) => *existing = channel,
                None => guild.channels.push(channel),
            }
            guild
                .channels
                .sort_by_key(|existing| (existing.position, existing.id));
        }
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{ChannelKind, Permissions, PublicUser, ReadState};

    use super::*;

    const GUILD: Snowflake = Snowflake::from_raw(100);
    const CATEGORY: Snowflake = Snowflake::from_raw(101);
    const GENERAL: Snowflake = Snowflake::from_raw(102);
    const ME: Snowflake = Snowflake::from_raw(1);
    const OTHER: Snowflake = Snowflake::from_raw(2);
    const NOW: u64 = 1_000_000;

    fn channel(
        id: Snowflake,
        kind: ChannelKind,
        parent_id: Option<Snowflake>,
        position: u32,
    ) -> Channel {
        Channel {
            id,
            guild_id: GUILD,
            kind,
            name: format!("c{}", id.raw()),
            topic: String::new(),
            parent_id,
            position,
        }
    }

    fn user(id: Snowflake) -> PublicUser {
        PublicUser {
            id,
            username: format!("user{}", id.raw()),
            display_name: format!("User {}", id.raw()),
        }
    }

    fn message(id: u64, author: Snowflake) -> Message {
        Message {
            id: Snowflake::from_raw(id),
            channel_id: GENERAL,
            author: user(author),
            content: format!("message {id}"),
            reply_to_id: None,
            edited_at_unix_milliseconds: None,
            nonce: None,
        }
    }

    fn ready_state() -> ClientState {
        let mut state = ClientState::default();
        state.apply_ready(Ready {
            session_id: "s".into(),
            user: CurrentUser {
                id: ME,
                username: "me".into(),
                display_name: "Me".into(),
                email: "me@example.org".into(),
                is_instance_administrator: false,
            },
            guilds: vec![Guild {
                id: GUILD,
                name: "Club".into(),
                owner_id: ME,
                my_permissions: Permissions::all(),
                channels: vec![
                    channel(CATEGORY, ChannelKind::Category, None, 0),
                    channel(GENERAL, ChannelKind::Text, Some(CATEGORY), 1),
                ],
            }],
            read_states: vec![ReadState {
                channel_id: GENERAL,
                last_read_message_id: Snowflake::from_raw(10),
            }],
            online_user_ids: vec![ME],
        });
        state
    }

    #[test]
    fn messages_stay_sorted_deduplicated_and_capped() {
        let mut state = ready_state();
        state.insert_messages(GENERAL, vec![message(30, OTHER), message(10, OTHER)]);
        state.apply_event(GatewayEvent::MessageCreate(message(20, OTHER)), NOW);
        state.apply_event(GatewayEvent::MessageCreate(message(20, OTHER)), NOW);
        let ids: Vec<u64> = state
            .messages(GENERAL)
            .iter()
            .map(|message| message.id.raw())
            .collect();
        assert_eq!(ids, [10, 20, 30]);

        let many = (1..=(MAXIMUM_CACHED_MESSAGES_PER_CHANNEL as u64 + 10))
            .map(|id| message(1_000 + id, OTHER))
            .collect();
        state.insert_messages(GENERAL, many);
        assert_eq!(
            state.messages(GENERAL).len(),
            MAXIMUM_CACHED_MESSAGES_PER_CHANNEL
        );
        assert_eq!(
            state.messages(GENERAL).last().unwrap().id.raw(),
            1_000 + MAXIMUM_CACHED_MESSAGES_PER_CHANNEL as u64 + 10,
            "the newest are kept"
        );
    }

    #[test]
    fn edits_deletes_and_unread_markers() {
        let mut state = ready_state();
        state.apply_event(GatewayEvent::MessageCreate(message(11, OTHER)), NOW);
        assert!(state.has_unread(GENERAL));
        state.mark_read(GENERAL, Snowflake::from_raw(11));
        state.mark_read(GENERAL, Snowflake::from_raw(5));
        assert!(!state.has_unread(GENERAL), "read position never moves back");

        let mut edited = message(11, OTHER);
        edited.content = "edited".into();
        edited.edited_at_unix_milliseconds = Some(NOW);
        let mut edited_later = edited.clone();
        edited_later.content = "edited later".into();
        edited_later.edited_at_unix_milliseconds = Some(NOW + 1);
        state.apply_event(GatewayEvent::MessageUpdate(edited_later), NOW);
        assert_eq!(
            state.apply_event(GatewayEvent::MessageUpdate(edited), NOW),
            StateChange::Nothing,
            "a late answer for an older edit is ignored"
        );
        assert_eq!(state.messages(GENERAL)[0].content, "edited later");
        let mut not_cached = message(3, OTHER);
        not_cached.channel_id = Snowflake::from_raw(999);
        assert_eq!(
            state.apply_event(GatewayEvent::MessageUpdate(not_cached), NOW),
            StateChange::Nothing
        );
        assert!(!state.has_unread(Snowflake::from_raw(999)));
        state.apply_event(
            GatewayEvent::MessageDelete {
                guild_id: GUILD,
                channel_id: GENERAL,
                id: Snowflake::from_raw(11),
            },
            NOW,
        );
        assert_eq!(state.messages(GENERAL), &[]);
    }

    #[test]
    fn channels_guilds_and_members_follow_events() {
        let mut state = ready_state();
        state.apply_event(
            GatewayEvent::ChannelCreate(channel(
                Snowflake::from_raw(103),
                ChannelKind::Text,
                Some(CATEGORY),
                2,
            )),
            NOW,
        );
        assert_eq!(state.guild(GUILD).unwrap().channels.len(), 3);
        state.apply_event(
            GatewayEvent::ChannelDelete {
                guild_id: GUILD,
                id: CATEGORY,
            },
            NOW,
        );
        let channels = &state.guild(GUILD).unwrap().channels;
        assert_eq!(channels.len(), 2);
        assert!(channels.iter().all(|channel| channel.parent_id.is_none()));

        state.set_members(
            GUILD,
            vec![Member {
                user: user(ME),
                joined_at_unix_milliseconds: 0,
            }],
        );
        state.apply_event(
            GatewayEvent::GuildMemberAdd {
                guild_id: GUILD,
                member: Member {
                    user: user(OTHER),
                    joined_at_unix_milliseconds: 1,
                },
            },
            NOW,
        );
        assert_eq!(state.members(GUILD).len(), 2);
        state.apply_event(
            GatewayEvent::GuildMemberRemove {
                guild_id: GUILD,
                user_id: OTHER,
            },
            NOW,
        );
        assert_eq!(state.members(GUILD).len(), 1);

        state.apply_event(GatewayEvent::GuildDelete { id: GUILD }, NOW);
        assert_eq!(state.guilds(), &[]);
    }

    #[test]
    fn presence_and_typing_indicators() {
        let mut state = ready_state();
        state.apply_event(
            GatewayEvent::PresenceUpdate {
                user_id: OTHER,
                status: PresenceStatus::Online,
            },
            NOW,
        );
        assert!(state.is_online(OTHER));
        state.apply_event(
            GatewayEvent::PresenceUpdate {
                user_id: OTHER,
                status: PresenceStatus::Offline,
            },
            NOW,
        );
        assert!(!state.is_online(OTHER));

        for user_id in [OTHER, ME] {
            state.apply_event(
                GatewayEvent::TypingStart {
                    guild_id: GUILD,
                    channel_id: GENERAL,
                    user_id,
                },
                NOW,
            );
        }
        assert_eq!(
            state.typing_user_ids(GENERAL, NOW + 1),
            [OTHER],
            "my own typing is not shown"
        );
        assert_eq!(
            state.typing_user_ids(GENERAL, NOW + TYPING_INDICATOR_LIFETIME_MILLISECONDS),
            Vec::<Snowflake>::new()
        );
        state.apply_event(
            GatewayEvent::TypingStart {
                guild_id: GUILD,
                channel_id: GENERAL,
                user_id: OTHER,
            },
            NOW,
        );
        state.apply_event(GatewayEvent::MessageCreate(message(50, OTHER)), NOW);
        assert_eq!(
            state.typing_user_ids(GENERAL, NOW + 1),
            Vec::<Snowflake>::new(),
            "sending clears typing"
        );
    }
}
