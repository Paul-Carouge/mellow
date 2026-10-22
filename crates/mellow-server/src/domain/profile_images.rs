//! Avatars and profile banners (ADR-0021): uploaded pictures fitted to their display size and
//! stored as lossless WebP, animated ones with a still first frame; each version its own id.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::OwnedSemaphorePermit;

use mellow_protocol::{CurrentUser, GatewayEvent, ProfileImage, PublicUser, Snowflake};
use sqlx::SqlitePool;

use crate::domain::attachments::Attachments;
use crate::domain::image_processing::{
    ImageProcessingError, ProcessedProfileImage, ProfileImageKind, process_profile_image,
};
use crate::domain::messages::MessageError;
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::events::EventHub;
use crate::storage::attachment_files::AttachmentFiles;
use crate::storage::messages::MessageStorageError;
use crate::storage::profile_images::{self as storage, NewProfileImage, ProfileImageSlot};

/// Past this, the client is told to try again later (decoding included).
const PROCESSING_TIME_BUDGET: Duration = Duration::from_secs(20);
/// Pictures read and held in memory at once until sent: a member list loads dozens together,
/// so a short wait for a slot beats an error on an image that never retries.
const CONCURRENT_READS: usize = 16;
const READ_SLOT_WAIT: Duration = Duration::from_secs(5);
/// A processed picture larger than this is refused: every viewer downloads it.
const MAXIMUM_STORED_BYTES: usize = 8 * 1024 * 1024;

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

/// One user's avatar and banner, read before a change is stored so that nothing can fail
/// between storing it and announcing it.
pub(crate) async fn profile_images_of_user(
    pool: &SqlitePool,
    user_id: Snowflake,
) -> Result<storage::UserProfileImages, MessageStorageError> {
    Ok(storage::profile_images_of_users(pool, &[user_id])
        .await?
        .remove(&user_id)
        .unwrap_or_default())
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
    reads: Arc<tokio::sync::Semaphore>,
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
            reads: Arc::new(tokio::sync::Semaphore::new(CONCURRENT_READS)),
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
        // Read before the change: once it is stored, nothing may fail before it is announced.
        let guild_ids: Vec<Snowflake> =
            crate::storage::guilds::list_user_guilds(&self.pool, user.id)
                .await
                .map_err(crate::domain::guilds::GuildError::from)?
                .into_iter()
                .map(|guild| guild.id)
                .collect();
        let kind = match slot {
            ProfileImageSlot::Avatar => ProfileImageKind::Avatar,
            ProfileImageSlot::Banner => ProfileImageKind::Banner,
        };
        let new_image = match bytes {
            Some(bytes) => Some(
                self.store(
                    bytes,
                    move |bytes| process_profile_image(bytes, kind),
                    now_unix_milliseconds,
                )
                .await?,
            ),
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
        let updated = self.user_after_change(user, slot, new_image).await;
        self.events.publish_to_user_and_guilds(
            updated.id,
            &guild_ids,
            &GatewayEvent::UserUpdate(public_user(&updated)),
        );
        Ok(updated)
    }

    /// The user as stored now, so a rename or the other picture changed meanwhile is kept; the
    /// request's copy with this change if it cannot be read.
    async fn user_after_change(
        &self,
        user: CurrentUser,
        slot: ProfileImageSlot,
        new_image: Option<NewProfileImage>,
    ) -> CurrentUser {
        match crate::storage::accounts::find_user_by_id(&self.pool, user.id).await {
            Ok(Some(stored)) => crate::domain::accounts::current_user(stored),
            stored => {
                if let Err(error) = stored {
                    tracing::warn!(%error, "cannot read back a user after a picture change");
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
                updated
            }
        }
    }

    /// Processes the picture with `process` (avatars, banners and icons alike) and writes its
    /// files.
    pub(crate) async fn store(
        &self,
        bytes: Vec<u8>,
        process: impl FnOnce(&[u8]) -> Result<ProcessedProfileImage, ImageProcessingError>
        + Send
        + 'static,
        now_unix_milliseconds: u64,
    ) -> Result<NewProfileImage, MessageError> {
        let permit = self.attachments.image_processing_permit()?;
        let processing = tokio::task::spawn_blocking(move || {
            // Held until the work really ends, even after the caller stopped waiting.
            let _permit = permit;
            process(&bytes)
        });
        let processed = tokio::time::timeout(PROCESSING_TIME_BUDGET, processing)
            .await
            .map_err(|_| MessageError::Busy)?
            .map_err(|error| MessageError::Internal(Box::new(error)))??;
        if processed.picture.bytes.len() > MAXIMUM_STORED_BYTES {
            return Err(ImageProcessingError::TooLarge.into());
        }
        let id = self.ids.next_id(now_unix_milliseconds)?;
        let size_bytes = u64::try_from(processed.picture.bytes.len()).unwrap_or(u64::MAX);
        self.files.write(id, processed.picture.bytes).await?;
        let still_id = match processed.still {
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
    pub(crate) async fn delete_files(&self, id: Snowflake, still_id: Option<Snowflake>) {
        let _ = self.files.delete(id).await;
        if let Some(still_id) = still_id {
            let _ = self.files.delete(still_id).await;
        }
    }

    /// The bytes of a version, or of its still frame, with the read slot the response keeps until
    /// sent; any signed-in user may read them.
    pub(crate) async fn read(
        &self,
        id: Snowflake,
        is_still_wanted: bool,
    ) -> Result<(Vec<u8>, OwnedSemaphorePermit), MessageError> {
        let image = storage::find_profile_image(&self.pool, id)
            .await?
            .ok_or(MessageError::MessageNotFound)?;
        let file_id = if is_still_wanted {
            image.still_id.unwrap_or(image.id)
        } else {
            image.id
        };
        self.read_file(file_id).await
    }

    /// The bytes of one stored picture file, with the read slot the response keeps until sent;
    /// callers check who may read it.
    pub(crate) async fn read_file(
        &self,
        file_id: Snowflake,
    ) -> Result<(Vec<u8>, OwnedSemaphorePermit), MessageError> {
        let permit = tokio::time::timeout(READ_SLOT_WAIT, self.reads.clone().acquire_owned())
            .await
            .map_err(|_| MessageError::Busy)?
            .map_err(|error| MessageError::Internal(Box::new(error)))?;
        Ok((self.files.read(file_id).await?, permit))
    }
}
