//! What the client knows, updated by `Ready`, gateway events and REST answers. Pure: no I/O,
//! so every UI behaves the same and the logic is tested without a server.

use std::collections::{BTreeMap, HashMap, HashSet};

use mellow_protocol::{
    Channel, CurrentUser, DirectMessageChannel, GatewayEvent, Guild, GuildSummary, Member, Message,
    Permissions, Presence, PresenceStatus, PublicUser, Ready, Role, SendMessageRequest, Snowflake,
    compute_channel_permissions, compute_guild_permissions,
};

/// Messages kept in memory per channel once the reader leaves it; older ones are fetched again
/// when scrolled to.
pub const MAXIMUM_CACHED_MESSAGES_PER_CHANNEL: usize = 500;
/// Messages asked per history request; a shorter page means the channel's start is reached.
pub const HISTORY_PAGE_SIZE: u32 = 50;
/// A typing indicator disappears after this long without a new `TYPING_START`.
pub const TYPING_INDICATOR_LIFETIME_MILLISECONDS: u64 = 8_000;

/// What a participant may do in a direct-message conversation, as the server's
/// `direct_message_permissions` decides; only to adapt the interface (ADR-0014).
const DIRECT_MESSAGE_PERMISSIONS: Permissions = Permissions::VIEW_CHANNEL
    .union(Permissions::SEND_MESSAGES)
    .union(Permissions::READ_MESSAGE_HISTORY)
    .union(Permissions::ATTACH_FILES);

