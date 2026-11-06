//! Each channel's pinned messages: loaded when a pins panel opens, then kept up to date by events.

use mellow_protocol::{Message, Snowflake};

use super::{ClientState, StateChange};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Pins {
    Loading,
    /// Newest pin first.
    Loaded(Vec<Message>),
    Failed,
}

impl Pins {
    pub(super) fn messages_mut(&mut self) -> &mut [Message] {
        match self {
            Self::Loaded(messages) => messages,
            Self::Loading | Self::Failed => &mut [],
        }
    }
}

/// Where a channel's pins stand, for the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinsStatus {
    Loading,
    Loaded,
    /// The last request failed: the panel offers to try again.
    Failed,
}

/// What to do with a pins answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinsAnswer {
    Applied,
    /// Pins changed while the answer was on its way: ask again (it stays loading).
    AskAgain,
    /// The channel went meanwhile.
    Dropped,
}

impl ClientState {
    /// Empty unless loaded.
    #[must_use]
    pub fn pinned_messages(&self, channel_id: Snowflake) -> &[Message] {
        match self.pins_by_channel.get(&channel_id) {
            Some(Pins::Loaded(messages)) => messages,
            _ => &[],
        }
    }

    /// `None` until a pins panel first opens on the channel.
    #[must_use]
    pub fn pins_status(&self, channel_id: Snowflake) -> Option<PinsStatus> {
        self.pins_by_channel
            .get(&channel_id)
            .map(|pins| match pins {
                Pins::Loading => PinsStatus::Loading,
                Pins::Loaded(_) => PinsStatus::Loaded,
                Pins::Failed => PinsStatus::Failed,
            })
    }

    /// Whether to ask the instance for the pins now: not when they are loaded or on their way,
    /// unless `force` (Try again). True marks them loading.
    pub fn begin_loading_pins(&mut self, channel_id: Snowflake, force: bool) -> bool {
        let is_settled = matches!(
            self.pins_by_channel.get(&channel_id),
            Some(Pins::Loaded(_) | Pins::Loading)
        );
        if is_settled && !force {
            return false;
        }
        self.pins_by_channel.insert(channel_id, Pins::Loading);
        self.pins_changed_while_loading.remove(&channel_id);
        true
    }

    /// The instance's answer to `begin_loading_pins`.
    pub fn finish_loading_pins(
        &mut self,
        channel_id: Snowflake,
        messages: Vec<Message>,
    ) -> PinsAnswer {
        if !self.is_known_channel(channel_id) {
            return PinsAnswer::Dropped;
        }
        // The answer may predate a pin, an unpin or a reaction.
        if self.pins_changed_while_loading.remove(&channel_id) {
            return PinsAnswer::AskAgain;
        }
        self.pins_by_channel
            .insert(channel_id, Pins::Loaded(messages));
        PinsAnswer::Applied
    }

    pub fn fail_loading_pins(&mut self, channel_id: Snowflake) -> StateChange {
        if !self.is_known_channel(channel_id) {
            return StateChange::Nothing;
        }
        self.pins_by_channel.insert(channel_id, Pins::Failed);
        StateChange::Pins { channel_id }
    }

    /// The instance answers a pin or an unpin with no message: the change is applied here, and its
    /// event repeats it.
    pub fn set_pinned(&mut self, message: &Message, is_pinned: bool) -> StateChange {
        let channel_id = message.channel_id;
        let mut changed = self
            .cached_history_message(channel_id, message.id)
            .unwrap_or(message)
            .clone();
        changed.pinned = is_pinned;
        if let Some(cached) = self.cached_history_message_mut(channel_id, message.id) {
            cached.pinned = is_pinned;
        }
        self.apply_pinned_state(&changed);
        StateChange::Message {
            channel_id,
            message_id: message.id,
        }
    }

