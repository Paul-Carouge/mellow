//! Guilds themselves, membership and permission snapshots, and what the `guild_*` storage modules
//! share (migrations 0002 and 0005).

use std::collections::HashMap;

use super::Database;

use mellow_protocol::{
    AuditChange, AuditChangeKey, GuildSummary, Permissions, ProfileImage, Role, Snowflake,
};

use crate::storage::audit_log::{NewAuditEntry, insert_audit_entry};
use crate::storage::guild_channels::{StoredChannel, channel_kind_to_text};
use crate::storage::guild_roles::{RoleRow, role_from_row, roles_of_guild};
use crate::storage::sqlite_integers::{
    IntegerOutOfRange, lookup_integer, to_snowflake, to_sqlite_integer,
};

/// Takes the write lock at once, so a read-then-write transaction waits for `busy_timeout`
/// instead of failing with `SQLITE_BUSY_SNAPSHOT` when another connection wrote in between.
pub(crate) const BEGIN_WRITE_TRANSACTION: &str = "BEGIN IMMEDIATE";

#[derive(Debug, thiserror::Error)]
pub(crate) enum GuildStorageError {
    #[error(transparent)]
    IntegerOutOfRange(#[from] IntegerOutOfRange),
    #[error("the channel kind stored in the database is unknown: {0}")]
    UnknownChannelKind(String),
    #[error("the overwrite kind stored in the database is unknown: {0}")]
    UnknownOverwriteKind(String),
    #[error("the audit action stored in the database is unknown: {0}")]
    UnknownAuditAction(String),
    #[error("the audit changes stored in the database cannot be read")]
    UnreadableAuditChanges,
    /// A quota checked atomically with the insert was reached.
    #[error("limit reached")]
    LimitReached,
    /// The referenced parent channel no longer exists.
    #[error("referenced row is missing")]
    ReferenceMissing,
    #[error("database error")]
    Database(#[from] sqlx::Error),
}

pub(super) fn to_count(value: u32) -> i64 {
    i64::from(value)
}

/// Permission bitfields use all 64 bits; SQLite stores them as the same bits in a signed integer.
pub(super) fn permissions_to_integer(permissions: Permissions) -> i64 {
    i64::from_ne_bytes(permissions.bits().to_ne_bytes())
}

pub(super) fn permissions_from_integer(value: i64) -> Permissions {
    Permissions::from_bits_retain(u64::from_ne_bytes(value.to_ne_bytes()))
}

/// Everything needed to create a guild in one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewGuild {
    pub(crate) id: Snowflake,
    pub(crate) name: String,
    pub(crate) owner_id: Snowflake,
    pub(crate) everyone_permissions: Permissions,
    pub(crate) created_at_unix_milliseconds: u64,
    /// Created in order, positions 0, 1, …
    pub(crate) initial_channels: Vec<StoredChannel>,
}

/// The guild, its @everyone role (same id), the owner's membership and the initial channels,
/// all or nothing, provided the owner owns fewer than `maximum_owned_guilds` guilds.
///
/// # Errors
/// `LimitReached` at the quota; database errors. Nothing is written on error.
pub(crate) async fn insert_guild(
    database: &Database,
    guild: &NewGuild,
    maximum_owned_guilds: u32,
) -> Result<(), GuildStorageError> {
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let guild_id = to_sqlite_integer(guild.id.raw())?;
    let owner_id = to_sqlite_integer(guild.owner_id.raw())?;
    let created_at = to_sqlite_integer(guild.created_at_unix_milliseconds)?;
    let owned_guilds: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM guilds WHERE owner_id = ?")
        .bind(owner_id)
        .fetch_one(&mut *transaction)
        .await?;
    if owned_guilds >= to_count(maximum_owned_guilds) {
        return Err(GuildStorageError::LimitReached);
    }
    sqlx::query(
        "INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) VALUES (?, ?, ?, ?)",
    )
    .bind(guild_id)
    .bind(&guild.name)
    .bind(owner_id)
    .bind(created_at)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO roles (id, guild_id, name, permissions, position) VALUES (?, ?, '@everyone', ?, 0)",
    )
    .bind(guild_id)
    .bind(guild_id)
    .bind(permissions_to_integer(guild.everyone_permissions))
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO guild_members (guild_id, user_id, joined_at_unix_milliseconds) VALUES (?, ?, ?)",
    )
    .bind(guild_id)
    .bind(owner_id)
    .bind(created_at)
    .execute(&mut *transaction)
    .await?;
    for channel in &guild.initial_channels {
        sqlx::query(
            "INSERT INTO channels (id, guild_id, kind, name, topic, parent_id, position) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
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
        .bind(i64::from(channel.position))
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

/// What a member's access depends on in one guild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Membership {
    pub(crate) guild_name: String,
    pub(crate) guild_icon: Option<ProfileImage>,
    pub(crate) owner_id: Snowflake,
    /// The member's roles, `@everyone` included, by position.
    pub(crate) member_roles: Vec<Role>,
}

impl Membership {
    /// Without `@everyone`, as on the wire.
    pub(crate) fn member_role_ids(&self, guild_id: Snowflake) -> Vec<Snowflake> {
        self.member_roles
            .iter()
            .map(|role| role.id)
            .filter(|role_id| *role_id != guild_id)
            .collect()
    }
}

fn guild_icon(
    icon_id: Option<i64>,
    is_animated: i64,
) -> Result<Option<ProfileImage>, GuildStorageError> {
    icon_id
        .map(|id| {
            Ok(ProfileImage {
                id: to_snowflake(id)?,
                is_animated: is_animated != 0,
            })
        })
        .transpose()
}

/// `None` if the guild does not exist or the user is not a member: callers answer both the same
/// way, so outsiders cannot probe which guilds exist.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn find_membership(
    database: &Database,
    guild_id: Snowflake,
    user_id: Snowflake,
) -> Result<Option<Membership>, GuildStorageError> {
    let (Some(guild_id), Some(user_id)) = (lookup_integer(guild_id), lookup_integer(user_id))
    else {
        return Ok(None);
    };
    let row: Option<(String, i64, Option<i64>, i64)> = sqlx::query_as(
        "SELECT guilds.name, guilds.owner_id, guilds.icon_id, guilds.icon_is_animated \
         FROM guild_members \
         JOIN guilds ON guilds.id = guild_members.guild_id \
         WHERE guild_members.guild_id = ? AND guild_members.user_id = ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(database.readers())
    .await?;
    let Some((guild_name, owner_id, icon_id, icon_is_animated)) = row else {
        return Ok(None);
    };
    let rows: Vec<RoleRow> = sqlx::query_as(
        "SELECT id, name, color, position, permissions, is_displayed_separately, is_mentionable \
         FROM roles WHERE id = ? OR id IN \
         (SELECT role_id FROM member_roles WHERE guild_id = ? AND user_id = ?) \
         ORDER BY position, id",
    )
    .bind(guild_id)
    .bind(guild_id)
    .bind(user_id)
    .fetch_all(database.readers())
    .await?;
    Ok(Some(Membership {
        guild_name,
        guild_icon: guild_icon(icon_id, icon_is_animated)?,
        owner_id: to_snowflake(owner_id)?,
        member_roles: rows
            .into_iter()
            .map(role_from_row)
            .collect::<Result<_, _>>()?,
    }))
}

/// Everything that decides a member's guild permissions apart from channel overwrites, as one
/// consistent read. `version` is the guild's `permission_version` it was read at.
#[derive(Debug)]
pub(crate) struct StoredPermissionSnapshot {
    pub(crate) version: i64,
    pub(crate) owner_id: Snowflake,
    /// By position, `@everyone` first.
    pub(crate) roles: Vec<Role>,
    /// Every member, with the ids of their roles (`@everyone` left out).
    pub(crate) role_ids_by_member: HashMap<Snowflake, Vec<Snowflake>>,
}

/// The guild's `permission_version`, bumped by triggers on members, roles, role assignments and
/// the owner (migration 0015); `None` if the guild does not exist.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn permission_version(
    database: &Database,
    guild_id: Snowflake,
) -> Result<Option<i64>, GuildStorageError> {
    Ok(
        sqlx::query_scalar("SELECT permission_version FROM guilds WHERE id = ?")
            .bind(to_sqlite_integer(guild_id.raw())?)
            .fetch_optional(database.readers())
            .await?,
    )
}