/// Most recent activity first, as in Discord's conversation list.
fn sort_by_last_activity(channels: &mut [DirectMessageChannel]) {
    channels
        .sort_by_key(|channel| std::cmp::Reverse(channel.last_message_id.unwrap_or(channel.id)));
}

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
    /// The conversation list: a new conversation or a new last message reorders it.
    DirectMessages,
    Messages {
        channel_id: Snowflake,
    },
    Members {
        guild_id: Snowflake,
    },
    Presence,
    /// A user's username or display name changed: members, message authors and, for the signed-in
    /// user, `current_user`.
    UserRenamed {
        user_id: Snowflake,
    },
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
            attachment_ids: Vec::new(),
            forwarded_from: None,
            poll: None,
            checklist: None,
            progress_bar: None,
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
    /// Most recent activity first.
    direct_message_channels: Vec<DirectMessageChannel>,
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

    /// Most recent activity first.
    #[must_use]
    pub fn direct_message_channels(&self) -> &[DirectMessageChannel] {
        &self.direct_message_channels
    }

    #[must_use]
    pub fn direct_message_channel(&self, channel_id: Snowflake) -> Option<&DirectMessageChannel> {
        self.direct_message_channels
            .iter()
            .find(|channel| channel.id == channel_id)
    }

    /// What the signed-in user may do in a channel, only to adapt the interface (ADR-0014);
    /// `None` for an unknown channel or before `Ready`.
    #[must_use]
    pub fn my_channel_permissions(&self, channel_id: Snowflake) -> Option<Permissions> {
        let current_user_id = self.current_user.as_ref()?.id;
        if self.direct_message_channel(channel_id).is_some() {
            return Some(DIRECT_MESSAGE_PERMISSIONS);
        }
        self.guilds.iter().find_map(|guild| {
            let channel = guild
                .channels
                .iter()
                .find(|channel| channel.id == channel_id)?;
            Some(compute_channel_permissions(
                guild.my_permissions,
                guild.id,
                current_user_id,
                &guild.my_role_ids,
                &channel.permission_overwrites,
            ))
        })
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
    /// A conversation also knows its last message without any cached (`Ready`, offline messages).
    pub fn has_unread(&self, channel_id: Snowflake) -> bool {
        let newest_cached = self.messages(channel_id).last().map(|message| message.id);
        let newest_known = self
            .direct_message_channel(channel_id)
            .and_then(|channel| channel.last_message_id);
        let newest = newest_cached.max(newest_known);
        match (newest, self.last_read_by_channel.get(&channel_id)) {
            (Some(newest), Some(last_read)) => newest > *last_read,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// The newest message the signed-in user has read in `channel_id`, if any.
    #[must_use]
    pub fn last_read_message_id(&self, channel_id: Snowflake) -> Option<Snowflake> {
        self.last_read_by_channel.get(&channel_id).copied()
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
        let mut direct_message_channels = ready.direct_message_channels;
        // `Ready` lists only conversations with a message: one opened here whose first message
        // is still unsent must stay, or its text would be lost.
        let previous_direct_message_channels = std::mem::take(&mut self.direct_message_channels);
        for previous in previous_direct_message_channels {
            let has_pending_message = pending_messages
                .iter()
                .any(|pending| pending.channel_id == previous.id);
            if has_pending_message
                && !direct_message_channels
                    .iter()
                    .any(|channel| channel.id == previous.id)
            {
                direct_message_channels.push(previous);
            }
        }
        sort_by_last_activity(&mut direct_message_channels);
        *self = Self {
            current_user: Some(ready.user),
            guilds: ready.guilds,
            direct_message_channels,
            last_read_by_channel: ready
                .read_states
                .into_iter()
                .map(|state| (state.channel_id, state.last_read_message_id))
                .collect(),
            online_user_ids: ready.online_user_ids.into_iter().collect(),
            ..Self::default()
        };
        // Channels deleted while we were away never send `CHANNEL_DELETE`.
        pending_messages.retain(|pending| self.is_known_channel(pending.channel_id));
        self.pending_messages = pending_messages;
        StateChange::Guilds
    }

    fn forget_cached_message(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> StateChange {
        if let Some(history) = self.history_by_channel.get_mut(&channel_id) {
            history.messages.retain(|message| message.id != message_id);
        }
        StateChange::Messages { channel_id }
    }

    pub fn apply_event(&mut self, event: GatewayEvent, now_unix_milliseconds: u64) -> StateChange {
        // An event racing the snapshot that hid its channel must not bring it back (ADR-0015).
        let channel_id = match &event {
            GatewayEvent::MessageCreate(message) | GatewayEvent::MessageUpdate(message) => {
                Some(message.channel_id)
            }
            GatewayEvent::MessageDelete { channel_id, .. }
            | GatewayEvent::TypingStart { channel_id, .. } => Some(*channel_id),
            GatewayEvent::ChannelUpdate(channel) => Some(channel.id),
            _ => None,
        };
        if channel_id.is_some_and(|channel_id| !self.is_known_channel(channel_id)) {
            return StateChange::Nothing;
        }
        self.apply_event_of_a_known_channel(event, now_unix_milliseconds)
    }

    fn apply_event_of_a_known_channel(
        &mut self,
        event: GatewayEvent,
        now_unix_milliseconds: u64,
    ) -> StateChange {
        match event {
            GatewayEvent::MessageCreate(message) => {
                let channel_id = message.channel_id;
                self.typing_until.remove(&(channel_id, message.author.id));
                self.note_direct_message_activity(channel_id, message.id);
                self.insert_messages(channel_id, vec![message])
            }
            GatewayEvent::MessageUpdate(message) => self.replace_cached_message(message),
            GatewayEvent::MessageDelete { channel_id, id, .. } => {
                self.forget_cached_message(channel_id, id)
            }
            GatewayEvent::ChannelCreate(channel) | GatewayEvent::ChannelUpdate(channel) => {
                self.upsert_channel(channel);
                StateChange::Guilds
            }
            GatewayEvent::ChannelDelete { guild_id, id } => self.remove_channel(guild_id, id),
            GatewayEvent::GuildCreate(guild) => self.replace_guild(*guild),
            GatewayEvent::DirectMessageChannelCreate(channel) => {
                self.upsert_direct_message_channel(channel)
            }
            GatewayEvent::GuildDelete { id } => self.remove_guild(id),
            GatewayEvent::GuildUpdate(summary) => self.update_guild_summary(summary),
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
            } => self.note_typing(channel_id, user_id, now_unix_milliseconds),
            // shortcut: idle and do not disturb show as online; show them once the desktop does.
            GatewayEvent::PresenceUpdate(Presence {
                user_id, status, ..
            }) => {
                match status {
                    PresenceStatus::Online
                    | PresenceStatus::Idle
                    | PresenceStatus::DoNotDisturb => self.online_user_ids.insert(user_id),
                    PresenceStatus::Offline => self.online_user_ids.remove(&user_id),
                };
                StateChange::Presence
            }
            GatewayEvent::UserUpdate(user) => self.rename_user(&user),
            // shortcut: friends, groups, reactions, link previews, polls, checklists, progress bars, custom emoji and voice are web-only (ADR-0023) until the desktop port.
            GatewayEvent::RelationshipAdd(_)
            | GatewayEvent::RelationshipRemove { .. }
            | GatewayEvent::DirectMessageChannelUpdate(_)
            | GatewayEvent::DirectMessageChannelDelete { .. }
            | GatewayEvent::MessageReactionAdd(_)
            | GatewayEvent::MessageReactionRemove(_)
            | GatewayEvent::MessageReactionRemoveAll { .. }
            | GatewayEvent::MessageReactionRemoveEmoji { .. }
            | GatewayEvent::MessageLinkPreviewsUpdate { .. }
            | GatewayEvent::MessagePollVoteAdd(_)
            | GatewayEvent::MessagePollVoteRemove(_)
            | GatewayEvent::MessageChecklistUpdate { .. }
            | GatewayEvent::MessageProgressBarUpdate { .. }
            | GatewayEvent::GuildEmojisUpdate { .. }
            | GatewayEvent::GuildSoundsUpdate { .. }
            | GatewayEvent::MyJoinSoundUpdate { .. }
            | GatewayEvent::VoiceSoundPlayed { .. }
            | GatewayEvent::VoiceStateUpdate { .. }
            | GatewayEvent::VoiceStateDelete { .. }
            | GatewayEvent::VoiceSessionDescription { .. }
            | GatewayEvent::VoiceSlotAssigned { .. }
            | GatewayEvent::VoiceJoinRefused { .. }
            | GatewayEvent::VoiceVideoRefused { .. }
            | GatewayEvent::VoiceVideoLayers { .. }
            | GatewayEvent::VoiceVideoPaused { .. }
            | GatewayEvent::VoiceDisconnected { .. }
            | GatewayEvent::Speaking { .. } => StateChange::Nothing,
        }
    }

    fn note_typing(
        &mut self,
        channel_id: Snowflake,
        user_id: Snowflake,
        now_unix_milliseconds: u64,
    ) -> StateChange {
        // Expired entries are dropped here, so the map never outgrows who types now.
        self.typing_until
            .retain(|_, until| *until > now_unix_milliseconds);
        self.typing_until.insert(
            (channel_id, user_id),
            now_unix_milliseconds.saturating_add(TYPING_INDICATOR_LIFETIME_MILLISECONDS),
        );
        StateChange::Typing { channel_id }
    }

    fn update_guild_summary(&mut self, summary: GuildSummary) -> StateChange {
        match self.guilds.iter_mut().find(|guild| guild.id == summary.id) {
            Some(guild) => {
                guild.name = summary.name;
                guild.icon = summary.icon;
                StateChange::Guilds
            }
            None => StateChange::Nothing,
        }
    }

    fn rename_user(&mut self, user: &PublicUser) -> StateChange {
        if let Some(current_user) = self
            .current_user
            .as_mut()
            .filter(|current_user| current_user.id == user.id)
        {
            current_user.username.clone_from(&user.username);
            current_user.display_name.clone_from(&user.display_name);
            current_user.avatar = user.avatar;
            current_user.banner = user.banner;
        }
        for members in self.members_by_guild.values_mut() {
            if let Some(member) = members.iter_mut().find(|member| member.user.id == user.id) {
                member.user.clone_from(user);
                members.sort_by(|left, right| left.user.username.cmp(&right.user.username));
            }
        }
        for history in self.history_by_channel.values_mut() {
            for message in &mut history.messages {
                if message.author.id == user.id {
                    message.author.clone_from(user);
                }
            }
        }
        for channel in &mut self.direct_message_channels {
            for recipient in &mut channel.recipients {
                if recipient.id == user.id {
                    recipient.clone_from(user);
                }
            }
        }
        StateChange::UserRenamed { user_id: user.id }
    }

    fn remove_channel(&mut self, guild_id: Snowflake, id: Snowflake) -> StateChange {
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
        // Events never carry reactions (`me` differs per viewer): the cached ones stay.
        let reactions = std::mem::take(&mut cached.reactions);
        *cached = Message {
            reactions,
            ..message
        };
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

    fn is_known_channel(&self, channel_id: Snowflake) -> bool {
        self.direct_message_channel(channel_id).is_some()
            || self.guilds.iter().any(|guild| {
                guild
                    .channels
                    .iter()
                    .any(|channel| channel.id == channel_id)
            })
    }

    /// Also applies the REST answer of opening a conversation. A conversation already known keeps
    /// its newer last message.
    pub fn upsert_direct_message_channel(&mut self, channel: DirectMessageChannel) -> StateChange {
        match self
            .direct_message_channels
            .iter_mut()
            .find(|existing| existing.id == channel.id)
        {
            Some(existing) => {
                existing.recipients = channel.recipients;
                existing.last_message_id = existing.last_message_id.max(channel.last_message_id);
            }
            None => self.direct_message_channels.push(channel),
        }
        sort_by_last_activity(&mut self.direct_message_channels);
        StateChange::DirectMessages
    }

    fn note_direct_message_activity(&mut self, channel_id: Snowflake, message_id: Snowflake) {
        if let Some(channel) = self
            .direct_message_channels
            .iter_mut()
            .find(|channel| channel.id == channel_id)
        {
            channel.last_message_id = channel.last_message_id.max(Some(message_id));
            sort_by_last_activity(&mut self.direct_message_channels);
        }
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
        self.typing_until
            .retain(|(channel_id, _), _| !hidden_channel_ids.contains(channel_id));
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
mod tests;
