//! Uploads and downloads of attachment files (ADR-0020). Access goes through the channel's
//! permissions (`Messages::require_channel_permissions`); the files are sealed on disk.

use std::sync::Arc;

use mellow_protocol::{Attachment, Permissions, Snowflake};
use sqlx::SqlitePool;

use crate::domain::message_rules::{UNKNOWN_CONTENT_TYPE, clean_file_name, sniff_content_type};
use crate::domain::messages::{MessageError, Messages};
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::storage::attachment_files::{AttachmentFileError, AttachmentFiles};
use crate::storage::attachments::{self as storage, NewAttachment};

const PURGE_INTERVAL: std::time::Duration = std::time::Duration::from_mins(10);

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

#[derive(Debug)]
pub(crate) struct Attachments {
    pool: SqlitePool,
    ids: Arc<SnowflakeGenerator>,
    messages: Arc<Messages>,
    files: AttachmentFiles,
}

impl Attachments {
    pub(crate) fn new(
        pool: SqlitePool,
        ids: Arc<SnowflakeGenerator>,
        messages: Arc<Messages>,
        files: AttachmentFiles,
    ) -> Self {
        Self {
            pool,
            ids,
            messages,
            files,
        }
    }

    /// Needs `VIEW_CHANNEL` and `ATTACH_FILES`. The type is read from the bytes, never from the
    /// client. The file is sealed and written before its row, so a row always has its file.
    pub(crate) async fn upload(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        raw_file_name: &str,
        bytes: Vec<u8>,
        now_unix_milliseconds: u64,
    ) -> Result<Attachment, MessageError> {
        self.messages
            .require_channel_permissions(
                user_id,
                channel_id,
                Permissions::VIEW_CHANNEL | Permissions::ATTACH_FILES,
            )
            .await?;
        let file_name = clean_file_name(raw_file_name)?;
        let content_type = sniff_content_type(&bytes).to_owned();
        let size_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let id = self.ids.next_id(now_unix_milliseconds)?;
        self.files.write(id, bytes).await?;
        let inserted = storage::insert_attachment(
            &self.pool,
            &NewAttachment {
                id,
                channel_id,
                uploader_id: user_id,
                file_name: file_name.clone(),
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
            file_name,
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

    /// Deletes uploads never sent, and files whose row is gone (their message, channel or guild
    /// was deleted). Run periodically; a failure is logged and retried at the next run.
    pub(crate) async fn purge(&self, now_unix_milliseconds: u64) -> Result<usize, MessageError> {
        let cutoff = now_unix_milliseconds.saturating_sub(UNSENT_UPLOAD_LIFETIME_MILLISECONDS);
        let mut deleted = 0;
        for id in storage::delete_unlinked_before(&self.pool, cutoff).await? {
            self.files.delete(id).await?;
            deleted += 1;
        }
        for id in self.files.stored_ids().await? {
            // A file written moments ago may not have its row yet (`upload` writes the file
            // first): only files older than an unsent upload's lifetime are swept.
            let is_old_enough = id.unix_milliseconds() < cutoff;
            if is_old_enough && !storage::attachment_exists(&self.pool, id).await? {
                self.files.delete(id).await?;
                deleted += 1;
            }
        }
        Ok(deleted)
    }
}
