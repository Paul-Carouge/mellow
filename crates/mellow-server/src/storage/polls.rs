//! Polls in messages and their votes (migration 0038, ADR-0043).

use std::collections::HashMap;

use mellow_protocol::Snowflake;

use super::Database;
use crate::storage::messages::{MessageStorageError, json_array_of_ids};
use crate::storage::reactions::StoredReactingUser;
use crate::storage::sqlite_integers::{
    convert_integer, lookup_integer, to_snowflake, to_sqlite_integer,
};

/// A poll to store with its message, validated by the domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewStoredPoll {
    pub(crate) question: String,
    /// Stored with ids 1, 2, … in this order.
    pub(crate) answers: Vec<String>,
    pub(crate) allows_multiple_answers: bool,
    pub(crate) expires_at_unix_milliseconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredPollAnswer {
    pub(crate) id: u32,
    pub(crate) text: String,
    pub(crate) vote_count: u32,
    pub(crate) has_viewer_voted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredPoll {
    pub(crate) question: String,
    pub(crate) answers: Vec<StoredPollAnswer>,
    pub(crate) allows_multiple_answers: bool,
    pub(crate) expires_at_unix_milliseconds: u64,
    pub(crate) ended_at_unix_milliseconds: Option<u64>,
}

/// What a vote did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VoteOutcome {
    /// Recorded; a single-choice poll moved the voter's earlier votes, listed here.
    Added {
        moved_from: Vec<u32>,
    },
    AlreadyCounted,
    /// The poll ended, or the message has no poll or no such answer.
    Refused(VoteRefusal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VoteRefusal {
    Ended,
    NoSuchAnswer,
}

/// Stores the message's poll, inside the transaction that stores the message.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn insert_poll(
    connection: &mut sqlx::SqliteConnection,
    message_id: Snowflake,
    poll: &NewStoredPoll,
) -> Result<(), MessageStorageError> {
    let message_id = to_sqlite_integer(message_id.raw())?;
    sqlx::query(
        "INSERT INTO polls (message_id, question, allows_multiple_answers, \
         expires_at_unix_milliseconds) VALUES (?, ?, ?, ?)",
    )
    .bind(message_id)
    .bind(&poll.question)
    .bind(poll.allows_multiple_answers)
    .bind(to_sqlite_integer(poll.expires_at_unix_milliseconds)?)
    .execute(&mut *connection)
    .await?;
    for (index, text) in poll.answers.iter().enumerate() {
        sqlx::query("INSERT INTO poll_answers (message_id, answer_id, text) VALUES (?, ?, ?)")
            .bind(message_id)
            .bind(convert_integer::<_, i64>(index + 1)?)
            .bind(text)
            .execute(&mut *connection)
            .await?;
    }
    Ok(())
}

/// Whether the message carries a poll.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn has_poll(
    database: &Database,
    message_id: Snowflake,
) -> Result<bool, MessageStorageError> {
    let Some(message_id) = lookup_integer(message_id) else {
        return Ok(false);
    };
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM polls WHERE message_id = ?)")
            .bind(message_id)
            .fetch_one(database.readers())
            .await?,
    )
}

/// The polls of `message_ids` that have one, with counts and `viewer_id`'s votes: two queries.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn polls_of_messages(
    database: &Database,
    message_ids: &[Snowflake],
    viewer_id: Option<Snowflake>,
) -> Result<HashMap<Snowflake, StoredPoll>, MessageStorageError> {
    if message_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let ids = json_array_of_ids(message_ids);
    let polls: Vec<(i64, String, bool, i64, Option<i64>)> = sqlx::query_as(
        "SELECT message_id, question, allows_multiple_answers, expires_at_unix_milliseconds, \
         ended_at_unix_milliseconds FROM polls \
         WHERE message_id IN (SELECT value FROM json_each(?))",
    )
    .bind(&ids)
    .fetch_all(database.readers())
    .await?;
    if polls.is_empty() {
        return Ok(HashMap::new());
    }
    let viewer = viewer_id
        .map(|viewer| to_sqlite_integer(viewer.raw()))
        .transpose()?
        .unwrap_or(0);
    let answers: Vec<(i64, i64, String, i64, bool)> = sqlx::query_as(
        "SELECT poll_answers.message_id, poll_answers.answer_id, poll_answers.text, \
         (SELECT COUNT(*) FROM poll_votes WHERE poll_votes.message_id = poll_answers.message_id \
             AND poll_votes.answer_id = poll_answers.answer_id), \
         EXISTS (SELECT 1 FROM poll_votes WHERE poll_votes.message_id = poll_answers.message_id \
             AND poll_votes.user_id = ? AND poll_votes.answer_id = poll_answers.answer_id) \
         FROM poll_answers WHERE poll_answers.message_id IN (SELECT value FROM json_each(?)) \
         ORDER BY poll_answers.message_id, poll_answers.answer_id",
    )
    .bind(viewer)
    .bind(&ids)
    .fetch_all(database.readers())
    .await?;
    let mut by_message: HashMap<Snowflake, StoredPoll> = HashMap::with_capacity(polls.len());
    for (message_id, question, allows_multiple_answers, expires_at, ended_at) in polls {
        by_message.insert(
            to_snowflake(message_id)?,
            StoredPoll {
                question,
                answers: Vec::new(),
                allows_multiple_answers,
                expires_at_unix_milliseconds: convert_integer(expires_at)?,
                ended_at_unix_milliseconds: ended_at.map(convert_integer).transpose()?,
            },
        );
    }
    for (message_id, answer_id, text, vote_count, has_viewer_voted) in answers {
        if let Some(poll) = by_message.get_mut(&to_snowflake(message_id)?) {
            poll.answers.push(StoredPollAnswer {
                id: convert_integer(answer_id)?,
                text,
                vote_count: convert_integer(vote_count)?,
                has_viewer_voted,
            });
        }
    }
    Ok(by_message)
}

