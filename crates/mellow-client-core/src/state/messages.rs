//! The message cache per channel: gateway events, history pages and our own sends merged by id.

use std::collections::BTreeMap;

use mellow_protocol::{Message, Snowflake};

use super::checklists::keep_newest;
use super::{ClientState, HISTORY_PAGE_SIZE, MAXIMUM_CACHED_MESSAGES_PER_CHANNEL, StateChange};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct ChannelHistory {
    /// Sorted by id (chronological).
    pub(super) messages: Vec<Message>,
    /// The channel's first message is cached: there is nothing older to fetch.
    has_reached_channel_start: bool,
    /// Pages loaded by scrolling up: new messages no longer push the oldest out until
    /// `trim_history`, since the reader may be looking at them.
    is_extended_by_older_pages: bool,
}

/// `None` (never edited) orders as the oldest edit.
fn edit_time(message: &Message) -> u64 {
    message.edited_at_unix_milliseconds.unwrap_or(0)
}

impl ClientState {
    #[must_use]
    pub fn messages(&self, channel_id: Snowflake) -> &[Message] {
        self.history_by_channel
            .get(&channel_id)
            .map_or(&[], |history| history.messages.as_slice())
    }

    /// One cached message, from the history or else the loaded pins.
    #[must_use]
    pub fn cached_message(&self, channel_id: Snowflake, message_id: Snowflake) -> Option<&Message> {
        self.cached_history_message(channel_id, message_id)
            .or_else(|| {
                self.pinned_messages(channel_id)
                    .iter()
                    .find(|message| message.id == message_id)
            })
    }

