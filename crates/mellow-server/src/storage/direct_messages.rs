//! Direct-message conversations (migration 0009). Their messages live in `messages`.

use mellow_protocol::Snowflake;

use super::Database;

use crate::storage::guilds::{
    BEGIN_WRITE_TRANSACTION, GuildStorageError, lookup_integer, to_id, to_integer,
};

const DIRECT_MESSAGE_CHANNEL_KIND: &str = "direct_message";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredRecipient {
    pub(crate) id: Snowflake,
    pub(crate) username: String,
    pub(crate) display_name: String,
}

/// A conversation as one of its participants sees it: everyone in it but them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredDirectMessageChannel {
    pub(crate) id: Snowflake,
    pub(crate) recipients: Vec<StoredRecipient>,
    pub(crate) last_message_id: Option<Snowflake>,
    /// `Some` for a group (ADR-0026).
    pub(crate) owner_id: Option<Snowflake>,
    pub(crate) name: String,
}

type ConversationRow = (
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<i64>,
    String,
    Option<i64>,
);

/// A macro so queries stay `&'static str` through `concat!` (sqlx refuses runtime-built SQL).
macro_rules! select_conversation_rows {
    () => {
        "SELECT viewer.channel_id, users.id, users.username, users.display_name, \
         (SELECT MAX(messages.id) FROM messages WHERE messages.channel_id = viewer.channel_id), \
         channels.name, group_conversations.owner_id \
         FROM direct_message_participants AS viewer \
         JOIN channels ON channels.id = viewer.channel_id \
         LEFT JOIN group_conversations ON group_conversations.channel_id = viewer.channel_id \
         LEFT JOIN direct_message_participants AS other \
           ON other.channel_id = viewer.channel_id AND other.user_id != viewer.user_id \
         LEFT JOIN users ON users.id = other.user_id \
         WHERE viewer.user_id = ? "
    };
}

/// Rows come ordered by channel, so each conversation's recipients are consecutive. A group whose
/// other people all left has one row without a recipient: it stays listed for the one left.
fn conversations_from_rows(
    rows: Vec<ConversationRow>,
) -> Result<Vec<StoredDirectMessageChannel>, GuildStorageError> {
    let mut conversations: Vec<StoredDirectMessageChannel> = Vec::new();
    for (channel_id, user_id, username, display_name, last_message_id, name, owner_id) in rows {
        let channel_id = to_id(channel_id)?;
        if conversations
            .last()
            .is_none_or(|conversation| conversation.id != channel_id)
        {
            conversations.push(StoredDirectMessageChannel {
                id: channel_id,
                recipients: Vec::new(),
                last_message_id: last_message_id.map(to_id).transpose()?,
                owner_id: owner_id.map(to_id).transpose()?,
                name,
            });
        }
        if let (Some(user_id), Some(username), Some(display_name), Some(conversation)) =
            (user_id, username, display_name, conversations.last_mut())
        {
            conversation.recipients.push(StoredRecipient {
                id: to_id(user_id)?,
                username,
                display_name,
            });
        }
    }
    Ok(conversations)
}

pub(crate) async fn list_user_conversations(
    database: &Database,
    user_id: Snowflake,
) -> Result<Vec<StoredDirectMessageChannel>, GuildStorageError> {
    let Some(user_id) = lookup_integer(user_id) else {
        return Ok(Vec::new());
    };
    let rows: Vec<ConversationRow> = sqlx::query_as(concat!(
        select_conversation_rows!(),
        "ORDER BY viewer.channel_id, users.id"
    ))
    .bind(user_id)
    .fetch_all(database.readers())
    .await?;
    conversations_from_rows(rows)
}

/// `None` unless `user_id` takes part in that conversation.
pub(crate) async fn find_user_conversation(
    database: &Database,
    user_id: Snowflake,
    channel_id: Snowflake,
) -> Result<Option<StoredDirectMessageChannel>, GuildStorageError> {
    let (Some(user_id), Some(channel_id)) = (lookup_integer(user_id), lookup_integer(channel_id))
    else {
        return Ok(None);
    };
    let rows: Vec<ConversationRow> = sqlx::query_as(concat!(
        select_conversation_rows!(),
        "AND viewer.channel_id = ? ORDER BY users.id"
    ))
    .bind(user_id)
    .bind(channel_id)
    .fetch_all(database.readers())
    .await?;
    Ok(conversations_from_rows(rows)?.into_iter().next())
}

