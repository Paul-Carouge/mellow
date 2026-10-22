//! Emoji reactions on messages (migration 0012).

use std::collections::HashMap;

use mellow_protocol::Snowflake;
use sqlx::SqlitePool;

use crate::storage::messages::{MessageStorageError, lookup_integer, to_id, to_integer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredReactionSummary {
    pub(crate) emoji: String,
    pub(crate) count: u32,
    pub(crate) includes_viewer: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredReactingUser {
    pub(crate) id: Snowflake,
    pub(crate) username: String,
    pub(crate) display_name: String,
}

/// What adding a reaction did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReactionAddOutcome {
    Added,
    /// The user had already reacted with this emoji; nothing changed.
    AlreadyThere,
    /// No one reacted with this emoji yet and the caller may not add a new one.
    NewEmojiRefused,
    /// The message already carries the maximum number of distinct emoji.
    LimitReached,
    MessageNotFound,
}

/// Adds the user's reaction. A new emoji on the message needs `may_add_new_emoji` and room under
/// `maximum_emoji_per_message`; both are checked in the same transaction as the insert.
///
/// # Errors
/// Fails on database errors. Nothing is written then.
pub(crate) async fn add_reaction(
    pool: &SqlitePool,
    message_id: Snowflake,
    user_id: Snowflake,
    emoji: &str,
    may_add_new_emoji: bool,
    maximum_emoji_per_message: u32,
) -> Result<ReactionAddOutcome, MessageStorageError> {
    let message_id = to_integer(message_id.raw())?;
    let user_id = to_integer(user_id.raw())?;
    // Immediate: the distinct-emoji count must not change between the check and the insert.
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let (message_exists, already_reacted, emoji_exists, distinct_emoji): (bool, bool, bool, i64) =
        sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM messages WHERE id = ?1), \
             EXISTS (SELECT 1 FROM message_reactions \
                 WHERE message_id = ?1 AND emoji = ?2 AND user_id = ?3), \
             EXISTS (SELECT 1 FROM message_reactions WHERE message_id = ?1 AND emoji = ?2), \
             (SELECT COUNT(DISTINCT emoji) FROM message_reactions WHERE message_id = ?1)",
        )
        .bind(message_id)
        .bind(emoji)
        .bind(user_id)
        .fetch_one(&mut *transaction)
        .await?;
    let outcome = if !message_exists {
        ReactionAddOutcome::MessageNotFound
    } else if already_reacted {
        ReactionAddOutcome::AlreadyThere
    } else if !emoji_exists && !may_add_new_emoji {
        ReactionAddOutcome::NewEmojiRefused
    } else if !emoji_exists && distinct_emoji >= i64::from(maximum_emoji_per_message) {
        ReactionAddOutcome::LimitReached
    } else {
        sqlx::query("INSERT INTO message_reactions (message_id, user_id, emoji) VALUES (?, ?, ?)")
            .bind(message_id)
            .bind(user_id)
            .bind(emoji)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        return Ok(ReactionAddOutcome::Added);
    };
    Ok(outcome)
}

/// Whether the user had reacted with this emoji on that message of `channel_id` (and no longer
/// has). A message of another channel, or none, removes nothing.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn remove_reaction(
    pool: &SqlitePool,
    channel_id: Snowflake,
    message_id: Snowflake,
    user_id: Snowflake,
    emoji: &str,
) -> Result<bool, MessageStorageError> {
    let Some(message_id) = lookup_integer(message_id) else {
        return Ok(false);
    };
    let removed = sqlx::query(
        "DELETE FROM message_reactions WHERE message_id = ? AND user_id = ? AND emoji = ? \
         AND EXISTS (SELECT 1 FROM messages WHERE messages.id = ? AND messages.channel_id = ?)",
    )
    .bind(message_id)
    .bind(to_integer(user_id.raw())?)
    .bind(emoji)
    .bind(message_id)
    .bind(to_integer(channel_id.raw())?)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(removed > 0)
}

/// The reactions of these messages, per emoji in the order each was first added, with whether
/// `viewer_id` is among them. Messages without reactions are left out.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn reactions_of_messages(
    pool: &SqlitePool,
    viewer_id: Snowflake,
    message_ids: &[Snowflake],
) -> Result<HashMap<Snowflake, Vec<StoredReactionSummary>>, MessageStorageError> {
    if message_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let ids: Vec<u64> = message_ids.iter().map(|id| id.raw()).collect();
    // The ids as a JSON array: one bound value, read by SQLite's json_each.
    let ids = serde_json::to_string(&ids).map_err(|_| MessageStorageError::IntegerOutOfRange)?;
    let rows: Vec<(i64, String, i64, bool)> = sqlx::query_as(
        "SELECT message_id, emoji, COUNT(*), MAX(user_id = ?) FROM message_reactions \
         WHERE message_id IN (SELECT value FROM json_each(?)) \
         GROUP BY message_id, emoji ORDER BY message_id, MIN(rowid)",
    )
    .bind(to_integer(viewer_id.raw())?)
    .bind(ids)
    .fetch_all(pool)
    .await?;
    let mut by_message: HashMap<Snowflake, Vec<StoredReactionSummary>> = HashMap::new();
    for (message_id, emoji, count, includes_viewer) in rows {
        by_message
            .entry(to_id(message_id)?)
            .or_default()
            .push(StoredReactionSummary {
                emoji,
                count: u32::try_from(count).map_err(|_| MessageStorageError::IntegerOutOfRange)?,
                includes_viewer,
            });
    }
    Ok(by_message)
}

/// Up to `limit` users who reacted with this emoji, by ascending id, after the `after` cursor.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn list_reacting_users(
    pool: &SqlitePool,
    message_id: Snowflake,
    emoji: &str,
    after: Option<Snowflake>,
    limit: u32,
) -> Result<Vec<StoredReactingUser>, MessageStorageError> {
    let Some(message_id) = lookup_integer(message_id) else {
        return Ok(Vec::new());
    };
    // A cursor beyond SQLite's range is past every stored id.
    let after = after.map_or(0, |cursor| i64::try_from(cursor.raw()).unwrap_or(i64::MAX));
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT users.id, users.username, users.display_name FROM message_reactions \
         JOIN users ON users.id = message_reactions.user_id \
         WHERE message_reactions.message_id = ? AND message_reactions.emoji = ? \
         AND message_reactions.user_id > ? ORDER BY message_reactions.user_id LIMIT ?",
    )
    .bind(message_id)
    .bind(emoji)
    .bind(after)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(id, username, display_name)| {
            Ok(StoredReactingUser {
                id: to_id(id)?,
                username,
                display_name,
            })
        })
        .collect()
}
