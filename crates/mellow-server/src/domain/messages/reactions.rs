//! Emoji reactions on messages. Who may react is decided by `permissions::may_react`
//! (ADR-0007); events reach the channel's audience only (ADR-0015).

use mellow_protocol::{
    GatewayEvent, MessageReaction, Permissions, PublicUser, ReactionUsersQuery, Snowflake,
};

use super::{MessageError, Messages, publish};
use crate::domain::guilds::GuildError;
use crate::domain::message_rules::{
    MAXIMUM_DISTINCT_EMOJI_PER_MESSAGE, reacting_users_page_size, reaction_emoji_key,
};
use crate::domain::permissions::{ReactionAction, may_react};
use crate::domain::profile_images::fill_profile_images;
use crate::storage::reactions::{self as storage, ReactionAddOutcome};

impl Messages {
    /// The channel, the message, the user's permissions there and the emoji's stored key. A
    /// channel the user cannot view is `NotFound`.
    async fn reaction_target(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
    ) -> Result<(super::MessageChannel, Snowflake, Permissions, String), MessageError> {
        let emoji = reaction_emoji_key(emoji)?;
        let (channel, permissions) = self
            .text_channel_and_permissions_requiring(user_id, channel_id, Permissions::VIEW_CHANNEL)
            .await?;
        let message = self.message_in_channel(channel.id(), message_id).await?;
        Ok((channel, message.id, permissions, emoji))
    }

    /// Reacts with `emoji`. Idempotent: reacting again changes nothing and sends no event.
    pub(crate) async fn add_reaction(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
    ) -> Result<(), MessageError> {
        let (channel, message_id, permissions, emoji) = self
            .reaction_target(user_id, channel_id, message_id, emoji)
            .await?;
        let may = |action| may_react(permissions, channel.guild_id().is_none(), action);
        if !may(ReactionAction::JoinExisting) {
            return Err(GuildError::MissingPermissions.into());
        }
        // Decided before the write, so publishing after the commit cannot fail (ADR-0015).
        let audience = self.audience(&channel).await?;
        match storage::add_reaction(
            &self.pool,
            message_id,
            user_id,
            &emoji,
            may(ReactionAction::AddNew),
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

    /// Takes the user's own reaction back. Idempotent: nothing to remove sends no event.
    pub(crate) async fn remove_own_reaction(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
    ) -> Result<(), MessageError> {
        let (channel, message_id, permissions, emoji) = self
            .reaction_target(user_id, channel_id, message_id, emoji)
            .await?;
        if !may_react(
            permissions,
            channel.guild_id().is_none(),
            ReactionAction::RemoveOwn,
        ) {
            return Err(GuildError::MissingPermissions.into());
        }
        let audience = self.audience(&channel).await?;
        if storage::remove_reaction(&self.pool, message_id, user_id, &emoji).await? {
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
        let (channel, message_id, permissions, emoji) = self
            .reaction_target(user_id, channel_id, message_id, emoji)
            .await?;
        if !may_react(
            permissions,
            channel.guild_id().is_none(),
            ReactionAction::List,
        ) {
            return Err(GuildError::MissingPermissions.into());
        }
        let mut users: Vec<PublicUser> =
            storage::list_reacting_users(&self.pool, message_id, &emoji, query.after, limit)
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
        fill_profile_images(&self.pool, users.iter_mut()).await?;
        Ok(users)
    }
}