/// The permissions of the guild's `@everyone` role; `None` if the guild does not exist.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn everyone_permissions(
    database: &Database,
    guild_id: Snowflake,
) -> Result<Option<Permissions>, GuildStorageError> {
    // `@everyone` has the guild's id.
    let permissions: Option<i64> = sqlx::query_scalar("SELECT permissions FROM roles WHERE id = ?")
        .bind(to_sqlite_integer(guild_id.raw())?)
        .fetch_optional(database.readers())
        .await?;
    Ok(permissions.map(permissions_from_integer))
}

/// Reads the guild's version, owner, roles and members in one read transaction, so they all
/// match `version`; `None` if the guild does not exist.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn permission_snapshot(
    database: &Database,
    guild_id: Snowflake,
) -> Result<Option<StoredPermissionSnapshot>, GuildStorageError> {
    let guild_id_integer = to_sqlite_integer(guild_id.raw())?;
    let mut transaction = database.readers().begin().await?;
    let Some((version, owner_id)): Option<(i64, i64)> =
        sqlx::query_as("SELECT permission_version, owner_id FROM guilds WHERE id = ?")
            .bind(guild_id_integer)
            .fetch_optional(&mut *transaction)
            .await?
    else {
        return Ok(None);
    };
    let roles = roles_of_guild(&mut *transaction, guild_id).await?;
    let member_ids: Vec<i64> =
        sqlx::query_scalar("SELECT user_id FROM guild_members WHERE guild_id = ?")
            .bind(guild_id_integer)
            .fetch_all(&mut *transaction)
            .await?;
    let assignments: Vec<(i64, i64)> =
        sqlx::query_as("SELECT user_id, role_id FROM member_roles WHERE guild_id = ?")
            .bind(guild_id_integer)
            .fetch_all(&mut *transaction)
            .await?;
    transaction.commit().await?;
    let mut role_ids_by_member: HashMap<Snowflake, Vec<Snowflake>> = member_ids
        .into_iter()
        .map(|user_id| Ok((to_snowflake(user_id)?, Vec::new())))
        .collect::<Result<_, GuildStorageError>>()?;
    for (user_id, role_id) in assignments {
        // The foreign key to guild_members guarantees the member is in the map.
        if let Some(role_ids) = role_ids_by_member.get_mut(&to_snowflake(user_id)?) {
            role_ids.push(to_snowflake(role_id)?);
        }
    }
    Ok(Some(StoredPermissionSnapshot {
        version,
        owner_id: to_snowflake(owner_id)?,
        roles,
        role_ids_by_member,
    }))
}

