//! Messages, pins and read states (migrations 0003 and 0012).

use mellow_protocol::Snowflake;

use super::Database;

use crate::storage::audit_log::{NewAuditEntry, insert_audit_entry};
use crate::storage::guilds::GuildStorageError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum MessageStorageError {
    #[error("value does not fit SQLite's signed 64-bit integer")]
    IntegerOutOfRange,
    #[error("database error")]
    Database(#[from] sqlx::Error),
    #[error("cannot record the audit entry")]
    AuditEntry(#[from] GuildStorageError),
    /// An attachment id that is not an unsent upload of this author to this channel.
    #[error("attachment unavailable")]
    AttachmentUnavailable,
}

pub(crate) fn to_integer(value: u64) -> Result<i64, MessageStorageError> {
    i64::try_from(value).map_err(|_| MessageStorageError::IntegerOutOfRange)
}

/// For lookups: an id SQLite cannot hold cannot exist.
pub(crate) fn lookup_integer(id: Snowflake) -> Option<i64> {
    i64::try_from(id.raw()).ok()
}

/// The ids as a JSON array: one bound value, read by SQLite's `json_each`. An id SQLite cannot
/// hold becomes a number no INTEGER column equals, so it matches nothing.
pub(crate) fn json_array_of_ids(ids: &[Snowflake]) -> String {
    let ids: Vec<String> = ids.iter().map(|id| id.raw().to_string()).collect();
    format!("[{}]", ids.join(","))
}

pub(crate) fn to_id(value: i64) -> Result<Snowflake, MessageStorageError> {
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
    pub(crate) is_pinned: bool,
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
    Option<i64>,
);

/// A macro so queries stay `&'static str` through `concat!` (sqlx refuses runtime-built SQL).
macro_rules! select_message_columns {
    () => {
        "SELECT messages.id, messages.channel_id, messages.author_id, users.username, \
         users.display_name, messages.content, messages.reply_to_id, \
         messages.edited_at_unix_milliseconds, messages.pinned_at_unix_milliseconds \
         FROM messages JOIN users ON users.id = messages.author_id "
    };
}

/// The same columns, for `UPDATE … RETURNING` and `INSERT … RETURNING` on `messages`.
macro_rules! returning_message_columns {
    () => {
        "RETURNING id, channel_id, author_id, \
         (SELECT username FROM users WHERE users.id = author_id), \
         (SELECT display_name FROM users WHERE users.id = author_id), \
         content, reply_to_id, edited_at_unix_milliseconds, pinned_at_unix_milliseconds"
    };
}

fn message_from_row(row: MessageRow) -> Result<StoredMessage, MessageStorageError> {
    let (
        id,
        channel_id,
        author_id,
        username,
        display_name,
        content,
        reply_to_id,
        edited_at,
        pinned_at,
    ) = row;
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
        is_pinned: pinned_at.is_some(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewMessage {
    pub(crate) id: Snowflake,
    pub(crate) channel_id: Snowflake,
    pub(crate) author_id: Snowflake,
    pub(crate) content: String,
    pub(crate) reply_to_id: Option<Snowflake>,
    /// Kept so a retry with the same nonce finds this message; never returned in history.
    pub(crate) nonce: Option<String>,
    /// Unsent uploads of the author to this channel, linked in the same transaction.
    pub(crate) attachment_ids: Vec<Snowflake>,
}

/// Inserts the message and returns it with its author's public fields.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn insert_message(
    database: &Database,
    message: &NewMessage,
) -> Result<StoredMessage, MessageStorageError> {
    let mut transaction = database.writer().begin().await?;
    let row: MessageRow = sqlx::query_as(concat!(
        "INSERT INTO messages (id, channel_id, author_id, content, reply_to_id, nonce) \
         VALUES (?, ?, ?, ?, ?, ?) ",
        returning_message_columns!()
    ))
    .bind(to_integer(message.id.raw())?)
    .bind(to_integer(message.channel_id.raw())?)
    .bind(to_integer(message.author_id.raw())?)
    .bind(&message.content)
    .bind(
        message
            .reply_to_id
            .map(|id| to_integer(id.raw()))
            .transpose()?,
    )
    .bind(&message.nonce)
    .fetch_one(&mut *transaction)
    .await?;
    for attachment_id in &message.attachment_ids {
        let linked = sqlx::query(
            "UPDATE attachments SET message_id = ? \
             WHERE id = ? AND uploader_id = ? AND channel_id = ? AND message_id IS NULL",
        )
        .bind(to_integer(message.id.raw())?)
        .bind(lookup_integer(*attachment_id).ok_or(MessageStorageError::AttachmentUnavailable)?)
        .bind(to_integer(message.author_id.raw())?)
        .bind(to_integer(message.channel_id.raw())?)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if linked != 1 {
            // Dropping the transaction rolls the message back too.
            return Err(MessageStorageError::AttachmentUnavailable);
        }
    }
    transaction.commit().await?;
    message_from_row(row)
}

/// The message if it exists in `channel_id`; a message of another channel is not found.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn find_message_in_channel(
    database: &Database,
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
    .fetch_optional(database.readers())
    .await?;
    row.map(message_from_row).transpose()
}

/// The message `author_id` already sent to `channel_id` with this nonce, if any.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn find_message_by_nonce(
    database: &Database,
    channel_id: Snowflake,
    author_id: Snowflake,
    nonce: &str,
) -> Result<Option<StoredMessage>, MessageStorageError> {
    let row: Option<MessageRow> = sqlx::query_as(concat!(
        select_message_columns!(),
        "WHERE messages.channel_id = ? AND messages.author_id = ? AND messages.nonce = ?"
    ))
    .bind(to_integer(channel_id.raw())?)
    .bind(to_integer(author_id.raw())?)
    .bind(nonce)
    .fetch_optional(database.readers())
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
    database: &Database,
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
            .fetch_all(database.readers())
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
        .fetch_all(database.readers())
        .await?,
        HistoryPage::After(cursor) => {
            sqlx::query_as(concat!(
                select_message_columns!(),
                "WHERE messages.channel_id = ? AND messages.id > ? ORDER BY messages.id LIMIT ?"
            ))
            .bind(channel_id)
            .bind(saturated_cursor(cursor))
            .bind(limit)
            .fetch_all(database.readers())
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
    database: &Database,
    message_id: Snowflake,
    content: &str,
    edited_at_unix_milliseconds: u64,
) -> Result<Option<StoredMessage>, MessageStorageError> {
    let row: Option<MessageRow> = sqlx::query_as(concat!(
        "UPDATE messages SET content = ?, edited_at_unix_milliseconds = ? WHERE id = ? ",
        returning_message_columns!()
    ))
    .bind(content)
    .bind(to_integer(edited_at_unix_milliseconds)?)
    .bind(to_integer(message_id.raw())?)
    .fetch_optional(database.writer())
    .await?;
    row.map(message_from_row).transpose()
}

/// `audit` records a moderator deleting someone else's message, in the same transaction; an
/// author deleting their own message records nothing.
///
/// # Errors
/// Fails on database errors. Nothing is written then.
pub(crate) async fn delete_message(
    database: &Database,
    message_id: Snowflake,
    audit: Option<&NewAuditEntry>,
) -> Result<(), MessageStorageError> {
    let mut transaction = database.writer().begin_with("BEGIN IMMEDIATE").await?;
    let deleted = sqlx::query("DELETE FROM messages WHERE id = ?")
        .bind(to_integer(message_id.raw())?)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
    if deleted > 0
        && let Some(audit) = audit
    {
        insert_audit_entry(&mut transaction, audit).await?;
    }
    transaction.commit().await?;
    Ok(())
}

/// What pinning a message did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PinOutcome {
    Pinned(StoredMessage),
    AlreadyPinned,
    /// The channel already holds the maximum number of pins.
    LimitReached,
    /// Deleted meanwhile, or never in this channel.
    MessageNotFound,
}

/// Pins the message unless it already is or the channel is full, and records `audit` in the same
/// transaction when it pinned.
///
/// # Errors
/// Fails on database errors. Nothing is written then.
pub(crate) async fn pin_message(
    database: &Database,
    channel_id: Snowflake,
    message_id: Snowflake,
    pinned_at_unix_milliseconds: u64,
    maximum_pins_per_channel: u32,
    audit: Option<&NewAuditEntry>,
) -> Result<PinOutcome, MessageStorageError> {
    let channel_id = to_integer(channel_id.raw())?;
    let message_id = to_integer(message_id.raw())?;
    // Immediate: the count and the update must see the same pins as concurrent pins.
    let mut transaction = database.writer().begin_with("BEGIN IMMEDIATE").await?;
    let pinned_at: Option<Option<i64>> = sqlx::query_scalar(
        "SELECT pinned_at_unix_milliseconds FROM messages WHERE id = ? AND channel_id = ?",
    )
    .bind(message_id)
    .bind(channel_id)
    .fetch_optional(&mut *transaction)
    .await?;
    match pinned_at {
        None => return Ok(PinOutcome::MessageNotFound),
        Some(Some(_)) => return Ok(PinOutcome::AlreadyPinned),
        Some(None) => {}
    }
    let pin_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM messages \
         WHERE channel_id = ? AND pinned_at_unix_milliseconds IS NOT NULL",
    )
    .bind(channel_id)
    .fetch_one(&mut *transaction)
    .await?;
    if pin_count >= i64::from(maximum_pins_per_channel) {
        return Ok(PinOutcome::LimitReached);
    }
    let row: MessageRow = sqlx::query_as(concat!(
        "UPDATE messages SET pinned_at_unix_milliseconds = ? WHERE id = ? ",
        returning_message_columns!()
    ))
    .bind(to_integer(pinned_at_unix_milliseconds)?)
    .bind(message_id)
    .fetch_one(&mut *transaction)
    .await?;
    if let Some(audit) = audit {
        insert_audit_entry(&mut transaction, audit).await?;
    }
    transaction.commit().await?;
    Ok(PinOutcome::Pinned(message_from_row(row)?))
}

