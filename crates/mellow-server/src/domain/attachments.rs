//! Uploads and downloads of attachment files (ADR-0020). Access goes through the channel's
//! permissions (`Messages::require_channel_permissions`); the files are sealed on disk.

use std::collections::HashSet;
use std::sync::Arc;

use crate::storage::Database;
use mellow_protocol::{Attachment, Permissions, Snowflake};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

use crate::domain::image_processing::optimize_attachment;

use crate::domain::message_rules::{UNKNOWN_CONTENT_TYPE, clean_file_name, sniff_content_type};
use crate::domain::messages::{MessageError, Messages};
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::storage::attachment_files::{AttachmentFileError, AttachmentFiles};
use crate::storage::attachments::{self as storage, NewAttachment};

const PURGE_INTERVAL: std::time::Duration = std::time::Duration::from_mins(10);
/// Uploads and downloads holding a whole file in memory at once (up to the size limit each):
/// beyond, the instance answers "busy" rather than running out of memory. Separate, so slow
/// uploads can never stop downloads; a download keeps its slot until its last byte is sent.
const CONCURRENT_UPLOADS: usize = 4;
const CONCURRENT_DOWNLOADS: usize = 8;
/// How long a download waits for a free slot before the instance answers "busy".
const DOWNLOAD_SLOT_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// Pictures decoded at once (avatars, banners, background compression): each can take a few
/// hundred MB and a core for seconds.
const CONCURRENT_IMAGE_PROCESSING: usize = 2;

/// Uploads waiting for compression; past this, new ones keep their original file.
const OPTIMIZATION_QUEUE_CAPACITY: usize = 64;
/// Past this, the original file is kept (decoding included).
const OPTIMIZATION_TIME_BUDGET: std::time::Duration = std::time::Duration::from_secs(30);
/// The types `optimize_attachment` can make smaller.
const OPTIMIZABLE_CONTENT_TYPES: [&str; 2] = ["image/png", "image/gif"];

/// Compresses uploaded pictures one at a time, after they were answered, until the server stops
/// (ADR-0021): one worker bounds the CPU it takes.
pub(crate) async fn optimize_attachments_in_background(attachments: Arc<Attachments>) {
    let queue = attachments
        .optimization_receiver
        .lock()
        .ok()
        .and_then(|mut receiver| receiver.take());
    let Some(mut queue) = queue else {
        return;
    };
    while let Some(attachment_id) = queue.recv().await {
        if let Err(error) = attachments.optimize(attachment_id).await {
            tracing::warn!(%error, "cannot compress an attachment");
        }
    }
}

/// A photo without its EXIF, XMP and other metadata (GPS positions), before anyone can download
/// it: a byte scan, no decoding. A JPEG whose metadata cannot be found is refused rather than
/// published with it.
async fn without_jpeg_metadata(bytes: Vec<u8>) -> Result<Vec<u8>, MessageError> {
    let stripped = tokio::task::spawn_blocking(move || {
        crate::domain::image_processing::strip_jpeg_metadata(&bytes)
    })
    .await
    .map_err(|error| MessageError::Internal(Box::new(error)))?;
    Ok(stripped?)
}

/// `animation.gif` becomes `animation.webp`; a name without that extension gains it.
fn with_webp_extension(file_name: &str) -> String {
    let stem = match file_name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && extension.eq_ignore_ascii_case("gif") => {
            stem
        }
        _ => file_name,
    };
    format!("{stem}.webp")
}

/// Runs `Attachments::purge` every few minutes until the server stops.
pub(crate) async fn purge_attachments_periodically(attachments: Arc<Attachments>) {
    let mut interval = tokio::time::interval(PURGE_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        match attachments
            .purge(crate::domain::clock::now_unix_milliseconds())
            .await
        {
            Ok(0) => {}
            Ok(deleted_files) => tracing::info!(deleted_files, "attachment files purged"),
            Err(error) => tracing::warn!(%error, "cannot purge attachment files"),
        }
    }
}

/// An upload not sent in a message within this long is deleted.
pub(crate) const UNSENT_UPLOAD_LIFETIME_MILLISECONDS: u64 = 60 * 60 * 1_000;

impl From<AttachmentFileError> for MessageError {
    fn from(error: AttachmentFileError) -> Self {
        Self::Internal(Box::new(error))
    }
}

/// An attachment the reader may download, its file not read yet.
#[derive(Debug)]
pub(crate) struct ReadableAttachment {
    id: Snowflake,
    file_name: String,
    content_type: String,
    size_bytes: u64,
}

