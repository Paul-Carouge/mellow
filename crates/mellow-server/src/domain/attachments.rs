//! Uploads and downloads of attachment files (ADR-0020). Access goes through the channel's
//! permissions (`Messages::require_channel_permissions`); the files are sealed on disk.

use std::collections::HashSet;
use std::sync::Arc;

use mellow_protocol::{Attachment, Permissions, Snowflake};
use sqlx::SqlitePool;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::domain::message_rules::{UNKNOWN_CONTENT_TYPE, clean_file_name, sniff_content_type};
use crate::domain::messages::{MessageError, Messages};
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::storage::attachment_files::{AttachmentFileError, AttachmentFiles};
use crate::storage::attachments::{self as storage, NewAttachment};

const PURGE_INTERVAL: std::time::Duration = std::time::Duration::from_mins(10);
/// Uploads and downloads holding a whole file in memory at once (up to the size limit each):
/// beyond, the instance answers "busy" rather than running out of memory.
const CONCURRENT_FILE_TRANSFERS: usize = 6;

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

/// A file to answer with, already checked against the reader's permissions.
#[derive(Debug)]
pub(crate) struct AttachmentDownload {
    pub(crate) file_name: String,
    pub(crate) content_type: String,
    pub(crate) bytes: Vec<u8>,
}

/// An upload allowed before its body is read: the checks come first, so a refused or
/// rate-limited client never makes the server buffer a file.
#[derive(Debug)]
pub(crate) struct UploadSlot {
    user_id: Snowflake,
    channel_id: Snowflake,
    file_name: String,
    stored_bytes: u64,
    _permit: OwnedSemaphorePermit,
}

#[derive(Debug)]
pub(crate) struct Attachments {
    pool: SqlitePool,
    ids: Arc<SnowflakeGenerator>,
    messages: Arc<Messages>,
    files: AttachmentFiles,
    transfers: Arc<Semaphore>,
    maximum_stored_bytes_per_user: u64,
}

impl Attachments {
    pub(crate) fn new(
        pool: SqlitePool,
        ids: Arc<SnowflakeGenerator>,
        messages: Arc<Messages>,
        files: AttachmentFiles,
        maximum_stored_bytes_per_user: u64,
    ) -> Self {
        Self {
            pool,
            ids,
            messages,
            files,
            transfers: Arc::new(Semaphore::new(CONCURRENT_FILE_TRANSFERS)),
            maximum_stored_bytes_per_user,
        }
    }

    fn transfer_permit(&self) -> Result<OwnedSemaphorePermit, MessageError> {
        self.transfers
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
        let stored_bytes = storage::stored_bytes_of_uploader(&self.pool, user_id).await?;
        if stored_bytes >= self.maximum_stored_bytes_per_user {
            return Err(MessageError::QuotaReached);
        }
        Ok(UploadSlot {
            user_id,
            channel_id,
            file_name,
            stored_bytes,
            _permit: self.transfer_permit()?,
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
        let size_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if slot.stored_bytes.saturating_add(size_bytes) > self.maximum_stored_bytes_per_user {
            return Err(MessageError::QuotaReached);
        }
        let content_type = sniff_content_type(&bytes).to_owned();
        let id = self.ids.next_id(now_unix_milliseconds)?;
        self.files.write(id, bytes).await?;
        let inserted = storage::insert_attachment(
            &self.pool,
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
        Ok(Attachment {
            id,
            file_name: slot.file_name,
            content_type,
            size_bytes,
        })
    }

    /// Needs `VIEW_CHANNEL` and `READ_MESSAGE_HISTORY` in the message's channel. An upload not
    /// sent yet, or one the reader cannot see, is `MessageNotFound`.
    pub(crate) async fn download(
        &self,
        user_id: Snowflake,
        attachment_id: Snowflake,
    ) -> Result<AttachmentDownload, MessageError> {
        let attachment = storage::find_attachment(&self.pool, attachment_id)
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
        let _permit = self.transfer_permit()?;
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
        })
    }

    /// Deletes uploads never sent, files whose row is gone (their message, channel or guild was
    /// deleted) and temporary files left by a crash. Only files older than an unsent upload's
    /// lifetime are swept: a file written moments ago may not have its row yet.
    pub(crate) async fn purge(&self, now_unix_milliseconds: u64) -> Result<usize, MessageError> {
        let cutoff = now_unix_milliseconds.saturating_sub(UNSENT_UPLOAD_LIFETIME_MILLISECONDS);
        let mut deleted = 0;
        for id in storage::delete_unlinked_before(&self.pool, cutoff).await? {
            self.files.delete(id).await?;
            deleted += 1;
        }
        let known_ids: HashSet<Snowflake> = storage::all_attachment_ids(&self.pool)
            .await?
            .into_iter()
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
