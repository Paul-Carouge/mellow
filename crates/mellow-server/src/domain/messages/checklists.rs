//! Checklists and progress bars in messages (ADR-0047), together "trackers": creating one with a
//! message, ticking items, editing items and updating a progress bar. Who may do what is decided
//! in `permissions` (`may_tick_checklist_items`, `may_edit_tracker`).

use mellow_protocol::{
    Checklist, ChecklistItem, ChecklistItemRequest, GatewayEvent, Message, NewProgressBar,
    Permissions, ProgressBar, SendMessageRequest, Snowflake,
};

use super::{MessageChannel, MessageError, Messages, publish};
use crate::domain::guilds::GuildError;
use crate::domain::message_rules::{
    MessageRuleViolation, validated_checklist, validated_checklist_item, validated_progress_bar,
};
use crate::domain::permissions::{may_edit_tracker, may_tick_checklist_items};
use crate::storage::Database;
use crate::storage::checklists::{self as storage, ChecklistChange, ChecklistOutcome, NewTracker};
use crate::storage::messages::StoredMessage;

/// The request's checklist or progress bar once valid: such a message carries nothing else of
/// its own, as a poll.
pub(super) fn checked_tracker(
    request: &SendMessageRequest,
) -> Result<Option<NewTracker>, MessageError> {
    let tracker = match (&request.checklist, &request.progress_bar) {
        (None, None) => return Ok(None),
        (Some(checklist), None) => NewTracker::Checklist(validated_checklist(checklist)?),
        (None, Some(bar)) => NewTracker::ProgressBar(validated_progress_bar(bar)?),
        (Some(_), Some(_)) => return Err(MessageRuleViolation::Checklist.into()),
    };
    let carries_something_else = !request.content.trim().is_empty()
        || !request.attachment_ids.is_empty()
        || request.reply_to_id.is_some()
        || request.forwarded_from.is_some()
        || request.poll.is_some();
    if carries_something_else {
        return Err(MessageRuleViolation::Checklist.into());
    }
    Ok(Some(tracker))
}

/// Sets the message's tracker as just created, as its first event shows it.
pub(super) fn show_new_tracker(message: &mut Message, tracker: &NewTracker) {
    match tracker {
        NewTracker::Checklist(checklist) => {
            message.checklist = Some(Checklist {
                title: checklist.title.clone(),
                items: (1..)
                    .zip(&checklist.items)
                    .map(|(id, text)| ChecklistItem {
                        id,
                        text: text.clone(),
                        checked: None,
                    })
                    .collect(),
                revision: 0,
            });
        }
        NewTracker::ProgressBar(bar) => {
            message.progress_bar = Some(ProgressBar {
                label: bar.label.clone(),
                value: bar.value,
                maximum: bar.maximum,
                revision: 0,
            });
        }
    }
}

/// Fills `checklist` and `progress_bar` of these messages, in three queries at most.
pub(super) async fn fill_trackers(
    database: &Database,
    messages: &mut [Message],
) -> Result<(), MessageError> {
    let ids: Vec<Snowflake> = messages.iter().map(|message| message.id).collect();
    let (mut checklists, mut bars) = storage::trackers_of_messages(database, &ids).await?;
    for message in messages {
        message.checklist = checklists.remove(&message.id);
        message.progress_bar = bars.remove(&message.id);
    }
    Ok(())
}

