//! Direct-message conversations (ARCHITECTURE.md §4.5): opening one, listing them, and telling a
//! participant about a conversation before its first message reaches them.

use mellow_protocol::{
    DirectMessageChannel, GatewayEvent, OpenDirectMessageRequest, PublicUser, Snowflake,
};

use crate::storage::Database;

use super::{MessageError, Messages};
use crate::domain::guild_rules::GuildRuleViolation;
use crate::domain::guilds::{GuildError, Guilds};
use crate::domain::profile_images::fill_profile_images;
use crate::storage::direct_messages::{self as storage, StoredDirectMessageChannel};

fn channel_from_stored(stored: StoredDirectMessageChannel) -> DirectMessageChannel {
    DirectMessageChannel {
        id: stored.id,
        recipients: stored
            .recipients
            .into_iter()
            .map(|recipient| PublicUser {
                id: recipient.id,
                username: recipient.username,
                display_name: recipient.display_name,
                avatar: None,
                banner: None,
            })
            .collect(),
        last_message_id: stored.last_message_id,
    }
}

impl Messages {
    /// The one-to-one conversation with the recipient, opened if needed. Only with someone who
    /// shares a guild with the caller, so accounts cannot be found or spammed by id; anyone else
    /// is `NotFound`, like an unknown user. Writing there keeps needing a shared guild
    /// (`direct_message_permissions`). Only the caller hears of a new conversation now; the
    /// recipient does with its first message.
    pub(crate) async fn open_direct_message(
        &self,
        user_id: Snowflake,
        request: OpenDirectMessageRequest,
        now_unix_milliseconds: u64,
    ) -> Result<DirectMessageChannel, MessageError> {
        if request.recipient_id == user_id {
            return Err(GuildError::from(GuildRuleViolation::DirectMessageRecipient).into());
        }
        if !storage::share_a_guild(&self.database, user_id, request.recipient_id)
            .await
            .map_err(GuildError::from)?
        {
            return Err(GuildError::NotFound.into());
        }
        let (channel_id, is_new) = storage::find_or_create_one_to_one(
            &self.database,
            user_id,
            request.recipient_id,
            self.ids.next_id(now_unix_milliseconds)?,
        )
        .await
        .map_err(GuildError::from)?;
        let channel = conversation_seen_by(&self.database, user_id, channel_id)
            .await?
            .ok_or(GuildError::NotFound)?;
        if is_new {
            self.guilds.publish_to_users(
                &[user_id],
                &GatewayEvent::DirectMessageChannelCreate(channel.clone()),
            );
        }
        Ok(channel)
    }

    /// The conversations listed at `Ready`: those with at least one message, so one opened by
    /// someone else and left empty never shows up.
    pub(crate) async fn list_direct_message_channels(
        &self,
        user_id: Snowflake,
    ) -> Result<Vec<DirectMessageChannel>, MessageError> {
        let mut channels: Vec<DirectMessageChannel> =
            storage::list_user_conversations(&self.database, user_id)
                .await
                .map_err(GuildError::from)?
                .into_iter()
                .filter(|conversation| conversation.last_message_id.is_some())
                .map(channel_from_stored)
                .collect();
        fill_profile_images(
            &self.database,
            channels
                .iter_mut()
                .flat_map(|channel| channel.recipients.iter_mut()),
        )
        .await?;
        Ok(channels)
    }
}

async fn conversation_seen_by(
    database: &Database,
    user_id: Snowflake,
    channel_id: Snowflake,
) -> Result<Option<DirectMessageChannel>, MessageError> {
    let Some(mut channel) = storage::find_user_conversation(database, user_id, channel_id)
        .await
        .map_err(GuildError::from)?
        .map(channel_from_stored)
    else {
        return Ok(None);
    };
    fill_profile_images(database, channel.recipients.iter_mut()).await?;
    Ok(Some(channel))
}

/// Whether `user_id` shares a guild with every other participant: writing needs it.
pub(super) async fn shares_a_guild_with_each(
    database: &Database,
    user_id: Snowflake,
    participant_ids: &[Snowflake],
) -> Result<bool, MessageError> {
    for other_id in participant_ids.iter().filter(|id| **id != user_id) {
        if !storage::share_a_guild(database, user_id, *other_id)
            .await
            .map_err(GuildError::from)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The conversation as each participant sees it, for `announce_conversation`.
pub(super) async fn conversation_views(
    database: &Database,
    channel_id: Snowflake,
    participant_ids: &[Snowflake],
) -> Result<Vec<(Snowflake, DirectMessageChannel)>, MessageError> {
    let mut views = Vec::with_capacity(participant_ids.len());
    for participant_id in participant_ids {
        if let Some(view) = conversation_seen_by(database, *participant_id, channel_id).await? {
            views.push((*participant_id, view));
        }
    }
    Ok(views)
}

/// Right before the first message, every participant receives the conversation as they see it:
/// the recipient to show the message, the author's other sessions too, since `Ready` lists only
/// conversations with a message.
pub(super) fn announce_conversation(
    guilds: &Guilds,
    views: Vec<(Snowflake, DirectMessageChannel)>,
    first_message_id: Snowflake,
) {
    for (participant_id, mut view) in views {
        view.last_message_id = Some(first_message_id);
        guilds.publish_to_users(
            &[participant_id],
            &GatewayEvent::DirectMessageChannelCreate(view),
        );
    }
}
