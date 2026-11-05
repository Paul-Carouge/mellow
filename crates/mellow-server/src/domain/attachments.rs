//! Uploads and downloads of attachment files (ADR-0020). Access goes through the channel's
//! permissions (`Messages::require_channel_permissions`); the files are sealed on disk.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use crate::storage::Database;
use mellow_protocol::{Attachment, Permissions, Snowflake};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

use crate::domain::image_processing::{
    ImageProcessingError, OPTIMIZABLE_CONTENT_TYPES, ProcessedImage, attachment_preview,
    link_preview_picture, optimize_attachment,
};
use crate::storage::link_previews::{self as link_preview_storage, StoredPreviewImage};

use crate::domain::image_recompression::ImageRecompression;
use crate::domain::message_rules::{UNKNOWN_CONTENT_TYPE, clean_file_name, sniff_content_type};
use crate::domain::messages::{MessageError, Messages};
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::storage::attachment_files::{AttachmentFileError, AttachmentFiles, SweptFiles};
use crate::storage::attachments::{
    self as storage, NewAttachment, StoredAttachment, StoredPreview,
};

const PURGE_INTERVAL: std::time::Duration = std::time::Duration::from_mins(10);
/// Uploads and downloads holding a whole file in memory at once (up to the size limit each):
/// beyond, the instance answers "busy" rather than running out of memory. Separate, so slow
/// uploads can never stop downloads; a download keeps its slot until its last byte is sent.
const CONCURRENT_UPLOADS: usize = 4;
const CONCURRENT_DOWNLOADS: usize = 8;
/// Downloads one reader holds at once, so one account (several tabs, or a client reading slowly
/// on purpose) can never take every slot: the others always keep at least two.
pub(crate) const CONCURRENT_DOWNLOADS_PER_READER: usize = 3;
/// How long a download waits for a free slot before the instance answers "busy".
const DOWNLOAD_SLOT_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// Pictures decoded at once (avatars, banners, background compression): each can take a few
/// hundred MB and a core for seconds. Only the one background worker handles pictures over 1 MP,
/// so at most one such job runs at a time (ADR-0033).
const CONCURRENT_IMAGE_PROCESSING: usize = 2;

/// Uploads waiting for compression; past this, new ones keep their original file.
const OPTIMIZATION_QUEUE_CAPACITY: usize = 64;
/// No encode starts past this, counted from the job's start; the best result found so far is
/// kept. An encode under way runs to its end (rav1e cannot be stopped): one AVIF of a 15 MP photo
/// takes 20–30 s of one core.
const OPTIMIZATION_TIME_BUDGET: std::time::Duration = std::time::Duration::from_secs(60);
/// Budget of the preview search, counted once the stored picture is done, so a large one that
/// used all of `OPTIMIZATION_TIME_BUDGET` still gets its preview. Its pictures are at most 0.48 MP:
/// one AVIF encode takes about 0.6 s.
const PREVIEW_TIME_BUDGET: std::time::Duration = std::time::Duration::from_secs(20);
/// How often an idle worker looks for a recompression request.
const RECOMPRESSION_REQUEST_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_mins(1);
/// Extensions a file name may end with that name a picture type, replaced when it is converted.
const PICTURE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "jfif", "gif", "webp", "avif"];

