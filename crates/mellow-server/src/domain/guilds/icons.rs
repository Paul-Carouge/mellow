//! Guild and channel icons: square pictures stored small, since they are shown small (ADR-0021).

use mellow_protocol::{
    AuditAction, Channel, ChannelKind, GatewayEvent, Guild, Permissions, Snowflake,
};

use super::{GuildError, Guilds, channel_from_stored};
use crate::domain::guild_rules::GuildRuleViolation;
use crate::domain::image_processing::{IconKind, SquareCrop, process_icon};
use crate::domain::messages::MessageError;
use crate::domain::profile_images::ProfileImages;
use crate::storage::guild_images::{self as storage, IconChange};
use crate::storage::guilds::StoredChannel;
use crate::storage::profile_images::NewProfileImage;

/// An uploaded picture and the square of it to keep, the centred one when `None`.
pub(crate) struct IconUpload {
    pub(crate) bytes: Vec<u8>,
    pub(crate) crop: Option<SquareCrop>,
}

impl Guilds {
    /// Needs `MANAGE_GUILD`. `None` removes the icon. Every member sees the change
    /// (`GUILD_UPDATE`).
    pub(crate) async fn replace_guild_icon(
        &self,
        images: &ProfileImages,
        user_id: Snowflake,
        guild_id: Snowflake,
        upload: Option<IconUpload>,
        now_unix_milliseconds: u64,
    ) -> Result<Guild, MessageError> {
        // Checked before processing too, so nobody else can spend processing slots.
        self.require_member_permissions(guild_id, user_id, Permissions::MANAGE_GUILD)
            .await?;
        let new_image = store_icon(images, upload, IconKind::Guild, now_unix_milliseconds).await?;
        // Processing takes seconds: the permission may have been taken away meanwhile.
        let checked_again = self
            .require_member_permissions(guild_id, user_id, Permissions::MANAGE_GUILD)
            .await;
        if let Err(error) = checked_again {
            discard(images, new_image).await;
            return Err(error.into());
        }
        let audit = self.audit_entry(guild_id, user_id, AuditAction::GuildUpdate, None, None)?;
        let change = storage::replace_guild_icon(
            &self.pool,
            guild_id,
            new_image,
            &audit,
            now_unix_milliseconds,
        )
        .await
        .map_err(GuildError::from);
        if let Some(summary) = keep_change_or_delete_new_files(images, new_image, change).await? {
            self.events
                .publish_to_guild(guild_id, &GatewayEvent::GuildUpdate(summary));
        }
        Ok(self.get_guild(user_id, guild_id).await?)
    }

    /// Needs `MANAGE_CHANNELS`; text channels only. `None` removes the icon. Whoever can view the
    /// channel sees the change (`CHANNEL_UPDATE`).
    pub(crate) async fn replace_channel_icon(
        &self,
        images: &ProfileImages,
        user_id: Snowflake,
        channel_id: Snowflake,
        upload: Option<IconUpload>,
        now_unix_milliseconds: u64,
    ) -> Result<Channel, MessageError> {
        // Checked before processing too, so nobody else can spend processing slots.
        self.channel_icon_editor_check(user_id, channel_id).await?;
        let new_image =
            store_icon(images, upload, IconKind::Channel, now_unix_milliseconds).await?;
        // Processing takes seconds: the permission, and who can view the channel, may have
        // changed meanwhile. The audience is decided before writing, so nothing can fail between
        // the write and its announcement (ADR-0015).
        let checked_again = async {
            let channel = self.channel_icon_editor_check(user_id, channel_id).await?;
            let audience = self.channel_audience(&channel).await?;
            let audit = self.audit_entry(
                channel.guild_id,
                user_id,
                AuditAction::ChannelUpdate,
                Some(channel.id),
                None,
            )?;
            Ok::<_, GuildError>((channel, audience, audit))
        }
        .await;
        let (channel, audience, audit) = match checked_again {
            Ok(checked) => checked,
            Err(error) => {
                discard(images, new_image).await;
                return Err(error.into());
            }
        };
        let change = storage::replace_channel_icon(
            &self.pool,
            channel.id,
            new_image,
            &audit,
            now_unix_milliseconds,
        )
        .await
        .map_err(GuildError::from);
        let Some(updated) = keep_change_or_delete_new_files(images, new_image, change).await?
        else {
            return Ok(channel_from_stored(channel));
        };
        let updated = channel_from_stored(updated);
        self.publish_to_audience(
            updated.guild_id,
            &audience,
            &GatewayEvent::ChannelUpdate(updated.clone()),
        );
        Ok(updated)
    }

    /// The text channel, if `user_id` may change its icon.
    async fn channel_icon_editor_check(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
    ) -> Result<StoredChannel, GuildError> {
        let (channel, permissions) = self
            .channel_and_member_permissions(user_id, channel_id)
            .await?;
        if !permissions.contains(Permissions::MANAGE_CHANNELS) {
            return Err(GuildError::MissingPermissions);
        }
        if channel.kind == ChannelKind::Category {
            return Err(GuildRuleViolation::CategoryIcon.into());
        }
        Ok(channel)
    }

    /// The file to send for an icon version: to members of its guild, and for a channel's icon
    /// only to those who can view the channel; `NotFound` for everyone else.
    pub(crate) async fn readable_icon_file_id(
        &self,
        user_id: Snowflake,
        id: Snowflake,
        is_still_wanted: bool,
    ) -> Result<Snowflake, GuildError> {
        let image = storage::find_guild_image(&self.pool, id)
            .await?
            .ok_or(GuildError::NotFound)?;
        match image.channel_id {
            Some(channel_id) => {
                self.channel_and_member_permissions(user_id, channel_id)
                    .await?;
            }
            None => {
                self.membership_and_permissions(image.guild_id, user_id)
                    .await?;
            }
        }
        Ok(if is_still_wanted {
            image.still_id.unwrap_or(image.id)
        } else {
            image.id
        })
    }
}

async fn store_icon(
    images: &ProfileImages,
    upload: Option<IconUpload>,
    kind: IconKind,
    now_unix_milliseconds: u64,
) -> Result<Option<NewProfileImage>, MessageError> {
    let Some(IconUpload { bytes, crop }) = upload else {
        return Ok(None);
    };
    let process = move |bytes: &[u8]| process_icon(bytes, kind, crop);
    Ok(Some(
        images.store(bytes, process, now_unix_milliseconds).await?,
    ))
}

/// Deletes the files no row points at after a replacement: the replaced version's once it is
/// done, the new version's when it was not stored. Answers what changed, `None` if nothing did.
async fn keep_change_or_delete_new_files<T>(
    images: &ProfileImages,
    new_image: Option<NewProfileImage>,
    change: Result<IconChange<T>, GuildError>,
) -> Result<Option<T>, MessageError> {
    let outcome = match change {
        Ok(IconChange::Changed { updated, replaced }) => {
            if let Some(replaced) = replaced {
                images.delete_files(replaced.id, replaced.still_id).await;
            }
            return Ok(Some(updated));
        }
        Ok(IconChange::Unchanged) => return Ok(None),
        Ok(IconChange::NotFound) => GuildError::NotFound,
        Err(error) => error,
    };
    discard(images, new_image).await;
    Err(outcome.into())
}

/// Deletes the files of a version that will not be stored.
async fn discard(images: &ProfileImages, new_image: Option<NewProfileImage>) {
    if let Some(new_image) = new_image {
        images.delete_files(new_image.id, new_image.still_id).await;
    }
}