impl ReadableAttachment {
    /// A strong HTTP entity tag. The file only changes when `Attachments::optimize` replaces it
    /// with a strictly smaller one, so its size tells versions apart; a replacement that could
    /// keep the size would need a stored version counter here instead.
    pub(crate) fn entity_tag(&self) -> String {
        format!("\"{}-{}\"", self.id, self.size_bytes)
    }
}

/// A file to answer with, already checked against the reader's permissions.
#[derive(Debug)]
pub(crate) struct AttachmentDownload {
    pub(crate) file_name: String,
    pub(crate) content_type: String,
    pub(crate) bytes: Vec<u8>,
    /// Kept by the response body until it is sent.
    pub(crate) permit: OwnedSemaphorePermit,
}

/// An upload allowed before its body is read: the checks come first, so a refused or
/// rate-limited client never makes the server buffer a file.
#[derive(Debug)]
pub(crate) struct UploadSlot {
    user_id: Snowflake,
    channel_id: Snowflake,
    file_name: String,
    _permit: OwnedSemaphorePermit,
    _one_upload_per_user: UploaderGuard,
}

/// Marks a user as uploading until the upload ends, so one account takes at most one upload
/// slot: slow bodies from it cannot fill the others.
#[derive(Debug)]
struct UploaderGuard {
    uploading_users: Arc<std::sync::Mutex<HashSet<Snowflake>>>,
    user_id: Snowflake,
}

impl Drop for UploaderGuard {
    fn drop(&mut self) {
        if let Ok(mut users) = self.uploading_users.lock() {
            users.remove(&self.user_id);
        }
    }
}

#[derive(Debug)]
pub(crate) struct Attachments {
    database: Database,
    ids: Arc<SnowflakeGenerator>,
    messages: Arc<Messages>,
    files: AttachmentFiles,
    uploads: Arc<Semaphore>,
    downloads: Arc<Semaphore>,
    uploading_users: Arc<std::sync::Mutex<HashSet<Snowflake>>>,
    image_processing: Arc<Semaphore>,
    maximum_stored_bytes_per_user: u64,
    optimization_sender: mpsc::Sender<Snowflake>,
    /// Taken once by `optimize_attachments_in_background`.
    optimization_receiver: std::sync::Mutex<Option<mpsc::Receiver<Snowflake>>>,
}

impl Attachments {
    pub(crate) fn new(
        database: Database,
        ids: Arc<SnowflakeGenerator>,
        messages: Arc<Messages>,
        files: AttachmentFiles,
        maximum_stored_bytes_per_user: u64,
    ) -> Self {
        let (optimization_sender, optimization_receiver) =
            mpsc::channel(OPTIMIZATION_QUEUE_CAPACITY);
        Self {
            database,
            ids,
            messages,
            files,
            uploads: Arc::new(Semaphore::new(CONCURRENT_UPLOADS)),
            downloads: Arc::new(Semaphore::new(CONCURRENT_DOWNLOADS)),
            uploading_users: Arc::default(),
            image_processing: Arc::new(Semaphore::new(CONCURRENT_IMAGE_PROCESSING)),
            maximum_stored_bytes_per_user,
            optimization_sender,
            optimization_receiver: std::sync::Mutex::new(Some(optimization_receiver)),
        }
    }

    #[cfg(test)]
    pub(crate) fn free_download_slots(&self) -> usize {
        self.downloads.available_permits()
    }

    /// A slot to decode a picture now, or `Busy`; the work holds it until it really ends.
    pub(crate) fn image_processing_permit(&self) -> Result<OwnedSemaphorePermit, MessageError> {
        self.image_processing
            .clone()
            .try_acquire_owned()
            .map_err(|_| MessageError::Busy)
    }

    /// Needs `VIEW_CHANNEL` and `ATTACH_FILES`, a valid name, room in the user's quota and a free
    /// transfer slot; then the caller reads the body and calls `finish_upload`.
    pub(crate) async fn start_upload(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        raw_file_name: &str,
    ) -> Result<UploadSlot, MessageError> {
        self.messages
            .require_channel_permissions(
                user_id,
                channel_id,
                Permissions::VIEW_CHANNEL | Permissions::ATTACH_FILES,
            )
            .await?;
        let file_name = clean_file_name(raw_file_name)?;
        let stored_bytes = storage::stored_bytes_of_uploader(&self.database, user_id).await?;
        if stored_bytes >= self.maximum_stored_bytes_per_user {
            return Err(MessageError::QuotaReached);
        }
        let is_first_upload_of_user = self
            .uploading_users
            .lock()
            .is_ok_and(|mut users| users.insert(user_id));
        if !is_first_upload_of_user {
            return Err(MessageError::Busy);
        }
        let one_upload_per_user = UploaderGuard {
            uploading_users: self.uploading_users.clone(),
            user_id,
        };
        Ok(UploadSlot {
            user_id,
            channel_id,
            file_name,
            _permit: self
                .uploads
                .clone()
                .try_acquire_owned()
                .map_err(|_| MessageError::Busy)?,
            _one_upload_per_user: one_upload_per_user,
        })
    }

