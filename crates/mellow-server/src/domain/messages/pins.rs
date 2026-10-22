//! Pinned messages. Who may pin is decided by `permissions::may_manage_pins` (ADR-0007); a pin or
//! unpin reaches the channel's audience as a `MESSAGE_UPDATE` (ADR-0015) and, in a guild, the
//! audit log.

use mellow_protocol::{AuditAction, GatewayEvent, Message, Permissions, Snowflake};

use super::{MessageError, Messages, attachments_of, message_from_stored, publish};
use crate::domain::guilds::GuildError;
use crate::domain::message_rules::MAXIMUM_PINS_PER_CHANNEL;
use crate::domain::permissions::may_manage_pins;
use crate::domain::profile_images::fill_profile_images;
use crate::storage::messages::{self as storage, PinOutcome, StoredMessage};

/// Whether the message ends up pinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PinChange {
    Pin { pinned_at_unix_milliseconds: u64 },
    Unpin,
}

impl Messages {
    /// Pins the message to its channel. Idempotent: pinning a pinned message changes nothing.
    pub(crate) async fn pin(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        now_unix_milliseconds: u64,
    ) -> Result<(), MessageError> {
        self.change_pin(
            user_id,
            channel_id,
            message_id,
            PinChange::Pin {
                pinned_at_unix_milliseconds: now_unix_milliseconds,
            },
        )
        .await
    }

    /// Unpins the message. Idempotent: unpinning a message that is not pinned changes nothing.
    pub(crate) async fn unpin(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Result<(), MessageError> {
        self.change_pin(user_id, channel_id, message_id, PinChange::Unpin)
            .await
    }

    async fn change_pin(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        change: PinChange,
    ) -> Result<(), MessageError> {
        let (channel, permissions) = self
            .text_channel_and_permissions_requiring(user_id, channel_id, Permissions::VIEW_CHANNEL)
            .await?;
        if !may_manage_pins(permissions, channel.guild_id().is_none()) {
            return Err(GuildError::MissingPermissions.into());
        }
        let message = self.message_in_channel(channel.id(), message_id).await?;
        let audience = self.audience(&channel).await?;
        let audit = channel
            .guild_id()
            .map(|guild_id| {
                self.guilds.audit_entry(
                    guild_id,
                    user_id,
                    match change {
                        PinChange::Pin { .. } => AuditAction::MessagePin,
                        PinChange::Unpin => AuditAction::MessageUnpin,
                    },
                    Some(message.id),
                    Some(channel.id()),
                )
            })
            .transpose()?;
        let changed: Option<StoredMessage> = match change {
            PinChange::Pin {
                pinned_at_unix_milliseconds,
            } => match storage::pin_message(
                &self.pool,
                channel.id(),
                message.id,
                pinned_at_unix_milliseconds,
                MAXIMUM_PINS_PER_CHANNEL,
                audit.as_ref(),
            )
            .await?
            {
                PinOutcome::Pinned(pinned) => Some(pinned),
                PinOutcome::AlreadyPinned => None,
                PinOutcome::LimitReached => {
                    return Err(GuildError::LimitReached("pins per channel").into());
                }
                PinOutcome::MessageNotFound => return Err(MessageError::MessageNotFound),
            },
            PinChange::Unpin => {
                storage::unpin_message(&self.pool, channel.id(), message.id, audit.as_ref()).await?
            }
        };
        if let Some(changed) = changed {
            let attachments = attachments_of(&self.pool, changed.id).await?;
            let mut changed = message_from_stored(changed, None, attachments);
            fill_profile_images(&self.pool, [&mut changed.author]).await?;
            publish(
                &self.guilds,
                &audience,
                &GatewayEvent::MessageUpdate(changed),
            );
        }
        Ok(())
    }

    /// The channel's pinned messages, the most recently pinned first. Needs `VIEW_CHANNEL` and
    /// `READ_MESSAGE_HISTORY`.
    pub(crate) async fn list_pins(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
    ) -> Result<Vec<Message>, MessageError> {
        let (channel, _) = self
            .text_channel_and_permissions_requiring(
                user_id,
                channel_id,
                Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY,
            )
            .await?;
        let pinned =
            storage::list_pinned_messages(&self.pool, channel.id(), MAXIMUM_PINS_PER_CHANNEL)
                .await?;
        self.messages_for_viewer(user_id, pinned).await
    }
}
