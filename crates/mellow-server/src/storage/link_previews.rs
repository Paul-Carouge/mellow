//! Link previews and the messages showing them (migration 0036, ADR-0041).

use std::collections::HashMap;

use mellow_protocol::Snowflake;

use super::Database;
use crate::storage::messages::{MessageStorageError, json_array_of_ids};
use crate::storage::sqlite_integers::{
    convert_integer, lookup_integer, to_snowflake, to_sqlite_integer,
};

/// A preview's picture as stored: a sealed file in the shared file store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredPreviewImage {
    pub(crate) file_id: Snowflake,
    pub(crate) content_type: String,
    pub(crate) size_bytes: u64,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredLinkPreview {
    pub(crate) id: Snowflake,
    pub(crate) url: String,
    pub(crate) title: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) site_name: Option<String>,
    pub(crate) theme_color: Option<u32>,
    pub(crate) image: Option<StoredPreviewImage>,
    /// The fetch gave nothing to show: kept only so the link is not fetched again too soon.
    pub(crate) is_empty: bool,
}

/// A preview to record, with when it was fetched and until when it may be reused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewLinkPreview {
    pub(crate) preview: StoredLinkPreview,
    pub(crate) url_sha256: [u8; 32],
    pub(crate) fetched_at_unix_milliseconds: u64,
    pub(crate) expires_at_unix_milliseconds: u64,
}

const PREVIEW_COLUMNS: &str = "link_previews.id, link_previews.url, link_previews.title, \
    link_previews.description, link_previews.site_name, link_previews.theme_color, \
    link_previews.image_file_id, link_previews.image_content_type, link_previews.image_size_bytes, \
    link_previews.image_width, link_previews.image_height, link_previews.is_empty";

type PreviewRow = (
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    bool,
);

/// A message's id, then a preview's columns.
type MessagePreviewRow = (
    i64,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    bool,
);

fn preview_from_row(row: PreviewRow) -> Result<StoredLinkPreview, MessageStorageError> {
    let (
        id,
        url,
        title,
        description,
        site_name,
        theme_color,
        image_file_id,
        image_content_type,
        image_size_bytes,
        image_width,
        image_height,
        is_empty,
    ) = row;
    let image = match (
        image_file_id,
        image_content_type,
        image_size_bytes,
        image_width,
        image_height,
    ) {
        (Some(file_id), Some(content_type), Some(size_bytes), Some(width), Some(height)) => {
            Some(StoredPreviewImage {
                file_id: to_snowflake(file_id)?,
                content_type,
                size_bytes: convert_integer(size_bytes)?,
                width: convert_integer(width)?,
                height: convert_integer(height)?,
            })
        }
        _ => None,
    };
    Ok(StoredLinkPreview {
        id: to_snowflake(id)?,
        url,
        title,
        description,
        site_name,
        theme_color: theme_color.map(convert_integer).transpose()?,
        image,
        is_empty,
    })
}

/// The newest preview of the link that is still fresh at `now`.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn find_fresh_preview(
    database: &Database,
    url_sha256: &[u8; 32],
    now_unix_milliseconds: u64,
) -> Result<Option<StoredLinkPreview>, MessageStorageError> {
    let row: Option<PreviewRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {PREVIEW_COLUMNS} FROM link_previews \
         WHERE url_sha256 = ? AND expires_at_unix_milliseconds > ? \
         ORDER BY fetched_at_unix_milliseconds DESC LIMIT 1"
    )))
    .bind(url_sha256.as_slice())
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .fetch_optional(database.readers())
    .await?;
    row.map(preview_from_row).transpose()
}

/// Records a fetched preview.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn insert_preview(
    database: &Database,
    new: &NewLinkPreview,
) -> Result<(), MessageStorageError> {
    let preview = &new.preview;
    let image = preview.image.as_ref();
    sqlx::query(
        "INSERT INTO link_previews (id, url_sha256, url, title, description, site_name, \
         theme_color, image_file_id, image_content_type, image_size_bytes, image_width, \
         image_height, is_empty, fetched_at_unix_milliseconds, expires_at_unix_milliseconds) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(to_sqlite_integer(preview.id.raw())?)
    .bind(new.url_sha256.as_slice())
    .bind(&preview.url)
    .bind(&preview.title)
    .bind(&preview.description)
    .bind(&preview.site_name)
    .bind(preview.theme_color.map(i64::from))
    .bind(
        image
            .map(|image| to_sqlite_integer(image.file_id.raw()))
            .transpose()?,
    )
    .bind(image.map(|image| image.content_type.as_str()))
    .bind(
        image
            .map(|image| to_sqlite_integer(image.size_bytes))
            .transpose()?,
    )
    .bind(image.map(|image| i64::from(image.width)))
    .bind(image.map(|image| i64::from(image.height)))
    .bind(preview.is_empty)
    .bind(to_sqlite_integer(new.fetched_at_unix_milliseconds)?)
    .bind(to_sqlite_integer(new.expires_at_unix_milliseconds)?)
    .execute(database.writer())
    .await?;
    Ok(())
}

/// Makes `preview_ids` the message's previews, in order, if its content is still
/// `expected_content` (an edit meanwhile queued its own previews). Returns whether it wrote.
///
/// # Errors
/// Fails on database errors. Nothing is written then.
pub(crate) async fn set_message_previews(
    database: &Database,
    message_id: Snowflake,
    expected_content: &str,
    preview_ids: &[Snowflake],
) -> Result<bool, MessageStorageError> {
    let message_id = to_sqlite_integer(message_id.raw())?;
    let mut transaction = database.writer().begin_with("BEGIN IMMEDIATE").await?;
    let is_unchanged: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM messages WHERE id = ? AND content = ?)")
            .bind(message_id)
            .bind(expected_content)
            .fetch_one(&mut *transaction)
            .await?;
    if !is_unchanged {
        return Ok(false);
    }
    sqlx::query("DELETE FROM message_link_previews WHERE message_id = ?")
        .bind(message_id)
        .execute(&mut *transaction)
        .await?;
    for (position, preview_id) in preview_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO message_link_previews (message_id, position, link_preview_id) \
             VALUES (?, ?, ?)",
        )
        .bind(message_id)
        .bind(convert_integer::<_, i64>(position)?)
        .bind(to_sqlite_integer(preview_id.raw())?)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(true)
}

