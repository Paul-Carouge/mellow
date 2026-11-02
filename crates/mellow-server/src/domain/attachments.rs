//! Uploads and downloads of attachment files (ADR-0020). Access goes through the channel's
//! permissions (`Messages::require_channel_permissions`); the files are sealed on disk.

use std::collections::HashSet;
use std::sync::Arc;

use crate::storage::Database;
use mellow_protocol::{Attachment, Permissions, Snowflake};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

use crate::domain::image_processing::{OPTIMIZABLE_CONTENT_TYPES, optimize_attachment};

use crate::domain::message_rules::{UNKNOWN_CONTENT_TYPE, clean_file_name, sniff_content_type};
use crate::domain::messages::{MessageError, Messages};
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::storage::attachment_files::{AttachmentFileError, AttachmentFiles, SweptFiles};
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
/// hundred MB and a core for seconds. Only the one background worker handles pictures over 8 MP,
/// so at most one such job runs at a time (ADR-0033).
const CONCURRENT_IMAGE_PROCESSING: usize = 2;

/// Uploads waiting for compression; past this, new ones keep their original file.
const OPTIMIZATION_QUEUE_CAPACITY: usize = 64;
/// No encode starts past this, counted from the job's start; the best result found so far is
/// kept. An encode under way runs to its end (rav1e cannot be stopped): one AVIF of a 15 MP photo
/// takes 20–30 s of one core.
const OPTIMIZATION_TIME_BUDGET: std::time::Duration = std::time::Duration::from_secs(60);
/// Extensions a file name may end with that name a picture type, replaced when it is converted.
const PICTURE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "jfif", "gif", "webp", "avif"];

/// Compresses uploaded pictures one at a time, after they were answered, until the server stops
/// (ADR-0021, ADR-0033): one worker bounds the CPU it takes. Whenever it runs out of work, at startup
/// included, it picks up the pictures never tried: jobs a full queue or a restart dropped.
pub(crate) async fn optimize_attachments_in_background(attachments: Arc<Attachments>) {
    let Some(mut queue) = attachments.take_optimization_queue() else {
        return;
    };
    loop {
        attachments.optimize_until_none_is_left(&mut queue).await;
        let Some(attachment_id) = queue.recv().await else {
            return;
        };
        attachments.optimize_and_record_attempt(attachment_id).await;
    }
}

/// The bytes to store and, for a picture, the size they show at. A photo goes without its EXIF,
/// XMP and other metadata (GPS positions), before anyone can download it: a byte scan, no
/// decoding. A JPEG whose metadata cannot be found is refused rather than published with it.
async fn stored_form_and_dimensions(
    bytes: Vec<u8>,
    content_type: String,
) -> Result<(Vec<u8>, Option<(u32, u32)>), MessageError> {
    let stored = tokio::task::spawn_blocking(move || {
        let bytes = if content_type == "image/jpeg" {
            crate::domain::image_processing::strip_jpeg_metadata(&bytes)?
        } else {
            bytes
        };
        let dimensions = crate::domain::image_processing::displayed_dimensions(&bytes);
        Ok::<_, crate::domain::image_processing::ImageProcessingError>((bytes, dimensions))
    })
    .await
    .map_err(|error| MessageError::Internal(Box::new(error)))?;
    Ok(stored?)
}

