//! What the client knows, updated by `Ready`, gateway events and REST answers. Pure: no I/O and
//! no clock (time is passed in), so every UI behaves the same and the logic is tested without a
//! server. The web client's `client.svelte.ts` is the reference model.

use std::collections::{BTreeMap, HashMap, HashSet};

use mellow_protocol::{
    CurrentUser, DirectMessageChannel, GatewayEvent, Guild, Member, Permissions, Presence, Ready,
    Relationship, Snowflake, VoiceState, compute_channel_permissions,
};

mod checklists;
mod direct_messages;
mod guilds;
mod mentions;
mod messages;
mod optimistic;
mod outbox;
mod pins;
mod polls;
mod presence;
mod reactions;
mod unreads;
mod voice;

pub use checklists::{
    ChecklistProgress, Revisioned, checklist_progress, keep_newest, progress_percent, set_item_tick,
};
pub use mentions::{MentionReader, does_message_mention_reader};
pub use optimistic::{ChangeOutcome, ChecklistTickChange, PollVoteChanges, ReactionToggle};
pub use outbox::{PendingMessage, PendingMessageStatus};
pub use pins::{PinsAnswer, PinsStatus};
pub use polls::{PollVoteChange, count_poll_vote, is_poll_ended, own_vote_changes_for_choice};
pub use reactions::{add_reaction, reaction_key, remove_emoji, remove_reaction, take_emoji_from};
pub use unreads::UnreadMarker;
pub use voice::{MINIMUM_SOUND_INDICATOR_MILLISECONDS, SoundIndicator};

use messages::ChannelHistory;
use optimistic::InFlightChanges;
use pins::Pins;
use unreads::ReadPositions;

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

/// What an update touched, so a UI refreshes only that part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateChange {
    /// Guild list, a guild's channels or roles, the signed-in user's permissions, or the user.
    Guilds,
    /// A guild's custom emoji (ADR-0028).
    GuildEmojis {
        guild_id: Snowflake,
    },
    /// A guild's soundboard sounds, or the user's join sound there (ADR-0032).
    GuildSounds {
        guild_id: Snowflake,
    },
    /// The conversation list: a new conversation, a new last message, a rename or new members.
    DirectMessages,
    /// The user can no longer read this conversation (left, or removed from the group): a UI
    /// showing it closes it.
    ConversationRemoved {
        channel_id: Snowflake,
    },
    /// A channel's cached messages, or its unread or mention badge.
    Messages {
        channel_id: Snowflake,
    },
    /// One message changed wherever it shows, history and pins alike: an edit, a pin, its
    /// reactions, poll, checklist, progress bar or link previews.
    Message {
        channel_id: Snowflake,
        message_id: Snowflake,
    },
    /// A channel's pinned messages: loaded, failed, or a message pinned or unpinned.
    Pins {
        channel_id: Snowflake,
    },
    Members {
        guild_id: Snowflake,
    },
    /// A user's status or custom status; for the signed-in user, also `current_user`.
    Presence {
        user_id: Snowflake,
    },
    /// Friends, requests and blocks (ADR-0025).
    Relationships,
    /// A user's name or pictures changed: members, message authors, conversations,
    /// relationships and, for the signed-in user, `current_user`.
    UserRenamed {
        user_id: Snowflake,
    },
    Typing {
        channel_id: Snowflake,
    },
    /// The signed-in user's read positions, unread markers and mention badges.
    ReadStates,
    /// Someone joined, moved within, changed their voice state in, or left voice channels of
    /// `guild_id`. `previous_channel_id` and `channel_id` say where they were and are (`None`:
    /// not connected), so a UI can announce a join or a leave in its own channel.
    VoiceParticipant {
        guild_id: Snowflake,
        user_id: Snowflake,
        previous_channel_id: Option<Snowflake>,
        channel_id: Option<Snowflake>,
    },
    /// Someone started or stopped speaking.
    Speaking {
        channel_id: Snowflake,
        user_id: Snowflake,
    },
    /// A soundboard sound was played in the voice channel the user is connected to (ADR-0032):
    /// the app plays it unless deafened or the player is muted for the user.
    SoundPlayed {
        channel_id: Snowflake,
        user_id: Snowflake,
        sound_id: Snowflake,
    },
    /// A sound's emoji now shows by `user_id`'s name (`ClientState::sound_indicator`).
    SoundIndicator {
        user_id: Snowflake,
    },
    /// Nothing visible changed. The voice session's own events (`VOICE_SESSION_DESCRIPTION`,
    /// `VOICE_JOIN_REFUSED`, `VOICE_SLOT_ASSIGNED`, `VOICE_VIDEO_*`, `VOICE_DISCONNECTED`) are
    /// not state: the app hands them to its voice session.
    Nothing,
}

