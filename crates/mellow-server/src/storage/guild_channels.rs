//! Guild channels and their permission overwrites (migrations 0002 and 0005).

use std::collections::HashMap;

use mellow_protocol::{
    ChannelKind, OverwriteKind, PermissionOverwrite, Permissions, ProfileImage, Snowflake,
    UserLimitChange,
};

use super::Database;
use super::guilds::{
    BEGIN_WRITE_TRANSACTION, GuildStorageError, delete_messages_in_batches,
    permissions_from_integer, permissions_to_integer, to_count,
};
use crate::storage::audit_log::{NewAuditEntry, insert_audit_entry};
use crate::storage::sqlite_integers::{
    convert_integer, lookup_integer, to_snowflake, to_sqlite_integer,
};

pub(super) fn channel_kind_to_text(kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::Category => "category",
        ChannelKind::Text => "text",
        ChannelKind::Voice => "voice",
    }
}

fn channel_kind_from_text(text: &str) -> Result<ChannelKind, GuildStorageError> {
    match text {
        "category" => Ok(ChannelKind::Category),
        "text" => Ok(ChannelKind::Text),
        "voice" => Ok(ChannelKind::Voice),
        other => Err(GuildStorageError::UnknownChannelKind(other.to_owned())),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredChannel {
    pub(crate) id: Snowflake,
    pub(crate) guild_id: Snowflake,
    pub(crate) kind: ChannelKind,
    pub(crate) name: String,
    pub(crate) topic: String,
    pub(crate) parent_id: Option<Snowflake>,
    pub(crate) position: u32,
    pub(crate) icon: Option<ProfileImage>,
    pub(crate) permission_overwrites: Vec<PermissionOverwrite>,
    /// Voice channels only (ADR-0030).
    pub(crate) user_limit: Option<u8>,
}

pub(crate) type ChannelRow = (
    i64,
    i64,
    String,
    String,
    String,
    Option<i64>,
    i64,
    Option<i64>,
    Option<i64>,
);

pub(crate) fn channel_from_row(row: ChannelRow) -> Result<StoredChannel, GuildStorageError> {
    let (id, guild_id, kind, name, topic, parent_id, position, icon_id, user_limit) = row;
    Ok(StoredChannel {
        id: to_snowflake(id)?,
        guild_id: to_snowflake(guild_id)?,
        kind: channel_kind_from_text(&kind)?,
        name,
        topic,
        parent_id: parent_id.map(to_snowflake).transpose()?,
        position: convert_integer::<_, u32>(position)?,
        // Channel icons are never animated.
        icon: icon_id
            .map(|id| {
                Ok::<_, GuildStorageError>(ProfileImage {
                    id: to_snowflake(id)?,
                    is_animated: false,
                })
            })
            .transpose()?,
        permission_overwrites: Vec::new(),
        user_limit: user_limit.map(convert_integer::<_, u8>).transpose()?,
    })
}

fn overwrite_kind_to_text(kind: OverwriteKind) -> &'static str {
    match kind {
        OverwriteKind::Role => "role",
        OverwriteKind::Member => "member",
    }
}

fn overwrite_kind_from_text(text: &str) -> Result<OverwriteKind, GuildStorageError> {
    match text {
        "role" => Ok(OverwriteKind::Role),
        "member" => Ok(OverwriteKind::Member),
        other => Err(GuildStorageError::UnknownOverwriteKind(other.to_owned())),
    }
}

type OverwriteRow = (i64, String, i64, i64);

fn overwrite_from_row(row: OverwriteRow) -> Result<PermissionOverwrite, GuildStorageError> {
    let (target_id, target_kind, allow, deny) = row;
    Ok(PermissionOverwrite {
        id: to_snowflake(target_id)?,
        kind: overwrite_kind_from_text(&target_kind)?,
        allow: permissions_from_integer(allow),
        deny: permissions_from_integer(deny),
    })
}

/// Fills `permission_overwrites` of `channels` with one query, ordered by target for stable output.
async fn attach_overwrites_to_guild_channels(
    database: &Database,
    guild_id: Snowflake,
    channels: &mut [StoredChannel],
) -> Result<(), GuildStorageError> {
    let rows: Vec<(i64, i64, String, i64, i64)> = sqlx::query_as(
        "SELECT channel_permission_overwrites.channel_id, target_id, target_kind, allow, deny \
         FROM channel_permission_overwrites \
         JOIN channels ON channels.id = channel_permission_overwrites.channel_id \
         WHERE channels.guild_id = ? ORDER BY target_id",
    )
    .bind(to_sqlite_integer(guild_id.raw())?)
    .fetch_all(database.readers())
    .await?;
    let mut overwrites_by_channel: HashMap<i64, Vec<PermissionOverwrite>> = HashMap::new();
    for (channel_id, target_id, target_kind, allow, deny) in rows {
        overwrites_by_channel
            .entry(channel_id)
            .or_default()
            .push(overwrite_from_row((target_id, target_kind, allow, deny))?);
    }
    for channel in channels {
        if let Some(overwrites) =
            overwrites_by_channel.remove(&to_sqlite_integer(channel.id.raw())?)
        {
            channel.permission_overwrites = overwrites;
        }
    }
    Ok(())
}

/// # Errors
/// Fails on database errors.
pub(crate) async fn list_guild_channels(
    database: &Database,
    guild_id: Snowflake,
) -> Result<Vec<StoredChannel>, GuildStorageError> {
    let rows: Vec<ChannelRow> = sqlx::query_as(
        "SELECT id, guild_id, kind, name, topic, parent_id, position, icon_id, user_limit FROM channels \
         WHERE guild_id = ? ORDER BY position, id",
    )
    .bind(to_sqlite_integer(guild_id.raw())?)
    .fetch_all(database.readers())
    .await?;
    let mut channels = rows
        .into_iter()
        .map(channel_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    attach_overwrites_to_guild_channels(database, guild_id, &mut channels).await?;
    Ok(channels)
}

/// # Errors
/// Fails on database errors.
pub(crate) async fn find_guild_channel(
    database: &Database,
    channel_id: Snowflake,
) -> Result<Option<StoredChannel>, GuildStorageError> {
    let Some(channel_id) = lookup_integer(channel_id) else {
        return Ok(None);
    };
    let row: Option<ChannelRow> = sqlx::query_as(
        "SELECT id, guild_id, kind, name, topic, parent_id, position, icon_id, user_limit FROM channels \
         WHERE id = ? AND guild_id IS NOT NULL",
    )
    .bind(channel_id)
    .fetch_optional(database.readers())
    .await?;
    attach_overwrites_to_channel(database.readers(), row.map(channel_from_row).transpose()?).await
}

pub(crate) async fn attach_overwrites_to_channel(
    executor: impl sqlx::SqliteExecutor<'_>,
    channel: Option<StoredChannel>,
) -> Result<Option<StoredChannel>, GuildStorageError> {
    let Some(mut channel) = channel else {
        return Ok(None);
    };
    let rows: Vec<OverwriteRow> = sqlx::query_as(
        "SELECT target_id, target_kind, allow, deny FROM channel_permission_overwrites \
         WHERE channel_id = ? ORDER BY target_id",
    )
    .bind(to_sqlite_integer(channel.id.raw())?)
    .fetch_all(executor)
    .await?;
    channel.permission_overwrites = rows
        .into_iter()
        .map(overwrite_from_row)
        .collect::<Result<_, _>>()?;
    Ok(Some(channel))
}

/// Creates or replaces the overwrite of `overwrite.id` in a channel, provided the channel has
/// fewer than `maximum_overwrites` overwrites or already holds one for that target (checked in
/// the same statement). On a replace, only the `held` bits take the new values: bits the actor
/// lacks keep what the row holds now, so a change made meanwhile by someone holding them stays.
///
/// # Errors
/// `LimitReached` at the quota; database errors.
pub(crate) async fn set_overwrite(
    database: &Database,
    channel_id: Snowflake,
    overwrite: &PermissionOverwrite,
    held: Permissions,
    maximum_overwrites: u32,
    audit: &NewAuditEntry,
) -> Result<(), GuildStorageError> {
    let channel_id_integer = to_sqlite_integer(channel_id.raw())?;
    let target_id_integer = to_sqlite_integer(overwrite.id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let written = sqlx::query(
        "INSERT INTO channel_permission_overwrites (channel_id, target_id, target_kind, allow, deny) \
         SELECT ?1, ?2, ?3, ?4, ?5 \
         WHERE (SELECT COUNT(*) FROM channel_permission_overwrites \
                WHERE channel_id = ?1 AND target_id != ?2) < ?6 \
         ON CONFLICT (channel_id, target_id) DO UPDATE SET \
         allow = (allow & ~?7) | (excluded.allow & ?7), deny = (deny & ~?7) | (excluded.deny & ?7)",
    )
    .bind(channel_id_integer)
    .bind(target_id_integer)
    .bind(overwrite_kind_to_text(overwrite.kind))
    .bind(permissions_to_integer(overwrite.allow))
    .bind(permissions_to_integer(overwrite.deny))
    .bind(to_count(maximum_overwrites))
    .bind(permissions_to_integer(held))
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if written == 0 {
        return Err(GuildStorageError::LimitReached);
    }
    insert_audit_entry(&mut transaction, audit).await?;
    transaction.commit().await?;
    Ok(())
}

/// Deletes the overwrite of `target_id`, provided every permission it holds now is in `held`.
/// Returns whether it was deleted.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn delete_overwrite(
    database: &Database,
    channel_id: Snowflake,
    target_id: Snowflake,
    held: Permissions,
    audit: &NewAuditEntry,
) -> Result<bool, GuildStorageError> {
    let (Some(channel_id), Some(target_id)) =
        (lookup_integer(channel_id), lookup_integer(target_id))
    else {
        return Ok(false);
    };
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let deleted = sqlx::query(
        "DELETE FROM channel_permission_overwrites \
         WHERE channel_id = ? AND target_id = ? AND ((allow | deny) & ~?) = 0",
    )
    .bind(channel_id)
    .bind(target_id)
    .bind(permissions_to_integer(held))
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if deleted == 0 {
        return Ok(false);
    }
    insert_audit_entry(&mut transaction, audit).await?;
    transaction.commit().await?;
    Ok(true)
}

/// Inserts the channel at the end of its guild (position = last + 1) and returns it, provided
/// the guild has fewer than `maximum_channels` channels (checked in the same statement).
///
/// # Errors
/// `LimitReached` at the quota, `ReferenceMissing` if the parent was deleted meanwhile.
pub(crate) async fn insert_channel_last(
    database: &Database,
    channel: &StoredChannel,
    maximum_channels: u32,
    audit: &NewAuditEntry,
) -> Result<StoredChannel, GuildStorageError> {
    let guild_id = to_sqlite_integer(channel.guild_id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let result: Result<Option<i64>, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO channels (id, guild_id, kind, name, topic, parent_id, user_limit, position) \
         SELECT ?, ?, ?, ?, ?, ?, ?, COALESCE(MAX(position) + 1, 0) FROM channels \
         WHERE guild_id = ? \
         HAVING COUNT(*) < ? \
         RETURNING position",
    )
    .bind(to_sqlite_integer(channel.id.raw())?)
    .bind(guild_id)
    .bind(channel_kind_to_text(channel.kind))
    .bind(&channel.name)
    .bind(&channel.topic)
    .bind(
        channel
            .parent_id
            .map(|id| to_sqlite_integer(id.raw()))
            .transpose()?,
    )
    .bind(channel.user_limit.map(i64::from))
    .bind(guild_id)
    .bind(to_count(maximum_channels))
    .fetch_optional(&mut *transaction)
    .await;
    let position = match result {
        Ok(Some(position)) => position,
        Ok(None) => return Err(GuildStorageError::LimitReached),
        Err(sqlx::Error::Database(error)) if error.is_foreign_key_violation() => {
            return Err(GuildStorageError::ReferenceMissing);
        }
        Err(error) => return Err(error.into()),
    };
    insert_audit_entry(&mut transaction, audit).await?;
    transaction.commit().await?;
    Ok(StoredChannel {
        position: convert_integer::<_, u32>(position)?,
        ..channel.clone()
    })
}

/// Gives the channels of `guild_id` the positions 0, 1, … and the categories in `new_order`,
/// provided the guild's channels are still `expected_order` (what the permission check saw), and
/// records `audits`. Returns whether it was written.
///
/// # Errors
/// Fails on database errors. Nothing is written on error.
pub(crate) async fn reorder_channels(
    database: &Database,
    guild_id: Snowflake,
    expected_order: &[(Snowflake, Option<Snowflake>)],
    new_order: &[(Snowflake, Option<Snowflake>)],
    audits: &[NewAuditEntry],
) -> Result<bool, GuildStorageError> {
    let guild_id_integer = to_sqlite_integer(guild_id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let stored_order: Vec<(i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, parent_id FROM channels WHERE guild_id = ? ORDER BY position, id",
    )
    .bind(guild_id_integer)
    .fetch_all(&mut *transaction)
    .await?;
    let stored_order: Vec<(Snowflake, Option<Snowflake>)> = stored_order
        .into_iter()
        .map(|(id, parent_id)| Ok((to_snowflake(id)?, parent_id.map(to_snowflake).transpose()?)))
        .collect::<Result<_, GuildStorageError>>()?;
    if stored_order != expected_order {
        return Ok(false);
    }
    for (position, (channel_id, parent_id)) in (0_i64..).zip(new_order) {
        sqlx::query(
            "UPDATE channels SET position = ?, parent_id = ? \
             WHERE id = ? AND guild_id = ? AND (position != ? OR parent_id IS NOT ?)",
        )
        .bind(position)
        .bind(
            parent_id
                .map(|id| to_sqlite_integer(id.raw()))
                .transpose()?,
        )
        .bind(to_sqlite_integer(channel_id.raw())?)
        .bind(guild_id_integer)
        .bind(position)
        .bind(
            parent_id
                .map(|id| to_sqlite_integer(id.raw()))
                .transpose()?,
        )
        .execute(&mut *transaction)
        .await?;
    }
    for audit in audits {
        insert_audit_entry(&mut transaction, audit).await?;
    }
    transaction.commit().await?;
    Ok(true)
}

/// The fields of a channel update; `None` keeps a field.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ChannelFieldChanges<'a> {
    pub(crate) name: Option<&'a str>,
    pub(crate) topic: Option<&'a str>,
    pub(crate) user_limit: UserLimitChange,
}

/// Writes only the given fields, so concurrent edits of different fields do not overwrite each
/// other. Returns the updated channel, or `None` if it no longer exists.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn update_channel_fields(
    database: &Database,
    channel_id: Snowflake,
    changes: ChannelFieldChanges<'_>,
    audit: &NewAuditEntry,
) -> Result<Option<StoredChannel>, GuildStorageError> {
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let row: Option<ChannelRow> = sqlx::query_as(
        "UPDATE channels SET name = COALESCE(?, name), topic = COALESCE(?, topic), \
         user_limit = CASE WHEN ? THEN user_limit ELSE ? END \
         WHERE id = ? AND guild_id IS NOT NULL \
         RETURNING id, guild_id, kind, name, topic, parent_id, position, icon_id, user_limit",
    )
    .bind(changes.name)
    .bind(changes.topic)
    .bind(changes.user_limit.is_keep())
    .bind(match changes.user_limit {
        UserLimitChange::Set(limit) => Some(i64::from(limit)),
        UserLimitChange::Keep | UserLimitChange::Clear => None,
    })
    .bind(to_sqlite_integer(channel_id.raw())?)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    insert_audit_entry(&mut transaction, audit).await?;
    transaction.commit().await?;
    attach_overwrites_to_channel(database.readers(), Some(channel_from_row(row)?)).await
}

/// Deletes the channel's messages in batches first, then the channel and its audit entry in one
/// transaction (whose cascade takes any message sent meanwhile). Await it to the end, from a
/// detached task: stopped half way, the channel stays with part of its history gone.
pub(crate) async fn delete_channel(
    database: &Database,
    channel_id: Snowflake,
    audit: &NewAuditEntry,
) -> Result<(), GuildStorageError> {
    delete_messages_in_batches(
        database,
        "DELETE FROM messages WHERE id IN \
         (SELECT id FROM messages WHERE channel_id = ?1 LIMIT ?2)",
        to_sqlite_integer(channel_id.raw())?,
    )
    .await?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let deleted = sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(to_sqlite_integer(channel_id.raw())?)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
    if deleted > 0 {
        insert_audit_entry(&mut transaction, audit).await?;
    }
    transaction.commit().await?;
    Ok(())
}