/// # Errors
/// Fails on database errors.
/// Messages deleted per transaction before a channel or guild goes. Each deletion also takes the
/// message out of the search index with FTS5's secure-delete, about 0.3 ms per message in a
/// database of a million (ADR-0029): one cascade of 200 000 messages held the writer for 68 s, a
/// batch holds it for about a third of a second, so other writes go in between.
const MESSAGES_DELETED_PER_TRANSACTION: i64 = 1_000;

/// Deletes the messages `statement` selects (?1 the scope, ?2 the batch size), one short
/// transaction per batch, until none is left.
pub(super) async fn delete_messages_in_batches(
    database: &Database,
    statement: &'static str,
    scope_id: i64,
) -> Result<(), GuildStorageError> {
    loop {
        let mut transaction = database
            .writer()
            .begin_with(BEGIN_WRITE_TRANSACTION)
            .await?;
        let deleted = sqlx::query(statement)
            .bind(scope_id)
            .bind(MESSAGES_DELETED_PER_TRANSACTION)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
        transaction.commit().await?;
        if deleted < MESSAGES_DELETED_PER_TRANSACTION.unsigned_abs() {
            return Ok(());
        }
    }
}

/// What renaming a guild did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuildRename {
    /// With the icon, which `GUILD_UPDATE` carries with the new name.
    Renamed {
        icon: Option<ProfileImage>,
    },
    /// The guild already had that name: nothing written or recorded.
    Unchanged,
    NotFound,
}