/// What `Ready` says about the instance, beyond the user's data.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct InstanceCapabilities {
    may_create_guilds: bool,
    is_voice_enabled: bool,
    maximum_attachment_bytes: Option<u64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ClientState {
    current_user: Option<CurrentUser>,
    capabilities: InstanceCapabilities,
    /// In server-rail order: `Ready` order, then joined guilds appended.
    guilds: Vec<Guild>,
    /// Most recent activity first.
    direct_message_channels: Vec<DirectMessageChannel>,
    /// At most `MAXIMUM_CACHED_MESSAGES_PER_CHANNEL` messages, except in the channel being read
    /// (`trim_history`).
    history_by_channel: HashMap<Snowflake, ChannelHistory>,
    /// Bumped when the cache is emptied (`Ready`): history answers to older requests are stale.
    history_generation: u64,
    /// Messages deleted this session: a history page or a send answer that left before the
    /// deletion must not bring them back. Ids are never reused, so they stay until sign-out; one
    /// id per deletion seen, a few bytes each.
    deleted_message_ids: HashSet<Snowflake>,
    members_by_guild: HashMap<Snowflake, Vec<Member>>,
    reads: ReadPositions,
    /// What others see of each user online now; absent users are offline.
    presence_by_user_id: HashMap<Snowflake, Presence>,
    /// Friends, requests and blocks, by the other person's id (ADR-0025).
    relationship_by_user_id: HashMap<Snowflake, Relationship>,
    /// (channel, user) → when the indicator expires.
    typing_until: BTreeMap<(Snowflake, Snowflake), u64>,
    /// In send order. Not cleared by `Ready`: a reconnect must not lose unsent text.
    pending_messages: Vec<PendingMessage>,
    /// Loaded when a pins panel opens; kept up to date by events.
    pins_by_channel: HashMap<Snowflake, Pins>,
    /// Channels whose pins changed while their list was loading: asked again once it arrives.
    pins_changed_while_loading: HashSet<Snowflake>,
    /// The user's own reaction clicks, poll votes and checklist ticks still on their way.
    in_flight: InFlightChanges,
    /// Who is in which voice channel, in join order (one connection each, ADR-0030).
    voice_states: Vec<VoiceState>,
    /// People whose voice the instance hears now (`SPEAKING`).
    speaking_user_ids: HashSet<Snowflake>,
    /// The sound each person played last, while its emoji shows by their name.
    sound_indicator_by_user_id: HashMap<Snowflake, SoundIndicator>,
    /// The user's soundboard waits until then between two sounds; kept across `Ready`, as the
    /// instance keeps its own.
    sound_cooldown_ends_at_unix_milliseconds: u64,
}

impl ClientState {
    #[must_use]
    pub fn current_user(&self) -> Option<&CurrentUser> {
        self.current_user.as_ref()
    }

    fn current_user_id(&self) -> Option<Snowflake> {
        self.current_user.as_ref().map(|user| user.id)
    }

    /// Whether this user may create servers (`Ready.may_create_guilds`).
    #[must_use]
    pub fn may_create_guilds(&self) -> bool {
        self.capabilities.may_create_guilds
    }

    /// Whether voice works on this instance (`Ready.voice_enabled`): hide voice actions otherwise.
    #[must_use]
    pub fn is_voice_enabled(&self) -> bool {
        self.capabilities.is_voice_enabled
    }

    /// The largest attachment the instance accepts, so a larger file is refused before its
    /// upload; `None` from older servers, whose upload answer tells.
    #[must_use]
    pub fn maximum_attachment_bytes(&self) -> Option<u64> {
        self.capabilities.maximum_attachment_bytes
    }

    /// Changes with every `Ready`: a history or member answer requested under another generation
    /// is stale and must be dropped.
    #[must_use]
    pub fn history_generation(&self) -> u64 {
        self.history_generation
    }

    #[must_use]
    pub fn guilds(&self) -> &[Guild] {
        &self.guilds
    }

    #[must_use]
    pub fn guild(&self, guild_id: Snowflake) -> Option<&Guild> {
        self.guilds.iter().find(|guild| guild.id == guild_id)
    }

