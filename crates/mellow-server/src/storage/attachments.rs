//! Attachment rows (migration 0010); the sealed files are in `attachment_files`.

use std::collections::HashMap;

use super::Database;

use mellow_protocol::Snowflake;

use crate::storage::messages::{MessageStorageError, json_array_of_ids};
use crate::storage::sqlite_integers::{
    convert_integer, lookup_integer, to_snowflake, to_sqlite_integer,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredAttachment {
    pub(crate) id: Snowflake,
    /// The sealed file holding its bytes: the attachment's own id until it is recompressed.
    pub(crate) file_id: Snowflake,
    pub(crate) channel_id: Snowflake,
    pub(crate) message_id: Option<Snowflake>,
    pub(crate) file_name: String,
    pub(crate) content_type: String,
    pub(crate) size_bytes: u64,
    /// As the picture shows; `None` for anything else and for uploads before migration 0026.
    pub(crate) dimensions: Option<(u32, u32)>,
    /// Shown in the message list in its place (migration 0028).
    pub(crate) preview: Option<StoredPreview>,
}

/// A picture attachment's smaller copy, in its own sealed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredPreview {
    pub(crate) file_id: Snowflake,
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
    pub(crate) dimensions: Option<(u32, u32)>,
    pub(crate) created_at_unix_milliseconds: u64,
}

/// What `attachment_from_row` reads, in its order.
const ATTACHMENT_COLUMNS: &str = "id, COALESCE(file_id, id), channel_id, message_id, file_name, \
     content_type, size_bytes, width, height, preview_file_id, preview_content_type, \
     preview_size_bytes";

type AttachmentRow = (
    i64,
    i64,
    i64,
    Option<i64>,
    String,
    String,
    i64,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<i64>,
);

fn attachment_from_row(row: AttachmentRow) -> Result<StoredAttachment, MessageStorageError> {
    let (
        id,
        file_id,
        channel_id,
        message_id,
        file_name,
        content_type,
        size_bytes,
        width,
        height,
        preview_file_id,
        preview_content_type,
        preview_size_bytes,
    ) = row;
    Ok(StoredAttachment {
        id: to_snowflake(id)?,
        file_id: to_snowflake(file_id)?,
        channel_id: to_snowflake(channel_id)?,
        message_id: message_id.map(to_snowflake).transpose()?,
        file_name,
        content_type,
        size_bytes: convert_integer::<_, u64>(size_bytes)?,
        dimensions: match (width, height) {
            (Some(width), Some(height)) => {
                Some((convert_integer(width)?, convert_integer(height)?))
            }
            _ => None,
        },
        preview: match (preview_file_id, preview_content_type, preview_size_bytes) {
            (Some(file_id), Some(content_type), Some(size_bytes)) => Some(StoredPreview {
                file_id: to_snowflake(file_id)?,
                content_type,
                size_bytes: convert_integer(size_bytes)?,
            }),
            _ => None,
        },
    })
}

pub(crate) async fn insert_attachment(
    database: &Database,
    attachment: &NewAttachment,
) -> Result<(), MessageStorageError> {
    sqlx::query(
        "INSERT INTO attachments (id, channel_id, uploader_id, message_id, file_name, \
         content_type, size_bytes, width, height, created_at_unix_milliseconds) \
         VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?, ?)",
    )
    .bind(to_sqlite_integer(attachment.id.raw())?)
    .bind(to_sqlite_integer(attachment.channel_id.raw())?)
    .bind(to_sqlite_integer(attachment.uploader_id.raw())?)
    .bind(&attachment.file_name)
    .bind(&attachment.content_type)
    .bind(to_sqlite_integer(attachment.size_bytes)?)
    .bind(attachment.dimensions.map(|(width, _)| i64::from(width)))
    .bind(attachment.dimensions.map(|(_, height)| i64::from(height)))
    .bind(to_sqlite_integer(attachment.created_at_unix_milliseconds)?)
    .execute(database.writer())
    .await?;
    Ok(())
}

/// Points the row at its compressed file, written under `new_file_id` (ADR-0033), if it still
/// holds `previous_file_id`; `false` when the attachment is gone or its file changed meanwhile.
pub(crate) async fn update_attachment_file(
    database: &Database,
    attachment_id: Snowflake,
    previous_file_id: Snowflake,
    new_file_id: Snowflake,
    file_name: &str,
    content_type: &str,
    size_bytes: u64,
) -> Result<bool, MessageStorageError> {
    let updated = sqlx::query(
        "UPDATE attachments SET file_id = ?, file_name = ?, content_type = ?, size_bytes = ? \
         WHERE id = ? AND COALESCE(file_id, id) = ?",
    )
    .bind(to_sqlite_integer(new_file_id.raw())?)
    .bind(file_name)
    .bind(content_type)
    .bind(to_sqlite_integer(size_bytes)?)
    .bind(to_sqlite_integer(attachment_id.raw())?)
    .bind(to_sqlite_integer(previous_file_id.raw())?)
    .execute(database.writer())
    .await?;
    Ok(updated.rows_affected() > 0)
}

/// Points the row at its preview's file; `false` when the attachment is gone.
pub(crate) async fn record_attachment_preview(
    database: &Database,
    attachment_id: Snowflake,
    preview: &StoredPreview,
) -> Result<bool, MessageStorageError> {
    let updated = sqlx::query(
        "UPDATE attachments SET preview_file_id = ?, preview_content_type = ?, \
         preview_size_bytes = ? WHERE id = ?",
    )
    .bind(to_sqlite_integer(preview.file_id.raw())?)
    .bind(&preview.content_type)
    .bind(to_sqlite_integer(preview.size_bytes)?)
    .bind(to_sqlite_integer(attachment_id.raw())?)
    .execute(database.writer())
    .await?;
    Ok(updated.rows_affected() > 0)
}