/// The previews of each of `message_ids` that has some, in their order: one query.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn previews_of_messages(
    database: &Database,
    message_ids: &[Snowflake],
) -> Result<HashMap<Snowflake, Vec<StoredLinkPreview>>, MessageStorageError> {
    if message_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<MessagePreviewRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT message_link_previews.message_id, {PREVIEW_COLUMNS} \
             FROM message_link_previews \
             JOIN link_previews ON link_previews.id = message_link_previews.link_preview_id \
             WHERE message_link_previews.message_id IN (SELECT value FROM json_each(?)) \
             ORDER BY message_link_previews.message_id, message_link_previews.position"
    )))
    .bind(json_array_of_ids(message_ids))
    .fetch_all(database.readers())
    .await?;
    let mut previews: HashMap<Snowflake, Vec<StoredLinkPreview>> = HashMap::new();
    for row in rows {
        let (
            message_id,
            id,
            url,
            title,
            description,
            site_name,
            theme,
            file,
            kind,
            size,
            width,
            height,
            is_empty,
        ) = row;
        previews
            .entry(to_snowflake(message_id)?)
            .or_default()
            .push(preview_from_row((
                id,
                url,
                title,
                description,
                site_name,
                theme,
                file,
                kind,
                size,
                width,
                height,
                is_empty,
            ))?);
    }
    Ok(previews)
}

/// The picture of a preview the message shows, if the message is in `channel_id`.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn find_message_preview_image(
    database: &Database,
    channel_id: Snowflake,
    message_id: Snowflake,
    preview_id: Snowflake,
) -> Result<Option<StoredPreviewImage>, MessageStorageError> {
    let (Some(channel_id), Some(message_id), Some(preview_id)) = (
        lookup_integer(channel_id),
        lookup_integer(message_id),
        lookup_integer(preview_id),
    ) else {
        return Ok(None);
    };
    let row: Option<(i64, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT link_previews.image_file_id, link_previews.image_content_type, \
         link_previews.image_size_bytes, link_previews.image_width, link_previews.image_height \
         FROM message_link_previews \
         JOIN messages ON messages.id = message_link_previews.message_id \
         JOIN link_previews ON link_previews.id = message_link_previews.link_preview_id \
         WHERE message_link_previews.message_id = ? AND messages.channel_id = ? \
         AND message_link_previews.link_preview_id = ? \
         AND link_previews.image_file_id IS NOT NULL",
    )
    .bind(message_id)
    .bind(channel_id)
    .bind(preview_id)
    .fetch_optional(database.readers())
    .await?;
    row.map(|(file_id, content_type, size_bytes, width, height)| {
        Ok(StoredPreviewImage {
            file_id: to_snowflake(file_id)?,
            content_type,
            size_bytes: convert_integer(size_bytes)?,
            width: convert_integer(width)?,
            height: convert_integer(height)?,
        })
    })
    .transpose()
}

/// Deletes previews expired before `now` that no message shows, and returns their pictures'
/// file ids, for the caller to delete after the commit.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn delete_expired_unshown_previews(
    database: &Database,
    now_unix_milliseconds: u64,
) -> Result<Vec<Snowflake>, MessageStorageError> {
    let file_ids: Vec<Option<i64>> = sqlx::query_scalar(
        "DELETE FROM link_previews WHERE expires_at_unix_milliseconds <= ? \
         AND NOT EXISTS (SELECT 1 FROM message_link_previews \
             WHERE message_link_previews.link_preview_id = link_previews.id) \
         RETURNING image_file_id",
    )
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .fetch_all(database.writer())
    .await?;
    file_ids
        .into_iter()
        .flatten()
        .map(|id| Ok(to_snowflake(id)?))
        .collect()
}

/// Every picture file a preview holds, for the orphan sweep.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn all_preview_image_file_ids(
    database: &Database,
) -> Result<Vec<Snowflake>, MessageStorageError> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT image_file_id FROM link_previews WHERE image_file_id IS NOT NULL",
    )
    .fetch_all(database.readers())
    .await?;
    ids.into_iter().map(|id| Ok(to_snowflake(id)?)).collect()
}