impl Messages {
    /// The channel, the user's permissions there and the message, for a user who reads it.
    async fn tracker_target(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Result<(MessageChannel, Permissions, StoredMessage), MessageError> {
        let (channel, permissions) = self
            .text_channel_and_permissions_requiring(
                user_id,
                channel_id,
                Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY,
            )
            .await?;
        let message = self.message_in_channel(channel.id(), message_id).await?;
        Ok((channel, permissions, message))
    }

    /// The channel and message of a tracker the user may edit (`may_edit_tracker`).
    async fn editable_tracker(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Result<(MessageChannel, StoredMessage), MessageError> {
        let (channel, permissions, message) =
            self.tracker_target(user_id, channel_id, message_id).await?;
        if !may_edit_tracker(permissions, message.author_id == user_id) {
            return Err(GuildError::MissingPermissions.into());
        }
        Ok((channel, message))
    }

    /// Ticks (`is_ticked`) or unticks an item; again is a no-op, and a ticked item keeps who
    /// ticked it first. Answers the checklist as it is now.
    pub(crate) async fn tick_checklist_item(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        item_id: u32,
        is_ticked: bool,
        now_unix_milliseconds: u64,
    ) -> Result<Checklist, MessageError> {
        let (channel, permissions, message) =
            self.tracker_target(user_id, channel_id, message_id).await?;
        if !may_tick_checklist_items(permissions) {
            return Err(GuildError::MissingPermissions.into());
        }
        let change = if is_ticked {
            ChecklistChange::Tick {
                item_id,
                user_id,
                at_unix_milliseconds: now_unix_milliseconds,
            }
        } else {
            ChecklistChange::Untick { item_id }
        };
        self.change_checklist(&channel, message.id, change).await
    }

    /// Adds an item at the end; its author or a moderator only.
    pub(crate) async fn add_checklist_item(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        request: &ChecklistItemRequest,
    ) -> Result<Checklist, MessageError> {
        let text = validated_checklist_item(&request.text)?;
        let (channel, message) = self
            .editable_tracker(user_id, channel_id, message_id)
            .await?;
        self.change_checklist(&channel, message.id, ChecklistChange::AddItem { text })
            .await
    }

    /// Renames an item, keeping its tick; its author or a moderator only.
    pub(crate) async fn rename_checklist_item(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        item_id: u32,
        request: &ChecklistItemRequest,
    ) -> Result<Checklist, MessageError> {
        let text = validated_checklist_item(&request.text)?;
        let (channel, message) = self
            .editable_tracker(user_id, channel_id, message_id)
            .await?;
        self.change_checklist(
            &channel,
            message.id,
            ChecklistChange::RenameItem { item_id, text },
        )
        .await
    }

    /// Removes an item, never the last one; its author or a moderator only.
    pub(crate) async fn remove_checklist_item(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        item_id: u32,
    ) -> Result<Checklist, MessageError> {
        let (channel, message) = self
            .editable_tracker(user_id, channel_id, message_id)
            .await?;
        self.change_checklist(
            &channel,
            message.id,
            ChecklistChange::RemoveItem { item_id },
        )
        .await
    }

    async fn change_checklist(
        &self,
        channel: &MessageChannel,
        message_id: Snowflake,
        change: ChecklistChange,
    ) -> Result<Checklist, MessageError> {
        // Decided before the write, so nothing can fail between the commit and the event.
        let audience = self.audience(channel).await?;
        match storage::change_checklist(&self.database, message_id, &change).await? {
            ChecklistOutcome::Changed(checklist) => {
                publish(
                    &self.guilds,
                    &audience,
                    &GatewayEvent::MessageChecklistUpdate {
                        guild_id: channel.guild_id(),
                        channel_id: channel.id(),
                        message_id,
                        checklist: checklist.clone(),
                    },
                );
                Ok(checklist)
            }
            ChecklistOutcome::Unchanged(checklist) => Ok(checklist),
            ChecklistOutcome::NoSuchItem | ChecklistOutcome::NoChecklist => {
                Err(MessageError::MessageNotFound)
            }
            ChecklistOutcome::Full | ChecklistOutcome::LastItem => {
                Err(MessageRuleViolation::Checklist.into())
            }
        }
    }

    /// Replaces a progress bar's label and value; its author or a moderator only.
    pub(crate) async fn update_progress_bar(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        request: &NewProgressBar,
    ) -> Result<ProgressBar, MessageError> {
        let bar = validated_progress_bar(request)?;
        let (channel, message) = self
            .editable_tracker(user_id, channel_id, message_id)
            .await?;
        let audience = self.audience(&channel).await?;
        let bar = storage::update_progress_bar(&self.database, message.id, &bar)
            .await?
            .ok_or(MessageError::MessageNotFound)?;
        publish(
            &self.guilds,
            &audience,
            &GatewayEvent::MessageProgressBarUpdate {
                guild_id: channel.guild_id(),
                channel_id: channel.id(),
                message_id: message.id,
                progress_bar: bar.clone(),
            },
        );
        Ok(bar)
    }
}