/// Records that the background compression tried this attachment, so it is not queued again.
pub(crate) async fn record_optimization_attempt(
    database: &Database,
    attachment_id: Snowflake,
    now_unix_milliseconds: u64,
) -> Result<(), MessageStorageError> {
    sqlx::query(
        "UPDATE attachments SET optimization_attempted_at_unix_milliseconds = ? WHERE id = ?",
    )
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .bind(to_sqlite_integer(attachment_id.raw())?)
    .execute(database.writer())
    .await?;
    Ok(())
}

/// Shared with the migration test that checks it stays on `attachments_awaiting_optimization`.
pub(crate) const ATTACHMENTS_AWAITING_OPTIMIZATION_QUERY: &str = "SELECT id FROM attachments WHERE optimization_attempted_at_unix_milliseconds IS NULL \
     AND content_type IN ('image/png', 'image/jpeg', 'image/gif', 'image/webp') \
     ORDER BY id DESC LIMIT ?";

/// The newest picture attachments the background compression never tried, at most `limit`. The
/// condition repeats migration 0027's partial index word for word, so the index answers it.
pub(crate) async fn newest_attachments_awaiting_optimization(
    database: &Database,
    limit: usize,
) -> Result<Vec<Snowflake>, MessageStorageError> {
    let rows: Vec<(i64,)> = sqlx::query_as(ATTACHMENTS_AWAITING_OPTIMIZATION_QUERY)
        .bind(convert_integer::<_, i64>(limit)?)
        .fetch_all(database.readers())
        .await?;
    Ok(rows
        .into_iter()
        .map(|(id,)| to_snowflake(id))
        .collect::<Result<_, _>>()?)
}

pub(crate) async fn find_attachment(
    database: &Database,
    attachment_id: Snowflake,
) -> Result<Option<StoredAttachment>, MessageStorageError> {
    let Some(attachment_id) = lookup_integer(attachment_id) else {
        return Ok(None);
    };
    let row: Option<AttachmentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ATTACHMENT_COLUMNS} FROM attachments WHERE id = ?"
    )))
    .bind(attachment_id)
    .fetch_optional(database.readers())
    .await?;
    row.map(attachment_from_row).transpose()
}

/// The attachments of each message, in upload order; messages without any are left out.
pub(crate) async fn attachments_of_messages(
    database: &Database,
    message_ids: &[Snowflake],
) -> Result<HashMap<Snowflake, Vec<StoredAttachment>>, MessageStorageError> {
    let mut by_message: HashMap<Snowflake, Vec<StoredAttachment>> = HashMap::new();
    if message_ids.is_empty() {
        return Ok(by_message);
    }
    let ids = json_array_of_ids(message_ids);
    let rows: Vec<AttachmentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ATTACHMENT_COLUMNS} FROM attachments \
         WHERE message_id IN (SELECT value FROM json_each(?)) ORDER BY id"
    )))
    .bind(ids)
    .fetch_all(database.readers())
    .await?;
    for row in rows {
        let attachment = attachment_from_row(row)?;
        if let Some(message_id) = attachment.message_id {
            by_message.entry(message_id).or_default().push(attachment);
        }
    }
    Ok(by_message)
}

/// Uploads never sent in a message: their rows go, and the ids of their files (previews
/// included) are returned so those can go too.
pub(crate) async fn delete_unlinked_before(
    database: &Database,
    cutoff_unix_milliseconds: u64,
) -> Result<Vec<Snowflake>, MessageStorageError> {
    let rows: Vec<(i64, Option<i64>)> = sqlx::query_as(
        "DELETE FROM attachments WHERE message_id IS NULL AND created_at_unix_milliseconds < ? \
         RETURNING COALESCE(file_id, id), preview_file_id",
    )
    .bind(to_sqlite_integer(cutoff_unix_milliseconds)?)
    .fetch_all(database.writer())
    .await?;
    file_ids_of_rows(rows)
}

/// The id of every attachment file, previews included, for the sweep of files whose row is gone
/// (one query per sweep).
pub(crate) async fn all_attachment_file_ids(
    database: &Database,
) -> Result<Vec<Snowflake>, MessageStorageError> {
    let rows: Vec<(i64, Option<i64>)> =
        sqlx::query_as("SELECT COALESCE(file_id, id), preview_file_id FROM attachments")
            .fetch_all(database.readers())
            .await?;
    file_ids_of_rows(rows)
}

fn file_ids_of_rows(rows: Vec<(i64, Option<i64>)>) -> Result<Vec<Snowflake>, MessageStorageError> {
    Ok(rows
        .into_iter()
        .flat_map(|(id, preview_file_id)| std::iter::once(id).chain(preview_file_id))
        .map(to_snowflake)
        .collect::<Result<_, _>>()?)
}

/// Shared with the migration test that checks it stays on `attachments_by_uploader_id`.
pub(crate) const STORED_BYTES_OF_UPLOADER_QUERY: &str =
    "SELECT COALESCE(SUM(size_bytes), 0) FROM attachments WHERE uploader_id = ?";

/// What a user's uploads take on disk, sent or not, for the per-user quota.
pub(crate) async fn stored_bytes_of_uploader(
    database: &Database,
    uploader_id: Snowflake,
) -> Result<u64, MessageStorageError> {
    let (total,): (i64,) = sqlx::query_as(STORED_BYTES_OF_UPLOADER_QUERY)
        .bind(to_sqlite_integer(uploader_id.raw())?)
        .fetch_one(database.readers())
        .await?;
    Ok(convert_integer::<_, u64>(total)?)
}