/// Whether the poll is still open at `now`: not past its expiry nor ended early.
const OPEN_POLL: &str = "SELECT allows_multiple_answers FROM polls WHERE message_id = ? \
     AND expires_at_unix_milliseconds > ? AND ended_at_unix_milliseconds IS NULL";

/// Records `user_id`'s vote while the poll is open; a single-choice poll moves the vote.
///
/// # Errors
/// Fails on database errors. Nothing is written then.
pub(crate) async fn add_vote(
    database: &Database,
    message_id: Snowflake,
    answer_id: u32,
    user_id: Snowflake,
    now_unix_milliseconds: u64,
) -> Result<VoteOutcome, MessageStorageError> {
    let message = to_sqlite_integer(message_id.raw())?;
    let user = to_sqlite_integer(user_id.raw())?;
    let answer = i64::from(answer_id);
    let mut transaction = database.writer().begin_with("BEGIN IMMEDIATE").await?;
    let Some(allows_multiple_answers): Option<bool> = sqlx::query_scalar(OPEN_POLL)
        .bind(message)
        .bind(to_sqlite_integer(now_unix_milliseconds)?)
        .fetch_optional(&mut *transaction)
        .await?
    else {
        return Ok(VoteOutcome::Refused(VoteRefusal::Ended));
    };
    let is_answer: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM poll_answers WHERE message_id = ? AND answer_id = ?)",
    )
    .bind(message)
    .bind(answer)
    .fetch_one(&mut *transaction)
    .await?;
    if !is_answer {
        return Ok(VoteOutcome::Refused(VoteRefusal::NoSuchAnswer));
    }
    let moved_from: Vec<i64> = if allows_multiple_answers {
        Vec::new()
    } else {
        sqlx::query_scalar(
            "DELETE FROM poll_votes WHERE message_id = ? AND user_id = ? AND answer_id != ? \
             RETURNING answer_id",
        )
        .bind(message)
        .bind(user)
        .bind(answer)
        .fetch_all(&mut *transaction)
        .await?
    };
    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO poll_votes (message_id, answer_id, user_id) VALUES (?, ?, ?)",
    )
    .bind(message)
    .bind(answer)
    .bind(user)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    transaction.commit().await?;
    if inserted == 0 {
        return Ok(VoteOutcome::AlreadyCounted);
    }
    Ok(VoteOutcome::Added {
        moved_from: moved_from
            .into_iter()
            .map(convert_integer)
            .collect::<Result<_, _>>()?,
    })
}

/// Takes `user_id`'s vote back while the poll is open. `Ok(true)` when one was removed.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn remove_vote(
    database: &Database,
    message_id: Snowflake,
    answer_id: u32,
    user_id: Snowflake,
    now_unix_milliseconds: u64,
) -> Result<Result<bool, VoteRefusal>, MessageStorageError> {
    let message = to_sqlite_integer(message_id.raw())?;
    let mut transaction = database.writer().begin_with("BEGIN IMMEDIATE").await?;
    let is_open = sqlx::query_scalar::<_, bool>(OPEN_POLL)
        .bind(message)
        .bind(to_sqlite_integer(now_unix_milliseconds)?)
        .fetch_optional(&mut *transaction)
        .await?
        .is_some();
    if !is_open {
        return Ok(Err(VoteRefusal::Ended));
    }
    let removed = sqlx::query(
        "DELETE FROM poll_votes WHERE message_id = ? AND user_id = ? AND answer_id = ?",
    )
    .bind(message)
    .bind(to_sqlite_integer(user_id.raw())?)
    .bind(i64::from(answer_id))
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    transaction.commit().await?;
    Ok(Ok(removed > 0))
}

/// Ends the poll now if it is still open. Returns whether it did.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn end_poll(
    database: &Database,
    message_id: Snowflake,
    now_unix_milliseconds: u64,
) -> Result<bool, MessageStorageError> {
    let now = to_sqlite_integer(now_unix_milliseconds)?;
    Ok(sqlx::query(
        "UPDATE polls SET ended_at_unix_milliseconds = ? WHERE message_id = ? \
         AND ended_at_unix_milliseconds IS NULL AND expires_at_unix_milliseconds > ?",
    )
    .bind(now)
    .bind(to_sqlite_integer(message_id.raw())?)
    .bind(now)
    .execute(database.writer())
    .await?
    .rows_affected()
        > 0)
}

/// Up to `limit` users who voted for the answer, by ascending id, after the `after` cursor.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn list_voters(
    database: &Database,
    message_id: Snowflake,
    answer_id: u32,
    after: Option<Snowflake>,
    limit: u32,
) -> Result<Vec<StoredReactingUser>, MessageStorageError> {
    let Some(message_id) = lookup_integer(message_id) else {
        return Ok(Vec::new());
    };
    // A cursor beyond SQLite's range is past every stored id.
    let after = after.map_or(0, |cursor| i64::try_from(cursor.raw()).unwrap_or(i64::MAX));
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT users.id, users.username, users.display_name FROM poll_votes \
         JOIN users ON users.id = poll_votes.user_id \
         WHERE poll_votes.message_id = ? AND poll_votes.answer_id = ? AND poll_votes.user_id > ? \
         ORDER BY poll_votes.user_id LIMIT ?",
    )
    .bind(message_id)
    .bind(i64::from(answer_id))
    .bind(after)
    .bind(i64::from(limit))
    .fetch_all(database.readers())
    .await?;
    rows.into_iter()
        .map(|(id, username, display_name)| {
            Ok(StoredReactingUser {
                id: to_snowflake(id)?,
                username,
                display_name,
            })
        })
        .collect()
}
