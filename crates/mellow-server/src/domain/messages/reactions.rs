//! Emoji reactions on messages. Who may react is decided by `permissions::may_react`
//! (ADR-0007); events reach the channel's audience only (ADR-0015).

use mellow_protocol::{
    AuditAction, GatewayEvent, MessageReaction, Permissions, PublicUser, ReactionUsersQuery,
    Snowflake,
};

use super::{MessageError, Messages, publish};
use crate::domain::guilds::GuildError;
use crate::domain::message_rules::{
    MAXIMUM_DISTINCT_EMOJI_PER_MESSAGE, MessageRuleViolation, custom_emoji_of_reaction_key,
    reacting_users_page_size, reaction_emoji_key,
};
use crate::domain::permissions::{ReactionAction, may_react, may_react_with_custom_emoji};
use crate::domain::profile_images::fill_profile_images;
use crate::storage::guild_emojis::find_guild_emoji;
use crate::storage::reactions::{self as storage, ReactionAddOutcome, ReactionSelection};

/// What a moderator removes from a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReactionRemoval {
    /// One user's reaction with one emoji.
    User { user_id: Snowflake, emoji: String },
    /// Every reaction with one emoji.
    Emoji(String),
    /// Every reaction of the message.
    All,
}

impl Messages {
    /// The channel, the user's permissions there and the emoji's stored key, once `action` is
    /// allowed (`reaction_channel`).
    async fn reaction_target(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
        action: ReactionAction,
    ) -> Result<(super::MessageChannel, Permissions, String), MessageError> {
        let emoji = reaction_emoji_key(emoji)?;
        let (channel, permissions) = self
            .reaction_channel(user_id, channel_id, message_id, action)
            .await?;
        Ok((channel, permissions, emoji))
    }

    /// The channel and the user's permissions there, once `action` is allowed. A channel the user
    /// cannot view is `NotFound`. Without the channel's history, every action but taking one's
    /// own reaction back answers `MessageNotFound` before any lookup, so an existing message cannot
    /// be told apart from a missing one; otherwise the message must exist in the channel.
    async fn reaction_channel(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        action: ReactionAction,
    ) -> Result<(super::MessageChannel, Permissions), MessageError> {
        let (channel, permissions) = self
            .text_channel_and_permissions_requiring(user_id, channel_id, Permissions::VIEW_CHANNEL)
            .await?;
        let needs_the_message = action != ReactionAction::RemoveOwn;
        if needs_the_message && !permissions.contains(Permissions::READ_MESSAGE_HISTORY) {
            return Err(MessageError::MessageNotFound);
        }
        if !may_react(permissions, channel.guild_id().is_none(), action) {
            return Err(GuildError::MissingPermissions.into());
        }
        if needs_the_message {
            self.message_in_channel(channel.id(), message_id).await?;
        }
        Ok((channel, permissions))
    }

    /// `key` itself for a Unicode emoji. A custom emoji must still exist and belong to the guild
    /// of the channel (`may_react_with_custom_emoji`); its key then takes the emoji's current
    /// name, whatever name the client sent (ADR-0028).
    async fn reaction_key_to_add(
        &self,
        key: String,
        channel_guild_id: Option<Snowflake>,
    ) -> Result<String, MessageError> {
        let Some((_, emoji_id)) = custom_emoji_of_reaction_key(&key) else {
            return Ok(key);
        };
        let stored = find_guild_emoji(&self.database, emoji_id)
            .await
            .map_err(GuildError::from)?;
        match stored {
            Some(stored) if may_react_with_custom_emoji(stored.guild_id, channel_guild_id) => {
                Ok(format!("{}:{emoji_id}", stored.emoji.name))
            }
            _ => Err(MessageRuleViolation::CustomReactionEmoji.into()),
        }
    }

    /// Reacts with `emoji`. Idempotent: reacting again changes nothing and sends no event.
    pub(crate) async fn add_reaction(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
    ) -> Result<(), MessageError> {
        let (channel, permissions, emoji) = self
            .reaction_target(
                user_id,
                channel_id,
                message_id,
                emoji,
                ReactionAction::JoinExisting,
            )
            .await?;
        let emoji = self.reaction_key_to_add(emoji, channel.guild_id()).await?;
        // Decided before the write, so publishing after the commit cannot fail (ADR-0015).
        let audience = self.audience(&channel).await?;
        match storage::add_reaction(
            &self.database,
            message_id,
            user_id,
            &emoji,
            may_react(
                permissions,
                channel.guild_id().is_none(),
                ReactionAction::AddNew,
            ),
            MAXIMUM_DISTINCT_EMOJI_PER_MESSAGE,
        )
        .await?
        {
            ReactionAddOutcome::Added => {
                publish(
                    &self.guilds,
                    &audience,
                    &GatewayEvent::MessageReactionAdd(MessageReaction {
                        guild_id: channel.guild_id(),
                        channel_id: channel.id(),
                        message_id,
                        user_id,
                        emoji: emoji.clone(),
                    }),
                );
                Ok(())
            }
            ReactionAddOutcome::AlreadyThere => Ok(()),
            ReactionAddOutcome::NewEmojiRefused => Err(GuildError::MissingPermissions.into()),
            ReactionAddOutcome::LimitReached => {
                Err(GuildError::LimitReached("distinct emoji per message").into())
            }
            ReactionAddOutcome::MessageNotFound => Err(MessageError::MessageNotFound),
        }
    }

