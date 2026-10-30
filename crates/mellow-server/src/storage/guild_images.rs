//! Guild and channel icons (migration 0014); their sealed files are in `attachment_files`.

use mellow_protocol::{AuditChange, AuditChangeKey, GuildSummary, ProfileImage, Snowflake};
use sqlx::SqliteConnection;

use super::Database;

use crate::storage::audit_log::{NewAuditEntry, insert_audit_entry};
use crate::storage::guilds::{
    BEGIN_WRITE_TRANSACTION, ChannelRow, GuildStorageError, StoredChannel,
    attach_overwrites_to_channel, channel_from_row, lookup_integer, to_id, to_integer,
};
use crate::storage::profile_images::NewProfileImage;

/// A stored icon version and what it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StoredGuildImage {
    pub(crate) id: Snowflake,
    pub(crate) guild_id: Snowflake,
    /// Set for a channel's icon.
    pub(crate) channel_id: Option<Snowflake>,
    pub(crate) still_id: Option<Snowflake>,
}

/// What replacing an icon did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IconChange<T> {
    NotFound,
    /// No icon was removed from a guild or channel without one: nothing written or recorded.
    Unchanged,
    /// `replaced` is the version whose files the caller deletes.
    Changed {
        updated: T,
        replaced: Option<StoredGuildImage>,
    },
}

type GuildImageRow = (i64, i64, Option<i64>, Option<i64>);

fn guild_image_from_row(row: GuildImageRow) -> Result<StoredGuildImage, GuildStorageError> {
    let (id, guild_id, channel_id, still_id) = row;
    Ok(StoredGuildImage {
        id: to_id(id)?,
        guild_id: to_id(guild_id)?,
        channel_id: channel_id.map(to_id).transpose()?,
        still_id: still_id.map(to_id).transpose()?,
    })
}

fn icon_id_as_audit_text(id: Option<i64>) -> String {
    id.map(|id| id.to_string()).unwrap_or_default()
}

