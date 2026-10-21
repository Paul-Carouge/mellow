//! Avatars and profile banners (ADR-0021): uploaded pictures fitted to their display size and
//! stored as lossless WebP, animated ones with a still first frame; each version its own id.

use std::sync::Arc;
use std::time::Duration;

use mellow_protocol::{CurrentUser, GatewayEvent, ProfileImage, PublicUser, Snowflake};
use sqlx::SqlitePool;

use crate::domain::attachments::Attachments;
use crate::domain::image_processing::{
    ImageProcessingError, ProfileImageKind, process_profile_image,
};
use crate::domain::messages::MessageError;
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::events::EventHub;
use crate::storage::attachment_files::AttachmentFiles;
use crate::storage::messages::MessageStorageError;
use crate::storage::profile_images::{self as storage, NewProfileImage, ProfileImageSlot};

/// Past this, the picture is refused rather than holding a worker (decoding included).
const PROCESSING_TIME_BUDGET: Duration = Duration::from_secs(20);

impl From<ImageProcessingError> for MessageError {
    fn from(error: ImageProcessingError) -> Self {
        match error {
            ImageProcessingError::Unsupported | ImageProcessingError::TooLarge => {
                crate::domain::message_rules::MessageRuleViolation::Picture.into()
            }
            ImageProcessingError::Encoding => Self::Internal(Box::new(error)),
        }
    }
}

/// Adds each user's current avatar and banner to user payloads built from other tables.
pub(crate) async fn fill_profile_images<'a>(
    pool: &SqlitePool,
    users: impl IntoIterator<Item = &'a mut PublicUser>,
) -> Result<(), MessageStorageError> {
    let mut users: Vec<&mut PublicUser> = users.into_iter().collect();
    let ids: Vec<Snowflake> = users.iter().map(|user| user.id).collect();
    let images = storage::profile_images_of_users(pool, &ids).await?;
    for user in &mut users {
        let found = images.get(&user.id).copied().unwrap_or_default();
        user.avatar = found.avatar;
        user.banner = found.banner;
    }
    Ok(())
}

/// The public part of the signed-in user, for `USER_UPDATE`.
pub(crate) fn public_user(user: &CurrentUser) -> PublicUser {
    PublicUser {
        id: user.id,
        username: user.username.clone(),
        display_name: user.display_name.clone(),
        avatar: user.avatar,
        banner: user.banner,
    }
}

#[derive(Debug)]
pub(crate) struct ProfileImages {
    pool: SqlitePool,
    ids: Arc<SnowflakeGenerator>,
    files: AttachmentFiles,
    attachments: Arc<Attachments>,
    events: Arc<EventHub>,
}

impl ProfileImages {
    pub(crate) fn new(
        pool: SqlitePool,
        ids: Arc<SnowflakeGenerator>,
        files: AttachmentFiles,
        attachments: Arc<Attachments>,
        events: Arc<EventHub>,
    ) -> Self {
        Self {
            pool,
            ids,
            files,
            attachments,
            events,
        }
    }

    /// Processes and stores a new avatar or banner, removes the one it replaces, and tells the
    /// user's sessions and guilds (`USER_UPDATE`). `None` removes the current one.
    pub(crate) async fn replace(
        &self,
        user: CurrentUser,
        slot: ProfileImageSlot,
        bytes: Option<Vec<u8>>,
        now_unix_milliseconds: u64,
    ) -> Result<CurrentUser, MessageError> {
        let new_image = match bytes {
            Some(bytes) => Some(self.store(slot, bytes, now_unix_milliseconds).await?),
            None => None,
        };
        let replaced = storage::replace_profile_image(
            &self.pool,
            user.id,
            slot,
            new_image,
            now_unix_milliseconds,
        )
        .await;
        let replaced = match replaced {
            Ok(replaced) => replaced,
            Err(error) => {
                if let Some(new_image) = new_image {
                    self.delete_files(new_image.id, new_image.still_id).await;
                }
                return Err(error.into());
            }
        };
        if let Some(replaced) = replaced {
            self.delete_files(replaced.id, replaced.still_id).await;
        }
        let new_profile_image = new_image.map(|image| ProfileImage {
            id: image.id,
            is_animated: image.still_id.is_some(),
        });
        let mut updated = user;
        match slot {
            ProfileImageSlot::Avatar => updated.avatar = new_profile_image,
            ProfileImageSlot::Banner => updated.banner = new_profile_image,
        }
        // Their guilds' members show the new picture at once, as for a rename.
        let guild_ids: Vec<Snowflake> =
            crate::storage::guilds::list_user_guilds(&self.pool, updated.id)
                .await
                .map_err(crate::domain::guilds::GuildError::from)?
                .into_iter()
                .map(|(guild_id, _)| guild_id)
                .collect();
        self.events.publish_to_user_and_guilds(
            updated.id,
            &guild_ids,
            &GatewayEvent::UserUpdate(public_user(&updated)),
        );
        Ok(updated)
    }

    /// Processes the picture and writes its files.
    async fn store(
        &self,
        slot: ProfileImageSlot,
        bytes: Vec<u8>,
        now_unix_milliseconds: u64,
    ) -> Result<NewProfileImage, MessageError> {
        let _permit = self.attachments.transfer_permit()?;
        let kind = match slot {
            ProfileImageSlot::Avatar => ProfileImageKind::Avatar,
            ProfileImageSlot::Banner => ProfileImageKind::Banner,
        };
        let processing = tokio::task::spawn_blocking(move || {
            let processed = process_profile_image(&bytes, kind)?;
            let still = if processed.is_animated {
                Some(crate::domain::image_processing::first_frame_webp(
                    &processed.bytes,
                )?)
            } else {
                None
            };
            Ok::<_, ImageProcessingError>((processed, still))
        });
        let (processed, still) = tokio::time::timeout(PROCESSING_TIME_BUDGET, processing)
            .await
            .map_err(|_| MessageError::from(ImageProcessingError::TooLarge))?
            .map_err(|error| MessageError::Internal(Box::new(error)))??;
        let id = self.ids.next_id(now_unix_milliseconds)?;
        let size_bytes = u64::try_from(processed.bytes.len()).unwrap_or(u64::MAX);
        self.files.write(id, processed.bytes).await?;
        let still_id = match still {
            Some(still) => {
                let still_id = self.ids.next_id(now_unix_milliseconds)?;
                if let Err(error) = self.files.write(still_id, still).await {
                    self.delete_files(id, None).await;
                    return Err(error.into());
                }
                Some(still_id)
            }
            None => None,
        };
        Ok(NewProfileImage {
            id,
            still_id,
            size_bytes,
        })
    }

    /// Files that no row points at any more; a failure leaves them to the orphan sweep.
    async fn delete_files(&self, id: Snowflake, still_id: Option<Snowflake>) {
        let _ = self.files.delete(id).await;
        if let Some(still_id) = still_id {
            let _ = self.files.delete(still_id).await;
        }
    }

    /// The bytes of a version, or of its still frame; any signed-in user may read them.
    pub(crate) async fn read(
        &self,
        id: Snowflake,
        is_still_wanted: bool,
    ) -> Result<Vec<u8>, MessageError> {
        let image = storage::find_profile_image(&self.pool, id)
            .await?
            .ok_or(MessageError::MessageNotFound)?;
        let file_id = if is_still_wanted {
            image.still_id.unwrap_or(image.id)
        } else {
            image.id
        };
        let _permit = self.attachments.transfer_permit()?;
        Ok(self.files.read(file_id).await?)
    }
}
