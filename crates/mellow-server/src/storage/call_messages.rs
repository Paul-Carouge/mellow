//! The record of a call's message (migration 0044, ADR-0062): who joined and when it ended,
//! written once when the call ends.

use std::collections::HashMap;

use mellow_protocol::{MessageCall, Snowflake};

use super::Database;
use crate::storage::messages::{MessageStorageError, json_array_of_ids};
use crate::storage::sqlite_integers::{
    convert_integer, lookup_integer, to_snowflake, to_sqlite_integer,
};

/// Marks a new message as a call's, inside the transaction that stores the message.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn insert_call_record(
    connection: &mut sqlx::SqliteConnection,
    message_id: Snowflake,
) -> Result<(), MessageStorageError> {
    sqlx::query("INSERT INTO call_messages (message_id) VALUES (?)")
        .bind(to_sqlite_integer(message_id.raw())?)
        .execute(&mut *connection)
        .await?;
    Ok(())
}

/// Whether the message is a call's.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn is_call_message(
    database: &Database,
    message_id: Snowflake,
) -> Result<bool, MessageStorageError> {
    let Some(message_id) = lookup_integer(message_id) else {
        return Ok(false);
    };
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM call_messages WHERE message_id = ?)")
            .bind(message_id)
            .fetch_one(database.readers())
            .await?,
    )
}

/// Records `user_id` among those who joined the call of `message_id`, once, while it goes on.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn record_participant(
    database: &Database,
    message_id: Snowflake,
    user_id: Snowflake,
) -> Result<(), MessageStorageError> {
    let message_id = to_sqlite_integer(message_id.raw())?;
    sqlx::query(
        "INSERT OR IGNORE INTO call_message_participants (message_id, user_id) \
         SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM call_messages \
             WHERE message_id = ?1 AND ended_at_unix_milliseconds IS NULL) \
         AND EXISTS (SELECT 1 FROM users WHERE id = ?2)",
    )
    .bind(message_id)
    .bind(to_sqlite_integer(user_id.raw())?)
    .execute(database.writer())
    .await?;
    Ok(())
}

/// Ends the call of `message_id` with who joined it, in one short write transaction. `false`
/// when there is nothing to end: the message was deleted, or the call already ended. Ids of
/// accounts that no longer exist are left out.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn end_call(
    database: &Database,
    message_id: Snowflake,
    participant_ids: &[Snowflake],
    ended_at_unix_milliseconds: u64,
) -> Result<bool, MessageStorageError> {
    let message_id = to_sqlite_integer(message_id.raw())?;
    let mut transaction = database.writer().begin().await?;
    let ended = sqlx::query(
        "UPDATE call_messages SET ended_at_unix_milliseconds = ? \
         WHERE message_id = ? AND ended_at_unix_milliseconds IS NULL",
    )
    .bind(to_sqlite_integer(ended_at_unix_milliseconds)?)
    .bind(message_id)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if ended != 1 {
        return Ok(false);
    }
    sqlx::query(
        "INSERT OR IGNORE INTO call_message_participants (message_id, user_id) \
         SELECT ?, users.id FROM users WHERE users.id IN (SELECT value FROM json_each(?))",
    )
    .bind(message_id)
    .bind(json_array_of_ids(participant_ids))
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(true)
}

/// Closes the calls a server stop left unfinished, at `now_unix_milliseconds`: no call is going
/// on when the server starts. Returns how many.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn end_unfinished_calls(
    database: &Database,
    now_unix_milliseconds: u64,
) -> Result<u64, MessageStorageError> {
    Ok(sqlx::query(
        "UPDATE call_messages SET ended_at_unix_milliseconds = ? \
         WHERE ended_at_unix_milliseconds IS NULL",
    )
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .execute(database.writer())
    .await?
    .rows_affected())
}

/// The call records of `message_ids` that are calls' messages: two queries.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn calls_of_messages(
    database: &Database,
    message_ids: &[Snowflake],
) -> Result<HashMap<Snowflake, MessageCall>, MessageStorageError> {
    if message_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let ids = json_array_of_ids(message_ids);
    let mut connection = database.readers().acquire().await?;
    let rows: Vec<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT message_id, ended_at_unix_milliseconds FROM call_messages \
         WHERE message_id IN (SELECT value FROM json_each(?))",
    )
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?;
    if rows.is_empty() {
        return Ok(HashMap::new());
    }
    let mut calls = HashMap::with_capacity(rows.len());
    for (message_id, ended_at) in rows {
        calls.insert(
            to_snowflake(message_id)?,
            MessageCall {
                participant_ids: Vec::new(),
                ended_at_unix_milliseconds: ended_at.map(convert_integer).transpose()?,
            },
        );
    }
    let participants: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT message_id, user_id FROM call_message_participants \
         WHERE message_id IN (SELECT value FROM json_each(?)) ORDER BY message_id, user_id",
    )
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?;
    for (message_id, user_id) in participants {
        if let Some(call) = calls.get_mut(&to_snowflake(message_id)?) {
            call.participant_ids.push(to_snowflake(user_id)?);
        }
    }
    Ok(calls)
}