    /// Takes the user's own reaction back. Idempotent: nothing to remove (a missing message
    /// included) sends no event and is no error.
    pub(crate) async fn remove_own_reaction(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
    ) -> Result<(), MessageError> {
        let (channel, _, emoji) = self
            .reaction_target(
                user_id,
                channel_id,
                message_id,
                emoji,
                ReactionAction::RemoveOwn,
            )
            .await?;
        let audience = self.audience(&channel).await?;
        if storage::remove_reaction(&self.database, channel.id(), message_id, user_id, &emoji)
            .await?
        {
            publish(
                &self.guilds,
                &audience,
                &GatewayEvent::MessageReactionRemove(MessageReaction {
                    guild_id: channel.guild_id(),
                    channel_id: channel.id(),
                    message_id,
                    user_id,
                    emoji,
                }),
            );
        }
        Ok(())
    }

    /// A moderator removes reactions of other people (`MANAGE_MESSAGES`, guild channels only),
    /// audited when something was removed. Removing one's own reaction this way is
    /// `remove_own_reaction`. Idempotent: nothing to remove sends no event.
    pub(crate) async fn remove_reactions(
        &self,
        moderator_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        removal: ReactionRemoval,
    ) -> Result<(), MessageError> {
        let removal = match removal {
            ReactionRemoval::User { user_id, emoji } if user_id == moderator_id => {
                return self
                    .remove_own_reaction(moderator_id, channel_id, message_id, &emoji)
                    .await;
            }
            ReactionRemoval::User { user_id, emoji } => ReactionRemoval::User {
                user_id,
                emoji: reaction_emoji_key(&emoji)?,
            },
            ReactionRemoval::Emoji(emoji) => ReactionRemoval::Emoji(reaction_emoji_key(&emoji)?),
            ReactionRemoval::All => ReactionRemoval::All,
        };
        let (channel, _) = self
            .reaction_channel(
                moderator_id,
                channel_id,
                message_id,
                ReactionAction::RemoveOthers,
            )
            .await?;
        let audience = self.audience(&channel).await?;
        let audit = channel
            .guild_id()
            .map(|guild_id| {
                self.guilds.audit_entry(
                    guild_id,
                    moderator_id,
                    AuditAction::MessageReactionRemove,
                    Some(message_id),
                    Some(channel.id()),
                )
            })
            .transpose()?;
        let selection = match &removal {
            ReactionRemoval::User { user_id, emoji } => ReactionSelection::OneUser {
                user_id: *user_id,
                emoji,
            },
            ReactionRemoval::Emoji(emoji) => ReactionSelection::OneEmoji(emoji),
            ReactionRemoval::All => ReactionSelection::All,
        };
        if !storage::remove_reactions_of_message(
            &self.database,
            message_id,
            selection,
            audit.as_ref(),
        )
        .await?
        {
            return Ok(());
        }
        let (guild_id, channel_id) = (channel.guild_id(), channel.id());
        let event = match removal {
            ReactionRemoval::User { user_id, emoji } => {
                GatewayEvent::MessageReactionRemove(MessageReaction {
                    guild_id,
                    channel_id,
                    message_id,
                    user_id,
                    emoji,
                })
            }
            ReactionRemoval::Emoji(emoji) => GatewayEvent::MessageReactionRemoveEmoji {
                guild_id,
                channel_id,
                message_id,
                emoji,
            },
            ReactionRemoval::All => GatewayEvent::MessageReactionRemoveAll {
                guild_id,
                channel_id,
                message_id,
            },
        };
        publish(&self.guilds, &audience, &event);
        Ok(())
    }

    /// Who reacted with `emoji`, by ascending user id, one page at a time.
    pub(crate) async fn list_reacting_users(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
        query: ReactionUsersQuery,
    ) -> Result<Vec<PublicUser>, MessageError> {
        let limit = reacting_users_page_size(query.limit)?;
        let (_, _, emoji) = self
            .reaction_target(user_id, channel_id, message_id, emoji, ReactionAction::List)
            .await?;
        let mut users: Vec<PublicUser> =
            storage::list_reacting_users(&self.database, message_id, &emoji, query.after, limit)
                .await?
                .into_iter()
                .map(|user| PublicUser {
                    id: user.id,
                    username: user.username,
                    display_name: user.display_name,
                    avatar: None,
                    banner: None,
                })
                .collect();
        fill_profile_images(&self.database, users.iter_mut()).await?;
        Ok(users)
    }
}
