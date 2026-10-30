//! Custom emoji (migration 0020, ADR-0028). Each one's picture is the guild image of the same id.

use mellow_protocol::{AuditChange, AuditChangeKey, CustomEmoji, Snowflake};
use sqlx::SqliteConnection;

use super::Database;

use crate::storage::audit_log::{NewAuditEntry, insert_audit_entry};
use crate::storage::guild_images::{StoredGuildImage, delete_guild_image, insert_guild_image};
use crate::storage::guilds::{
    BEGIN_WRITE_TRANSACTION, GuildStorageError, lookup_integer, to_id, to_integer,
};
use crate::storage::profile_images::NewProfileImage;

/// A stored emoji and the guild it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredGuildEmoji {
    pub(crate) guild_id: Snowflake,
    pub(crate) emoji: CustomEmoji,
}

/// What creating or renaming an emoji did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EmojiWriteOutcome {
    /// Written: the guild's emoji afterwards, as `GUILD_EMOJIS_UPDATE` carries them.
    Written(Vec<CustomEmoji>),
    /// A rename to the name it already has: nothing written or recorded.
    Unchanged(CustomEmoji),
    /// No such guild, or no such emoji in it.
    NotFound,
    NameTaken,
    /// The guild holds as many emoji of that kind (still or animated) as it may.
    LimitReached,
}

/// What deleting an emoji did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EmojiDeleteOutcome {
    /// `emojis` are the guild's afterwards; `image` is the picture whose files the caller deletes.
    Deleted {
        emojis: Vec<CustomEmoji>,
        image: StoredGuildImage,
    },
    NotFound,
}

type EmojiRow = (i64, String, bool);

fn emoji_from_row((id, name, is_animated): EmojiRow) -> Result<CustomEmoji, GuildStorageError> {
    Ok(CustomEmoji {
        id: to_id(id)?,
        name,
        is_animated,
    })
}

const SELECT_EMOJIS_OF_GUILD: &str = "SELECT emojis.id, emojis.name, images.still_id IS NOT NULL \
     FROM guild_emojis AS emojis JOIN guild_images AS images ON images.id = emojis.id \
     WHERE emojis.guild_id = ? ORDER BY emojis.id";

async fn emojis_of_guild(
    connection: &mut SqliteConnection,
    guild_id: i64,
) -> Result<Vec<CustomEmoji>, GuildStorageError> {
    let rows: Vec<EmojiRow> = sqlx::query_as(SELECT_EMOJIS_OF_GUILD)
        .bind(guild_id)
        .fetch_all(&mut *connection)
        .await?;
    rows.into_iter().map(emoji_from_row).collect()
}

/// The guild's emoji, oldest first.
pub(crate) async fn list_guild_emojis(
    database: &Database,
    guild_id: Snowflake,
) -> Result<Vec<CustomEmoji>, GuildStorageError> {
    let Some(guild_id) = lookup_integer(guild_id) else {
        return Ok(Vec::new());
    };
    let rows: Vec<EmojiRow> = sqlx::query_as(SELECT_EMOJIS_OF_GUILD)
        .bind(guild_id)
        .fetch_all(database.readers())
        .await?;
    rows.into_iter().map(emoji_from_row).collect()
}

pub(crate) async fn find_guild_emoji(
    database: &Database,
    id: Snowflake,
) -> Result<Option<StoredGuildEmoji>, GuildStorageError> {
    let Some(id) = lookup_integer(id) else {
        return Ok(None);
    };
    let row: Option<(i64, i64, String, bool)> = sqlx::query_as(
        "SELECT emojis.guild_id, emojis.id, emojis.name, images.still_id IS NOT NULL \
         FROM guild_emojis AS emojis JOIN guild_images AS images ON images.id = emojis.id \
         WHERE emojis.id = ?",
    )
    .bind(id)
    .fetch_optional(database.readers())
    .await?;
    row.map(|(guild_id, id, name, is_animated)| {
        Ok(StoredGuildEmoji {
            guild_id: to_id(guild_id)?,
            emoji: emoji_from_row((id, name, is_animated))?,
        })
    })
    .transpose()
}

fn with_name_change(audit: &NewAuditEntry, old_name: &str, new_name: &str) -> NewAuditEntry {
    NewAuditEntry {
        changes: vec![AuditChange {
            key: AuditChangeKey::Name,
            old_value: old_name.to_owned(),
            new_value: new_name.to_owned(),
        }],
        ..audit.clone()
    }
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(error) if error.is_unique_violation())
}

