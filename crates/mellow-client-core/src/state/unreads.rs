//! Read positions, unread channels and mention badges (ADR-0040).

use std::collections::{HashMap, HashSet};

use mellow_protocol::{
    ChannelUnread, MAXIMUM_COUNTED_UNREAD_MENTIONS, Message, ReadState, Snowflake,
};

use super::mentions::{MentionReader, does_message_mention_reader};
use super::{ClientState, StateChange};

/// Where the user stands in each channel.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct ReadPositions {
    /// The newest message read, as this client knows it: ahead of the server until sent.
    last_read_by_channel: HashMap<Snowflake, Snowflake>,
    /// The read position the server confirmed.
    read_on_server_by_channel: HashMap<Snowflake, Snowflake>,
    /// Channels read further than the server knows; kept after a failed send so the next one
    /// retries.
    unsent_read_channel_ids: HashSet<Snowflake>,
    /// Each channel's newest message as far as this session knows (`Ready.channel_unreads`, then
    /// `MESSAGE_CREATE`), so a channel never opened still shows unread.
    newest_message_id_by_channel: HashMap<Snowflake, Snowflake>,
    /// Unread messages that mention the user (in a conversation: unread messages from others);
    /// absent means none.
    mention_count_by_channel: HashMap<Snowflake, u32>,
    /// Each channel's newest message when the server built `Ready`. The gateway replays what was
    /// committed while it did, and `Ready` already counted those: a replayed message at or below
    /// this id is cached but not counted again.
    newest_counted_by_ready: HashMap<Snowflake, Snowflake>,
}

impl ReadPositions {
    /// What `Ready` says, plus the reads this client made that the server has not heard yet:
    /// they go out with the next send.
    /// shortcut: servers before `v0.10.0` send no `channel_unreads`, so their mention badges
    /// start at zero; they are no longer supported.
    pub(super) fn after_ready(
        self,
        read_states: &[ReadState],
        channel_unreads: &[ChannelUnread],
        is_known_channel: impl Fn(Snowflake) -> bool,
    ) -> Self {
        let read_on_server_by_channel: HashMap<Snowflake, Snowflake> = read_states
            .iter()
            .map(|state| (state.channel_id, state.last_read_message_id))
            .collect();
        let mut last_read_by_channel = read_on_server_by_channel.clone();
        let mut unsent_read_channel_ids = HashSet::new();
        for channel_id in self.unsent_read_channel_ids {
            let Some(unsent) = self.last_read_by_channel.get(&channel_id).copied() else {
                continue;
            };
            let on_server = read_on_server_by_channel.get(&channel_id).copied();
            if is_known_channel(channel_id) && on_server.is_none_or(|on_server| unsent > on_server)
            {
                last_read_by_channel.insert(channel_id, unsent);
                unsent_read_channel_ids.insert(channel_id);
            }
        }
        Self {
            last_read_by_channel,
            read_on_server_by_channel,
            unsent_read_channel_ids,
            newest_message_id_by_channel: channel_unreads
                .iter()
                .map(|unread| (unread.channel_id, unread.last_message_id))
                .collect(),
            mention_count_by_channel: channel_unreads
                .iter()
                .filter(|unread| unread.mention_count > 0)
                .map(|unread| (unread.channel_id, unread.mention_count))
                .collect(),
            newest_counted_by_ready: channel_unreads
                .iter()
                .map(|unread| (unread.channel_id, unread.last_message_id))
                .collect(),
        }
    }

    pub(super) fn note_newest_message(&mut self, channel_id: Snowflake, message_id: Snowflake) {
        let newest = self
            .newest_message_id_by_channel
            .entry(channel_id)
            .or_insert(message_id);
        *newest = (*newest).max(message_id);
    }

    pub(super) fn is_counted_by_ready(&self, channel_id: Snowflake, message_id: Snowflake) -> bool {
        self.newest_counted_by_ready
            .get(&channel_id)
            .is_some_and(|counted| message_id <= *counted)
    }

    pub(super) fn forget_channel(&mut self, channel_id: Snowflake) {
        self.newest_message_id_by_channel.remove(&channel_id);
        self.mention_count_by_channel.remove(&channel_id);
    }
}

/// The "New" divider and the jump bar of one visit to a channel (DESIGN.md §5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnreadMarker {
    /// The divider goes after this message.
    pub last_read_message_id: Snowflake,
    /// The newest message when the channel was opened.
    pub newest_at_open_message_id: Snowflake,
}

