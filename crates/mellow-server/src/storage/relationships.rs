//! Friends, friend requests and blocks (migration 0018, ADR-0025). A friendship or a pending
//! request is written on both sides in one transaction; a block only on the blocker's.

use mellow_protocol::{RelationshipKind, Snowflake};

use super::Database;

use crate::storage::guilds::BEGIN_WRITE_TRANSACTION;
use crate::storage::sqlite_integers::{
    IntegerOutOfRange, convert_integer, lookup_integer, to_snowflake, to_sqlite_integer,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum RelationshipStorageError {
    #[error(transparent)]
    IntegerOutOfRange(#[from] IntegerOutOfRange),
    #[error("the relationship kind stored in the database is unknown: {0}")]
    UnknownRelationshipKind(String),
    #[error("database error")]
    Database(#[from] sqlx::Error),
}

fn kind_to_text(kind: RelationshipKind) -> &'static str {
    match kind {
        RelationshipKind::Friend => "friend",
        RelationshipKind::IncomingRequest => "incoming_request",
        RelationshipKind::OutgoingRequest => "outgoing_request",
        RelationshipKind::Blocked => "blocked",
    }
}

fn kind_from_text(text: &str) -> Result<RelationshipKind, RelationshipStorageError> {
    match text {
        "friend" => Ok(RelationshipKind::Friend),
        "incoming_request" => Ok(RelationshipKind::IncomingRequest),
        "outgoing_request" => Ok(RelationshipKind::OutgoingRequest),
        "blocked" => Ok(RelationshipKind::Blocked),
        other => Err(RelationshipStorageError::UnknownRelationshipKind(
            other.to_owned(),
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredRelationship {
    pub(crate) user_id: Snowflake,
    pub(crate) username: String,
    pub(crate) display_name: String,
    pub(crate) kind: RelationshipKind,
    pub(crate) since_unix_milliseconds: u64,
}

/// Every relationship of `user_id`, oldest first.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn list_relationships(
    database: &Database,
    user_id: Snowflake,
) -> Result<Vec<StoredRelationship>, RelationshipStorageError> {
    let rows: Vec<(i64, String, String, String, i64)> = sqlx::query_as(
        "SELECT users.id, users.username, users.display_name, relationships.kind, \
         relationships.since_unix_milliseconds FROM relationships \
         JOIN users ON users.id = relationships.other_user_id \
         WHERE relationships.user_id = ? \
         ORDER BY relationships.since_unix_milliseconds, users.id",
    )
    .bind(to_sqlite_integer(user_id.raw())?)
    .fetch_all(database.readers())
    .await?;
    rows.into_iter()
        .map(|(id, username, display_name, kind, since)| {
            Ok(StoredRelationship {
                user_id: to_snowflake(id)?,
                username,
                display_name,
                kind: kind_from_text(&kind)?,
                since_unix_milliseconds: convert_integer(since)?,
            })
        })
        .collect()
}

/// The ids of `user_id`'s friends.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn friend_ids(
    database: &Database,
    user_id: Snowflake,
) -> Result<Vec<Snowflake>, RelationshipStorageError> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT other_user_id FROM relationships WHERE user_id = ? AND kind = 'friend'",
    )
    .bind(to_sqlite_integer(user_id.raw())?)
    .fetch_all(database.readers())
    .await?;
    Ok(ids
        .into_iter()
        .map(to_snowflake)
        .collect::<Result<_, _>>()?)
}

/// Whether `user_id` and `other_user_id` are friends.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn are_friends(
    database: &Database,
    user_id: Snowflake,
    other_user_id: Snowflake,
) -> Result<bool, RelationshipStorageError> {
    let (Some(user_id), Some(other_user_id)) =
        (lookup_integer(user_id), lookup_integer(other_user_id))
    else {
        return Ok(false);
    };
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM relationships \
         WHERE user_id = ? AND other_user_id = ? AND kind = 'friend')",
    )
    .bind(user_id)
    .bind(other_user_id)
    .fetch_one(database.readers())
    .await?)
}

/// The enabled account with this username, case-insensitively.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn find_enabled_user_id_by_username(
    database: &Database,
    username: &str,
) -> Result<Option<Snowflake>, RelationshipStorageError> {
    let id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM users WHERE username = ? COLLATE NOCASE \
         AND disabled_at_unix_milliseconds IS NULL",
    )
    .bind(username)
    .fetch_optional(database.readers())
    .await?;
    Ok(id.map(to_snowflake).transpose()?)
}