    /// The type is read from the bytes, never from the client. The file is sealed and written
    /// before its row, so a row always has its file.
    pub(crate) async fn finish_upload(
        &self,
        slot: UploadSlot,
        bytes: Vec<u8>,
        now_unix_milliseconds: u64,
    ) -> Result<Attachment, MessageError> {
        let content_type = sniff_content_type(&bytes).to_owned();
        let bytes = if content_type == "image/jpeg" {
            without_jpeg_metadata(bytes).await?
        } else {
            bytes
        };
        let size_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        // Read now, not when the upload started: a user's uploads are one at a time, so this
        // total cannot change before the row is inserted.
        let stored_bytes = storage::stored_bytes_of_uploader(&self.database, slot.user_id).await?;
        if stored_bytes.saturating_add(size_bytes) > self.maximum_stored_bytes_per_user {
            return Err(MessageError::QuotaReached);
        }
        let id = self.ids.next_id(now_unix_milliseconds)?;
        self.files.write(id, bytes).await?;
        let inserted = storage::insert_attachment(
            &self.database,
            &NewAttachment {
                id,
                channel_id: slot.channel_id,
                uploader_id: slot.user_id,
                file_name: slot.file_name.clone(),
                content_type: content_type.clone(),
                size_bytes,
                created_at_unix_milliseconds: now_unix_milliseconds,
            },
        )
        .await;
        if let Err(error) = inserted {
            // No row will ever point at it: the file goes now rather than at the next sweep.
            let _ = self.files.delete(id).await;
            return Err(error.into());
        }
        if OPTIMIZABLE_CONTENT_TYPES.contains(&content_type.as_str()) {
            // A full queue keeps the original: answering the upload never waits on compression.
            let _ = self.optimization_sender.try_send(id);
        }
        Ok(Attachment {
            id,
            file_name: slot.file_name,
            content_type,
            size_bytes,
        })
    }