/// Compresses uploaded pictures one at a time, after they were answered, until the server stops
/// (ADR-0021, ADR-0033): one worker bounds the CPU it takes. Whenever it runs out of work, at startup
/// included, it picks up the pictures never tried: jobs a full queue or a restart dropped. With
/// nothing of that left, it recompresses the pictures stored before ADR-0033 one by one, once an
/// administrator asked for it.
pub(crate) async fn optimize_attachments_in_background(
    attachments: Arc<Attachments>,
    recompression: ImageRecompression,
) {
    let Some(mut queue) = attachments.take_optimization_queue() else {
        return;
    };
    loop {
        attachments.optimize_until_none_is_left(&mut queue).await;
        if recompression.recompress_next().await {
            continue;
        }
        let received = tokio::select! {
            received = queue.recv() => received,
            // The request comes from another process (`mellow-server admin`): look again later.
            () = tokio::time::sleep(RECOMPRESSION_REQUEST_CHECK_INTERVAL) => continue,
        };
        let Some(attachment_id) = received else {
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

/// The id of every file a row points at: attachments and their previews, avatars, banners, icons,
/// sounds and link preview pictures, which share the file store (ADR-0021, ADR-0032, ADR-0041).
pub(crate) async fn file_ids_in_use(
    database: &Database,
) -> Result<HashSet<Snowflake>, MessageError> {
    Ok(storage::all_attachment_file_ids(database)
        .await?
        .into_iter()
        .chain(crate::storage::profile_images::all_profile_image_file_ids(database).await?)
        .chain(
            crate::storage::guild_images::all_guild_image_file_ids(database)
                .await
                .map_err(crate::domain::guilds::GuildError::from)?,
        )
        .chain(
            crate::storage::guild_sounds::all_guild_sound_ids(database)
                .await
                .map_err(crate::domain::guilds::GuildError::from)?,
        )
        .chain(link_preview_storage::all_preview_image_file_ids(database).await?)
        .collect())
}

/// An expired link preview no message shows stays this long, so a fetch job that found it fresh
/// can still link it (a job takes well under a minute).
const LINK_PREVIEW_PURGE_GRACE_MILLISECONDS: u64 = 10 * 60 * 1_000;

/// An upload not sent in a message within this long is deleted.
pub(crate) const UNSENT_UPLOAD_LIFETIME_MILLISECONDS: u64 = 60 * 60 * 1_000;

impl From<AttachmentFileError> for MessageError {
    fn from(error: AttachmentFileError) -> Self {
        Self::Internal(Box::new(error))
    }
}

/// Which of an attachment's files to serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttachmentFileKind {
    Original,
    /// The smaller copy for the message list, or the original when it has none.
    Preview,
}

/// An attachment file the reader may download, not read yet.
#[derive(Debug)]
pub(crate) struct ReadableAttachment {
    /// Who asked: their downloads count against `CONCURRENT_DOWNLOADS_PER_READER`.
    reader_id: Snowflake,
    /// The file's own id: a preview's, or a recompressed file's, differs from its attachment's.
    id: Snowflake,
    /// Names this version of the file in its entity tag: the file id, except for a link
    /// preview's picture, whose file id would tell when the link was first fetched (ADR-0041).
    tag_id: Snowflake,
    file_name: String,
    content_type: String,
    size_bytes: u64,
}

impl ReadableAttachment {
    /// A strong HTTP entity tag. Each version of a file has its own id (`Attachments::optimize`
    /// writes a replacement under a new one), and a preview's id differs from the original's, so
    /// no two versions share a tag.
    pub(crate) fn entity_tag(&self) -> String {
        format!("\"{}-{}\"", self.tag_id, self.size_bytes)
    }
}

/// A file to answer with, already checked against the reader's permissions.
#[derive(Debug)]
pub(crate) struct AttachmentDownload {
    pub(crate) file_name: String,
    pub(crate) content_type: String,
    pub(crate) bytes: Vec<u8>,
    /// Kept by the response body until it is sent.
    pub(crate) permit: DownloadSlot,
}

/// A download's place in the instance's slots and in its reader's share of them.
#[derive(Debug)]
pub(crate) struct DownloadSlot {
    _transfer: OwnedSemaphorePermit,
    _reader: ReaderDownloadSlot,
}

/// One of a reader's `CONCURRENT_DOWNLOADS_PER_READER` slots; the reader is forgotten once
/// nothing of theirs holds or waits for one, so the map only grows with live downloads.
#[derive(Debug)]
struct ReaderDownloadSlot {
    permit: Option<OwnedSemaphorePermit>,
    downloading_readers: Arc<Mutex<HashMap<Snowflake, Arc<Semaphore>>>>,
    reader_id: Snowflake,
}

impl Drop for ReaderDownloadSlot {
    fn drop(&mut self) {
        let mut readers = self
            .downloading_readers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        drop(self.permit.take());
        // Every clone is taken under this lock, and permits and waiters hold one: a count of one
        // means only the map still knows the reader.
        if readers
            .get(&self.reader_id)
            .is_some_and(|slots| Arc::strong_count(slots) == 1)
        {
            readers.remove(&self.reader_id);
        }
    }
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
    downloading_readers: Arc<Mutex<HashMap<Snowflake, Arc<Semaphore>>>>,
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
            downloading_readers: Arc::default(),
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

    /// A slot to decode a picture, waiting for one: for background work nobody waits on.
    pub(crate) async fn wait_for_image_processing_permit(
        &self,
    ) -> Result<OwnedSemaphorePermit, MessageError> {
        self.image_processing
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| MessageError::Internal(Box::new(error)))
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

    /// Writes a sealed file as the background compression would, for tests that swap files.
    #[cfg(test)]
    pub(crate) async fn write_file(&self, id: Snowflake, bytes: Vec<u8>) {
        self.files.write(id, bytes).await.unwrap();
    }

    /// Every file in the store, for tests that check what a sweep left.
    #[cfg(test)]
    pub(crate) async fn stored_file_ids(&self) -> Vec<Snowflake> {
        self.files.stored_ids().await.unwrap()
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
    /// Then stores a preview for the message list when one is smaller than the stored file.
    pub(crate) async fn optimize(&self, attachment_id: Snowflake) -> Result<(), MessageError> {
        let Some(attachment) = storage::find_attachment(&self.database, attachment_id).await?
        else {
            return Ok(());
        };
        let permit = self.wait_for_image_processing_permit().await?;
        let original = self.files.read(attachment.file_id).await?;
        let content_type = attachment.content_type.clone();
        let deadline = std::time::Instant::now() + OPTIMIZATION_TIME_BUDGET;
        // Waited for to its end, so this worker runs one job at a time and the other slot stays
        // free for avatars and banners. Both come from the upload, which only this job has: no
        // AV1 decoder reads the stored AVIF back.
        let (optimized, preview) = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let optimized = optimize_attachment(&original, &content_type, deadline);
            let preview_deadline = std::time::Instant::now() + PREVIEW_TIME_BUDGET;
            let preview = attachment_preview(&original, &content_type, preview_deadline);
            (optimized, preview)
        })
        .await
        .map_err(|error| MessageError::Internal(Box::new(error)))?;
        let stored_size_bytes = match optimized {
            Ok(Some(optimized)) => match self.replace_file(&attachment, optimized).await? {
                Some(size_bytes) => size_bytes,
                None => return Ok(()),
            },
            // Undecodable or not smaller: the upload stays, and may still get a preview.
            Ok(None) | Err(ImageProcessingError::Unsupported | ImageProcessingError::TooLarge) => {
                attachment.size_bytes
            }
            // A failing encoder is a server fault, not a property of the picture: say so.
            Err(error) => {
                tracing::warn!(%error, "cannot compress an attachment");
                attachment.size_bytes
            }
        };
        let preview = match preview {
            Ok(Some(preview)) => preview,
            Ok(None) | Err(ImageProcessingError::Unsupported | ImageProcessingError::TooLarge) => {
                return Ok(());
            }
            Err(error) => {
                tracing::warn!(%error, "cannot make an attachment preview");
                return Ok(());
            }
        };
        // Not smaller than the file itself: showing that file costs no more.
        if u64::try_from(preview.bytes.len()).unwrap_or(u64::MAX) >= stored_size_bytes {
            return Ok(());
        }
        self.store_preview(attachment.id, preview).await
    }

    /// Replaces the file with its smaller form when it is smaller; the size of the file stored
    /// afterwards, or `None` when the attachment was deleted or its file replaced meanwhile. The new
    /// form is written under a new file id and the row switched to it before the old file goes, so
    /// a crash at any point leaves a row that matches its file; a file left without its row is
    /// swept by `purge`.
    async fn replace_file(
        &self,
        attachment: &StoredAttachment,
        optimized: ProcessedImage,
    ) -> Result<Option<u64>, MessageError> {
        let file_name = if optimized.content_type == attachment.content_type {
            attachment.file_name.clone()
        } else {
            with_extension_of(&attachment.file_name, optimized.content_type)
        };
        let size_bytes = u64::try_from(optimized.bytes.len()).unwrap_or(u64::MAX);
        if size_bytes >= attachment.size_bytes {
            return Ok(Some(attachment.size_bytes));
        }
        let new_file_id = self
            .ids
            .next_id(crate::domain::clock::now_unix_milliseconds())?;
        self.files.write(new_file_id, optimized.bytes).await?;
        let updated = storage::update_attachment_file(
            &self.database,
            attachment.file_id,
            new_file_id,
            &file_name,
            optimized.content_type,
            size_bytes,
        )
        .await;
        let (file_to_delete, outcome) = match updated {
            Ok(true) => (attachment.file_id, Ok(Some(size_bytes))),
            Ok(false) => (new_file_id, Ok(None)),
            Err(error) => (new_file_id, Err(error.into())),
        };
        let _ = self.files.delete(file_to_delete).await;
        outcome
    }

    /// Writes the preview's sealed file, then points the row at it. A file left without its row
    /// (attachment deleted meanwhile, failed delete, crash in between) is swept by `purge`.
    async fn store_preview(
        &self,
        attachment_id: Snowflake,
        preview: ProcessedImage,
    ) -> Result<(), MessageError> {
        let file_id = self
            .ids
            .next_id(crate::domain::clock::now_unix_milliseconds())?;
        let size_bytes = u64::try_from(preview.bytes.len()).unwrap_or(u64::MAX);
        self.files.write(file_id, preview.bytes).await?;
        let preview = StoredPreview {
            file_id,
            content_type: preview.content_type.to_owned(),
            size_bytes,
        };
        let recorded =
            storage::record_attachment_preview(&self.database, attachment_id, &preview).await;
        if !matches!(recorded, Ok(true)) {
            let _ = self.files.delete(preview.file_id).await;
        }
        recorded?;
        Ok(())
    }

    /// Needs `VIEW_CHANNEL` and `READ_MESSAGE_HISTORY` in the message's channel. An upload not
    /// sent yet, or one the reader cannot see, is `MessageNotFound`. Reads no file: a client
    /// revalidating its copy is answered from this alone.
    pub(crate) async fn find_readable(
        &self,
        user_id: Snowflake,
        attachment_id: Snowflake,
        kind: AttachmentFileKind,
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
        Ok(match (kind, attachment.preview) {
            (AttachmentFileKind::Preview, Some(preview)) => ReadableAttachment {
                reader_id: user_id,
                id: preview.file_id,
                tag_id: preview.file_id,
                file_name: with_extension_of(&attachment.file_name, &preview.content_type),
                content_type: preview.content_type,
                size_bytes: preview.size_bytes,
            },
            (AttachmentFileKind::Original | AttachmentFileKind::Preview, _) => ReadableAttachment {
                reader_id: user_id,
                id: attachment.file_id,
                tag_id: attachment.file_id,
                file_name: attachment.file_name,
                content_type: attachment.content_type,
                size_bytes: attachment.size_bytes,
            },
        })
    }

    /// A link preview's picture (ADR-0041), if the reader may read the message showing it:
    /// `VIEW_CHANNEL` and `READ_MESSAGE_HISTORY` in its channel, as for attachments.
    pub(crate) async fn find_readable_link_preview_image(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        preview_id: Snowflake,
    ) -> Result<ReadableAttachment, MessageError> {
        self.messages
            .require_channel_permissions(
                user_id,
                channel_id,
                Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY,
            )
            .await?;
        let image = link_preview_storage::find_message_preview_image(
            &self.database,
            channel_id,
            message_id,
            preview_id,
        )
        .await?
        .ok_or(MessageError::MessageNotFound)?;
        Ok(ReadableAttachment {
            reader_id: user_id,
            id: image.file_id,
            tag_id: preview_id,
            file_name: with_extension_of("preview", &image.content_type),
            content_type: image.content_type,
            size_bytes: image.size_bytes,
        })
    }

    /// Stores a link preview's picture (ADR-0041): recognised by its bytes, re-encoded by the
    /// visually lossless pipeline within the attachment preview box, sealed in the file store.
    /// `None` when the bytes are no picture Mellow shows or nothing could be encoded.
    pub(crate) async fn store_link_preview_picture(
        &self,
        bytes: Vec<u8>,
    ) -> Result<Option<StoredPreviewImage>, MessageError> {
        let content_type = sniff_content_type(&bytes);
        if !OPTIMIZABLE_CONTENT_TYPES.contains(&content_type) {
            return Ok(None);
        }
        let permit = self.wait_for_image_processing_permit().await?;
        let deadline = std::time::Instant::now() + PREVIEW_TIME_BUDGET;
        let processed = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            link_preview_picture(&bytes, content_type, deadline)
        })
        .await
        .map_err(|error| MessageError::Internal(Box::new(error)))?;
        let (picture, width, height) = match processed {
            Ok(Some(processed)) => processed,
            Ok(None) | Err(ImageProcessingError::Unsupported | ImageProcessingError::TooLarge) => {
                return Ok(None);
            }
            Err(error) => {
                tracing::warn!(%error, "cannot make a link preview picture");
                return Ok(None);
            }
        };
        let file_id = self
            .ids
            .next_id(crate::domain::clock::now_unix_milliseconds())?;
        let size_bytes = u64::try_from(picture.bytes.len()).unwrap_or(u64::MAX);
        self.files.write(file_id, picture.bytes).await?;
        Ok(Some(StoredPreviewImage {
            file_id,
            content_type: picture.content_type.to_owned(),
            size_bytes,
            width,
            height,
        }))
    }

    /// Reads and decrypts a file `find_readable` allowed, waiting a moment for one of the
    /// reader's slots, then one of the instance's, before answering `Busy`: a burst of previews
    /// queues rather than failing.
    pub(crate) async fn download(
        &self,
        attachment: ReadableAttachment,
    ) -> Result<AttachmentDownload, MessageError> {
        let permit = tokio::time::timeout(DOWNLOAD_SLOT_WAIT, async {
            let reader = self.reader_download_slot(attachment.reader_id).await?;
            let transfer = self.downloads.clone().acquire_owned().await?;
            Ok::<_, tokio::sync::AcquireError>(DownloadSlot {
                _transfer: transfer,
                _reader: reader,
            })
        })
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

    async fn reader_download_slot(
        &self,
        reader_id: Snowflake,
    ) -> Result<ReaderDownloadSlot, tokio::sync::AcquireError> {
        let slots = self
            .downloading_readers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(reader_id)
            .or_insert_with(|| Arc::new(Semaphore::new(CONCURRENT_DOWNLOADS_PER_READER)))
            .clone();
        // Built before the wait ends, so a wait cut short still tries to forget an idle reader.
        // An entry it leaves (the wait's own clone still alive at that point) is one per account
        // at most, and reused by their next download.
        let mut slot = ReaderDownloadSlot {
            permit: None,
            downloading_readers: self.downloading_readers.clone(),
            reader_id,
        };
        slot.permit = Some(slots.acquire_owned().await?);
        Ok(slot)
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
        // Their pictures have no row any more: the orphan sweep below deletes them.
        link_preview_storage::delete_expired_unshown_previews(
            &self.database,
            now_unix_milliseconds,
            LINK_PREVIEW_PURGE_GRACE_MILLISECONDS,
        )
        .await?;
        // A file left behind here has no row any more: the orphan sweep below retries it.
        for id in storage::delete_unlinked_before(&self.database, cutoff).await? {
            self.delete_swept_file(id, &mut swept).await;
        }
        let known_ids = file_ids_in_use(&self.database).await?;
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
    use crate::http::test_support::test_state;

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
        // In another shard directory, so locking the first one leaves this one deletable.
        let deletable = (1..256)
            .map(|offset| Snowflake::from_raw(undeletable.raw() + offset))
            .find(|id| files.path_of(*id).parent() != Some(locked_shard.as_path()))
            .unwrap();
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

    #[tokio::test]
    async fn the_purge_sweeps_orphans_moved_to_their_shard_and_those_left_where_they_were() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let files = &state.attachments.files;
        let now = crate::domain::clock::now_unix_milliseconds();
        let ids = SnowflakeGenerator::default();
        let moved = ids
            .next_id(now - 2 * UNSENT_UPLOAD_LIFETIME_MILLISECONDS)
            .unwrap();
        let left_in_place = ids
            .next_id(now - 2 * UNSENT_UPLOAD_LIFETIME_MILLISECONDS + 1)
            .unwrap();
        for id in [moved, left_in_place] {
            files.write(id, b"orphan".to_vec()).await.unwrap();
            let legacy = files.legacy_path_of(id);
            std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
            std::fs::rename(files.path_of(id), legacy).unwrap();
        }
        std::fs::create_dir_all(files.path_of(moved).parent().unwrap()).unwrap();
        std::fs::rename(files.legacy_path_of(moved), files.path_of(moved)).unwrap();

        let swept = state.attachments.purge(now).await;
        let remaining = files.stored_ids().await.unwrap();
        database.close().await;

        assert_eq!(
            swept.unwrap(),
            SweptFiles {
                deleted: 2,
                failed: 0
            }
        );
        assert_eq!(remaining, Vec::new());
    }

    /// A stored file `reader_id` may read, as `find_readable` answers it.
    async fn readable_file(state: &crate::http::AppState, reader_id: u64) -> ReadableAttachment {
        let id = SnowflakeGenerator::default()
            .next_id(crate::domain::clock::now_unix_milliseconds())
            .unwrap();
        state
            .attachments
            .files
            .write(id, vec![7; 10])
            .await
            .unwrap();
        ReadableAttachment {
            reader_id: Snowflake::from_raw(reader_id),
            id,
            tag_id: id,
            file_name: "notes.txt".into(),
            content_type: UNKNOWN_CONTENT_TYPE.into(),
            size_bytes: 10,
        }
    }

    #[tokio::test]
    async fn one_reader_holds_at_most_its_share_of_the_download_slots() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let attachments = &state.attachments;
        let mut held = Vec::new();
        for _ in 0..CONCURRENT_DOWNLOADS_PER_READER {
            let readable = readable_file(&state, 1).await;
            held.push(attachments.download(readable).await.unwrap());
        }
        let one_more_of_the_same_reader = readable_file(&state, 1).await;
        let another_reader = readable_file(&state, 2).await;

        // Only the slot wait runs on the paused clock: the files were read above.
        tokio::time::pause();
        let past_the_cap = attachments.download(one_more_of_the_same_reader).await;
        let second_reader = attachments.download(another_reader).await;
        tokio::time::resume();
        let is_second_reader_served = second_reader.is_ok();
        let readers_tracked_while_held = attachments.downloading_readers.lock().unwrap().len();
        drop(second_reader);
        drop(held);
        let readers_tracked_after = attachments.downloading_readers.lock().unwrap().len();
        database.close().await;

        assert!(matches!(past_the_cap, Err(MessageError::Busy)));
        assert!(
            is_second_reader_served,
            "another reader still gets a slot while the first holds its maximum"
        );
        assert_eq!(readers_tracked_while_held, 2);
        assert_eq!(readers_tracked_after, 0, "finished readers are forgotten");
    }

    /// A sent PNG attachment of 1 000 bytes, its file written; answers its row.
    async fn stored_png_attachment(state: &crate::http::AppState) -> StoredAttachment {
        let id = SnowflakeGenerator::default()
            .next_id(crate::domain::clock::now_unix_milliseconds())
            .unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (20, 10, 'text', 'general', 0); \
             INSERT INTO messages (id, channel_id, author_id, content) VALUES (30, 20, 1, ''); \
             INSERT INTO attachments (id, channel_id, uploader_id, message_id, file_name, \
             content_type, size_bytes, created_at_unix_milliseconds) \
             VALUES ({id}, 20, 1, 30, 'photo.png', 'image/png', 1000, 0);"
        )))
        .execute(state.attachments.database.writer())
        .await
        .unwrap();
        state
            .attachments
            .files
            .write(id, vec![7; 1_000])
            .await
            .unwrap();
        storage::find_attachment(&state.attachments.database, id)
            .await
            .unwrap()
            .unwrap()
    }

    fn smaller_avif() -> ProcessedImage {
        ProcessedImage {
            bytes: vec![1; 10],
            content_type: "image/avif",
            is_animated: false,
        }
    }

    #[tokio::test]
    async fn a_compressed_attachment_moves_to_a_new_file_and_the_old_one_is_deleted() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let before = stored_png_attachment(&state).await;

        let replaced = state
            .attachments
            .replace_file(&before, smaller_avif())
            .await
            .unwrap();
        let after = storage::find_attachment(&database, before.id)
            .await
            .unwrap()
            .unwrap();
        let stored_files = state.attachments.files.stored_ids().await.unwrap();
        let files_in_use = file_ids_in_use(&database).await.unwrap();
        database.close().await;

        assert_eq!(replaced, Some(10));
        assert_eq!(
            before.file_id, before.id,
            "an upload starts under its own id"
        );
        assert_ne!(after.file_id, before.file_id);
        assert_eq!(
            (after.file_name.as_str(), after.content_type.as_str()),
            ("photo.avif", "image/avif")
        );
        assert_eq!(stored_files, vec![after.file_id], "the old file is gone");
        assert!(files_in_use.contains(&after.file_id));
        assert!(!files_in_use.contains(&before.file_id));
    }

    #[tokio::test]
    async fn a_compressed_attachment_whose_file_changed_meanwhile_keeps_the_file_its_row_names() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let read_before_the_swap = stored_png_attachment(&state).await;
        // Another job replaced the file between the read and the swap. The instance's generator,
        // not a second one: two generators give the same id within the same millisecond.
        let current_file = state
            .attachments
            .ids
            .next_id(crate::domain::clock::now_unix_milliseconds())
            .unwrap();
        state
            .attachments
            .files
            .write(current_file, vec![2; 20])
            .await
            .unwrap();
        sqlx::query("UPDATE attachments SET file_id = ? WHERE id = ?")
            .bind(i64::try_from(current_file.raw()).unwrap())
            .bind(i64::try_from(read_before_the_swap.id.raw()).unwrap())
            .execute(database.writer())
            .await
            .unwrap();

        let replaced = state
            .attachments
            .replace_file(&read_before_the_swap, smaller_avif())
            .await
            .unwrap();
        let after = storage::find_attachment(&database, read_before_the_swap.id)
            .await
            .unwrap()
            .unwrap();
        let mut stored_files = state.attachments.files.stored_ids().await.unwrap();
        stored_files.sort_unstable();
        database.close().await;

        assert_eq!(replaced, None);
        assert_eq!(after.file_id, current_file, "the row is left as it is");
        assert_eq!(after.content_type, "image/png");
        let mut expected_files = vec![read_before_the_swap.file_id, current_file];
        expected_files.sort_unstable();
        assert_eq!(
            stored_files, expected_files,
            "the new file is deleted, the one the row names stays"
        );
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
