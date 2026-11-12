//! GIFs a member starred (migration 0037, ADR-0042).

use mellow_protocol::Snowflake;

use super::Database;
use crate::storage::messages::MessageStorageError;
use crate::storage::sqlite_integers::to_sqlite_integer;

/// The member's starred slugs, newest first.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn favourite_slugs(
    database: &Database,
    user_id: Snowflake,
) -> Result<Vec<String>, MessageStorageError> {
    Ok(sqlx::query_scalar(
        "SELECT slug FROM gif_favourites WHERE user_id = ? \
         ORDER BY added_at_unix_milliseconds DESC, slug",
    )
    .bind(to_sqlite_integer(user_id.raw())?)
    .fetch_all(database.readers())
    .await?)
}

/// Stars `slug` for the member unless they already hold `maximum` others; starring it again
/// moves it first. Returns whether it is starred now.
///
/// # Errors
/// Fails on database errors. Nothing is written then.
pub(crate) async fn add_favourite(
    database: &Database,
    user_id: Snowflake,
    slug: &str,
    now_unix_milliseconds: u64,
    maximum: u32,
) -> Result<bool, MessageStorageError> {
    let user_id = to_sqlite_integer(user_id.raw())?;
    let mut transaction = database.writer().begin_with("BEGIN IMMEDIATE").await?;
    let others: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM gif_favourites WHERE user_id = ? AND slug != ?")
            .bind(user_id)
            .bind(slug)
            .fetch_one(&mut *transaction)
            .await?;
    if others >= i64::from(maximum) {
        return Ok(false);
    }
    // After every other favourite even within the same millisecond, so "newest first" never
    // falls back to the slug order.
    sqlx::query(
        "INSERT INTO gif_favourites (user_id, slug, added_at_unix_milliseconds) VALUES (?, ?, \
         MAX(?, COALESCE((SELECT MAX(added_at_unix_milliseconds) + 1 FROM gif_favourites \
         WHERE user_id = ? AND slug != ?), 0))) \
         ON CONFLICT (user_id, slug) DO UPDATE SET \
         added_at_unix_milliseconds = excluded.added_at_unix_milliseconds",
    )
    .bind(user_id)
    .bind(slug)
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .bind(user_id)
    .bind(slug)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(true)
}

/// Unstars `slug`; nothing happens when it was not starred.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn remove_favourite(
    database: &Database,
    user_id: Snowflake,
    slug: &str,
) -> Result<(), MessageStorageError> {
    sqlx::query("DELETE FROM gif_favourites WHERE user_id = ? AND slug = ?")
        .bind(to_sqlite_integer(user_id.raw())?)
        .bind(slug)
        .execute(database.writer())
        .await?;
    Ok(())
}
