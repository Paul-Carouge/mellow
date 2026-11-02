//! The record of completed backups (migration 0030).

use super::Database;

use crate::storage::messages::MessageStorageError;
use crate::storage::sqlite_integers::to_sqlite_integer;

pub(crate) async fn record_completed_backup(
    database: &Database,
    now_unix_milliseconds: u64,
) -> Result<(), MessageStorageError> {
    sqlx::query("INSERT INTO completed_backups (completed_at_unix_milliseconds) VALUES (?)")
        .bind(to_sqlite_integer(now_unix_milliseconds)?)
        .execute(database.writer())
        .await?;
    Ok(())
}