    pub(super) fn cached_history_message(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Option<&Message> {
        let messages = self.messages(channel_id);
        let index = messages
            .binary_search_by_key(&message_id, |message| message.id)
            .ok()?;
        messages.get(index)
    }

    /// False once the channel's first message is cached; true while its history never loaded.
    #[must_use]
    pub fn has_older_history(&self, channel_id: Snowflake) -> bool {
        !self
            .history_by_channel
            .get(&channel_id)
            .is_some_and(|history| history.has_reached_channel_start)
    }

    /// `MESSAGE_CREATE`, once its channel is known.
    pub(super) fn take_in_new_message(&mut self, message: Message) -> StateChange {
        let channel_id = message.channel_id;
        self.typing_until.remove(&(channel_id, message.author.id));
        self.note_direct_message_activity(channel_id, message.id);
        let previous = self.cached_history_message(channel_id, message.id).cloned();
        self.reads.note_newest_message(channel_id, message.id);
        let is_counted_by_ready = self.reads.is_counted_by_ready(channel_id, message.id);
        let counted = message.clone();
        self.insert_messages(channel_id, vec![message]);
        if !is_counted_by_ready {
            self.adjust_mention_count(channel_id, previous.as_ref(), Some(&counted));
        }
        StateChange::Messages { channel_id }
    }

    /// The echo of our own send carries its nonce: the pending copy goes.
    pub(super) fn take_echoed_pending_message(&mut self, message: &Message) {
        if let Some(nonce) = &message.nonce
            && Some(message.author.id) == self.current_user_id()
        {
            self.pending_messages
                .retain(|pending| &pending.nonce != nonce);
        }
    }

    /// `MESSAGE_UPDATE` (and our own edit's answer): only cached messages change, so an edit of
    /// older history neither appears out of context nor marks a never-opened channel unread.
    /// An older edit never replaces a newer one: our own REST answer can arrive after the gateway
    /// delivered a later edit. Reactions, poll votes, checklist ticks and link previews follow
    /// their own events: an update may have read them earlier, so the cached ones mostly stay.
    pub(super) fn apply_message_update(&mut self, updated: Message) -> StateChange {
        let channel_id = updated.channel_id;
        let message_id = updated.id;
        let are_pins_changed = self.apply_pinned_state(&updated);
        let Some(previous) = self.cached_history_message(channel_id, message_id).cloned() else {
            return if are_pins_changed {
                StateChange::Message {
                    channel_id,
                    message_id,
                }
            } else {
                StateChange::Nothing
            };
        };
        if edit_time(&updated) < edit_time(&previous) {
            return StateChange::Nothing;
        }
        let merged = merged_update(&previous, updated);
        self.adjust_mention_count(channel_id, Some(&previous), Some(&merged));
        if let Some(cached) = self.cached_history_message_mut(channel_id, message_id) {
            *cached = merged;
        }
        StateChange::Message {
            channel_id,
            message_id,
        }
    }

    /// `MESSAGE_DELETE`, once its channel is known (`apply_event` already noted the id).
    pub(super) fn forget_deleted_message(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> StateChange {
        let deleted = self.cached_history_message(channel_id, message_id).cloned();
        self.adjust_mention_count(channel_id, deleted.as_ref(), None);
        if let Some(history) = self.history_by_channel.get_mut(&channel_id) {
            history.messages.retain(|message| message.id != message_id);
        }
        StateChange::Messages { channel_id }
    }

    pub(super) fn cached_history_message_mut(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Option<&mut Message> {
        let history = self.history_by_channel.get_mut(&channel_id)?;
        let index = history
            .messages
            .binary_search_by_key(&message_id, |message| message.id)
            .ok()?;
        history.messages.get_mut(index)
    }

    /// Changes one message wherever it is cached, history and loaded pins alike; `change` says
    /// whether it changed anything.
    pub(super) fn update_cached_message(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        mut change: impl FnMut(&mut Message) -> bool,
    ) -> StateChange {
        let mut is_changed = false;
        if let Some(message) = self.cached_history_message_mut(channel_id, message_id) {
            is_changed |= change(message);
        }
        if let Some(message) = self.loaded_pin_mut(channel_id, message_id) {
            is_changed |= change(message);
        }
        // The list on its way may predate this change.
        self.note_pins_changed_while_loading(channel_id);
        if is_changed {
            StateChange::Message {
                channel_id,
                message_id,
            }
        } else {
            StateChange::Nothing
        }
    }

    /// Merges new messages (gateway events, our own sends), deduplicated by id and kept sorted.
    /// The oldest go beyond `MAXIMUM_CACHED_MESSAGES_PER_CHANNEL`, unless pages loaded by
    /// scrolling up made the channel longer: those stay until `trim_history`.
    pub fn insert_messages(
        &mut self,
        channel_id: Snowflake,
        incoming: Vec<Message>,
    ) -> StateChange {
        let deleted_message_ids = &self.deleted_message_ids;
        let history = self.history_by_channel.entry(channel_id).or_default();
        let mut merged = merge_messages(
            std::mem::take(&mut history.messages),
            incoming
                .into_iter()
                .filter(|message| !deleted_message_ids.contains(&message.id)),
        );
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
        let deleted_message_ids = &self.deleted_message_ids;
        let history = self.history_by_channel.entry(channel_id).or_default();
        history.messages = merge_messages(
            std::mem::take(&mut history.messages),
            page.into_iter()
                .filter(|message| !deleted_message_ids.contains(&message.id)),
        );
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
}

/// `updated` as it replaces `previous` (an edit at least as recent): what only events bring stays
/// from the cache.
fn merged_update(previous: &Message, updated: Message) -> Message {
    let is_newer_edit = edit_time(previous) < edit_time(&updated);
    let poll = match (updated.poll, &previous.poll) {
        // Votes follow their own events: only the poll's end is taken, and an end never reverts
        // (a pin's update may have read the poll before it).
        (Some(mut poll), Some(cached)) => {
            poll.answers.clone_from(&cached.answers);
            poll.ended_at_unix_milliseconds = poll
                .ended_at_unix_milliseconds
                .or(cached.ended_at_unix_milliseconds);
            Some(poll)
        }
        (poll, _) => poll,
    };
    let mut checklist = previous.checklist.clone();
    keep_newest(&mut checklist, updated.checklist);
    let mut progress_bar = previous.progress_bar.clone();
    keep_newest(&mut progress_bar, updated.progress_bar);
    Message {
        // Events never carry reactions (`me` differs per viewer).
        reactions: previous.reactions.clone(),
        poll,
        checklist,
        progress_bar,
        // Previews come later in their own event: only a newer edit's are taken, never a pin's
        // or a REST echo's read before the fetcher wrote them.
        link_previews: if is_newer_edit {
            updated.link_previews
        } else {
            previous.link_previews.clone()
        },
        ..updated
    }
}

/// A history page or a send answer replacing a cached copy of the same edit: what only events
/// brought since stays.
fn with_what_only_events_brought(incoming: Message, cached: &Message) -> Message {
    let mut checklist = cached.checklist.clone();
    keep_newest(&mut checklist, incoming.checklist);
    let mut progress_bar = cached.progress_bar.clone();
    keep_newest(&mut progress_bar, incoming.progress_bar);
    Message {
        link_previews: if incoming.link_previews.is_empty() {
            cached.link_previews.clone()
        } else {
            incoming.link_previews
        },
        reactions: if incoming.reactions.is_empty() {
            cached.reactions.clone()
        } else {
            incoming.reactions
        },
        poll: incoming.poll.or_else(|| cached.poll.clone()),
        checklist,
        progress_bar,
        ..incoming
    }
}

/// Sorted by id and deduplicated. An incoming copy replaces a cached one unless it holds an older
/// edit (a page that left before the edit must not put the older text back).
fn merge_messages(cached: Vec<Message>, incoming: impl Iterator<Item = Message>) -> Vec<Message> {
    let mut by_id: BTreeMap<Snowflake, Message> = cached
        .into_iter()
        .map(|message| (message.id, message))
        .collect();
    for message in incoming {
        match by_id.get(&message.id) {
            Some(cached) if edit_time(cached) > edit_time(&message) => {}
            Some(cached) if edit_time(cached) == edit_time(&message) => {
                let merged = with_what_only_events_brought(message, cached);
                by_id.insert(merged.id, merged);
            }
            _ => {
                by_id.insert(message.id, message);
            }
        }
    }
    by_id.into_values().collect()
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{GatewayEvent, Poll, PollAnswer, ReactionSummary};

    use super::super::tests::{GENERAL, NOW, OTHER, message, ready_state};
    use super::*;

    fn thumbs() -> Vec<ReactionSummary> {
        vec![ReactionSummary {
            emoji: "👍".into(),
            count: 1,
            me: false,
        }]
    }

    #[test]
    fn a_late_page_never_brings_back_a_deleted_message_or_an_older_edit() {
        let mut state = ready_state();
        let mut edited = message(11, OTHER);
        edited.content = "edited".into();
        edited.edited_at_unix_milliseconds = Some(NOW);
        state.apply_event(GatewayEvent::MessageCreate(edited), NOW);
        state.apply_event(
            GatewayEvent::MessageDelete {
                guild_id: None,
                channel_id: GENERAL,
                id: Snowflake::from_raw(12),
            },
            NOW,
        );
        state.insert_history_page(GENERAL, vec![message(11, OTHER), message(12, OTHER)]);
        let cached = state.messages(GENERAL);
        assert_eq!(cached.len(), 1, "the deleted message stays gone");
        assert_eq!(cached[0].content, "edited", "the older text stays away");
    }

    #[test]
    fn a_page_of_the_same_edit_keeps_what_only_events_brought() {
        let mut state = ready_state();
        state.insert_messages(GENERAL, vec![message(11, OTHER)]);
        state.apply_event(
            GatewayEvent::MessageReactionAdd(mellow_protocol::MessageReaction {
                guild_id: None,
                channel_id: GENERAL,
                message_id: Snowflake::from_raw(11),
                user_id: OTHER,
                emoji: "👍".into(),
            }),
            NOW,
        );
        state.insert_history_page(GENERAL, vec![message(11, OTHER)]);
        assert_eq!(state.messages(GENERAL)[0].reactions, thumbs());
    }

    #[test]
    fn an_update_takes_a_polls_end_but_keeps_its_counted_votes() {
        let mut state = ready_state();
        let poll = Poll {
            question: "Lunch?".into(),
            answers: vec![PollAnswer {
                id: 1,
                text: "Pizza".into(),
                vote_count: 3,
                me_voted: true,
            }],
            allows_multiple_answers: false,
            expires_at_unix_milliseconds: NOW * 2,
            ended_at_unix_milliseconds: None,
        };
        let mut asked = message(11, OTHER);
        asked.poll = Some(poll.clone());
        state.insert_messages(GENERAL, vec![asked.clone()]);
        let mut ended = asked.clone();
        let mut ended_poll = poll.clone();
        ended_poll.answers[0].vote_count = 0;
        ended_poll.answers[0].me_voted = false;
        ended_poll.ended_at_unix_milliseconds = Some(NOW);
        ended.poll = Some(ended_poll);
        state.apply_event(GatewayEvent::MessageUpdate(ended), NOW);
        let cached = state.messages(GENERAL)[0].poll.clone().unwrap();
        assert_eq!(cached.answers, poll.answers);
        assert_eq!(cached.ended_at_unix_milliseconds, Some(NOW));

        // A pin's update read before the end never reverts it.
        let mut pinned = asked;
        pinned.pinned = true;
        state.apply_event(GatewayEvent::MessageUpdate(pinned), NOW);
        let cached = state.messages(GENERAL)[0].poll.clone().unwrap();
        assert_eq!(cached.ended_at_unix_milliseconds, Some(NOW));
    }

    #[test]
    fn link_previews_come_from_their_event_or_a_newer_edit_only() {
        let mut state = ready_state();
        state.insert_messages(GENERAL, vec![message(11, OTHER)]);
        let preview = mellow_protocol::LinkPreview {
            id: Snowflake::from_raw(500),
            url: "https://example.org/".into(),
            title: None,
            description: None,
            site_name: None,
            theme_color: None,
            image: None,
            embed: None,
        };
        let change = state.apply_event(
            GatewayEvent::MessageLinkPreviewsUpdate {
                guild_id: None,
                channel_id: GENERAL,
                message_id: Snowflake::from_raw(11),
                link_previews: vec![preview.clone()],
            },
            NOW,
        );
        assert_eq!(
            change,
            StateChange::Message {
                channel_id: GENERAL,
                message_id: Snowflake::from_raw(11)
            }
        );
        let mut pinned = message(11, OTHER);
        pinned.pinned = true;
        state.apply_event(GatewayEvent::MessageUpdate(pinned), NOW);
        assert_eq!(state.messages(GENERAL)[0].link_previews, [preview]);
        let mut edited = message(11, OTHER);
        edited.edited_at_unix_milliseconds = Some(NOW);
        state.apply_event(GatewayEvent::MessageUpdate(edited), NOW);
        assert_eq!(state.messages(GENERAL)[0].link_previews, []);
    }
}