    /// Replaces the file with its compressed form when smaller. Anything undecodable, too slow or
    /// not smaller keeps the original.
    pub(crate) async fn optimize(&self, attachment_id: Snowflake) -> Result<(), MessageError> {
        let Some(attachment) = storage::find_attachment(&self.database, attachment_id).await?
        else {
            return Ok(());
        };
        // Waits for a slot rather than failing: nobody is waiting on this one.
        let permit = self
            .image_processing
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| MessageError::Internal(Box::new(error)))?;
        let original = self.files.read(attachment.id).await?;
        let content_type = attachment.content_type.clone();
        let mut processing = tokio::task::spawn_blocking(move || {
            // Held until the work really ends: a timed-out job still bounds the next ones.
            let _permit = permit;
            let optimized = optimize_attachment(&original, &content_type);
            (original, optimized)
        });
        let Ok(joined) = tokio::time::timeout(OPTIMIZATION_TIME_BUDGET, &mut processing).await
        else {
            // Too slow: the original stays. Waiting for the work to end keeps this worker to one
            // job at a time, leaving the other slot to avatars and banners.
            let _ = processing.await;
            return Ok(());
        };
        let (original, optimized) =
            joined.map_err(|error| MessageError::Internal(Box::new(error)))?;
        let Ok(Some(optimized)) = optimized else {
            return Ok(());
        };
        let file_name = if optimized.content_type == attachment.content_type {
            attachment.file_name
        } else {
            with_webp_extension(&attachment.file_name)
        };
        let size_bytes = u64::try_from(optimized.bytes.len()).unwrap_or(u64::MAX);
        // `ReadableAttachment::entity_tag` relies on a replacement never keeping the size.
        if size_bytes >= attachment.size_bytes {
            return Ok(());
        }
        // shortcut: the file is replaced under the same id before its row changes, so a crash in
        // between leaves a WebP described as GIF (served as a download, not inline); a separate
        // file id per version would close it, worth doing if crashes there are ever seen.
        self.files.write(attachment.id, optimized.bytes).await?;
        let updated = storage::update_attachment_file(
            &self.database,
            attachment.id,
            &file_name,
            optimized.content_type,
            size_bytes,
        )
        .await;
        match updated {
            Ok(true) => Ok(()),
            // Deleted meanwhile: the file has no row any more.
            Ok(false) => {
                let _ = self.files.delete(attachment.id).await;
                Ok(())
            }
            Err(error) => {
                // The row still describes the original: put it back so downloads keep matching.
                self.files.write(attachment.id, original).await?;
                Err(error.into())
            }
        }
    }

    /// Needs `VIEW_CHANNEL` and `READ_MESSAGE_HISTORY` in the message's channel. An upload not
    /// sent yet, or one the reader cannot see, is `MessageNotFound`. Reads no file: a client
    /// revalidating its copy is answered from this alone.
    pub(crate) async fn find_readable(
        &self,
        user_id: Snowflake,
        attachment_id: Snowflake,
    ) -> Result<ReadableAttachment, MessageError> {
        let attachment = storage::find_attachment(&self.database, attachment_id)
            .await?
            .filter(|attachment| attachment.message_id.is_some())
            .ok_or(MessageError::MessageNotFound)?;
        self.messages
            .require_channel_permissions(
                user_id,
                attachment.channel_id,
                Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY,
            )
            .await?;
        Ok(ReadableAttachment {
            id: attachment.id,
            file_name: attachment.file_name,
            content_type: attachment.content_type,
            size_bytes: attachment.size_bytes,
        })
    }

    /// Reads and decrypts a file `find_readable` allowed, waiting a moment for a transfer slot
    /// before answering `Busy`: a burst of previews queues rather than failing.
    pub(crate) async fn download(
        &self,
        attachment: ReadableAttachment,
    ) -> Result<AttachmentDownload, MessageError> {
        let permit =
            tokio::time::timeout(DOWNLOAD_SLOT_WAIT, self.downloads.clone().acquire_owned())
                .await
                .map_err(|_| MessageError::Busy)?
                .map_err(|error| MessageError::Internal(Box::new(error)))?;
        let bytes = self.files.read(attachment.id).await?;
        // Checked again on the bytes served, whatever the row says.
        let content_type = if sniff_content_type(&bytes) == attachment.content_type {
            attachment.content_type
        } else {
            UNKNOWN_CONTENT_TYPE.to_owned()
        };
        Ok(AttachmentDownload {
            file_name: attachment.file_name,
            content_type,
            bytes,
            permit,
        })
    }

    /// Deletes uploads never sent, files whose row is gone (their message, channel or guild was
    /// deleted) and temporary files left by a crash. Only files older than an unsent upload's
    /// lifetime are swept: a file written moments ago may not have its row yet.
    pub(crate) async fn purge(&self, now_unix_milliseconds: u64) -> Result<usize, MessageError> {
        let cutoff = now_unix_milliseconds.saturating_sub(UNSENT_UPLOAD_LIFETIME_MILLISECONDS);
        let mut deleted = 0;
        for id in storage::delete_unlinked_before(&self.database, cutoff).await? {
            self.files.delete(id).await?;
            deleted += 1;
        }
        // Avatars, banners, icons and sounds share the file store (ADR-0021, ADR-0032).
        let known_ids: HashSet<Snowflake> = storage::all_attachment_ids(&self.database)
            .await?
            .into_iter()
            .chain(
                crate::storage::profile_images::all_profile_image_file_ids(&self.database).await?,
            )
            .chain(
                crate::storage::guild_images::all_guild_image_file_ids(&self.database)
                    .await
                    .map_err(crate::domain::guilds::GuildError::from)?,
            )
            .chain(
                crate::storage::guild_sounds::all_guild_sound_ids(&self.database)
                    .await
                    .map_err(crate::domain::guilds::GuildError::from)?,
            )
            .collect();
        for id in self.files.stored_ids().await? {
            if id.unix_milliseconds() < cutoff && !known_ids.contains(&id) {
                self.files.delete(id).await?;
                deleted += 1;
            }
        }
        deleted += self.files.remove_partial_files_before(cutoff).await?;
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::with_webp_extension;

    #[test]
    fn a_converted_gif_is_named_webp() {
        assert_eq!(with_webp_extension("party.gif"), "party.webp");
        assert_eq!(with_webp_extension("PARTY.GIF"), "PARTY.webp");
        assert_eq!(with_webp_extension("archive.tar.gif"), "archive.tar.webp");
        assert_eq!(with_webp_extension("no-extension"), "no-extension.webp");
        assert_eq!(with_webp_extension(".gif"), ".gif.webp");
    }
}