/// Everyone taking part in the conversation; empty when it is not one (a guild channel or an
/// unknown id).
pub(crate) async fn list_participant_ids(
    database: &Database,
    channel_id: Snowflake,
) -> Result<Vec<Snowflake>, GuildStorageError> {
    let Some(channel_id) = lookup_integer(channel_id) else {
        return Ok(Vec::new());
    };
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT user_id FROM direct_message_participants WHERE channel_id = ? ORDER BY user_id",
    )
    .bind(channel_id)
    .fetch_all(database.readers())
    .await?;
    rows.into_iter().map(|(user_id,)| to_id(user_id)).collect()
}

/// The one-to-one conversation of the pair, created with `new_channel_id` when there is none
/// yet. Answers its id and whether it was created now.
pub(crate) async fn find_or_create_one_to_one(
    database: &Database,
    user_id: Snowflake,
    other_user_id: Snowflake,
    new_channel_id: Snowflake,
) -> Result<(Snowflake, bool), GuildStorageError> {
    let (lower, higher) = if user_id.raw() < other_user_id.raw() {
        (user_id, other_user_id)
    } else {
        (other_user_id, user_id)
    };
    let lower = to_integer(lower.raw())?;
    let higher = to_integer(higher.raw())?;
    let channel_id = to_integer(new_channel_id.raw())?;
    // The common case, reopening, takes no write lock.
    if let Some(existing) = find_pair(database, lower, higher).await? {
        return Ok((existing, false));
    }
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    sqlx::query(
        "INSERT INTO channels (id, guild_id, kind, name, topic, parent_id, position) \
         VALUES (?, NULL, ?, '', '', NULL, 0)",
    )
    .bind(channel_id)
    .bind(DIRECT_MESSAGE_CHANNEL_KIND)
    .execute(&mut *transaction)
    .await?;
    let inserted = sqlx::query(
        "INSERT INTO direct_message_pairs (lower_user_id, higher_user_id, channel_id) \
         VALUES (?, ?, ?) ON CONFLICT (lower_user_id, higher_user_id) DO NOTHING",
    )
    .bind(lower)
    .bind(higher)
    .bind(channel_id)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if inserted == 0 {
        // Already there: drop the channel inserted above with the transaction.
        transaction.rollback().await?;
        let existing = find_pair(database, lower, higher)
            .await?
            .ok_or(GuildStorageError::Database(sqlx::Error::RowNotFound))?;
        return Ok((existing, false));
    }
    for participant in [lower, higher] {
        sqlx::query("INSERT INTO direct_message_participants (channel_id, user_id) VALUES (?, ?)")
            .bind(channel_id)
            .bind(participant)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok((new_channel_id, true))
}

async fn find_pair(
    database: &Database,
    lower: i64,
    higher: i64,
) -> Result<Option<Snowflake>, GuildStorageError> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT channel_id FROM direct_message_pairs WHERE lower_user_id = ? AND higher_user_id = ?",
    )
    .bind(lower)
    .bind(higher)
    .fetch_optional(database.readers())
    .await?;
    row.map(|(channel_id,)| to_id(channel_id)).transpose()
}

/// The owner of the group; `None` for a one-to-one conversation or anything else.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn group_owner_id(
    database: &Database,
    channel_id: Snowflake,
) -> Result<Option<Snowflake>, GuildStorageError> {
    let Some(channel_id) = lookup_integer(channel_id) else {
        return Ok(None);
    };
    let owner_id: Option<i64> =
        sqlx::query_scalar("SELECT owner_id FROM group_conversations WHERE channel_id = ?")
            .bind(channel_id)
            .fetch_optional(database.readers())
            .await?;
    owner_id.map(to_id).transpose()
}

