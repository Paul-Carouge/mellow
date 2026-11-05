//! Checklists and progress bars in messages (migration 0040, ADR-0047). Every change bumps the
//! row's `revision` in its own short write transaction and reads the result back there, so the
//! event carries exactly what was committed.

use std::collections::HashMap;

use mellow_protocol::{
    Checklist, ChecklistItem, ChecklistTick, MAXIMUM_CHECKLIST_ITEMS, NewChecklist, NewProgressBar,
    ProgressBar, Snowflake,
};

use super::Database;
use crate::storage::messages::{MessageStorageError, json_array_of_ids};
use crate::storage::sqlite_integers::{
    convert_integer, lookup_integer, to_snowflake, to_sqlite_integer,
};

/// A checklist or a progress bar to store with its message, validated by the domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NewTracker {
    /// Items get ids 1, 2, … in this order.
    Checklist(NewChecklist),
    ProgressBar(NewProgressBar),
}

/// A change to one checklist, validated by the domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChecklistChange {
    Tick {
        item_id: u32,
        user_id: Snowflake,
        at_unix_milliseconds: u64,
    },
    Untick {
        item_id: u32,
    },
    AddItem {
        text: String,
    },
    RenameItem {
        item_id: u32,
        text: String,
    },
    RemoveItem {
        item_id: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChecklistOutcome {
    /// Written, with a new revision.
    Changed(Checklist),
    /// Already so (ticking a ticked item keeps its first tick): nothing written.
    Unchanged(Checklist),
    NoSuchItem,
    /// The message has no checklist (deleted meanwhile).
    NoChecklist,
    /// Adding would pass `MAXIMUM_CHECKLIST_ITEMS`.
    Full,
    /// Removing would leave it empty.
    LastItem,
}

/// Stores the message's checklist or progress bar, inside the transaction that stores the
/// message.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn insert_tracker(
    connection: &mut sqlx::SqliteConnection,
    message_id: Snowflake,
    tracker: &NewTracker,
) -> Result<(), MessageStorageError> {
    let message_id = to_sqlite_integer(message_id.raw())?;
    match tracker {
        NewTracker::Checklist(checklist) => {
            sqlx::query(
                "INSERT INTO checklists (message_id, title, revision, next_item_id) \
                 VALUES (?, ?, 0, ?)",
            )
            .bind(message_id)
            .bind(&checklist.title)
            .bind(convert_integer::<_, i64>(checklist.items.len() + 1)?)
            .execute(&mut *connection)
            .await?;
            for (index, text) in checklist.items.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO checklist_items (message_id, item_id, text) VALUES (?, ?, ?)",
                )
                .bind(message_id)
                .bind(convert_integer::<_, i64>(index + 1)?)
                .bind(text)
                .execute(&mut *connection)
                .await?;
            }
        }
        NewTracker::ProgressBar(bar) => {
            sqlx::query(
                "INSERT INTO progress_bars (message_id, label, value, maximum, revision) \
                 VALUES (?, ?, ?, ?, 0)",
            )
            .bind(message_id)
            .bind(&bar.label)
            .bind(i64::from(bar.value))
            .bind(i64::from(bar.maximum))
            .execute(&mut *connection)
            .await?;
        }
    }
    Ok(())
}

/// Whether the message carries a checklist or a progress bar.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn has_tracker(
    database: &Database,
    message_id: Snowflake,
) -> Result<bool, MessageStorageError> {
    let Some(message_id) = lookup_integer(message_id) else {
        return Ok(false);
    };
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM checklists WHERE message_id = ?1) \
         OR EXISTS (SELECT 1 FROM progress_bars WHERE message_id = ?1)",
    )
    .bind(message_id)
    .fetch_one(database.readers())
    .await?)
}

/// The checklists and progress bars of `message_ids` that have one: three queries at most.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn trackers_of_messages(
    database: &Database,
    message_ids: &[Snowflake],
) -> Result<
    (
        HashMap<Snowflake, Checklist>,
        HashMap<Snowflake, ProgressBar>,
    ),
    MessageStorageError,
