//! What the client knows, updated by `Ready`, gateway events and REST answers. Pure: no I/O,
//! so every UI behaves the same and the logic is tested without a server.

use std::collections::{BTreeMap, HashMap, HashSet};

use mellow_protocol::{
    Channel, CurrentUser, GatewayEvent, Guild, Member, Message, PresenceStatus, Ready, Role,
    SendMessageRequest, Snowflake, compute_guild_permissions,
};

/// Messages kept in memory per channel once the reader leaves it; older ones are fetched again
/// when scrolled to.
pub const MAXIMUM_CACHED_MESSAGES_PER_CHANNEL: usize = 500;
/// Messages asked per history request; a shorter page means the channel's start is reached.
pub const HISTORY_PAGE_SIZE: u32 = 50;
/// A typing indicator disappears after this long without a new `TYPING_START`.
pub const TYPING_INDICATOR_LIFETIME_MILLISECONDS: u64 = 8_000;

/// Only to adapt the interface: the server decides (ADR-0014).
fn recompute_my_permissions(guild: &mut Guild, current_user_id: Snowflake) {
    let member_roles = guild
        .roles
        .iter()
        .filter(|role| role.id == guild.id || guild.my_role_ids.contains(&role.id));
    guild.my_permissions = compute_guild_permissions(current_user_id, guild.owner_id, member_roles);
}

/// What an update touched, so a UI refreshes only that part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateChange {
    /// Guild list, a guild's channels or roles, the signed-in user's permissions, or the user.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingMessageStatus {
    Sending,
    /// Kept with its text until the user retries or discards it.
    Failed,
}

/// A message we sent that the server has not confirmed yet (the outbox, ARCHITECTURE.md §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMessage {
    /// Sent with every attempt, so a retry of a send that did reach the server returns the
    /// stored message instead of a duplicate.
    pub nonce: String,
    pub channel_id: Snowflake,
    pub content: String,
    pub reply_to_id: Option<Snowflake>,
    pub status: PendingMessageStatus,
}

