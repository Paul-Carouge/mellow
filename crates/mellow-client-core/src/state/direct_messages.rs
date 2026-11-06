//! Direct-message conversations and groups (ADR-0026).

use mellow_protocol::{DirectMessageChannel, Snowflake};

use super::{ClientState, StateChange};

/// Most recent activity first, as in Discord's conversation list.
pub(super) fn sort_by_last_activity(channels: &mut [DirectMessageChannel]) {
    channels
        .sort_by_key(|channel| std::cmp::Reverse(channel.last_message_id.unwrap_or(channel.id)));
}

impl ClientState {
    /// `DIRECT_MESSAGE_CHANNEL_CREATE` and `_UPDATE` (a group renamed, new participants, a new
    /// owner), and the REST answer of opening a conversation. A conversation already known keeps
    /// its newer last message.
    pub fn upsert_direct_message_channel(
        &mut self,
        mut channel: DirectMessageChannel,
    ) -> StateChange {
        match self
            .direct_message_channels
            .iter_mut()
            .find(|existing| existing.id == channel.id)
        {
            Some(existing) => {
                channel.last_message_id = existing.last_message_id.max(channel.last_message_id);
                *existing = channel;
            }
            None => self.direct_message_channels.push(channel),
        }
        sort_by_last_activity(&mut self.direct_message_channels);
        StateChange::DirectMessages
    }

    /// The instance's answer about a group (renamed, someone added), unless the group left
    /// meanwhile (a delete event came first).
    pub fn apply_group_answer(&mut self, group: DirectMessageChannel) -> StateChange {
        if self.direct_message_channel(group.id).is_none() {
            return StateChange::Nothing;
        }
        self.upsert_direct_message_channel(group)
    }

    /// The owner removed someone: at once, not on the gateway's update, so a UI can move focus
    /// to the next row when this one goes.
    pub fn remove_group_participant(
        &mut self,
        channel_id: Snowflake,
        user_id: Snowflake,
    ) -> StateChange {
        let Some(group) = self
            .direct_message_channels
            .iter_mut()
            .find(|channel| channel.id == channel_id)
        else {
            return StateChange::Nothing;
        };
        group.recipients.retain(|recipient| recipient.id != user_id);
        StateChange::DirectMessages
    }

    /// `DIRECT_MESSAGE_CHANNEL_DELETE`, or the answer to leaving a group: a conversation the user
    /// can no longer read goes, with everything kept about it.
    pub fn remove_direct_message_channel(&mut self, channel_id: Snowflake) -> StateChange {
        let before = self.direct_message_channels.len();
        self.direct_message_channels
            .retain(|channel| channel.id != channel_id);
        if self.direct_message_channels.len() == before {
            return StateChange::Nothing;
        }
        self.forget_channel(channel_id);
        StateChange::ConversationRemoved { channel_id }
    }

    pub(super) fn note_direct_message_activity(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) {
        if let Some(channel) = self
            .direct_message_channels
            .iter_mut()
            .find(|channel| channel.id == channel_id)
        {
            channel.last_message_id = channel.last_message_id.max(Some(message_id));
            sort_by_last_activity(&mut self.direct_message_channels);
        }
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::GatewayEvent;

    use super::super::tests::{ME, NOW, OTHER, message, ready_state, user};
    use super::*;

    const GROUP: Snowflake = Snowflake::from_raw(300);
    const THIRD: Snowflake = Snowflake::from_raw(3);

    fn group(name: &str) -> DirectMessageChannel {
        DirectMessageChannel {
            id: GROUP,
            recipients: vec![user(OTHER), user(THIRD)],
            last_message_id: None,
            owner_id: Some(ME),
            name: name.into(),
        }
    }

    #[test]
    fn groups_are_renamed_by_their_update_and_keep_their_last_message() {
        let mut state = ready_state();
        state.upsert_direct_message_channel(group("Band"));
        let mut posted = message(310, OTHER);
        posted.channel_id = GROUP;
        state.apply_event(GatewayEvent::MessageCreate(posted), NOW);
        state.apply_event(
            GatewayEvent::DirectMessageChannelUpdate(group("Choir")),
            NOW,
        );
        let renamed = state.direct_message_channel(GROUP).unwrap();
        assert_eq!(renamed.name, "Choir");
        assert_eq!(renamed.last_message_id, Some(Snowflake::from_raw(310)));
        state.remove_group_participant(GROUP, THIRD);
        assert_eq!(
            state.direct_message_channel(GROUP).unwrap().recipients,
            [user(OTHER)]
        );
    }

    #[test]
    fn a_group_left_or_closed_goes_with_its_messages_and_late_answers_do_not_bring_it_back() {
        let mut state = ready_state();
        state.upsert_direct_message_channel(group("Band"));
        let mut posted = message(310, OTHER);
        posted.channel_id = GROUP;
        state.apply_event(GatewayEvent::MessageCreate(posted), NOW);
        assert_eq!(
            state.apply_event(GatewayEvent::DirectMessageChannelDelete { id: GROUP }, NOW),
            StateChange::ConversationRemoved { channel_id: GROUP }
        );
        assert_eq!(state.messages(GROUP), []);
        assert!(!state.has_unread(GROUP));
        assert_eq!(
            state.apply_group_answer(group("Late")),
            StateChange::Nothing
        );
        assert!(state.direct_message_channel(GROUP).is_none());
    }
}
