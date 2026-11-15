//! What private calls need from conversations (ADR-0062): who may take part and who rings,
//! decided by `permissions`, and the call's message, its trace in the conversation. The live call
//! itself is the voice service's.

use std::collections::HashSet;

use mellow_protocol::{GatewayEvent, Message, MessageCall, Snowflake};

use super::{
    MessageAudience, MessageChannel, MessageError, Messages, direct_messages, is_empty_channel,
    message_from_stored, one_to_one_conversation_views, publish,
};
use crate::domain::guilds::GuildError;
use crate::domain::message_rules::MessageRuleViolation;
use crate::domain::permissions::{
    direct_message_permissions, may_take_part_in_call, rings_in_call,
};
use crate::domain::profile_images::{fill_profile_images, profile_images_of_user};
use crate::storage::Database;
use crate::storage::call_messages as storage;
use crate::storage::direct_messages as direct_message_storage;
use crate::storage::messages::{self as message_storage, MessageMentions, NewMessage};
use crate::storage::relationships as relationship_storage;

/// Whether a user may take part in a conversation's call, read fresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallAccess {
    /// They may start, answer or join it; the call's events go to `participant_ids`, the
    /// conversation's participants, them included.
    Allowed {
        participant_ids: Vec<Snowflake>,
        is_group: bool,
    },
    /// Not a conversation, not theirs, or they may no longer write there.
    Refused,
}

/// Who a new call rings (`Messages::call_ringing_plan`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallRingingPlan {
    /// Everyone but the caller, as the conversation sees the call ringing.
    pub(crate) rung: Vec<Snowflake>,
    /// Listed, but their devices never ring: a block either way with the caller.
    pub(crate) silent: HashSet<Snowflake>,
    /// Neither silent nor the caller's friends: rung within their budget.
    pub(crate) strangers: HashSet<Snowflake>,
}

/// A conversation's people and who of them may take part in its call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallMembers {
    pub(crate) participant_ids: Vec<Snowflake>,
    pub(crate) allowed_ids: HashSet<Snowflake>,
}

/// Fills `call` of these messages, in two queries at most.
pub(super) async fn fill_calls(
    database: &Database,
    messages: &mut [Message],
) -> Result<(), MessageError> {
    let ids: Vec<Snowflake> = messages.iter().map(|message| message.id).collect();
    let mut calls = storage::calls_of_messages(database, &ids).await?;
    for message in messages {
        message.call = calls.remove(&message.id);
    }
    Ok(())
}

impl Messages {
    /// Whether `user_id` may take part in a call of `channel_id` (`may_take_part_in_call`).
    pub(crate) async fn call_access(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
    ) -> Result<CallAccess, MessageError> {
        let participant_ids =
            direct_message_storage::list_participant_ids(&self.database, channel_id)
                .await
                .map_err(GuildError::from)?;
        if !participant_ids.contains(&user_id) {
            return Ok(CallAccess::Refused);
        }
        let is_group = direct_message_storage::group_owner_id(&self.database, channel_id)
            .await
            .map_err(GuildError::from)?
            .is_some();
        let may_reach_each_other = !is_group
            && direct_messages::may_reach_each(&self.database, user_id, &participant_ids).await?;
        let permissions = direct_message_permissions(true, is_group, may_reach_each_other);
        Ok(if may_take_part_in_call(permissions) {
            CallAccess::Allowed {
                participant_ids,
                is_group,
            }
        } else {
            CallAccess::Refused
        })
    }

    /// Whom a call `caller_id` starts rings among `participant_ids`: everyone but the caller is
    /// listed (`rung`); those with a block either way with the caller ring silently
    /// (`rings_in_call` false), and those who are not the caller's friends are `strangers`, whose
    /// rings the voice service counts. One query each for blocks and friends.
    pub(crate) async fn call_ringing_plan(
        &self,
        caller_id: Snowflake,
        participant_ids: &[Snowflake],
    ) -> Result<CallRingingPlan, MessageError> {
        let blocked = self
            .ids_with_a_block_with(caller_id, participant_ids)
            .await?;
        let friends: HashSet<Snowflake> =
            relationship_storage::friend_ids(&self.database, caller_id)
                .await
                .map_err(GuildError::from)?
                .into_iter()
                .collect();
        let ringing_ids: Vec<Snowflake> = participant_ids
            .iter()
            .copied()
            .filter(|id| *id != caller_id)
            .collect();
        let silent_ids: HashSet<Snowflake> = ringing_ids
            .iter()
            .copied()
            .filter(|id| !rings_in_call(false, blocked.contains(id)))
            .collect();
        let stranger_ids = ringing_ids
            .iter()
            .copied()
            .filter(|id| !silent_ids.contains(id) && !friends.contains(id))
            .collect();
        Ok(CallRingingPlan {
            rung: ringing_ids,
            silent: silent_ids,
            strangers: stranger_ids,
        })
    }

    /// Those of `other_ids` with a block either way with `user_id`.
    pub(crate) async fn ids_with_a_block_with(
        &self,
        user_id: Snowflake,
        other_ids: &[Snowflake],
    ) -> Result<HashSet<Snowflake>, MessageError> {
        Ok(
            relationship_storage::ids_with_a_block_either_way(&self.database, user_id, other_ids)
                .await
                .map_err(GuildError::from)?,
        )
    }

