//! Attachment rows (migration 0010); the sealed files are in `attachment_files`.

use std::collections::HashMap;

use mellow_protocol::Snowflake;
use sqlx::SqlitePool;

use crate::storage::messages::{MessageStorageError, lookup_integer, to_id, to_integer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredAttachment {
    pub(crate) id: Snowflake,
    pub(crate) channel_id: Snowflake,
    pub(crate) message_id: Option<Snowflake>,
    pub(crate) file_name: String,
    pub(crate) content_type: String,
    pub(crate) size_bytes: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct NewAttachment {
    pub(crate) id: Snowflake,
    pub(crate) channel_id: Snowflake,
    pub(crate) uploader_id: Snowflake,
    pub(crate) file_name: String,
    pub(crate) content_type: String,
    pub(crate) size_bytes: u64,
    pub(crate) created_at_unix_milliseconds: u64,
}

type AttachmentRow = (i64, i64, Option<i64>, String, String, i64);

fn attachment_from_row(row: AttachmentRow) -> Result<StoredAttachment, MessageStorageError> {
    let (id, channel_id, message_id, file_name, content_type, size_bytes) = row;
    Ok(StoredAttachment {
        id: to_id(id)?,
        channel_id: to_id(channel_id)?,
        message_id: message_id.map(to_id).transpose()?,
        file_name,
        content_type,
        size_bytes: u64::try_from(size_bytes)
            .map_err(|_| MessageStorageError::IntegerOutOfRange)?,
    })
}

pub(crate) async fn insert_attachment(
    pool: &SqlitePool,
    attachment: &NewAttachment,
) -> Result<(), MessageStorageError> {
    sqlx::query(
        "INSERT INTO attachments (id, channel_id, uploader_id, message_id, file_name, \
         content_type, size_bytes, created_at_unix_milliseconds) \
         VALUES (?, ?, ?, NULL, ?, ?, ?, ?)",
    )
    .bind(to_integer(attachment.id.raw())?)
    .bind(to_integer(attachment.channel_id.raw())?)
    .bind(to_integer(attachment.uploader_id.raw())?)
    .bind(&attachment.file_name)
    .bind(&attachment.content_type)
    .bind(to_integer(attachment.size_bytes)?)
    .bind(to_integer(attachment.created_at_unix_milliseconds)?)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn find_attachment(
    pool: &SqlitePool,
    attachment_id: Snowflake,
) -> Result<Option<StoredAttachment>, MessageStorageError> {
    let Some(attachment_id) = lookup_integer(attachment_id) else {
        return Ok(None);
    };
    let row: Option<AttachmentRow> = sqlx::query_as(
        "SELECT id, channel_id, message_id, file_name, content_type, size_bytes \
         FROM attachments WHERE id = ?",
    )
    .bind(attachment_id)
    .fetch_optional(pool)
    .await?;
    row.map(attachment_from_row).transpose()
}

/// The attachments of each message, in upload order; messages without any are left out.
pub(crate) async fn attachments_of_messages(
    pool: &SqlitePool,
    message_ids: &[Snowflake],
) -> Result<HashMap<Snowflake, Vec<StoredAttachment>>, MessageStorageError> {
    let mut by_message: HashMap<Snowflake, Vec<StoredAttachment>> = HashMap::new();
    // shortcut: one query per message of the page (at most 100, with an index); one query with a
    // JSON array of ids when history pages grow.
    for message_id in message_ids {
        let Some(id) = lookup_integer(*message_id) else {
            continue;
        };
        let rows: Vec<AttachmentRow> = sqlx::query_as(
            "SELECT id, channel_id, message_id, file_name, content_type, size_bytes \
             FROM attachments WHERE message_id = ? ORDER BY id",
        )
        .bind(id)
        .fetch_all(pool)
        .await?;
        if !rows.is_empty() {
            by_message.insert(
                *message_id,
                rows.into_iter()
                    .map(attachment_from_row)
                    .collect::<Result<_, _>>()?,
            );
        }
    }
    Ok(by_message)
}

/// Uploads never sent in a message: their rows go, and their ids are returned so their files
/// can go too.
pub(crate) async fn delete_unlinked_before(
    pool: &SqlitePool,
    cutoff_unix_milliseconds: u64,
) -> Result<Vec<Snowflake>, MessageStorageError> {
    let rows: Vec<(i64,)> = sqlx::query_as(
        "DELETE FROM attachments WHERE message_id IS NULL AND created_at_unix_milliseconds < ? \
         RETURNING id",
    )
    .bind(to_integer(cutoff_unix_milliseconds)?)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(|(id,)| to_id(id)).collect()
}

/// Every attachment id, for the sweep of files whose row is gone (one query per sweep).
pub(crate) async fn all_attachment_ids(
    pool: &SqlitePool,
) -> Result<Vec<Snowflake>, MessageStorageError> {
    let rows: Vec<(i64,)> = sqlx::query_as("SELECT id FROM attachments")
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(|(id,)| to_id(id)).collect()
}

/// What a user's uploads take on disk, sent or not, for the per-user quota.
pub(crate) async fn stored_bytes_of_uploader(
    pool: &SqlitePool,
    uploader_id: Snowflake,
) -> Result<u64, MessageStorageError> {
    let (total,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(size_bytes), 0) FROM attachments WHERE uploader_id = ?",
    )
    .bind(to_integer(uploader_id.raw())?)
    .fetch_one(pool)
    .await?;
    u64::try_from(total).map_err(|_| MessageStorageError::IntegerOutOfRange)
}