/// Stores the emoji `image` names `name` in the guild, provided the guild holds fewer than
/// `maximum_per_kind` emoji of its kind (still or animated) and no other emoji of that name,
/// recording `audit` in the same transaction.
///
/// # Errors
/// Fails on database errors; nothing is written then.
pub(crate) async fn insert_guild_emoji(
    database: &Database,
    guild_id: Snowflake,
    name: &str,
    image: NewProfileImage,
    maximum_per_kind: u32,
    audit: &NewAuditEntry,
    now_unix_milliseconds: u64,
) -> Result<EmojiWriteOutcome, GuildStorageError> {
    let guild = to_integer(guild_id.raw())?;
    let is_animated = image.still_id.is_some();
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let is_guild_known: Option<i64> = sqlx::query_scalar("SELECT 1 FROM guilds WHERE id = ?")
        .bind(guild)
        .fetch_optional(&mut *transaction)
        .await?;
    if is_guild_known.is_none() {
        return Ok(EmojiWriteOutcome::NotFound);
    }
    let same_kind_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM guild_emojis AS emojis \
         JOIN guild_images AS images ON images.id = emojis.id \
         WHERE emojis.guild_id = ? AND (images.still_id IS NOT NULL) = ?",
    )
    .bind(guild)
    .bind(is_animated)
    .fetch_one(&mut *transaction)
    .await?;
    if same_kind_count >= i64::from(maximum_per_kind) {
        return Ok(EmojiWriteOutcome::LimitReached);
    }
    let Some(id) = insert_guild_image(
        &mut transaction,
        guild,
        None,
        Some(image),
        now_unix_milliseconds,
    )
    .await?
    else {
        return Err(GuildStorageError::ReferenceMissing);
    };
    let inserted = sqlx::query("INSERT INTO guild_emojis (id, guild_id, name) VALUES (?, ?, ?)")
        .bind(id)
        .bind(guild)
        .bind(name)
        .execute(&mut *transaction)
        .await;
    match inserted {
        Ok(_) => {}
        Err(error) if is_unique_violation(&error) => return Ok(EmojiWriteOutcome::NameTaken),
        Err(error) => return Err(error.into()),
    }
    insert_audit_entry(&mut transaction, &with_name_change(audit, "", name)).await?;
    // Read before the commit: once it is done, nothing may fail before the caller announces it.
    let emojis = emojis_of_guild(&mut transaction, guild).await?;
    transaction.commit().await?;
    Ok(EmojiWriteOutcome::Written(emojis))
}

/// Renames the guild's emoji, recording `audit` in the same transaction.
///
/// # Errors
/// Fails on database errors; nothing is written then.
pub(crate) async fn rename_guild_emoji(
    database: &Database,
    guild_id: Snowflake,
    emoji_id: Snowflake,
    name: &str,
    audit: &NewAuditEntry,
) -> Result<EmojiWriteOutcome, GuildStorageError> {
    let (Some(guild), Some(id)) = (lookup_integer(guild_id), lookup_integer(emoji_id)) else {
        return Ok(EmojiWriteOutcome::NotFound);
    };
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let old_name: Option<String> =
        sqlx::query_scalar("SELECT name FROM guild_emojis WHERE id = ? AND guild_id = ?")
            .bind(id)
            .bind(guild)
            .fetch_optional(&mut *transaction)
            .await?;
    let Some(old_name) = old_name else {
        return Ok(EmojiWriteOutcome::NotFound);
    };
    if old_name == name {
        let emoji = emojis_of_guild(&mut transaction, guild)
            .await?
            .into_iter()
            .find(|emoji| emoji.id == emoji_id)
            .ok_or(GuildStorageError::ReferenceMissing)?;
        return Ok(EmojiWriteOutcome::Unchanged(emoji));
    }
    let renamed = sqlx::query("UPDATE guild_emojis SET name = ? WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(&mut *transaction)
        .await;
    match renamed {
        Ok(_) => {}
        Err(error) if is_unique_violation(&error) => return Ok(EmojiWriteOutcome::NameTaken),
        Err(error) => return Err(error.into()),
    }
    insert_audit_entry(&mut transaction, &with_name_change(audit, &old_name, name)).await?;
    let emojis = emojis_of_guild(&mut transaction, guild).await?;
    transaction.commit().await?;
    Ok(EmojiWriteOutcome::Written(emojis))
}

/// Deletes the guild's emoji and its picture's row, recording `audit` in the same transaction.
///
/// # Errors
/// Fails on database errors; nothing is written then.
pub(crate) async fn delete_guild_emoji(
    database: &Database,
    guild_id: Snowflake,
    emoji_id: Snowflake,
    audit: &NewAuditEntry,
) -> Result<EmojiDeleteOutcome, GuildStorageError> {
    let (Some(guild), Some(id)) = (lookup_integer(guild_id), lookup_integer(emoji_id)) else {
        return Ok(EmojiDeleteOutcome::NotFound);
    };
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let name: Option<String> =
        sqlx::query_scalar("SELECT name FROM guild_emojis WHERE id = ? AND guild_id = ?")
            .bind(id)
            .bind(guild)
            .fetch_optional(&mut *transaction)
            .await?;
    let Some(name) = name else {
        return Ok(EmojiDeleteOutcome::NotFound);
    };
    // The emoji's row goes with its picture's (ON DELETE CASCADE).
    let image = delete_guild_image(&mut transaction, Some(id))
        .await?
        .ok_or(GuildStorageError::ReferenceMissing)?;
    insert_audit_entry(&mut transaction, &with_name_change(audit, &name, "")).await?;
    let emojis = emojis_of_guild(&mut transaction, guild).await?;
    transaction.commit().await?;
    Ok(EmojiDeleteOutcome::Deleted { emojis, image })
}
