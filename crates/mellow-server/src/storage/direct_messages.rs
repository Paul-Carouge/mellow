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
}

type ConversationRow = (i64, i64, String, String, Option<i64>);

/// A macro so queries stay `&'static str` through `concat!` (sqlx refuses runtime-built SQL).
macro_rules! select_conversation_rows {
    () => {
        "SELECT viewer.channel_id, users.id, users.username, users.display_name, \
         (SELECT MAX(messages.id) FROM messages WHERE messages.channel_id = viewer.channel_id) \
         FROM direct_message_participants AS viewer \
         JOIN direct_message_participants AS other \
           ON other.channel_id = viewer.channel_id AND other.user_id != viewer.user_id \
         JOIN users ON users.id = other.user_id \
         WHERE viewer.user_id = ? "
    };
}

/// Rows come ordered by channel, so each conversation's recipients are consecutive.
fn conversations_from_rows(
    rows: Vec<ConversationRow>,
) -> Result<Vec<StoredDirectMessageChannel>, GuildStorageError> {
    let mut conversations: Vec<StoredDirectMessageChannel> = Vec::new();
    for (channel_id, user_id, username, display_name, last_message_id) in rows {
        let channel_id = to_id(channel_id)?;
        let recipient = StoredRecipient {
            id: to_id(user_id)?,
            username,
            display_name,
        };
        match conversations.last_mut() {
            Some(conversation) if conversation.id == channel_id => {
                conversation.recipients.push(recipient);
            }
            _ => conversations.push(StoredDirectMessageChannel {
                id: channel_id,
                recipients: vec![recipient],
                last_message_id: last_message_id.map(to_id).transpose()?,
            }),
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

pub(crate) async fn share_a_guild(
    database: &Database,
    user_id: Snowflake,
    other_user_id: Snowflake,
) -> Result<bool, GuildStorageError> {
    let (Some(user_id), Some(other_user_id)) =
        (lookup_integer(user_id), lookup_integer(other_user_id))
    else {
        return Ok(false);
    };
    let (shares,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM guild_members AS mine \
         JOIN guild_members AS theirs ON theirs.guild_id = mine.guild_id \
         WHERE mine.user_id = ? AND theirs.user_id = ?)",
    )
    .bind(user_id)
    .bind(other_user_id)
    .fetch_one(database.readers())
    .await?;
    Ok(shares)
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