/// Inserts the new version's row, if any, and answers its id for the owner's column.
pub(crate) async fn insert_guild_image(
    connection: &mut SqliteConnection,
    guild_id: i64,
    channel_id: Option<i64>,
    new_image: Option<NewProfileImage>,
    now_unix_milliseconds: u64,
) -> Result<Option<i64>, GuildStorageError> {
    let Some(image) = new_image else {
        return Ok(None);
    };
    let id = to_integer(image.id.raw())?;
    sqlx::query(
        "INSERT INTO guild_images (id, guild_id, channel_id, still_id, size_bytes, \
         created_at_unix_milliseconds) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(guild_id)
    .bind(channel_id)
    .bind(image.still_id.map(|id| to_integer(id.raw())).transpose()?)
    .bind(to_integer(image.size_bytes)?)
    .bind(to_integer(now_unix_milliseconds)?)
    .execute(&mut *connection)
    .await?;
    Ok(Some(id))
}

pub(crate) async fn delete_guild_image(
    connection: &mut SqliteConnection,
    id: Option<i64>,
) -> Result<Option<StoredGuildImage>, GuildStorageError> {
    let Some(id) = id else {
        return Ok(None);
    };
    let row: Option<GuildImageRow> = sqlx::query_as(
        "DELETE FROM guild_images WHERE id = ? RETURNING id, guild_id, channel_id, still_id",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await?;
    row.map(guild_image_from_row).transpose()
}

fn with_icon_change(
    audit: &NewAuditEntry,
    previous: Option<i64>,
    new: Option<i64>,
) -> NewAuditEntry {
    NewAuditEntry {
        changes: vec![AuditChange {
            key: AuditChangeKey::Icon,
            old_value: icon_id_as_audit_text(previous),
            new_value: icon_id_as_audit_text(new),
        }],
        ..audit.clone()
    }
}

/// Stores a new version and makes it the guild's icon (`None` removes it), recording `audit`
/// with the change, in one transaction. Answers the guild as `GUILD_UPDATE` carries it.
///
/// # Errors
/// Fails on database errors; nothing is written then.
pub(crate) async fn replace_guild_icon(
    database: &Database,
    guild_id: Snowflake,
    new_image: Option<NewProfileImage>,
    audit: &NewAuditEntry,
    now_unix_milliseconds: u64,
) -> Result<IconChange<GuildSummary>, GuildStorageError> {
    let guild = to_integer(guild_id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let previous: Option<Option<i64>> =
        sqlx::query_scalar("SELECT icon_id FROM guilds WHERE id = ?")
            .bind(guild)
            .fetch_optional(&mut *transaction)
            .await?;
    let Some(previous) = previous else {
        return Ok(IconChange::NotFound);
    };
    if previous.is_none() && new_image.is_none() {
        return Ok(IconChange::Unchanged);
    }
    let new_id = insert_guild_image(
        &mut transaction,
        guild,
        None,
        new_image,
        now_unix_milliseconds,
    )
    .await?;
    let is_animated = new_image.is_some_and(|image| image.still_id.is_some());
    let name: String = sqlx::query_scalar(
        "UPDATE guilds SET icon_id = ?, icon_is_animated = ? WHERE id = ? RETURNING name",
    )
    .bind(new_id)
    .bind(i64::from(is_animated))
    .bind(guild)
    .fetch_one(&mut *transaction)
    .await?;
    let replaced = delete_guild_image(&mut transaction, previous).await?;
    insert_audit_entry(&mut transaction, &with_icon_change(audit, previous, new_id)).await?;
    transaction.commit().await?;
    Ok(IconChange::Changed {
        updated: GuildSummary {
            id: guild_id,
            name,
            icon: new_image.map(|image| ProfileImage {
                id: image.id,
                is_animated,
            }),
        },
        replaced,
    })
}

/// Stores a new version and makes it the guild channel's icon (`None` removes it), recording
/// `audit` with the change, in one transaction. Answers the updated channel.
///
/// # Errors
/// Fails on database errors; nothing is written then.
pub(crate) async fn replace_channel_icon(
    database: &Database,
    channel_id: Snowflake,
    new_image: Option<NewProfileImage>,
    audit: &NewAuditEntry,
    now_unix_milliseconds: u64,
) -> Result<IconChange<StoredChannel>, GuildStorageError> {
    let channel = to_integer(channel_id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let row: Option<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT guild_id, icon_id FROM channels WHERE id = ? AND guild_id IS NOT NULL",
    )
    .bind(channel)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some((guild, previous)) = row else {
        return Ok(IconChange::NotFound);
    };
    if previous.is_none() && new_image.is_none() {
        return Ok(IconChange::Unchanged);
    }
    let new_id = insert_guild_image(
        &mut transaction,
        guild,
        Some(channel),
        new_image,
        now_unix_milliseconds,
    )
    .await?;
    let row: ChannelRow = sqlx::query_as(
        "UPDATE channels SET icon_id = ? WHERE id = ? \
         RETURNING id, guild_id, kind, name, topic, parent_id, position, icon_id",
    )
    .bind(new_id)
    .bind(channel)
    .fetch_one(&mut *transaction)
    .await?;
    let replaced = delete_guild_image(&mut transaction, previous).await?;
    insert_audit_entry(&mut transaction, &with_icon_change(audit, previous, new_id)).await?;
    // Read before the commit: once it is done, nothing may fail before the caller announces it.
    let updated = attach_overwrites_to_channel(&mut *transaction, Some(channel_from_row(row)?))
        .await?
        .ok_or(GuildStorageError::ReferenceMissing)?;
    transaction.commit().await?;
    Ok(IconChange::Changed { updated, replaced })
}

pub(crate) async fn find_guild_image(
    database: &Database,
    id: Snowflake,
) -> Result<Option<StoredGuildImage>, GuildStorageError> {
    let Some(id) = lookup_integer(id) else {
        return Ok(None);
    };
    let row: Option<GuildImageRow> =
        sqlx::query_as("SELECT id, guild_id, channel_id, still_id FROM guild_images WHERE id = ?")
            .bind(id)
            .fetch_optional(database.readers())
            .await?;
    row.map(guild_image_from_row).transpose()
}

/// Every file id of every icon, still frames included, for the orphan sweep.
pub(crate) async fn all_guild_image_file_ids(
    database: &Database,
) -> Result<Vec<Snowflake>, GuildStorageError> {
    let rows: Vec<(i64, Option<i64>)> = sqlx::query_as("SELECT id, still_id FROM guild_images")
        .fetch_all(database.readers())
        .await?;
    let mut ids = Vec::with_capacity(rows.len() * 2);
    for (id, still_id) in rows {
        ids.push(to_id(id)?);
        if let Some(still_id) = still_id {
            ids.push(to_id(still_id)?);
        }
    }
    Ok(ids)
}