    /// The guild a channel belongs to; `None` for a conversation or an unknown channel.
    #[must_use]
    pub fn guild_of_channel(&self, channel_id: Snowflake) -> Option<&Guild> {
        self.guilds.iter().find(|guild| {
            guild
                .channels
                .iter()
                .any(|channel| channel.id == channel_id)
        })
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
        let current_user_id = self.current_user_id()?;
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
    pub fn members(&self, guild_id: Snowflake) -> &[Member] {
        self.members_by_guild
            .get(&guild_id)
            .map_or(&[], Vec::as_slice)
    }

    /// Users typing in `channel_id` at `now_unix_milliseconds`, excluding the signed-in user.
    #[must_use]
    pub fn typing_user_ids(
        &self,
        channel_id: Snowflake,
        now_unix_milliseconds: u64,
    ) -> Vec<Snowflake> {
        let me = self.current_user_id();
        self.typing_until
            .range(
                (channel_id, Snowflake::from_raw(0))..=(channel_id, Snowflake::from_raw(u64::MAX)),
            )
            .filter(|((_, user_id), until)| **until > now_unix_milliseconds && Some(*user_id) != me)
            .map(|((_, user_id), _)| *user_id)
            .collect()
    }

    /// Replaces everything with a fresh `Ready` (identify after a lost session). What the user did
    /// that the instance may not know yet survives: unsent messages, unsent read positions,
    /// deleted ids, changes in flight and the soundboard cooldown.
    pub fn apply_ready(&mut self, ready: Ready) -> StateChange {
        let previous = std::mem::take(self);
        let mut pending_messages = previous.pending_messages;
        let mut direct_message_channels = ready.direct_message_channels;
        // `Ready` lists only conversations with a message: one opened here whose first message
        // is still unsent must stay, or its text would be lost.
        for conversation in previous.direct_message_channels {
            let has_pending_message = pending_messages
                .iter()
                .any(|pending| pending.channel_id == conversation.id);
            if has_pending_message
                && !direct_message_channels
                    .iter()
                    .any(|channel| channel.id == conversation.id)
            {
                direct_message_channels.push(conversation);
            }
        }
        direct_messages::sort_by_last_activity(&mut direct_message_channels);
        // Older servers send no presences: their online users are all online.
        let presences = if ready.presences.is_empty() {
            ready
                .online_user_ids
                .iter()
                .map(|user_id| Presence {
                    user_id: *user_id,
                    status: mellow_protocol::PresenceStatus::Online,
                    custom_status: None,
                })
                .collect()
        } else {
            ready.presences
        };
        *self = Self {
            current_user: Some(ready.user),
            capabilities: InstanceCapabilities {
                may_create_guilds: ready.may_create_guilds,
                is_voice_enabled: ready.voice_enabled,
                maximum_attachment_bytes: ready.maximum_attachment_bytes,
            },
            guilds: ready.guilds,
            direct_message_channels,
            history_generation: previous.history_generation.wrapping_add(1),
            deleted_message_ids: previous.deleted_message_ids,
            presence_by_user_id: presences
                .into_iter()
                .map(|presence| (presence.user_id, presence))
                .collect(),
            relationship_by_user_id: ready
                .relationships
                .into_iter()
                .map(|relationship| (relationship.user.id, relationship))
                .collect(),
            in_flight: previous.in_flight,
            sound_cooldown_ends_at_unix_milliseconds: previous
                .sound_cooldown_ends_at_unix_milliseconds,
            ..Self::default()
        };
        // Channels deleted while we were away never send `CHANNEL_DELETE`.
        pending_messages.retain(|pending| self.is_known_channel(pending.channel_id));
        self.pending_messages = pending_messages;
        self.reads =
            previous
                .reads
                .after_ready(&ready.read_states, &ready.channel_unreads, |channel_id| {
                    self.is_known_channel(channel_id)
                });
        self.voice_states = ready
            .voice_states
            .into_iter()
            .filter(|state| self.is_known_channel(state.channel_id))
            .collect();
        StateChange::Guilds
    }

    pub fn apply_event(&mut self, event: GatewayEvent, now_unix_milliseconds: u64) -> StateChange {
        match &event {
            // The user's own sessions get the send's nonce: its pending twin goes, even when the
            // send's answer is lost or comes later, and even in a channel no longer known.
            GatewayEvent::MessageCreate(message) => self.take_echoed_pending_message(message),
            // A late page must not bring it back, wherever it was.
            GatewayEvent::MessageDelete { channel_id, id, .. } => {
                self.deleted_message_ids.insert(*id);
                self.drop_pin(*channel_id, *id);
            }
            _ => {}
        }
        // An event racing the snapshot that hid its channel must not bring it back (ADR-0015).
        if event_channel_id(&event).is_some_and(|channel_id| !self.is_known_channel(channel_id)) {
            return StateChange::Nothing;
        }
        self.apply_event_of_a_known_channel(event, now_unix_milliseconds)
    }

    #[expect(clippy::too_many_lines, reason = "one short arm per gateway event")]
    fn apply_event_of_a_known_channel(
        &mut self,
        event: GatewayEvent,
        now_unix_milliseconds: u64,
    ) -> StateChange {
        match event {
            GatewayEvent::MessageCreate(message) => self.take_in_new_message(message),
            GatewayEvent::MessageUpdate(message) => self.apply_message_update(message),
            GatewayEvent::MessageDelete { channel_id, id, .. } => {
                self.forget_deleted_message(channel_id, id)
            }
            GatewayEvent::MessageLinkPreviewsUpdate {
                channel_id,
                message_id,
                link_previews,
                ..
            } => self.update_cached_message(channel_id, message_id, |message| {
                let is_changed = message.link_previews != link_previews;
                message.link_previews.clone_from(&link_previews);
                is_changed
            }),
            GatewayEvent::MessageReactionAdd(reaction) => {
                let is_mine = Some(reaction.user_id) == self.current_user_id();
                self.update_cached_message(reaction.channel_id, reaction.message_id, |message| {
                    add_reaction(&mut message.reactions, &reaction.emoji, is_mine)
                })
            }
            GatewayEvent::MessageReactionRemove(reaction) => {
                let is_mine = Some(reaction.user_id) == self.current_user_id();
                self.update_cached_message(reaction.channel_id, reaction.message_id, |message| {
                    remove_reaction(&mut message.reactions, &reaction.emoji, is_mine)
                })
            }
            GatewayEvent::MessageReactionRemoveEmoji {
                channel_id,
                message_id,
                emoji,
                ..
            } => self.update_cached_message(channel_id, message_id, |message| {
                remove_emoji(&mut message.reactions, &emoji)
            }),
            GatewayEvent::MessageReactionRemoveAll {
                channel_id,
                message_id,
                ..
            } => self.remove_all_reactions(channel_id, message_id),
            GatewayEvent::MessagePollVoteAdd(vote) => self.count_poll_vote_event(vote, true),
            GatewayEvent::MessagePollVoteRemove(vote) => self.count_poll_vote_event(vote, false),
            GatewayEvent::MessageChecklistUpdate {
                channel_id,
                message_id,
                checklist,
                ..
            } => self.apply_checklist(channel_id, message_id, &checklist),
            GatewayEvent::MessageProgressBarUpdate {
                channel_id,
                message_id,
                progress_bar,
                ..
            } => self.apply_progress_bar(channel_id, message_id, &progress_bar),
            GatewayEvent::ChannelCreate(channel) | GatewayEvent::ChannelUpdate(channel) => {
                self.upsert_channel(channel);
                StateChange::Guilds
            }
            GatewayEvent::ChannelDelete { guild_id, id } => self.remove_channel(guild_id, id),
            GatewayEvent::GuildCreate(guild) => self.replace_guild(*guild),
            GatewayEvent::GuildUpdate(summary) => self.update_guild_summary(summary),
            GatewayEvent::GuildDelete { id } => self.remove_guild(id),
            GatewayEvent::GuildMemberAdd { guild_id, member } => self.add_member(guild_id, member),
            GatewayEvent::GuildMemberRemove { guild_id, user_id } => {
                self.remove_member(guild_id, user_id)
            }
            GatewayEvent::GuildMemberUpdate {
                guild_id,
                user_id,
                role_ids,
            } => self.update_member_roles(guild_id, user_id, role_ids),
            GatewayEvent::GuildRolesUpdate { guild_id, roles } => {
                self.replace_roles(guild_id, roles)
            }
            GatewayEvent::GuildEmojisUpdate { guild_id, emojis } => {
                self.replace_emojis(guild_id, emojis)
            }
            GatewayEvent::GuildSoundsUpdate { guild_id, sounds } => {
                self.replace_sounds(guild_id, sounds)
            }
            GatewayEvent::MyJoinSoundUpdate { guild_id, sound_id } => {
                self.set_my_join_sound(guild_id, sound_id)
            }
            GatewayEvent::TypingStart {
                channel_id,
                user_id,
                ..
            } => self.note_typing(channel_id, user_id, now_unix_milliseconds),
            GatewayEvent::PresenceUpdate(presence) => self.apply_presence(presence),
            GatewayEvent::UserUpdate(user) => self.apply_user_update(&user),
            GatewayEvent::DirectMessageChannelCreate(channel)
            | GatewayEvent::DirectMessageChannelUpdate(channel) => {
                self.upsert_direct_message_channel(channel)
            }
            GatewayEvent::DirectMessageChannelDelete { id } => {
                self.remove_direct_message_channel(id)
            }
            GatewayEvent::RelationshipAdd(relationship) => self.apply_relationship(relationship),
            GatewayEvent::RelationshipRemove { user_id } => self.forget_relationship(user_id),
            GatewayEvent::VoiceStateUpdate { voice_state } => self.apply_voice_state(voice_state),
            GatewayEvent::VoiceStateDelete {
                channel_id,
                user_id,
                ..
            } => self.drop_voice_state(user_id, channel_id),
            GatewayEvent::Speaking {
                channel_id,
                user_id,
                speaking,
                ..
            } => self.note_speaking(channel_id, user_id, speaking),
            GatewayEvent::VoiceSoundPlayed {
                channel_id,
                user_id,
                sound_id,
            } => self.note_sound_played(channel_id, user_id, sound_id, now_unix_milliseconds),
            GatewayEvent::VoiceSessionDescription { .. }
            | GatewayEvent::VoiceJoinRefused { .. }
            | GatewayEvent::VoiceSlotAssigned { .. }
            | GatewayEvent::VoiceVideoRefused { .. }
            | GatewayEvent::VoiceVideoLayers { .. }
            | GatewayEvent::VoiceVideoPaused { .. }
            | GatewayEvent::VoiceDisconnected { .. } => StateChange::Nothing,
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

    fn is_known_channel(&self, channel_id: Snowflake) -> bool {
        self.direct_message_channel(channel_id).is_some()
            || self.guild_of_channel(channel_id).is_some()
    }

    /// A channel gone or hidden from us: its cache, counts, pins, unsent messages, voice
    /// participants and typing indicators go too.
    fn forget_channel(&mut self, channel_id: Snowflake) {
        self.history_by_channel.remove(&channel_id);
        self.reads.forget_channel(channel_id);
        self.pins_by_channel.remove(&channel_id);
        self.pins_changed_while_loading.remove(&channel_id);
        self.pending_messages
            .retain(|pending| pending.channel_id != channel_id);
        let speaking_user_ids = &mut self.speaking_user_ids;
        self.voice_states.retain(|state| {
            let is_kept = state.channel_id != channel_id;
            if !is_kept {
                speaking_user_ids.remove(&state.user_id);
            }
            is_kept
        });
        self.typing_until
            .retain(|(typing_channel_id, _), _| *typing_channel_id != channel_id);
    }
}

/// The channel an event is about, for the race guard of `apply_event`. Voice states have their
/// own rule (`apply_voice_state`).
fn event_channel_id(event: &GatewayEvent) -> Option<Snowflake> {
    match event {
        GatewayEvent::MessageCreate(message) | GatewayEvent::MessageUpdate(message) => {
            Some(message.channel_id)
        }
        GatewayEvent::MessageReactionAdd(reaction)
        | GatewayEvent::MessageReactionRemove(reaction) => Some(reaction.channel_id),
        GatewayEvent::MessagePollVoteAdd(vote) | GatewayEvent::MessagePollVoteRemove(vote) => {
            Some(vote.channel_id)
        }
        GatewayEvent::MessageDelete { channel_id, .. }
        | GatewayEvent::MessageLinkPreviewsUpdate { channel_id, .. }
        | GatewayEvent::MessageReactionRemoveAll { channel_id, .. }
        | GatewayEvent::MessageReactionRemoveEmoji { channel_id, .. }
        | GatewayEvent::MessageChecklistUpdate { channel_id, .. }
        | GatewayEvent::MessageProgressBarUpdate { channel_id, .. }
        | GatewayEvent::TypingStart { channel_id, .. } => Some(*channel_id),
        GatewayEvent::ChannelUpdate(channel) => Some(channel.id),
        _ => None,
    }
}

/// The reducer's scenarios; the per-module tests share its fixtures.
#[cfg(test)]
mod tests;