/// Whether two people may open and write direct messages to each other (ADR-0025): they share a
/// guild or are friends, and neither blocked the other.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn may_reach_each_other(
    database: &Database,
    user_id: Snowflake,
    other_user_id: Snowflake,
) -> Result<bool, RelationshipStorageError> {
    let (Some(user_id), Some(other_user_id)) =
        (lookup_integer(user_id), lookup_integer(other_user_id))
    else {
        return Ok(false);
    };
    let (may_reach,): (bool,) = sqlx::query_as(
        "SELECT NOT EXISTS (SELECT 1 FROM relationships WHERE kind = 'blocked' \
             AND ((user_id = ?1 AND other_user_id = ?2) OR (user_id = ?2 AND other_user_id = ?1))) \
         AND (EXISTS (SELECT 1 FROM relationships WHERE user_id = ?1 AND other_user_id = ?2 \
                 AND kind = 'friend') \
             OR EXISTS (SELECT 1 FROM guild_members AS mine \
                 JOIN guild_members AS theirs ON theirs.guild_id = mine.guild_id \
                 WHERE mine.user_id = ?1 AND theirs.user_id = ?2))",
    )
    .bind(user_id)
    .bind(other_user_id)
    .fetch_one(database.readers())
    .await?;
    Ok(may_reach)
}

async fn kind_between(
    connection: &mut sqlx::SqliteConnection,
    user_id: i64,
    other_user_id: i64,
) -> Result<Option<RelationshipKind>, RelationshipStorageError> {
    let kind: Option<String> = sqlx::query_scalar(
        "SELECT kind FROM relationships WHERE user_id = ? AND other_user_id = ?",
    )
    .bind(user_id)
    .bind(other_user_id)
    .fetch_optional(&mut *connection)
    .await?;
    kind.as_deref().map(kind_from_text).transpose()
}

/// `user_id`'s friends and outgoing requests: what their own cap bounds (blocks never count).
async fn count_friends_and_outgoing_requests(
    connection: &mut sqlx::SqliteConnection,
    user_id: i64,
) -> Result<usize, RelationshipStorageError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM relationships \
         WHERE user_id = ? AND kind IN ('friend', 'outgoing_request')",
    )
    .bind(user_id)
    .fetch_one(&mut *connection)
    .await?;
    Ok(convert_integer::<_, usize>(count)?)
}

async fn count_incoming_requests(
    connection: &mut sqlx::SqliteConnection,
    user_id: i64,
) -> Result<usize, RelationshipStorageError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM relationships WHERE user_id = ? AND kind = 'incoming_request'",
    )
    .bind(user_id)
    .fetch_one(&mut *connection)
    .await?;
    Ok(convert_integer::<_, usize>(count)?)
}

/// Writes `user_id`'s side; never `REPLACE`, which would bypass the row's other constraints.
async fn set_kind(
    connection: &mut sqlx::SqliteConnection,
    user_id: i64,
    other_user_id: i64,
    kind: RelationshipKind,
    now: i64,
) -> Result<(), RelationshipStorageError> {
    sqlx::query(
        "INSERT INTO relationships (user_id, other_user_id, kind, since_unix_milliseconds) \
         VALUES (?, ?, ?, ?) ON CONFLICT (user_id, other_user_id) \
         DO UPDATE SET kind = excluded.kind, since_unix_milliseconds = excluded.since_unix_milliseconds",
    )
    .bind(user_id)
    .bind(other_user_id)
    .bind(kind_to_text(kind))
    .bind(now)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

async fn delete_side(
    connection: &mut sqlx::SqliteConnection,
    user_id: i64,
    other_user_id: i64,
) -> Result<(), RelationshipStorageError> {
    sqlx::query("DELETE FROM relationships WHERE user_id = ? AND other_user_id = ?")
        .bind(user_id)
        .bind(other_user_id)
        .execute(&mut *connection)
        .await?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FriendRequestOutcome {
    /// A new request: outgoing for the sender, incoming for the recipient.
    Requested,
    /// The recipient had already asked: both are friends now.
    Accepted,
    /// Nothing changed: already friends, or already asked.
    Unchanged,
    /// A block on either side, or the recipient's pending requests are full; answered like a
    /// missing account (ADR-0025).
    Refused,
    /// The sender has `maximum_friends_and_requests` friends and outgoing requests already.
    LimitReached,
}

/// Asks `recipient_id` to be friends, or accepts their request, deciding on the rows as they are
/// inside the transaction.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn send_friend_request(
    database: &Database,
    sender_id: Snowflake,
    recipient_id: Snowflake,
    now_unix_milliseconds: u64,
    maximum_friends_and_requests: usize,
    maximum_incoming_requests: usize,
) -> Result<FriendRequestOutcome, RelationshipStorageError> {
    let (sender, recipient) = (
        to_sqlite_integer(sender_id.raw())?,
        to_sqlite_integer(recipient_id.raw())?,
    );
    let now = to_sqlite_integer(now_unix_milliseconds)?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let mine = kind_between(&mut transaction, sender, recipient).await?;
    let theirs = kind_between(&mut transaction, recipient, sender).await?;
    let outcome = match (mine, theirs) {
        (Some(RelationshipKind::Blocked), _) | (_, Some(RelationshipKind::Blocked)) => {
            FriendRequestOutcome::Refused
        }
        (Some(RelationshipKind::Friend | RelationshipKind::OutgoingRequest), _) => {
            FriendRequestOutcome::Unchanged
        }
        (Some(RelationshipKind::IncomingRequest), _) => {
            if count_friends_and_outgoing_requests(&mut transaction, sender).await?
                >= maximum_friends_and_requests
            {
                FriendRequestOutcome::LimitReached
            } else {
                set_kind(
                    &mut transaction,
                    sender,
                    recipient,
                    RelationshipKind::Friend,
                    now,
                )
                .await?;
                set_kind(
                    &mut transaction,
                    recipient,
                    sender,
                    RelationshipKind::Friend,
                    now,
                )
                .await?;
                FriendRequestOutcome::Accepted
            }
        }
        (None, _) => {
            if count_friends_and_outgoing_requests(&mut transaction, sender).await?
                >= maximum_friends_and_requests
            {
                FriendRequestOutcome::LimitReached
            } else if count_incoming_requests(&mut transaction, recipient).await?
                >= maximum_incoming_requests
            {
                FriendRequestOutcome::Refused
            } else {
                set_kind(
                    &mut transaction,
                    sender,
                    recipient,
                    RelationshipKind::OutgoingRequest,
                    now,
                )
                .await?;
                set_kind(
                    &mut transaction,
                    recipient,
                    sender,
                    RelationshipKind::IncomingRequest,
                    now,
                )
                .await?;
                FriendRequestOutcome::Requested
            }
        }
    };
    transaction.commit().await?;
    Ok(outcome)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AcceptOutcome {
    Accepted,
    NoRequest,
    /// The user has `maximum_friends_and_requests` friends and outgoing requests already.
    LimitReached,
}

/// Accepts `other_user_id`'s request to `user_id`.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn accept_friend_request(
    database: &Database,
    user_id: Snowflake,
    other_user_id: Snowflake,
    now_unix_milliseconds: u64,
    maximum_friends_and_requests: usize,
) -> Result<AcceptOutcome, RelationshipStorageError> {
    let (user, other) = (
        to_sqlite_integer(user_id.raw())?,
        to_sqlite_integer(other_user_id.raw())?,
    );
    let now = to_sqlite_integer(now_unix_milliseconds)?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    if kind_between(&mut transaction, user, other).await? != Some(RelationshipKind::IncomingRequest)
    {
        return Ok(AcceptOutcome::NoRequest);
    }
    if count_friends_and_outgoing_requests(&mut transaction, user).await?
        >= maximum_friends_and_requests
    {
        return Ok(AcceptOutcome::LimitReached);
    }
    set_kind(&mut transaction, user, other, RelationshipKind::Friend, now).await?;
    set_kind(&mut transaction, other, user, RelationshipKind::Friend, now).await?;
    transaction.commit().await?;
    Ok(AcceptOutcome::Accepted)
}

/// What `remove_relationship` or `block_user` removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Removal {
    /// The user's own row before the change.
    pub(crate) previous: Option<RelationshipKind>,
    /// The other person's row about the user, if it went too.
    pub(crate) other_side_removed: Option<RelationshipKind>,
}