> {
    if message_ids.is_empty() {
        return Ok((HashMap::new(), HashMap::new()));
    }
    let ids = json_array_of_ids(message_ids);
    let mut connection = database.readers().acquire().await?;
    let checklists = checklists_of(&mut connection, &ids).await?;
    let rows: Vec<(i64, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT message_id, label, value, maximum, revision FROM progress_bars \
         WHERE message_id IN (SELECT value FROM json_each(?))",
    )
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?;
    let mut bars = HashMap::with_capacity(rows.len());
    for (message_id, label, value, maximum, revision) in rows {
        bars.insert(
            to_snowflake(message_id)?,
            ProgressBar {
                label,
                value: convert_integer(value)?,
                maximum: convert_integer(maximum)?,
                revision: convert_integer(revision)?,
            },
        );
    }
    Ok((checklists, bars))
}

/// Message id, item id, text, who ticked it and when.
type StoredItemRow = (i64, i64, String, Option<i64>, Option<i64>);

/// The checklists of the messages in the JSON array `ids`, items in id order.
async fn checklists_of(
    connection: &mut sqlx::SqliteConnection,
    ids: &str,
) -> Result<HashMap<Snowflake, Checklist>, MessageStorageError> {
    let rows: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT message_id, title, revision FROM checklists \
         WHERE message_id IN (SELECT value FROM json_each(?))",
    )
    .bind(ids)
    .fetch_all(&mut *connection)
    .await?;
    if rows.is_empty() {
        return Ok(HashMap::new());
    }
    let mut checklists = HashMap::with_capacity(rows.len());
    for (message_id, title, revision) in rows {
        checklists.insert(
            to_snowflake(message_id)?,
            Checklist {
                title,
                items: Vec::new(),
                revision: convert_integer(revision)?,
            },
        );
    }
    let items: Vec<StoredItemRow> = sqlx::query_as(
        "SELECT message_id, item_id, text, checked_by_user_id, checked_at_unix_milliseconds \
         FROM checklist_items WHERE message_id IN (SELECT value FROM json_each(?)) \
         ORDER BY message_id, item_id",
    )
    .bind(ids)
    .fetch_all(&mut *connection)
    .await?;
    for (message_id, item_id, text, checked_by, checked_at) in items {
        let checked = match (checked_by, checked_at) {
            (Some(user_id), Some(at)) => Some(ChecklistTick {
                user_id: to_snowflake(user_id)?,
                at_unix_milliseconds: convert_integer(at)?,
            }),
            _ => None,
        };
        if let Some(checklist) = checklists.get_mut(&to_snowflake(message_id)?) {
            checklist.items.push(ChecklistItem {
                id: convert_integer(item_id)?,
                text,
                checked,
            });
        }
    }
    Ok(checklists)
}