impl PendingMessage {
    fn send_request(&self) -> SendMessageRequest {
        SendMessageRequest {
            content: self.content.clone(),
            reply_to_id: self.reply_to_id,
            nonce: Some(self.nonce.clone()),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ChannelHistory {
    /// Sorted by id (chronological).
    messages: Vec<Message>,
    /// The channel's first message is cached: there is nothing older to fetch.
    has_reached_channel_start: bool,
    /// Pages loaded by scrolling up: new messages no longer push the oldest out until
    /// `trim_history`, since the reader may be looking at them.
    is_extended_by_older_pages: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ClientState {
    current_user: Option<CurrentUser>,
    /// In server-rail order: `Ready` order, then joined guilds appended.
    guilds: Vec<Guild>,
    /// At most `MAXIMUM_CACHED_MESSAGES_PER_CHANNEL` messages, except in the channel being read
    /// (`trim_history`).
    history_by_channel: HashMap<Snowflake, ChannelHistory>,
    members_by_guild: HashMap<Snowflake, Vec<Member>>,
    last_read_by_channel: HashMap<Snowflake, Snowflake>,
    online_user_ids: HashSet<Snowflake>,
    /// (channel, user) → when the indicator expires.
    typing_until: BTreeMap<(Snowflake, Snowflake), u64>,
    /// In send order. Not cleared by `Ready`: a reconnect must not lose unsent text.
    pending_messages: Vec<PendingMessage>,
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
        self.history_by_channel
            .get(&channel_id)
            .map_or(&[], |history| history.messages.as_slice())
    }

    /// False once the channel's first message is cached; true while its history never loaded.
    #[must_use]
    pub fn has_older_history(&self, channel_id: Snowflake) -> bool {
        !self
            .history_by_channel
            .get(&channel_id)
            .is_some_and(|history| history.has_reached_channel_start)
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

    /// Our unconfirmed messages in `channel_id`, in send order (shown after the confirmed ones).
    pub fn pending_messages(&self, channel_id: Snowflake) -> impl Iterator<Item = &PendingMessage> {
        self.pending_messages
            .iter()
            .filter(move |pending| pending.channel_id == channel_id)
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
        let mut pending_messages = std::mem::take(&mut self.pending_messages);
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
        // Channels deleted while we were away never send `CHANNEL_DELETE`.
        pending_messages.retain(|pending| {
            self.guilds.iter().any(|guild| {
                guild
                    .channels
                    .iter()
                    .any(|channel| channel.id == pending.channel_id)
            })
        });
        self.pending_messages = pending_messages;
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
                if let Some(history) = self.history_by_channel.get_mut(&channel_id) {
                    history.messages.retain(|message| message.id != id);
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
                self.history_by_channel.remove(&id);
                self.pending_messages
                    .retain(|pending| pending.channel_id != id);
                StateChange::Guilds
            }
            GatewayEvent::GuildCreate(guild) => self.replace_guild(*guild),
            GatewayEvent::GuildDelete { id } => self.remove_guild(id),
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
            GatewayEvent::GuildMemberUpdate {
                guild_id,
                user_id,
                role_ids,
            } => self.update_member_roles(guild_id, user_id, role_ids),
            GatewayEvent::GuildRolesUpdate { guild_id, roles } => {
                self.replace_roles(guild_id, roles)
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
            .history_by_channel
            .get_mut(&channel_id)
            .and_then(|history| {
                let index = history
                    .messages
                    .binary_search_by_key(&message.id, |existing| existing.id)
                    .ok()?;
                history.messages.get_mut(index)
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

    /// Merges new messages (gateway events, our own sends), deduplicated by id and kept sorted.
    /// The oldest go beyond `MAXIMUM_CACHED_MESSAGES_PER_CHANNEL`, unless pages loaded by
    /// scrolling up made the channel longer: those stay until `trim_history`.
    pub fn insert_messages(
        &mut self,
        channel_id: Snowflake,
        incoming: Vec<Message>,
    ) -> StateChange {
        let history = self.history_by_channel.entry(channel_id).or_default();
        let mut merged = merge_messages(std::mem::take(&mut history.messages), incoming);
        let excess = merged
            .len()
            .saturating_sub(MAXIMUM_CACHED_MESSAGES_PER_CHANNEL);
        if excess > 0 && !history.is_extended_by_older_pages {
            merged.drain(..excess);
            history.has_reached_channel_start = false;
        }
        history.messages = merged;
        StateChange::Messages { channel_id }
    }

    /// Merges a page of history requested with `limit: HISTORY_PAGE_SIZE`, the latest one or one
    /// `before` the oldest cached message. Never trims: the reader is scrolling through it.
    pub fn insert_history_page(
        &mut self,
        channel_id: Snowflake,
        page: Vec<Message>,
    ) -> StateChange {
        let is_last_page = page.len() < usize::try_from(HISTORY_PAGE_SIZE).unwrap_or(usize::MAX);
        let history = self.history_by_channel.entry(channel_id).or_default();
        history.messages = merge_messages(std::mem::take(&mut history.messages), page);
        history.has_reached_channel_start |= is_last_page;
        history.is_extended_by_older_pages = true;
        StateChange::Messages { channel_id }
    }

    /// Keeps only the newest `MAXIMUM_CACHED_MESSAGES_PER_CHANNEL` messages of a channel the
    /// reader left, so pages loaded by scrolling up do not pile up across channels.
    pub fn trim_history(&mut self, channel_id: Snowflake) {
        let Some(history) = self.history_by_channel.get_mut(&channel_id) else {
            return;
        };
        let excess = history
            .messages
            .len()
            .saturating_sub(MAXIMUM_CACHED_MESSAGES_PER_CHANNEL);
        if excess > 0 {
            history.messages.drain(..excess);
            history.has_reached_channel_start = false;
        }
        history.is_extended_by_older_pages = false;
    }

    /// Adds a message to the outbox and returns the request that sends it. `nonce` must be unique
    /// for this user and channel, forever: the server deduplicates on it (`generate_message_nonce`).
    pub fn queue_message(
        &mut self,
        channel_id: Snowflake,
        content: String,
        reply_to_id: Option<Snowflake>,
        nonce: String,
    ) -> SendMessageRequest {
        let pending = PendingMessage {
            nonce,
            channel_id,
            content,
            reply_to_id,
            status: PendingMessageStatus::Sending,
        };
        let request = pending.send_request();
        self.pending_messages.push(pending);
        request
    }

    /// The server stored the message sent with `nonce`: it replaces its pending copy. `nonce` is
    /// the one we sent, not the server's echo, which an instance could forge.
    /// The message lands in the channel it was sent to, whatever channel the answer names.
    pub fn confirm_sent_message(&mut self, nonce: &str, message: Message) -> StateChange {
        let Some(index) = self
            .pending_messages
            .iter()
            .position(|pending| pending.nonce == nonce)
        else {
            return StateChange::Nothing;
        };
        let channel_id = self.pending_messages.remove(index).channel_id;
        self.insert_messages(channel_id, vec![message])
    }

    pub fn mark_send_failed(&mut self, nonce: &str) -> StateChange {
        match self.pending_message_mut(nonce) {
            Some(pending) => {
                pending.status = PendingMessageStatus::Failed;
                StateChange::Messages {
                    channel_id: pending.channel_id,
                }
            }
            None => StateChange::Nothing,
        }
    }

    /// Only a failed message is sent again, so a double click never sends twice at once. Returns
    /// the channel it was first sent to with the request: a retry never follows the selection.
    pub fn retry_pending_message(
        &mut self,
        nonce: &str,
    ) -> Option<(Snowflake, SendMessageRequest)> {
        let pending = self
            .pending_message_mut(nonce)
            .filter(|pending| pending.status == PendingMessageStatus::Failed)?;
        pending.status = PendingMessageStatus::Sending;
        Some((pending.channel_id, pending.send_request()))
    }

    /// After a reconnect: marks every failed message as sending again and returns its channel,
    /// nonce and request, in send order, for the caller to send one after another.
    pub fn retry_failed_messages(&mut self) -> Vec<(Snowflake, String, SendMessageRequest)> {
        self.pending_messages
            .iter_mut()
            .filter(|pending| pending.status == PendingMessageStatus::Failed)
            .map(|pending| {
                pending.status = PendingMessageStatus::Sending;
                (
                    pending.channel_id,
                    pending.nonce.clone(),
                    pending.send_request(),
                )
            })
            .collect()
    }

    /// The instance refused to store it (`ClientError::refuses_message_for_good`): it leaves the
    /// outbox, and its text is returned so the user can fix it.
    pub fn remove_refused_message(&mut self, nonce: &str) -> Option<PendingMessage> {
        let index = self
            .pending_messages
            .iter()
            .position(|pending| pending.nonce == nonce)?;
        Some(self.pending_messages.remove(index))
    }

    /// Only a failed message is discarded: one still sending may already be stored. A failed one
    /// may be too, when only the answer was lost: discarding then hides it here, not on the server.
    pub fn discard_failed_message(&mut self, nonce: &str) -> StateChange {
        let Some(index) = self.pending_messages.iter().position(|pending| {
            pending.nonce == nonce && pending.status == PendingMessageStatus::Failed
        }) else {
            return StateChange::Nothing;
        };
        let channel_id = self.pending_messages.remove(index).channel_id;
        StateChange::Messages { channel_id }
    }

    fn pending_message_mut(&mut self, nonce: &str) -> Option<&mut PendingMessage> {
        self.pending_messages
            .iter_mut()
            .find(|pending| pending.nonce == nonce)
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

    /// A fresh snapshot also follows permission changes (ADR-0015): channels it no longer lists
    /// were hidden from us, so their cached messages and unsent text go too.
    fn replace_guild(&mut self, guild: Guild) -> StateChange {
        let Some(existing) = self
            .guilds
            .iter_mut()
            .find(|existing| existing.id == guild.id)
        else {
            self.guilds.push(guild);
            return StateChange::Guilds;
        };
        let hidden_channel_ids: Vec<Snowflake> = existing
            .channels
            .iter()
            .map(|channel| channel.id)
            .filter(|id| !guild.channels.iter().any(|channel| channel.id == *id))
            .collect();
        *existing = guild;
        for channel_id in &hidden_channel_ids {
            self.history_by_channel.remove(channel_id);
        }
        self.pending_messages
            .retain(|pending| !hidden_channel_ids.contains(&pending.channel_id));
        StateChange::Guilds
    }

    fn remove_guild(&mut self, id: Snowflake) -> StateChange {
        if let Some(guild) = self.guild(id) {
            let channel_ids: Vec<Snowflake> =
                guild.channels.iter().map(|channel| channel.id).collect();
            self.pending_messages
                .retain(|pending| !channel_ids.contains(&pending.channel_id));
            for channel_id in channel_ids {
                self.history_by_channel.remove(&channel_id);
            }
        }
        self.guilds.retain(|guild| guild.id != id);
        self.members_by_guild.remove(&id);
        StateChange::Guilds
    }

    /// A deleted role leaves its members here too: the server sends no `GUILD_MEMBER_UPDATE`.
    fn replace_roles(&mut self, guild_id: Snowflake, roles: Vec<Role>) -> StateChange {
        let current_user_id = self.current_user.as_ref().map(|user| user.id);
        if let Some(guild) = self.guilds.iter_mut().find(|guild| guild.id == guild_id) {
            guild.roles = roles;
            let is_known_role =
                |role_id: &Snowflake| guild.roles.iter().any(|role| role.id == *role_id);
            guild.my_role_ids.retain(is_known_role);
            if let Some(members) = self.members_by_guild.get_mut(&guild_id) {
                for member in members {
                    member.role_ids.retain(is_known_role);
                }
            }
            if let Some(current_user_id) = current_user_id {
                recompute_my_permissions(guild, current_user_id);
            }
        }
        StateChange::Guilds
    }

    fn update_member_roles(
        &mut self,
        guild_id: Snowflake,
        user_id: Snowflake,
        role_ids: Vec<Snowflake>,
    ) -> StateChange {
        if let Some(member) = self
            .members_by_guild
            .get_mut(&guild_id)
            .and_then(|members| members.iter_mut().find(|member| member.user.id == user_id))
        {
            member.role_ids.clone_from(&role_ids);
        }
        if self
            .current_user
            .as_ref()
            .is_some_and(|user| user.id == user_id)
            && let Some(guild) = self.guilds.iter_mut().find(|guild| guild.id == guild_id)
        {
            guild.my_role_ids = role_ids;
            recompute_my_permissions(guild, user_id);
            return StateChange::Guilds;
        }
        StateChange::Members { guild_id }
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

/// Sorted by id and deduplicated; on equal ids the incoming copy wins, so edits replace cached
/// versions.
fn merge_messages(cached: Vec<Message>, incoming: Vec<Message>) -> Vec<Message> {
    let mut merged: Vec<Message> = incoming.into_iter().chain(cached).collect();
    // One stable sort for the whole batch, whatever order a server sends: never O(n²) on the UI
    // thread. Stability keeps the incoming copy first among equal ids.
    merged.sort_by_key(|message| message.id);
    merged.dedup_by_key(|message| message.id);
    merged
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
            permission_overwrites: Vec::new(),
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
                my_role_ids: Vec::new(),
                roles: Vec::new(),
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
    fn history_pages_extend_the_open_channel_until_its_start() {
        let page_size = u64::from(HISTORY_PAGE_SIZE);
        let page = |newest: u64| {
            (newest + 1 - page_size..=newest)
                .map(|id| message(id, OTHER))
                .collect::<Vec<_>>()
        };
        let mut state = ready_state();
        assert!(state.has_older_history(GENERAL), "never loaded yet");

        // Full pages, newest first, until the channel holds more than the cap.
        let mut newest = 100_000;
        while state.messages(GENERAL).len() <= MAXIMUM_CACHED_MESSAGES_PER_CHANNEL {
            state.insert_history_page(GENERAL, page(newest));
            newest -= page_size;
            assert!(
                state.has_older_history(GENERAL),
                "a full page may not be the last"
            );
        }
        let loaded = state.messages(GENERAL).len();
        state.apply_event(GatewayEvent::MessageCreate(message(200_000, OTHER)), NOW);
        assert_eq!(
            state.messages(GENERAL).len(),
            loaded + 1,
            "a new message never drops the older pages being read"
        );

        state.insert_history_page(GENERAL, vec![message(1, OTHER), message(2, OTHER)]);
        assert!(
            !state.has_older_history(GENERAL),
            "a short page is the last"
        );
        assert_eq!(state.messages(GENERAL).first().unwrap().id.raw(), 1);

        state.trim_history(GENERAL);
        assert_eq!(
            state.messages(GENERAL).len(),
            MAXIMUM_CACHED_MESSAGES_PER_CHANNEL
        );
        assert_eq!(state.messages(GENERAL).last().unwrap().id.raw(), 200_000);
        assert!(
            state.has_older_history(GENERAL),
            "trimmed messages must be fetched again"
        );
    }

    #[test]
    fn an_open_channel_at_exactly_the_cap_keeps_its_older_pages() {
        let page_size = u64::from(HISTORY_PAGE_SIZE);
        let mut state = ready_state();
        let cap = MAXIMUM_CACHED_MESSAGES_PER_CHANNEL as u64;
        for newest in (page_size..=cap).rev().step_by(HISTORY_PAGE_SIZE as usize) {
            let page = (newest + 1 - page_size..=newest)
                .map(|id| message(id, OTHER))
                .collect();
            state.insert_history_page(GENERAL, page);
        }
        assert_eq!(
            state.messages(GENERAL).len(),
            MAXIMUM_CACHED_MESSAGES_PER_CHANNEL
        );
        state.apply_event(GatewayEvent::MessageCreate(message(cap + 1, OTHER)), NOW);
        assert_eq!(
            state.messages(GENERAL).len(),
            MAXIMUM_CACHED_MESSAGES_PER_CHANNEL + 1,
            "the oldest row the reader may be looking at stays"
        );
    }

    #[test]
    fn a_short_latest_page_holds_the_whole_channel() {
        let mut state = ready_state();
        state.insert_history_page(GENERAL, vec![message(10, OTHER)]);
        assert!(!state.has_older_history(GENERAL));
        state.apply_event(GatewayEvent::MessageCreate(message(20, OTHER)), NOW);
        assert!(
            !state.has_older_history(GENERAL),
            "new messages keep the start"
        );
        state.trim_history(GENERAL);
        assert!(!state.has_older_history(GENERAL), "nothing was trimmed");
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
                role_ids: Vec::new(),
                joined_at_unix_milliseconds: 0,
            }],
        );
        state.apply_event(
            GatewayEvent::GuildMemberAdd {
                guild_id: GUILD,
                member: Member {
                    user: user(OTHER),
                    role_ids: Vec::new(),
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
    fn role_events_update_roles_member_role_ids_and_my_permissions() {
        const CLUB: Snowflake = Snowflake::from_raw(300);
        const MODERATOR: Snowflake = Snowflake::from_raw(301);
        let role = |id, position, permissions| Role {
            id,
            name: "role".into(),
            color: 0,
            position,
            permissions,
        };
        let everyone = role(CLUB, 0, Permissions::DEFAULT_EVERYONE);
        let mut state = ready_state();
        state.apply_event(
            GatewayEvent::GuildCreate(Box::new(Guild {
                id: CLUB,
                name: "Other's club".into(),
                owner_id: OTHER,
                my_permissions: Permissions::DEFAULT_EVERYONE,
                my_role_ids: Vec::new(),
                roles: vec![everyone.clone()],
                channels: Vec::new(),
            })),
            NOW,
        );
        state.set_members(
            CLUB,
            vec![Member {
                user: user(ME),
                role_ids: Vec::new(),
                joined_at_unix_milliseconds: 0,
            }],
        );
        let roles_with_moderator = vec![
            everyone.clone(),
            role(MODERATOR, 1, Permissions::MANAGE_ROLES),
        ];
        state.apply_event(
            GatewayEvent::GuildRolesUpdate {
                guild_id: CLUB,
                roles: roles_with_moderator,
            },
            NOW,
        );
        assert_eq!(state.guild(CLUB).unwrap().roles.len(), 2);
        assert_eq!(
            state.guild(CLUB).unwrap().my_permissions,
            Permissions::DEFAULT_EVERYONE,
            "a role nobody gave me changes nothing"
        );

        assert_eq!(
            state.apply_event(
                GatewayEvent::GuildMemberUpdate {
                    guild_id: CLUB,
                    user_id: ME,
                    role_ids: vec![MODERATOR],
                },
                NOW,
            ),
            StateChange::Guilds
        );
        let club = state.guild(CLUB).unwrap();
        assert_eq!(club.my_role_ids, vec![MODERATOR]);
        assert!(club.my_permissions.contains(Permissions::MANAGE_ROLES));
        assert_eq!(state.members(CLUB)[0].role_ids, vec![MODERATOR]);

        state.apply_event(
            GatewayEvent::GuildRolesUpdate {
                guild_id: CLUB,
                roles: vec![everyone],
            },
            NOW,
        );
        let club = state.guild(CLUB).unwrap();
        assert_eq!(
            club.my_role_ids,
            Vec::<Snowflake>::new(),
            "a deleted role leaves its members"
        );
        assert_eq!(club.my_permissions, Permissions::DEFAULT_EVERYONE);
        assert_eq!(state.members(CLUB)[0].role_ids, Vec::<Snowflake>::new());
    }

    #[test]
    fn a_snapshot_without_a_channel_forgets_its_messages_and_unsent_text() {
        let mut state = ready_state();
        state.insert_messages(GENERAL, vec![message(20, OTHER)]);
        state.queue_message(GENERAL, "draft".into(), None, "nonce-1".into());
        let mut snapshot = state.guild(GUILD).unwrap().clone();
        snapshot.channels.retain(|channel| channel.id != GENERAL);
        state.apply_event(GatewayEvent::GuildCreate(Box::new(snapshot)), NOW);
        assert_eq!(state.messages(GENERAL).len(), 0);
        assert_eq!(state.pending_messages(GENERAL).count(), 0);
        assert_eq!(state.guild(GUILD).unwrap().channels.len(), 1);
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

    #[test]
    fn outbox_keeps_unsent_text_until_confirmed_retried_or_discarded() {
        let mut state = ready_state();
        let request = state.queue_message(GENERAL, "hi".into(), None, "n1".into());
        assert_eq!(request.nonce.as_deref(), Some("n1"));
        state.queue_message(GENERAL, "again".into(), None, "n2".into());
        // A sending message can neither be retried (double send) nor discarded (maybe stored).
        assert_eq!(state.retry_pending_message("n1"), None);
        assert_eq!(state.discard_failed_message("n1"), StateChange::Nothing);

        state.mark_send_failed("n1");
        // A reconnect keeps the outbox.
        state.apply_ready(Ready {
            session_id: "s2".into(),
            user: state.current_user().unwrap().clone(),
            guilds: state.guilds().to_vec(),
            read_states: Vec::new(),
            online_user_ids: Vec::new(),
        });
        let statuses: Vec<(&str, PendingMessageStatus)> = state
            .pending_messages(GENERAL)
            .map(|pending| (pending.nonce.as_str(), pending.status))
            .collect();
        assert_eq!(
            statuses,
            [
                ("n1", PendingMessageStatus::Failed),
                ("n2", PendingMessageStatus::Sending)
            ]
        );

        let (channel_id, retried) = state.retry_pending_message("n1").unwrap();
        assert_eq!(channel_id, GENERAL);
        assert_eq!(
            (retried.content.as_str(), retried.nonce.as_deref()),
            ("hi", Some("n1"))
        );
        assert_eq!(state.retry_pending_message("n1"), None);

        // The answer lands in the channel we sent to, even if it names another one.
        let mut answer = message(50, ME);
        answer.channel_id = Snowflake::from_raw(999);
        state.confirm_sent_message("n1", answer);
        assert_eq!(state.messages(GENERAL).last().unwrap().id.raw(), 50);
        assert_eq!(state.pending_messages(GENERAL).count(), 1);
        assert_eq!(
            state.confirm_sent_message("unknown", message(51, ME)),
            StateChange::Nothing
        );

        state.mark_send_failed("n2");
        state.discard_failed_message("n2");
        assert_eq!(state.pending_messages(GENERAL).count(), 0);
    }

    #[test]
    fn a_reconnect_resends_only_failed_messages_in_send_order() {
        let mut state = ready_state();
        for nonce in ["n1", "n2", "n3"] {
            state.queue_message(GENERAL, nonce.into(), None, nonce.into());
        }
        state.mark_send_failed("n3");
        state.mark_send_failed("n1");
        let resent: Vec<(Snowflake, String, Option<String>)> = state
            .retry_failed_messages()
            .into_iter()
            .map(|(channel_id, nonce, request)| (channel_id, nonce, request.nonce))
            .collect();
        assert_eq!(
            resent,
            [
                (GENERAL, "n1".to_owned(), Some("n1".to_owned())),
                (GENERAL, "n3".to_owned(), Some("n3".to_owned()))
            ]
        );
        assert!(
            state
                .pending_messages(GENERAL)
                .all(|pending| pending.status == PendingMessageStatus::Sending)
        );
        // Already sending again: a second reconnect does not send them twice at once.
        assert_eq!(state.retry_failed_messages(), []);
    }

    #[test]
    fn outbox_forgets_messages_of_deleted_channels_and_guilds() {
        let mut state = ready_state();
        state.queue_message(GENERAL, "a".into(), None, "n1".into());
        state.apply_event(
            GatewayEvent::ChannelDelete {
                guild_id: GUILD,
                id: GENERAL,
            },
            NOW,
        );
        assert_eq!(state.pending_messages(GENERAL).count(), 0);

        let mut state = ready_state();
        state.queue_message(GENERAL, "a".into(), None, "n1".into());
        state.apply_event(GatewayEvent::GuildDelete { id: GUILD }, NOW);
        assert_eq!(state.pending_messages(GENERAL).count(), 0);

        // Left or deleted while disconnected: the next `Ready` no longer lists the channel.
        let mut state = ready_state();
        state.queue_message(GENERAL, "a".into(), None, "n1".into());
        state.apply_ready(Ready {
            session_id: "s2".into(),
            user: state.current_user().unwrap().clone(),
            guilds: Vec::new(),
            read_states: Vec::new(),
            online_user_ids: Vec::new(),
        });
        assert_eq!(state.pending_messages(GENERAL).count(), 0);
    }

    #[test]
    fn a_refused_message_leaves_the_outbox_with_its_text() {
        let mut state = ready_state();
        state.queue_message(GENERAL, "too long".into(), None, "n1".into());
        let refused = state.remove_refused_message("n1").unwrap();
        assert_eq!(refused.content, "too long");
        assert_eq!(state.pending_messages(GENERAL).count(), 0);
        assert_eq!(state.remove_refused_message("n1"), None);
    }
}