impl ClientState {
    /// True when the newest known message is after the user's read position. A channel knows
    /// its newest message without any cached (`Ready.channel_unreads`, a conversation's last
    /// message).
    #[must_use]
    pub fn has_unread(&self, channel_id: Snowflake) -> bool {
        let newest_cached = self.messages(channel_id).last().map(|message| message.id);
        let newest_of_conversation = self
            .direct_message_channel(channel_id)
            .and_then(|channel| channel.last_message_id);
        let newest_reported = self
            .reads
            .newest_message_id_by_channel
            .get(&channel_id)
            .copied();
        let newest = newest_cached
            .max(newest_of_conversation)
            .max(newest_reported);
        match (newest, self.last_read_message_id(channel_id)) {
            (Some(newest), Some(last_read)) => newest > last_read,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// The newest message the signed-in user has read in `channel_id`, if any.
    #[must_use]
    pub fn last_read_message_id(&self, channel_id: Snowflake) -> Option<Snowflake> {
        self.reads.last_read_by_channel.get(&channel_id).copied()
    }

    /// The red count of a channel; the server stops at `MAXIMUM_COUNTED_UNREAD_MENTIONS` ("99+").
    #[must_use]
    pub fn mention_count(&self, channel_id: Snowflake) -> u32 {
        self.reads
            .mention_count_by_channel
            .get(&channel_id)
            .copied()
            .unwrap_or(0)
    }

    /// The red count on a server: the sum of its channels.
    #[must_use]
    pub fn guild_mention_count(&self, guild_id: Snowflake) -> u32 {
        self.guild(guild_id).map_or(0, |guild| {
            guild
                .channels
                .iter()
                .map(|channel| self.mention_count(channel.id))
                .fold(0, u32::saturating_add)
        })
    }

    /// The red count on Home: the sum of the conversations.
    #[must_use]
    pub fn direct_messages_mention_count(&self) -> u32 {
        self.direct_message_channels
            .iter()
            .map(|channel| self.mention_count(channel.id))
            .fold(0, u32::saturating_add)
    }

    /// The divider and jump bar for a visit starting now, to ask before `mark_read`: only what
    /// others wrote since the last read; a channel never read has no line to draw.
    #[must_use]
    pub fn unread_marker(&self, channel_id: Snowflake) -> Option<UnreadMarker> {
        let last_read = self.last_read_message_id(channel_id)?;
        let messages = self.messages(channel_id);
        let newest = messages.last()?;
        let me = self.current_user_id();
        messages
            .iter()
            .any(|message| message.id > last_read && Some(message.author.id) != me)
            .then_some(UnreadMarker {
                last_read_message_id: last_read,
                newest_at_open_message_id: newest.id,
            })
    }

    /// The user read `channel_id` up to `message_id` here: its mentions are read too. Never moves
    /// backwards, like the server; the server hears it with the next `take_read_states_to_send`.
    pub fn mark_read(&mut self, channel_id: Snowflake, message_id: Snowflake) -> StateChange {
        self.reads.mention_count_by_channel.remove(&channel_id);
        let last_read = self
            .reads
            .last_read_by_channel
            .entry(channel_id)
            .or_insert(message_id);
        if message_id > *last_read {
            *last_read = message_id;
        }
        let is_ahead_of_server = self
            .reads
            .read_on_server_by_channel
            .get(&channel_id)
            .is_none_or(|on_server| *last_read > *on_server);
        if is_ahead_of_server {
            self.reads.unsent_read_channel_ids.insert(channel_id);
        }
        StateChange::ReadStates
    }

    /// Each channel's newest read position the server does not have yet, to send now (debounced
    /// by the caller, and right away when the app goes to the background).
    pub fn take_read_states_to_send(&mut self) -> Vec<ReadState> {
        let reads = &mut self.reads;
        let mut to_send: Vec<ReadState> = reads
            .unsent_read_channel_ids
            .drain()
            .filter_map(|channel_id| {
                let last_read = *reads.last_read_by_channel.get(&channel_id)?;
                let is_ahead = reads
                    .read_on_server_by_channel
                    .get(&channel_id)
                    .is_none_or(|on_server| last_read > *on_server);
                is_ahead.then_some(ReadState {
                    channel_id,
                    last_read_message_id: last_read,
                })
            })
            .collect();
        to_send.sort_by_key(|state| state.channel_id);
        to_send
    }

    /// The server took a read position; it keeps the newest it was sent, whatever the order of
    /// the answers.
    pub fn confirm_read_state_sent(&mut self, sent: &ReadState) {
        let on_server = self
            .reads
            .read_on_server_by_channel
            .entry(sent.channel_id)
            .or_insert(sent.last_read_message_id);
        *on_server = (*on_server).max(sent.last_read_message_id);
    }

    /// Sending a read position failed. One worth retrying (`ClientError::is_worth_retrying`)
    /// goes with the next send; a refusal (a deleted message, a hidden channel) would fail every
    /// time. The local position never rolls back.
    pub fn note_read_state_send_failed(&mut self, channel_id: Snowflake, is_worth_retrying: bool) {
        if is_worth_retrying && self.reads.last_read_by_channel.contains_key(&channel_id) {
            self.reads.unsent_read_channel_ids.insert(channel_id);
        }
    }

    /// Whether `message` mentions the signed-in user where it was written: its tint, and whether
    /// a notification may say so.
    #[must_use]
    pub fn does_message_mention_current_user(&self, message: &Message) -> bool {
        let Some(user_id) = self.current_user_id() else {
            return false;
        };
        let guild = self.guild_of_channel(message.channel_id);
        let reader = MentionReader {
            user_id,
            guild_id: guild.map(|guild| guild.id),
            role_ids: guild.map_or(&[], |guild| guild.my_role_ids.as_slice()),
        };
        does_message_mention_reader(message, &reader)
    }

    /// Whether the message counts in its channel's red badge: after the read position, from
    /// someone not blocked (the server leaves them out too), and mentioning the user, or anything
    /// in a conversation.
    fn is_unread_mention_of_current_user(&self, message: &Message) -> bool {
        let Some(user_id) = self.current_user_id() else {
            return false;
        };
        if message.author.id == user_id || self.is_blocked(message.author.id) {
            return false;
        }
        if self
            .last_read_message_id(message.channel_id)
            .is_some_and(|last_read| message.id <= last_read)
        {
            return false;
        }
        self.direct_message_channel(message.channel_id).is_some()
            || self.does_message_mention_current_user(message)
    }

    /// Follows a message from `before` to `after` (`None`: not known here, or deleted).
    /// shortcut: a message this session never cached (counted in `Ready` only) cannot be
    /// followed, so its edits and deletion leave the count as it is until the next `Ready`;
    /// upgrade if the gateway ever sends mention counts.
    pub(super) fn adjust_mention_count(
        &mut self,
        channel_id: Snowflake,
        before: Option<&Message>,
        after: Option<&Message>,
    ) {
        let counts = |message: Option<&Message>| {
            message.is_some_and(|message| self.is_unread_mention_of_current_user(message))
        };
        let (was_counted, is_counted) = (counts(before), counts(after));
        let count = self.mention_count(channel_id);
        let next = match (was_counted, is_counted) {
            (false, true) => count.saturating_add(1),
            // The server stops counting at the maximum: "at least 100" minus one is not 99.
            (true, false) if count < MAXIMUM_COUNTED_UNREAD_MENTIONS => count.saturating_sub(1),
            _ => return,
        };
        if next == 0 {
            self.reads.mention_count_by_channel.remove(&channel_id);
        } else {
            self.reads.mention_count_by_channel.insert(channel_id, next);
        }
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{GatewayEvent, Ready};

    use super::super::tests::{GENERAL, GUILD, ME, NOW, OTHER, message, ready_state};
    use super::*;

    fn mentioning_me(id: u64) -> Message {
        let mut mentioning = message(id, OTHER);
        mentioning.mention_ids = vec![ME];
        mentioning
    }

    fn ready_with_unreads(state: &ClientState, channel_unreads: Vec<ChannelUnread>) -> Ready {
        let current = state.current_user().unwrap().clone();
        Ready {
            session_id: "s2".into(),
            user: current,
            guilds: state.guilds().to_vec(),
            read_states: vec![ReadState {
                channel_id: GENERAL,
                last_read_message_id: Snowflake::from_raw(10),
            }],
            online_user_ids: Vec::new(),
            direct_message_channels: Vec::new(),
            channel_unreads,
            presences: Vec::new(),
            relationships: Vec::new(),
            may_create_guilds: true,
            voice_states: Vec::new(),
            voice_enabled: false,
            maximum_attachment_bytes: None,
        }
    }

    #[test]
    fn a_never_opened_channel_is_unread_from_ready_and_counts_its_mentions() {
        let mut state = ready_state();
        let ready = ready_with_unreads(
            &state,
            vec![ChannelUnread {
                channel_id: GENERAL,
                last_message_id: Snowflake::from_raw(20),
                mention_count: 2,
            }],
        );
        state.apply_ready(ready);
        assert!(state.has_unread(GENERAL));
        assert_eq!(state.mention_count(GENERAL), 2);
        assert_eq!(state.guild_mention_count(GUILD), 2);

        // A replay of what `Ready` already counted is cached but not counted again.
        state.apply_event(GatewayEvent::MessageCreate(mentioning_me(20)), NOW);
        assert_eq!(state.mention_count(GENERAL), 2);
        state.apply_event(GatewayEvent::MessageCreate(mentioning_me(21)), NOW);
        assert_eq!(state.mention_count(GENERAL), 3);
        state.apply_event(
            GatewayEvent::MessageDelete {
                guild_id: Some(GUILD),
                channel_id: GENERAL,
                id: Snowflake::from_raw(21),
            },
            NOW,
        );
        assert_eq!(state.mention_count(GENERAL), 2);
        state.mark_read(GENERAL, Snowflake::from_raw(21));
        assert_eq!(state.mention_count(GENERAL), 0);
        assert!(!state.has_unread(GENERAL));
    }

    #[test]
    fn an_edit_that_adds_or_drops_a_mention_follows_the_count_and_blocked_authors_never_count() {
        let mut state = ready_state();
        state.apply_event(GatewayEvent::MessageCreate(message(11, OTHER)), NOW);
        assert_eq!(state.mention_count(GENERAL), 0);
        let mut edited = mentioning_me(11);
        edited.edited_at_unix_milliseconds = Some(NOW);
        state.apply_event(GatewayEvent::MessageUpdate(edited), NOW);
        assert_eq!(state.mention_count(GENERAL), 1);

        state.apply_event(
            GatewayEvent::RelationshipAdd(mellow_protocol::Relationship {
                user: super::super::tests::user(OTHER),
                kind: mellow_protocol::RelationshipKind::Blocked,
                since_unix_milliseconds: 0,
            }),
            NOW,
        );
        state.apply_event(GatewayEvent::MessageCreate(mentioning_me(12)), NOW);
        assert_eq!(
            state.mention_count(GENERAL),
            1,
            "a blocked author is left out"
        );
    }

    #[test]
    fn a_saturated_count_does_not_drop_below_its_maximum() {
        let mut state = ready_state();
        let ready = ready_with_unreads(
            &state,
            vec![ChannelUnread {
                channel_id: GENERAL,
                last_message_id: Snowflake::from_raw(20),
                mention_count: MAXIMUM_COUNTED_UNREAD_MENTIONS,
            }],
        );
        state.apply_ready(ready);
        state.apply_event(GatewayEvent::MessageCreate(mentioning_me(21)), NOW);
        state.apply_event(
            GatewayEvent::MessageDelete {
                guild_id: Some(GUILD),
                channel_id: GENERAL,
                id: Snowflake::from_raw(21),
            },
            NOW,
        );
        assert_eq!(
            state.mention_count(GENERAL),
            MAXIMUM_COUNTED_UNREAD_MENTIONS + 1,
            "at least 100 minus one is not 99"
        );
    }

    #[test]
    fn read_positions_go_to_the_server_once_and_survive_a_ready_until_sent() {
        let mut state = ready_state();
        state.mark_read(GENERAL, Snowflake::from_raw(5));
        assert_eq!(
            state.take_read_states_to_send(),
            [],
            "behind the server's position"
        );
        state.mark_read(GENERAL, Snowflake::from_raw(30));
        let to_send = state.take_read_states_to_send();
        assert_eq!(
            to_send,
            [ReadState {
                channel_id: GENERAL,
                last_read_message_id: Snowflake::from_raw(30)
            }]
        );
        assert_eq!(state.take_read_states_to_send(), [], "sent once");
        state.note_read_state_send_failed(GENERAL, true);

        // The send failed; a new session's older position does not undo the read.
        let ready = ready_with_unreads(&state, Vec::new());
        state.apply_ready(ready);
        assert_eq!(
            state.last_read_message_id(GENERAL),
            Some(Snowflake::from_raw(30))
        );
        let to_send = state.take_read_states_to_send();
        assert_eq!(to_send.len(), 1);
        state.confirm_read_state_sent(&to_send[0]);
        state.note_read_state_send_failed(GENERAL, false);
        assert_eq!(state.take_read_states_to_send(), []);
    }

    #[test]
    fn the_unread_marker_shows_only_what_others_wrote_since_the_last_read() {
        let mut state = ready_state();
        state.insert_messages(GENERAL, vec![message(9, OTHER), message(11, ME)]);
        assert_eq!(state.unread_marker(GENERAL), None, "only my own since");
        state.insert_messages(GENERAL, vec![message(12, OTHER)]);
        assert_eq!(
            state.unread_marker(GENERAL),
            Some(UnreadMarker {
                last_read_message_id: Snowflake::from_raw(10),
                newest_at_open_message_id: Snowflake::from_raw(12),
            })
        );
    }
}