/// Applies `change` to the message's checklist and reads it back, in one write transaction.
///
/// # Errors
/// Fails on database errors. Nothing is written then.
pub(crate) async fn change_checklist(
    database: &Database,
    message_id: Snowflake,
    change: &ChecklistChange,
) -> Result<ChecklistOutcome, MessageStorageError> {
    let message = to_sqlite_integer(message_id.raw())?;
    let mut transaction = database.writer().begin_with("BEGIN IMMEDIATE").await?;
    let Some((next_item_id, item_count)): Option<(i64, i64)> = sqlx::query_as(
        "SELECT next_item_id, (SELECT COUNT(*) FROM checklist_items WHERE message_id = ?1) \
         FROM checklists WHERE message_id = ?1",
    )
    .bind(message)
    .fetch_optional(&mut *transaction)
    .await?
    else {
        return Ok(ChecklistOutcome::NoChecklist);
    };
    let item_id = match change {
        ChecklistChange::Tick { item_id, .. }
        | ChecklistChange::Untick { item_id }
        | ChecklistChange::RenameItem { item_id, .. }
        | ChecklistChange::RemoveItem { item_id } => Some(i64::from(*item_id)),
        ChecklistChange::AddItem { .. } => None,
    };
    // The item's text and whether it is ticked.
    let current_item: Option<(String, bool)> = match item_id {
        Some(item_id) => match item_state(&mut transaction, message, item_id).await? {
            None => return Ok(ChecklistOutcome::NoSuchItem),
            item => item,
        },
        None => None,
    };
    let is_checked = current_item.as_ref().is_some_and(|(_, checked)| *checked);
    let write = match change {
        ChecklistChange::Tick {
            user_id,
            at_unix_milliseconds,
            ..
        } if !is_checked => Some(
            sqlx::query(
                "UPDATE checklist_items SET checked_by_user_id = ?, \
                 checked_at_unix_milliseconds = ? WHERE message_id = ? AND item_id = ?",
            )
            .bind(to_sqlite_integer(user_id.raw())?)
            .bind(to_sqlite_integer(*at_unix_milliseconds)?),
        ),
        ChecklistChange::Untick { .. } if is_checked => Some(sqlx::query(
            "UPDATE checklist_items SET checked_by_user_id = NULL, \
             checked_at_unix_milliseconds = NULL WHERE message_id = ? AND item_id = ?",
        )),
        ChecklistChange::RenameItem { text, .. }
            if current_item
                .as_ref()
                .is_some_and(|(current, _)| current != text) =>
        {
            Some(
                sqlx::query(
                    "UPDATE checklist_items SET text = ? WHERE message_id = ? AND item_id = ?",
                )
                .bind(text.as_str()),
            )
        }
        ChecklistChange::RemoveItem { .. } if item_count <= 1 => {
            return Ok(ChecklistOutcome::LastItem);
        }
        ChecklistChange::RemoveItem { .. } => Some(sqlx::query(
            "DELETE FROM checklist_items WHERE message_id = ? AND item_id = ?",
        )),
        ChecklistChange::AddItem { .. }
            if item_count >= convert_integer::<_, i64>(MAXIMUM_CHECKLIST_ITEMS)? =>
        {
            return Ok(ChecklistOutcome::Full);
        }
        ChecklistChange::AddItem { text } => Some(
            sqlx::query("INSERT INTO checklist_items (text, message_id, item_id) VALUES (?, ?, ?)")
                .bind(text.as_str()),
        ),
        ChecklistChange::Tick { .. }
        | ChecklistChange::Untick { .. }
        | ChecklistChange::RenameItem { .. } => None,
    };
    let has_changed = write.is_some();
    if let Some(write) = write {
        write
            .bind(message)
            .bind(item_id.unwrap_or(next_item_id))
            .execute(&mut *transaction)
            .await?;
        bump_revision(&mut transaction, message, item_id.is_none()).await?;
    }
    let checklist = checklists_of(&mut transaction, &json_array_of_ids(&[message_id]))
        .await?
        .remove(&message_id);
    transaction.commit().await?;
    Ok(match (checklist, has_changed) {
        (None, _) => ChecklistOutcome::NoChecklist,
        (Some(checklist), true) => ChecklistOutcome::Changed(checklist),
        (Some(checklist), false) => ChecklistOutcome::Unchanged(checklist),
    })
}

async fn item_state(
    connection: &mut sqlx::SqliteConnection,
    message: i64,
    item_id: i64,
) -> Result<Option<(String, bool)>, MessageStorageError> {
    Ok(sqlx::query_as(
        "SELECT text, checked_at_unix_milliseconds IS NOT NULL FROM checklist_items \
         WHERE message_id = ? AND item_id = ?",
    )
    .bind(message)
    .bind(item_id)
    .fetch_optional(&mut *connection)
    .await?)
}

/// After a change; an added item takes `next_item_id`, which then moves on.
async fn bump_revision(
    connection: &mut sqlx::SqliteConnection,
    message: i64,
    is_item_added: bool,
) -> Result<(), MessageStorageError> {
    sqlx::query(
        "UPDATE checklists SET revision = revision + 1, next_item_id = next_item_id + ? \
         WHERE message_id = ?",
    )
    .bind(i64::from(is_item_added))
    .bind(message)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

/// Replaces the message's progress bar and bumps its revision. `None` when it has none.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn update_progress_bar(
    database: &Database,
    message_id: Snowflake,
    bar: &NewProgressBar,
) -> Result<Option<ProgressBar>, MessageStorageError> {
    let row: Option<i64> = sqlx::query_scalar(
        "UPDATE progress_bars SET label = ?, value = ?, maximum = ?, revision = revision + 1 \
         WHERE message_id = ? RETURNING revision",
    )
    .bind(&bar.label)
    .bind(i64::from(bar.value))
    .bind(i64::from(bar.maximum))
    .bind(to_sqlite_integer(message_id.raw())?)
    .fetch_optional(database.writer())
    .await?;
    row.map(|revision| {
        Ok(ProgressBar {
            label: bar.label.clone(),
            value: bar.value,
            maximum: bar.maximum,
            revision: convert_integer(revision)?,
        })
    })
    .transpose()
}