    /// Who is in the conversation `channel_id` and who of them may take part in its call, in one
    /// pass (`may_take_part_in_call`): a group allows all its participants, a one-to-one
    /// conversation both or neither, from one reachability read.
    pub(crate) async fn conversation_call_members(
        &self,
        channel_id: Snowflake,
    ) -> Result<CallMembers, MessageError> {
        let participant_ids =
            direct_message_storage::list_participant_ids(&self.database, channel_id)
                .await
                .map_err(GuildError::from)?;
        let is_group = direct_message_storage::group_owner_id(&self.database, channel_id)
            .await
            .map_err(GuildError::from)?
            .is_some();
        let may_reach_each_other = match (is_group, participant_ids.as_slice()) {
            (false, [first, second]) => {
                relationship_storage::may_reach_each_other(&self.database, *first, *second)
                    .await
                    .map_err(GuildError::from)?
            }
            _ => false,
        };
        let permissions = direct_message_permissions(true, is_group, may_reach_each_other);
        let allowed_ids = if may_take_part_in_call(permissions) {
            participant_ids.iter().copied().collect()
        } else {
            HashSet::new()
        };
        Ok(CallMembers {
            participant_ids,
            allowed_ids,
        })
    }

    /// Records that `user_id` joined the call of `message_id`, at their first join: a server stop
    /// then leaves an honest record (ADR-0062).
    pub(crate) async fn record_call_participant(
        &self,
        message_id: Snowflake,
        user_id: Snowflake,
    ) -> Result<(), MessageError> {
        Ok(storage::record_participant(&self.database, message_id, user_id).await?)
    }

    /// Posts the message of a call `caller_id` starts in the conversation `participant_ids` are
    /// in, as the conversation's next message (`send`'s order), announcing a one-to-one
    /// conversation to the callee when it is its first message. The caller's access is checked
    /// beforehand (`call_access`).
    pub(crate) async fn post_call_message(
        &self,
        caller_id: Snowflake,
        channel_id: Snowflake,
        participant_ids: Vec<Snowflake>,
        now_unix_milliseconds: u64,
    ) -> Result<Message, MessageError> {
        let is_group = direct_message_storage::group_owner_id(&self.database, channel_id)
            .await
            .map_err(GuildError::from)?
            .is_some();
        let channel = MessageChannel::Direct {
            id: channel_id,
            participant_ids: participant_ids.clone(),
            is_group,
        };
        let conversation_views = one_to_one_conversation_views(&self.database, &channel).await?;
        let audience = MessageAudience::Participants(participant_ids);
        let author_images = profile_images_of_user(&self.database, caller_id).await?;
        let _send_order = self.send_order.clone().lock_owned().await;
        let is_first_direct_message =
            !conversation_views.is_empty() && is_empty_channel(&self.database, channel_id).await?;
        let stored = message_storage::insert_message(
            &self.database,
            &NewMessage {
                id: self.ids.next_id(now_unix_milliseconds)?,
                channel_id,
                author_id: caller_id,
                content: String::new(),
                reply_to_id: None,
                nonce: None,
                attachment_ids: Vec::new(),
                mentions: MessageMentions::default(),
                forward: None,
                poll: None,
                tracker: None,
                is_call: true,
            },
        )
        .await?;
        if is_first_direct_message {
            direct_messages::announce_conversation(&self.guilds, conversation_views, stored.id);
        }
        let mut message = message_from_stored(stored, None, Vec::new());
        message.author.avatar = author_images.avatar;
        message.author.banner = author_images.banner;
        message.call = Some(MessageCall {
            participant_ids: Vec::new(),
            ended_at_unix_milliseconds: None,
        });
        publish(
            &self.guilds,
            &audience,
            &GatewayEvent::MessageCreate(message.clone()),
        );
        Ok(message)
    }

    /// Writes who joined the call of `message_id` and when it ended, and shows it to the
    /// conversation's participants (`MESSAGE_UPDATE`). Nothing when the message was deleted
    /// meanwhile, or the call already ended.
    pub(crate) async fn end_call_message(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        joined_ids: &[Snowflake],
        now_unix_milliseconds: u64,
    ) -> Result<(), MessageError> {
        if !storage::end_call(
            &self.database,
            message_id,
            joined_ids,
            now_unix_milliseconds,
        )
        .await?
        {
            return Ok(());
        }
        let Some(stored) =
            message_storage::find_message_in_channel(&self.database, channel_id, message_id)
                .await?
        else {
            return Ok(());
        };
        let mut message = message_from_stored(stored, None, Vec::new());
        fill_profile_images(&self.database, [&mut message.author]).await?;
        fill_calls(&self.database, std::slice::from_mut(&mut message)).await?;
        let participant_ids =
            direct_message_storage::list_participant_ids(&self.database, channel_id)
                .await
                .map_err(GuildError::from)?;
        publish(
            &self.guilds,
            &MessageAudience::Participants(participant_ids),
            &GatewayEvent::MessageUpdate(message),
        );
        Ok(())
    }

    /// Closes the calls a server stop left unfinished (ADR-0062); `serve` calls it before the
    /// first connection, so no event is due.
    pub(crate) async fn end_unfinished_calls(
        &self,
        now_unix_milliseconds: u64,
    ) -> Result<u64, MessageError> {
        Ok(storage::end_unfinished_calls(&self.database, now_unix_milliseconds).await?)
    }

    /// Refuses editing or forwarding a call's message: its record is the server's.
    pub(super) async fn require_not_a_call_message(
        &self,
        message_id: Snowflake,
    ) -> Result<(), MessageError> {
        if storage::is_call_message(&self.database, message_id).await? {
            return Err(MessageRuleViolation::CallChange.into());
        }
        Ok(())
    }
}