/// Unpins the message and records `audit` in the same transaction; `None` when it was not pinned
/// (or is gone), and then nothing is written.
///
/// # Errors
/// Fails on database errors. Nothing is written then.
pub(crate) async fn unpin_message(
    database: &Database,
    channel_id: Snowflake,
    message_id: Snowflake,
    audit: Option<&NewAuditEntry>,
) -> Result<Option<StoredMessage>, MessageStorageError> {
    let mut transaction = database.writer().begin_with("BEGIN IMMEDIATE").await?;
    let row: Option<MessageRow> = sqlx::query_as(concat!(
        "UPDATE messages SET pinned_at_unix_milliseconds = NULL \
         WHERE id = ? AND channel_id = ? AND pinned_at_unix_milliseconds IS NOT NULL ",
        returning_message_columns!()
    ))
    .bind(to_integer(message_id.raw())?)
    .bind(to_integer(channel_id.raw())?)
    .fetch_optional(&mut *transaction)
    .await?;
    if row.is_some()
        && let Some(audit) = audit
    {
        insert_audit_entry(&mut transaction, audit).await?;
    }
    transaction.commit().await?;
    row.map(message_from_row).transpose()
}

/// The channel's pinned messages, the most recently pinned first.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn list_pinned_messages(
    database: &Database,
    channel_id: Snowflake,
    limit: u32,
) -> Result<Vec<StoredMessage>, MessageStorageError> {
    let rows: Vec<MessageRow> = sqlx::query_as(concat!(
        select_message_columns!(),
        "WHERE messages.channel_id = ? AND messages.pinned_at_unix_milliseconds IS NOT NULL \
         ORDER BY messages.pinned_at_unix_milliseconds DESC, messages.id DESC LIMIT ?"
    ))
    .bind(to_integer(channel_id.raw())?)
    .bind(i64::from(limit))
    .fetch_all(database.readers())
    .await?;
    rows.into_iter().map(message_from_row).collect()
}

/// Moves the user's read position forward only: an older acknowledgement arriving late never
/// marks read messages unread again.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn advance_read_state(
    database: &Database,
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
    .execute(database.writer())
    .await?;
    Ok(())
}

/// Read positions in channels of guilds the user is still a member of.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn list_read_states(
    database: &Database,
    user_id: Snowflake,
) -> Result<Vec<(Snowflake, Snowflake)>, MessageStorageError> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT read_states.channel_id, read_states.last_read_message_id FROM read_states \
         JOIN channels ON channels.id = read_states.channel_id \
         WHERE read_states.user_id = ? AND (\
             EXISTS (SELECT 1 FROM guild_members WHERE guild_members.guild_id = channels.guild_id \
                 AND guild_members.user_id = read_states.user_id) \
             OR EXISTS (SELECT 1 FROM direct_message_participants \
                 WHERE direct_message_participants.channel_id = channels.id \
                 AND direct_message_participants.user_id = read_states.user_id))",
    )
    .bind(to_integer(user_id.raw())?)
    .fetch_all(database.readers())
    .await?;
    rows.into_iter()
        .map(|(channel_id, message_id)| Ok((to_id(channel_id)?, to_id(message_id)?)))
        .collect()
}