/// Renames the guild and records `audit` with the name change, in one transaction. The old name
/// is read in that transaction, so the log holds the name actually replaced.
///
/// # Errors
/// Fails on database errors; nothing is written then.
pub(crate) async fn update_guild_name(
    database: &Database,
    guild_id: Snowflake,
    name: &str,
    audit: &NewAuditEntry,
) -> Result<GuildRename, GuildStorageError> {
    let guild_id = to_sqlite_integer(guild_id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let stored: Option<(String, Option<i64>, i64)> =
        sqlx::query_as("SELECT name, icon_id, icon_is_animated FROM guilds WHERE id = ?")
            .bind(guild_id)
            .fetch_optional(&mut *transaction)
            .await?;
    let Some((stored, icon_id, icon_is_animated)) = stored else {
        return Ok(GuildRename::NotFound);
    };
    let icon = guild_icon(icon_id, icon_is_animated)?;
    if stored == name {
        return Ok(GuildRename::Unchanged);
    }
    sqlx::query("UPDATE guilds SET name = ? WHERE id = ?")
        .bind(name)
        .bind(guild_id)
        .execute(&mut *transaction)
        .await?;
    let audit = NewAuditEntry {
        changes: vec![AuditChange {
            key: AuditChangeKey::Name,
            old_value: stored,
            new_value: name.to_owned(),
        }],
        ..audit.clone()
    };
    insert_audit_entry(&mut transaction, &audit).await?;
    transaction.commit().await?;
    Ok(GuildRename::Renamed { icon })
}

/// Deletes the guild if `owner_id` still owns it, with everything it holds: roles, members,
/// channels and their messages, invites, bans and its audit log go by cascade (attachment files
/// are left to the orphan sweep). Returns who was a member then, or `None` if nothing was deleted.
///
/// # Errors
/// Fails on database errors; nothing is deleted then.
pub(crate) async fn delete_guild(
    database: &Database,
    guild_id: Snowflake,
    owner_id: Snowflake,
) -> Result<Option<Vec<Snowflake>>, GuildStorageError> {
    let guild_id = to_sqlite_integer(guild_id.raw())?;
    let owner_id = to_sqlite_integer(owner_id.raw())?;
    let is_owned_by_caller: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM guilds WHERE id = ? AND owner_id = ?)")
            .bind(guild_id)
            .bind(owner_id)
            .fetch_one(database.writer())
            .await?;
    if !is_owned_by_caller {
        return Ok(None);
    }
    delete_messages_in_batches(
        database,
        "DELETE FROM messages WHERE id IN (SELECT id FROM messages \
         WHERE channel_id IN (SELECT id FROM channels WHERE guild_id = ?1) LIMIT ?2)",
        guild_id,
    )
    .await?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let member_ids: Vec<i64> =
        sqlx::query_scalar("SELECT user_id FROM guild_members WHERE guild_id = ?")
            .bind(guild_id)
            .fetch_all(&mut *transaction)
            .await?;
    // Converted before the delete, so nothing can fail once it is committed.
    let member_ids = member_ids
        .into_iter()
        .map(to_snowflake)
        .collect::<Result<Vec<_>, _>>()?;
    let deleted = sqlx::query("DELETE FROM guilds WHERE id = ? AND owner_id = ?")
        .bind(guild_id)
        .bind(owner_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
    if deleted == 0 {
        return Ok(None);
    }
    transaction.commit().await?;
    Ok(Some(member_ids))
}

/// The guild's name and icon; `None` if it does not exist.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn find_guild_summary(
    database: &Database,
    guild_id: Snowflake,
) -> Result<Option<GuildSummary>, GuildStorageError> {
    let Some(guild_id) = lookup_integer(guild_id) else {
        return Ok(None);
    };
    let row: Option<(i64, String, Option<i64>, i64)> =
        sqlx::query_as("SELECT id, name, icon_id, icon_is_animated FROM guilds WHERE id = ?")
            .bind(guild_id)
            .fetch_optional(database.readers())
            .await?;
    row.map(|(id, name, icon_id, icon_is_animated)| {
        Ok(GuildSummary {
            id: to_snowflake(id)?,
            name,
            icon: guild_icon(icon_id, icon_is_animated)?,
        })
    })
    .transpose()
}

/// # Errors
/// Fails on database errors.
pub(crate) async fn list_user_guilds(
    database: &Database,
    user_id: Snowflake,
) -> Result<Vec<GuildSummary>, GuildStorageError> {
    let rows: Vec<(i64, String, Option<i64>, i64)> = sqlx::query_as(
        "SELECT guilds.id, guilds.name, guilds.icon_id, guilds.icon_is_animated FROM guild_members \
         JOIN guilds ON guilds.id = guild_members.guild_id \
         WHERE guild_members.user_id = ? ORDER BY guild_members.joined_at_unix_milliseconds, guilds.id",
    )
    .bind(to_sqlite_integer(user_id.raw())?)
    .fetch_all(database.readers())
    .await?;
    rows.into_iter()
        .map(|(id, name, icon_id, icon_is_animated)| {
            Ok(GuildSummary {
                id: to_snowflake(id)?,
                name,
                icon: guild_icon(icon_id, icon_is_animated)?,
            })
        })
        .collect()
}

/// # Errors
/// Fails on database errors.
pub(crate) async fn user_exists(
    database: &Database,
    user_id: Snowflake,
) -> Result<bool, GuildStorageError> {
    let Some(user_id) = lookup_integer(user_id) else {
        return Ok(false);
    };
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = ?)")
            .bind(user_id)
            .fetch_one(database.readers())
            .await?,
    )
}