    /// A message pinned, unpinned or edited: the loaded pins follow, newest pin first. True when
    /// they changed.
    pub(super) fn apply_pinned_state(&mut self, message: &Message) -> bool {
        if self.deleted_message_ids.contains(&message.id) {
            return false;
        }
        self.note_pins_changed_while_loading(message.channel_id);
        let cached_reactions = self
            .cached_history_message(message.channel_id, message.id)
            .map(|cached| cached.reactions.clone());
        let Some(Pins::Loaded(pins)) = self.pins_by_channel.get_mut(&message.channel_id) else {
            return false;
        };
        let existing = pins.iter().position(|pin| pin.id == message.id);
        match (existing, message.pinned) {
            (None, false) => false,
            (Some(index), false) => {
                pins.remove(index);
                true
            }
            (Some(index), true) => {
                if let Some(pin) = pins.get_mut(index) {
                    let reactions = std::mem::take(&mut pin.reactions);
                    *pin = Message {
                        reactions,
                        ..message.clone()
                    };
                }
                true
            }
            (None, true) => {
                pins.insert(
                    0,
                    Message {
                        reactions: cached_reactions.unwrap_or_default(),
                        ..message.clone()
                    },
                );
                true
            }
        }
    }

    pub(super) fn drop_pin(&mut self, channel_id: Snowflake, message_id: Snowflake) {
        self.note_pins_changed_while_loading(channel_id);
        if let Some(Pins::Loaded(pins)) = self.pins_by_channel.get_mut(&channel_id) {
            pins.retain(|pin| pin.id != message_id);
        }
    }

    pub(super) fn loaded_pin_mut(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Option<&mut Message> {
        match self.pins_by_channel.get_mut(&channel_id)? {
            Pins::Loaded(pins) => pins.iter_mut().find(|pin| pin.id == message_id),
            Pins::Loading | Pins::Failed => None,
        }
    }

    pub(super) fn note_pins_changed_while_loading(&mut self, channel_id: Snowflake) {
        if matches!(self.pins_by_channel.get(&channel_id), Some(Pins::Loading)) {
            self.pins_changed_while_loading.insert(channel_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{GatewayEvent, MessageReaction};

    use super::super::tests::{GENERAL, NOW, OTHER, message, ready_state};
    use super::*;

    fn pinned(id: u64) -> Message {
        Message {
            pinned: true,
            ..message(id, OTHER)
        }
    }

    #[test]
    fn an_answer_older_than_a_pin_is_asked_again() {
        let mut state = ready_state();
        assert!(state.begin_loading_pins(GENERAL, false));
        assert!(
            !state.begin_loading_pins(GENERAL, false),
            "already on its way"
        );
        state.apply_event(GatewayEvent::MessageUpdate(pinned(11)), NOW);
        assert_eq!(
            state.finish_loading_pins(GENERAL, Vec::new()),
            PinsAnswer::AskAgain
        );
        assert_eq!(state.pins_status(GENERAL), Some(PinsStatus::Loading));
        assert_eq!(
            state.finish_loading_pins(GENERAL, vec![pinned(11)]),
            PinsAnswer::Applied
        );
        assert_eq!(state.pinned_messages(GENERAL), [pinned(11)]);
        assert!(!state.begin_loading_pins(GENERAL, false));
        assert!(state.begin_loading_pins(GENERAL, true), "Try again");
    }

    #[test]
    fn loaded_pins_follow_pins_unpins_reactions_and_deletions() {
        let mut state = ready_state();
        state.begin_loading_pins(GENERAL, false);
        state.finish_loading_pins(GENERAL, vec![pinned(11)]);
        state.insert_messages(GENERAL, vec![message(12, OTHER)]);
        state.set_pinned(&message(12, OTHER), true);
        let ids: Vec<u64> = state
            .pinned_messages(GENERAL)
            .iter()
            .map(|pin| pin.id.raw())
            .collect();
        assert_eq!(ids, [12, 11], "newest pin first");
        assert!(state.messages(GENERAL)[0].pinned);

        state.apply_event(
            GatewayEvent::MessageReactionAdd(MessageReaction {
                guild_id: None,
                channel_id: GENERAL,
                message_id: Snowflake::from_raw(11),
                user_id: OTHER,
                emoji: "👍".into(),
            }),
            NOW,
        );
        assert_eq!(state.pinned_messages(GENERAL)[1].reactions.len(), 1);
        state.apply_event(GatewayEvent::MessageUpdate(message(12, OTHER)), NOW);
        state.apply_event(
            GatewayEvent::MessageDelete {
                guild_id: None,
                channel_id: GENERAL,
                id: Snowflake::from_raw(11),
            },
            NOW,
        );
        assert_eq!(state.pinned_messages(GENERAL), []);
        assert_eq!(
            state.fail_loading_pins(Snowflake::from_raw(999)),
            StateChange::Nothing
        );
    }
}