/// Ends a friendship, refuses or cancels a request (both sides go), or unblocks (only the
/// blocker's row goes).
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn remove_relationship(
    database: &Database,
    user_id: Snowflake,
    other_user_id: Snowflake,
) -> Result<Removal, RelationshipStorageError> {
    // An id no row can hold names no relationship: `NotFound`, not an internal error.
    let Some(other) = lookup_integer(other_user_id) else {
        return Ok(Removal {
            previous: None,
            other_side_removed: None,
        });
    };
    let user = to_sqlite_integer(user_id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let previous = kind_between(&mut transaction, user, other).await?;
    let mut other_side_removed = None;
    if previous.is_some() {
        delete_side(&mut transaction, user, other).await?;
    }
    if previous.is_some_and(|kind| kind != RelationshipKind::Blocked) {
        other_side_removed = kind_between(&mut transaction, other, user).await?;
        delete_side(&mut transaction, other, user).await?;
    }
    transaction.commit().await?;
    Ok(Removal {
        previous,
        other_side_removed,
    })
}

/// Blocks `other_user_id`: the user's row becomes `blocked`, and the other person's row about
/// them goes unless it is a block too. No relationship limit applies: blocking must always work.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn block_user(
    database: &Database,
    user_id: Snowflake,
    other_user_id: Snowflake,
    now_unix_milliseconds: u64,
) -> Result<Removal, RelationshipStorageError> {
    let (user, other) = (
        to_sqlite_integer(user_id.raw())?,
        to_sqlite_integer(other_user_id.raw())?,
    );
    let now = to_sqlite_integer(now_unix_milliseconds)?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let previous = kind_between(&mut transaction, user, other).await?;
    let theirs = kind_between(&mut transaction, other, user).await?;
    let other_side_removed = theirs.filter(|kind| *kind != RelationshipKind::Blocked);
    if other_side_removed.is_some() {
        delete_side(&mut transaction, other, user).await?;
    }
    set_kind(
        &mut transaction,
        user,
        other,
        RelationshipKind::Blocked,
        now,
    )
    .await?;
    transaction.commit().await?;
    Ok(Removal {
        previous,
        other_side_removed,
    })
}