/// `photo.jpg` stored as AVIF becomes `photo.avif`; a name without a picture extension gains one.
fn with_extension_of(file_name: &str, content_type: &str) -> String {
    let extension = content_type.strip_prefix("image/").unwrap_or("bin");
    let stem = match file_name.rsplit_once('.') {
        Some((stem, current))
            if !stem.is_empty()
                && PICTURE_EXTENSIONS
                    .iter()
                    .any(|known| current.eq_ignore_ascii_case(known)) =>
        {
            stem
        }
        _ => file_name,
    };
    format!("{stem}.{extension}")
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
            Ok(SweptFiles {
                deleted: 0,
                failed: 0,
            }) => {}
            Ok(SweptFiles { deleted, failed }) => tracing::info!(
                deleted_files = deleted,
                undeletable_files = failed,
                "attachment files purged"
            ),
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
        let (bytes, dimensions) = stored_form_and_dimensions(bytes, content_type.clone()).await?;
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
                dimensions,
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
            // Answering the upload never waits on compression: a full queue drops the job, and the
            // worker picks the upload up again once idle (its attempt is not recorded yet).
            let _ = self.optimization_sender.try_send(id);
        }
        Ok(Attachment {
            id,
            file_name: slot.file_name,
            content_type,
            size_bytes,
            width: dimensions.map(|(width, _)| width),
            height: dimensions.map(|(_, height)| height),
        })
    }

    /// Taken once, by the single worker: `None` afterwards.
    fn take_optimization_queue(&self) -> Option<mpsc::Receiver<Snowflake>> {
        self.optimization_receiver
            .lock()
            .ok()
            .and_then(|mut receiver| receiver.take())
    }

    /// Works through the queue, refilling it with pictures never tried, until none is left.
    async fn optimize_until_none_is_left(&self, queue: &mut mpsc::Receiver<Snowflake>) {
        loop {
            let Ok(attachment_id) = queue.try_recv() else {
                if self.queue_attachments_awaiting_optimization().await == 0 {
                    return;
                }
                continue;
            };
            if !self.optimize_and_record_attempt(attachment_id).await {
                // Unrecorded, the same pictures would be queued again forever: wait for the next
                // upload instead.
                return;
            }
        }
    }

    /// Queues the newest pictures never tried, as many as the queue takes; how many were queued.
    async fn queue_attachments_awaiting_optimization(&self) -> usize {
        let awaiting = storage::newest_attachments_awaiting_optimization(
            &self.database,
            self.optimization_sender.capacity(),
        )
        .await;
        match awaiting {
            Ok(attachment_ids) => attachment_ids
                .into_iter()
                .take_while(|attachment_id| {
                    self.optimization_sender.try_send(*attachment_id).is_ok()
                })
                .count(),
            Err(error) => {
                tracing::warn!(%error, "cannot list the attachments awaiting compression");
                0
            }
        }
    }

    /// Records the attempt, then runs `optimize`: a picture is tried once, whatever the outcome,
    /// even when it kills the process (out of memory), which would otherwise retry it at every
    /// start. `false`, and nothing tried, when the attempt could not be recorded.
    async fn optimize_and_record_attempt(&self, attachment_id: Snowflake) -> bool {
        let recorded = storage::record_optimization_attempt(
            &self.database,
            attachment_id,
            crate::domain::clock::now_unix_milliseconds(),
        )
        .await;
        if let Err(error) = &recorded {
            tracing::warn!(%error, "cannot record an attachment compression attempt");
            return false;
        }
        if let Err(error) = self.optimize(attachment_id).await {
            tracing::warn!(%error, "cannot compress an attachment");
        }
        true
    }

    /// What the worker does after a restart, run to the end: for tests, which cannot wait on it.
    #[cfg(test)]
    pub(crate) async fn optimize_every_attachment_awaiting(&self) {
        let mut queue = self.take_optimization_queue().unwrap();
        self.optimize_until_none_is_left(&mut queue).await;
    }

    /// The same instance after a restart: same database and files, a fresh empty queue.
    #[cfg(test)]
    pub(crate) fn restarted(&self) -> Self {
        Self::new(
            self.database.clone(),
            self.ids.clone(),
            self.messages.clone(),
            self.files.clone(),
            self.maximum_stored_bytes_per_user,
        )
    }

    /// Replaces the file with its compressed form when smaller (ADR-0033): no original is kept.
    /// Anything undecodable, failing the quality check or not smaller keeps the file as it is.
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
        let deadline = std::time::Instant::now() + OPTIMIZATION_TIME_BUDGET;
        // Waited for to its end, so this worker runs one job at a time and the other slot stays
        // free for avatars and banners.
        let (original, optimized) = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let optimized = optimize_attachment(&original, &content_type, deadline);
            (original, optimized)
        })
        .await
        .map_err(|error| MessageError::Internal(Box::new(error)))?;
        let Ok(Some(optimized)) = optimized else {
            return Ok(());
        };
        let file_name = if optimized.content_type == attachment.content_type {
            attachment.file_name
        } else {
            with_extension_of(&attachment.file_name, optimized.content_type)
        };
        let size_bytes = u64::try_from(optimized.bytes.len()).unwrap_or(u64::MAX);
        // `ReadableAttachment::entity_tag` relies on a replacement never keeping the size.
        if size_bytes >= attachment.size_bytes {
            return Ok(());
        }
        // shortcut: the file is replaced under the same id before its row changes, so a crash in
        // between leaves an AVIF described as PNG (served as a download, not inline); a separate
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
    /// A file that cannot be deleted is counted and skipped, never stopping the others; a
    /// database error stops the sweep.
    pub(crate) async fn purge(
        &self,
        now_unix_milliseconds: u64,
    ) -> Result<SweptFiles, MessageError> {
        let cutoff = now_unix_milliseconds.saturating_sub(UNSENT_UPLOAD_LIFETIME_MILLISECONDS);
        let mut swept = SweptFiles::default();
        // A file left behind here has no row any more: the orphan sweep below retries it.
        for id in storage::delete_unlinked_before(&self.database, cutoff).await? {
            self.delete_swept_file(id, &mut swept).await;
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
                self.delete_swept_file(id, &mut swept).await;
            }
        }
        let partial = self.files.remove_partial_files_before(cutoff).await?;
        swept.deleted += partial.deleted;
        swept.failed += partial.failed;
        Ok(swept)
    }

    async fn delete_swept_file(&self, id: Snowflake, swept: &mut SweptFiles) {
        match self.files.delete(id).await {
            Ok(()) => swept.deleted += 1,
            Err(error) => {
                // No id: it would tie the log to a user's upload.
                tracing::warn!(%error, "cannot delete an attachment file");
                swept.failed += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::tests::migrated_database;
    use crate::http::tests::test_state;

    #[cfg(unix)]
    #[tokio::test]
    async fn an_undeletable_file_does_not_stop_the_purge_of_the_others() {
        use std::os::unix::fs::PermissionsExt as _;

        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let files = &state.attachments.files;
        let now = crate::domain::clock::now_unix_milliseconds();
        let old = now - 2 * UNSENT_UPLOAD_LIFETIME_MILLISECONDS;
        let ids = SnowflakeGenerator::default();
        let undeletable = ids.next_id(old).unwrap();
        let locked_shard = files.path_of(undeletable).parent().unwrap().to_path_buf();
        // Generated ids leave their low bits at zero: the next raw id is in another shard
        // directory, so locking the first one leaves this one deletable.
        let deletable = Snowflake::from_raw(undeletable.raw() + 1);
        files.write(undeletable, b"orphan".to_vec()).await.unwrap();
        files.write(deletable, b"orphan".to_vec()).await.unwrap();
        std::fs::set_permissions(&locked_shard, std::fs::Permissions::from_mode(0o500)).unwrap();
        // Root deletes through any permission: the scenario cannot be built there.
        let is_lock_effective = std::fs::write(locked_shard.join("probe"), b"").is_err();

        let swept = state.attachments.purge(now).await;
        std::fs::set_permissions(&locked_shard, std::fs::Permissions::from_mode(0o700)).unwrap();
        let remaining = files.stored_ids().await.unwrap();
        database.close().await;

        if !is_lock_effective {
            return;
        }
        assert_eq!(
            swept.unwrap(),
            SweptFiles {
                deleted: 1,
                failed: 1
            }
        );
        assert_eq!(remaining, vec![undeletable]);
    }

    #[test]
    fn a_converted_picture_is_named_after_its_new_type() {
        assert_eq!(with_extension_of("party.gif", "image/webp"), "party.webp");
        assert_eq!(with_extension_of("PARTY.GIF", "image/webp"), "PARTY.webp");
        assert_eq!(with_extension_of("photo.JPG", "image/avif"), "photo.avif");
        assert_eq!(with_extension_of("shot.jpeg", "image/avif"), "shot.avif");
        assert_eq!(
            with_extension_of("archive.tar.png", "image/avif"),
            "archive.tar.avif"
        );
        assert_eq!(
            with_extension_of("notes.txt", "image/webp"),
            "notes.txt.webp"
        );
        assert_eq!(
            with_extension_of("no-extension", "image/avif"),
            "no-extension.avif"
        );
        assert_eq!(with_extension_of(".gif", "image/webp"), ".gif.webp");
    }
}