/// Creates a group of `participant_ids` (the owner included) as `channel_id`.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn create_group(
    database: &Database,
    channel_id: Snowflake,
    owner_id: Snowflake,
    participant_ids: &[Snowflake],
    name: &str,
) -> Result<(), GuildStorageError> {
    let channel = to_integer(channel_id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    sqlx::query(
        "INSERT INTO channels (id, guild_id, kind, name, topic, parent_id, position) \
         VALUES (?, NULL, ?, ?, '', NULL, 0)",
    )
    .bind(channel)
    .bind(DIRECT_MESSAGE_CHANNEL_KIND)
    .bind(name)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("INSERT INTO group_conversations (channel_id, owner_id) VALUES (?, ?)")
        .bind(channel)
        .bind(to_integer(owner_id.raw())?)
        .execute(&mut *transaction)
        .await?;
    for participant_id in participant_ids {
        sqlx::query("INSERT INTO direct_message_participants (channel_id, user_id) VALUES (?, ?)")
            .bind(channel)
            .bind(to_integer(participant_id.raw())?)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroupAddOutcome {
    Added,
    AlreadyIn,
    /// The group already has `maximum` people.
    Full,
    NotAGroup,
}

/// Adds `user_id` to the group, deciding on its rows as they are inside the transaction.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn add_group_participant(
    database: &Database,
    channel_id: Snowflake,
    user_id: Snowflake,
    maximum: usize,
) -> Result<GroupAddOutcome, GuildStorageError> {
    let (channel, user) = (to_integer(channel_id.raw())?, to_integer(user_id.raw())?);
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let is_group: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM group_conversations WHERE channel_id = ?)",
    )
    .bind(channel)
    .fetch_one(&mut *transaction)
    .await?;
    if !is_group {
        return Ok(GroupAddOutcome::NotAGroup);
    }
    let participant_ids: Vec<i64> =
        sqlx::query_scalar("SELECT user_id FROM direct_message_participants WHERE channel_id = ?")
            .bind(channel)
            .fetch_all(&mut *transaction)
            .await?;
    let outcome = if participant_ids.contains(&user) {
        GroupAddOutcome::AlreadyIn
    } else if participant_ids.len() >= maximum {
        GroupAddOutcome::Full
    } else {
        sqlx::query("INSERT INTO direct_message_participants (channel_id, user_id) VALUES (?, ?)")
            .bind(channel)
            .bind(user)
            .execute(&mut *transaction)
            .await?;
        GroupAddOutcome::Added
    };
    transaction.commit().await?;
    Ok(outcome)
}

/// What `remove_group_participant` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GroupRemoval {
    pub(crate) was_participant: bool,
    /// The last person left: the conversation and its messages are gone.
    pub(crate) deleted_conversation: bool,
}

/// Removes `user_id` from the group; the lowest remaining id becomes owner when the owner leaves,
/// and the conversation goes with its last participant (ADR-0026).
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn remove_group_participant(
    database: &Database,
    channel_id: Snowflake,
    user_id: Snowflake,
) -> Result<GroupRemoval, GuildStorageError> {
    let (channel, user) = (to_integer(channel_id.raw())?, to_integer(user_id.raw())?);
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let removed = sqlx::query(
        "DELETE FROM direct_message_participants WHERE channel_id = ? AND user_id = ? \
         AND EXISTS (SELECT 1 FROM group_conversations WHERE channel_id = ?)",
    )
    .bind(channel)
    .bind(user)
    .bind(channel)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if removed == 0 {
        return Ok(GroupRemoval {
            was_participant: false,
            deleted_conversation: false,
        });
    }
    let next_owner: Option<i64> = sqlx::query_scalar(
        "SELECT MIN(user_id) FROM direct_message_participants WHERE channel_id = ?",
    )
    .bind(channel)
    .fetch_one(&mut *transaction)
    .await?;
    let deleted_conversation = match next_owner {
        None => {
            sqlx::query("DELETE FROM channels WHERE id = ?")
                .bind(channel)
                .execute(&mut *transaction)
                .await?;
            true
        }
        Some(next_owner) => {
            sqlx::query(
                "UPDATE group_conversations SET owner_id = ? WHERE channel_id = ? AND owner_id = ?",
            )
            .bind(next_owner)
            .bind(channel)
            .bind(user)
            .execute(&mut *transaction)
            .await?;
            false
        }
    };
    transaction.commit().await?;
    Ok(GroupRemoval {
        was_participant: true,
        deleted_conversation,
    })
}

/// Renames the group; `false` if it is not one.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn rename_group(
    database: &Database,
    channel_id: Snowflake,
    name: &str,
) -> Result<bool, GuildStorageError> {
    let renamed = sqlx::query(
        "UPDATE channels SET name = ? WHERE id = ? \
         AND EXISTS (SELECT 1 FROM group_conversations WHERE channel_id = channels.id)",
    )
    .bind(name)
    .bind(to_integer(channel_id.raw())?)
    .execute(database.writer())
    .await?
    .rows_affected();
    Ok(renamed == 1)
}
