//! Messages and read states (migration 0003).

use mellow_protocol::Snowflake;
use sqlx::SqlitePool;

#[derive(Debug, thiserror::Error)]
pub(crate) enum MessageStorageError {
    #[error("value does not fit SQLite's signed 64-bit integer")]
    IntegerOutOfRange,
    #[error("database error")]
    Database(#[from] sqlx::Error),
}

fn to_integer(value: u64) -> Result<i64, MessageStorageError> {
    i64::try_from(value).map_err(|_| MessageStorageError::IntegerOutOfRange)
}

/// For lookups: an id SQLite cannot hold cannot exist.
fn lookup_integer(id: Snowflake) -> Option<i64> {
    i64::try_from(id.raw()).ok()
}

fn to_id(value: i64) -> Result<Snowflake, MessageStorageError> {
    u64::try_from(value)
        .map(Snowflake::from_raw)
        .map_err(|_| MessageStorageError::IntegerOutOfRange)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredMessage {
    pub(crate) id: Snowflake,
    pub(crate) channel_id: Snowflake,
    pub(crate) author_id: Snowflake,
    pub(crate) author_username: String,
    pub(crate) author_display_name: String,
    pub(crate) content: String,
    pub(crate) reply_to_id: Option<Snowflake>,
    pub(crate) edited_at_unix_milliseconds: Option<u64>,
}

type MessageRow = (
    i64,
    i64,
    i64,
    String,
    String,
    String,
    Option<i64>,
    Option<i64>,
);

/// A macro so queries stay `&'static str` through `concat!` (sqlx refuses runtime-built SQL).
macro_rules! select_message_columns {
    () => {
        "SELECT messages.id, messages.channel_id, messages.author_id, users.username, \
         users.display_name, messages.content, messages.reply_to_id, \
         messages.edited_at_unix_milliseconds \
         FROM messages JOIN users ON users.id = messages.author_id "
    };
}

fn message_from_row(row: MessageRow) -> Result<StoredMessage, MessageStorageError> {
    let (id, channel_id, author_id, username, display_name, content, reply_to_id, edited_at) = row;
    Ok(StoredMessage {
        id: to_id(id)?,
        channel_id: to_id(channel_id)?,
        author_id: to_id(author_id)?,
        author_username: username,
        author_display_name: display_name,
        content,
        reply_to_id: reply_to_id.map(to_id).transpose()?,
        edited_at_unix_milliseconds: edited_at
            .map(|value| u64::try_from(value).map_err(|_| MessageStorageError::IntegerOutOfRange))
            .transpose()?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewMessage {
    pub(crate) id: Snowflake,
    pub(crate) channel_id: Snowflake,
    pub(crate) author_id: Snowflake,
    pub(crate) content: String,
    pub(crate) reply_to_id: Option<Snowflake>,
}

/// Inserts the message and returns it with its author's public fields.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn insert_message(
    pool: &SqlitePool,
    message: &NewMessage,
) -> Result<StoredMessage, MessageStorageError> {
    let row: MessageRow = sqlx::query_as(
        "INSERT INTO messages (id, channel_id, author_id, content, reply_to_id) VALUES (?, ?, ?, ?, ?) \
         RETURNING id, channel_id, author_id, \
         (SELECT username FROM users WHERE users.id = author_id), \
         (SELECT display_name FROM users WHERE users.id = author_id), \
         content, reply_to_id, edited_at_unix_milliseconds",
    )
    .bind(to_integer(message.id.raw())?)
    .bind(to_integer(message.channel_id.raw())?)
    .bind(to_integer(message.author_id.raw())?)
    .bind(&message.content)
    .bind(message.reply_to_id.map(|id| to_integer(id.raw())).transpose()?)
    .fetch_one(pool)
    .await?;
    message_from_row(row)
}

/// The message if it exists in `channel_id`; a message of another channel is not found.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn find_message_in_channel(
    pool: &SqlitePool,
    channel_id: Snowflake,
    message_id: Snowflake,
) -> Result<Option<StoredMessage>, MessageStorageError> {
    let (Some(channel_id), Some(message_id)) =
        (lookup_integer(channel_id), lookup_integer(message_id))
    else {
        return Ok(None);
    };
    let row: Option<MessageRow> = sqlx::query_as(concat!(
        select_message_columns!(),
        "WHERE messages.channel_id = ? AND messages.id = ?"
    ))
    .bind(channel_id)
    .bind(message_id)
    .fetch_optional(pool)
    .await?;
    row.map(message_from_row).transpose()
}

/// Which page of history to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoryPage {
    Newest,
    Before(Snowflake),
    After(Snowflake),
}

/// Up to `limit` messages of the page, in chronological order (oldest first).
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn list_messages(
    pool: &SqlitePool,
    channel_id: Snowflake,
    page: HistoryPage,
    limit: u32,
) -> Result<Vec<StoredMessage>, MessageStorageError> {
    let channel_id = to_integer(channel_id.raw())?;
    let limit = i64::from(limit);
    // A cursor beyond SQLite's range is past every stored id.
    let saturated_cursor = |cursor: Snowflake| i64::try_from(cursor.raw()).unwrap_or(i64::MAX);
    let rows: Vec<MessageRow> = match page {
        HistoryPage::Newest => {
            sqlx::query_as(concat!(
                "SELECT * FROM (",
                select_message_columns!(),
                "WHERE messages.channel_id = ? ORDER BY messages.id DESC LIMIT ?",
                ") ORDER BY 1"
            ))
            .bind(channel_id)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
        HistoryPage::Before(cursor) => sqlx::query_as(concat!(
            "SELECT * FROM (",
            select_message_columns!(),
            "WHERE messages.channel_id = ? AND messages.id < ? ORDER BY messages.id DESC LIMIT ?",
            ") ORDER BY 1"
        ))
        .bind(channel_id)
        .bind(saturated_cursor(cursor))
        .bind(limit)
        .fetch_all(pool)
        .await?,
        HistoryPage::After(cursor) => {
            sqlx::query_as(concat!(
                select_message_columns!(),
                "WHERE messages.channel_id = ? AND messages.id > ? ORDER BY messages.id LIMIT ?"
            ))
            .bind(channel_id)
            .bind(saturated_cursor(cursor))
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
    };
    rows.into_iter().map(message_from_row).collect()
}

/// Replaces the content and stamps the edit time; returns the updated message, or `None` if it
/// was deleted meanwhile.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn update_message_content(
    pool: &SqlitePool,
    message_id: Snowflake,
    content: &str,
    edited_at_unix_milliseconds: u64,
) -> Result<Option<StoredMessage>, MessageStorageError> {
    let row: Option<MessageRow> = sqlx::query_as(
        "UPDATE messages SET content = ?, edited_at_unix_milliseconds = ? WHERE id = ? \
         RETURNING id, channel_id, author_id, \
         (SELECT username FROM users WHERE users.id = author_id), \
         (SELECT display_name FROM users WHERE users.id = author_id), \
         content, reply_to_id, edited_at_unix_milliseconds",
    )
    .bind(content)
    .bind(to_integer(edited_at_unix_milliseconds)?)
    .bind(to_integer(message_id.raw())?)
    .fetch_optional(pool)
    .await?;
    row.map(message_from_row).transpose()
}

/// # Errors
/// Fails on database errors.
pub(crate) async fn delete_message(
    pool: &SqlitePool,
    message_id: Snowflake,
) -> Result<(), MessageStorageError> {
    sqlx::query("DELETE FROM messages WHERE id = ?")
        .bind(to_integer(message_id.raw())?)
        .execute(pool)
        .await?;
    Ok(())
}

/// Moves the user's read position forward only: an older acknowledgement arriving late never
/// marks read messages unread again.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn advance_read_state(
    pool: &SqlitePool,
    user_id: Snowflake,
    channel_id: Snowflake,
    last_read_message_id: Snowflake,
) -> Result<(), MessageStorageError> {
    sqlx::query(
        "INSERT INTO read_states (user_id, channel_id, last_read_message_id) VALUES (?, ?, ?) \
         ON CONFLICT (user_id, channel_id) DO UPDATE \
         SET last_read_message_id = MAX(last_read_message_id, excluded.last_read_message_id)",
    )
    .bind(to_integer(user_id.raw())?)
    .bind(to_integer(channel_id.raw())?)
    .bind(to_integer(last_read_message_id.raw())?)
    .execute(pool)
    .await?;
    Ok(())
}

/// Read positions in channels of guilds the user is still a member of.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn list_read_states(
    pool: &SqlitePool,
    user_id: Snowflake,
) -> Result<Vec<(Snowflake, Snowflake)>, MessageStorageError> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT read_states.channel_id, read_states.last_read_message_id FROM read_states \
         JOIN channels ON channels.id = read_states.channel_id \
         JOIN guild_members ON guild_members.guild_id = channels.guild_id \
             AND guild_members.user_id = read_states.user_id \
         WHERE read_states.user_id = ?",
    )
    .bind(to_integer(user_id.raw())?)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(channel_id, message_id)| Ok((to_id(channel_id)?, to_id(message_id)?)))
        .collect()
}
