//! Direct-message conversations (ARCHITECTURE.md §4.3): opening one, listing them, and telling a
//! participant about a conversation before its first message reaches them.

use mellow_protocol::{
    DirectMessageChannel, GatewayEvent, OpenDirectMessageRequest, PublicUser, Snowflake,
};

use sqlx::SqlitePool;

use super::{MessageError, Messages};
use crate::domain::guild_rules::GuildRuleViolation;
use crate::domain::guilds::{GuildError, Guilds};
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
            })
            .collect(),
        last_message_id: stored.last_message_id,
    }
}

impl Messages {
    /// The one-to-one conversation with the recipient, opened if needed. Only with someone who
    /// shares a guild with the caller, so accounts cannot be found or spammed by id; anyone else
    /// is `NotFound`, like an unknown user. Only the caller hears of a new conversation now; the
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
        if !storage::share_a_guild(&self.pool, user_id, request.recipient_id)
            .await
            .map_err(GuildError::from)?
        {
            return Err(GuildError::NotFound.into());
        }
        let (channel_id, is_new) = storage::find_or_create_one_to_one(
            &self.pool,
            user_id,
            request.recipient_id,
            self.ids.next_id(now_unix_milliseconds)?,
        )
        .await
        .map_err(GuildError::from)?;
        let channel = conversation_seen_by(&self.pool, user_id, channel_id)
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
        Ok(storage::list_user_conversations(&self.pool, user_id)
            .await
            .map_err(GuildError::from)?
            .into_iter()
            .filter(|conversation| conversation.last_message_id.is_some())
            .map(channel_from_stored)
            .collect())
    }
}

async fn conversation_seen_by(
    pool: &SqlitePool,
    user_id: Snowflake,
    channel_id: Snowflake,
) -> Result<Option<DirectMessageChannel>, MessageError> {
    Ok(storage::find_user_conversation(pool, user_id, channel_id)
        .await
        .map_err(GuildError::from)?
        .map(channel_from_stored))
}

/// Before the first message of a conversation, each other participant receives the conversation
/// itself, as they see it, so their client can show the message.
pub(super) async fn announce_conversation_to_recipients(
    pool: &SqlitePool,
    guilds: &Guilds,
    author_id: Snowflake,
    channel_id: Snowflake,
    participant_ids: &[Snowflake],
) -> Result<(), MessageError> {
    for participant_id in participant_ids.iter().filter(|id| **id != author_id) {
        if let Some(channel) = conversation_seen_by(pool, *participant_id, channel_id).await? {
            guilds.publish_to_users(
                &[*participant_id],
                &GatewayEvent::DirectMessageChannelCreate(channel),
            );
        }
    }
    Ok(())
}
