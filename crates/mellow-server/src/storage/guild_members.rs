//! Guild members and bans.

use std::collections::HashMap;

use mellow_protocol::Snowflake;

use super::Database;
use super::guild_roles::member_role_ids;
use super::guilds::{BEGIN_WRITE_TRANSACTION, GuildStorageError, to_count};
use crate::storage::audit_log::{NewAuditEntry, insert_audit_entry};
use crate::storage::sqlite_integers::{
    convert_integer, lookup_integer, to_snowflake, to_sqlite_integer,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredMember {
    pub(crate) user_id: Snowflake,
    pub(crate) username: String,
    pub(crate) display_name: String,
    /// Named roles only, by position.
    pub(crate) role_ids: Vec<Snowflake>,
    pub(crate) joined_at_unix_milliseconds: u64,
}

/// At most `maximum_members` members, by username.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn list_members(
    database: &Database,
    guild_id: Snowflake,
    maximum_members: u32,
) -> Result<Vec<StoredMember>, GuildStorageError> {
    let guild_id_integer = to_sqlite_integer(guild_id.raw())?;
    let rows: Vec<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT users.id, users.username, users.display_name, guild_members.joined_at_unix_milliseconds \
         FROM guild_members JOIN users ON users.id = guild_members.user_id \
         WHERE guild_members.guild_id = ? ORDER BY users.username LIMIT ?",
    )
    .bind(guild_id_integer)
    .bind(to_count(maximum_members))
    .fetch_all(database.readers())
    .await?;
    let assignments: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT member_roles.user_id, member_roles.role_id FROM member_roles \
         JOIN roles ON roles.id = member_roles.role_id \
         WHERE member_roles.guild_id = ? AND member_roles.user_id IN \
         (SELECT guild_members.user_id FROM guild_members \
          JOIN users ON users.id = guild_members.user_id \
          WHERE guild_members.guild_id = ? ORDER BY users.username LIMIT ?) \
         ORDER BY roles.position, roles.id",
    )
    .bind(guild_id_integer)
    .bind(guild_id_integer)
    .bind(to_count(maximum_members))
    .fetch_all(database.readers())
    .await?;
    let mut role_ids_by_user: HashMap<i64, Vec<Snowflake>> = HashMap::new();
    for (user_id, role_id) in assignments {
        role_ids_by_user
            .entry(user_id)
            .or_default()
            .push(to_snowflake(role_id)?);
    }
    rows.into_iter()
        .map(|(user_id, username, display_name, joined_at)| {
            Ok(StoredMember {
                role_ids: role_ids_by_user.remove(&user_id).unwrap_or_default(),
                user_id: to_snowflake(user_id)?,
                username,
                display_name,
                joined_at_unix_milliseconds: convert_integer::<_, u64>(joined_at)?,
            })
        })
        .collect()
}

/// `audit` records a kick; leaving the guild records nothing.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn delete_member(
    database: &Database,
    guild_id: Snowflake,
    user_id: Snowflake,
    audit: Option<&NewAuditEntry>,
) -> Result<(), GuildStorageError> {
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let removed = sqlx::query("DELETE FROM guild_members WHERE guild_id = ? AND user_id = ?")
        .bind(to_sqlite_integer(guild_id.raw())?)
        .bind(to_sqlite_integer(user_id.raw())?)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
    if removed > 0
        && let Some(audit) = audit
    {
        insert_audit_entry(&mut transaction, audit).await?;
    }
    transaction.commit().await?;
    Ok(())
}

/// # Errors
/// Fails on database errors.
pub(crate) async fn find_member(
    database: &Database,
    guild_id: Snowflake,
    user_id: Snowflake,
) -> Result<Option<StoredMember>, GuildStorageError> {
    let row: Option<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT users.id, users.username, users.display_name, guild_members.joined_at_unix_milliseconds \
         FROM guild_members JOIN users ON users.id = guild_members.user_id \
         WHERE guild_members.guild_id = ? AND guild_members.user_id = ?",
    )
    .bind(to_sqlite_integer(guild_id.raw())?)
    .bind(to_sqlite_integer(user_id.raw())?)
    .fetch_optional(database.readers())
    .await?;
    let Some((user_id, username, display_name, joined_at)) = row else {
        return Ok(None);
    };
    let user_id = to_snowflake(user_id)?;
    Ok(Some(StoredMember {
        role_ids: member_role_ids(database, guild_id, user_id).await?,
        user_id,
        username,
        display_name,
        joined_at_unix_milliseconds: convert_integer::<_, u64>(joined_at)?,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredBan {
    pub(crate) user_id: Snowflake,
    pub(crate) username: String,
    pub(crate) display_name: String,
    pub(crate) reason: String,
    pub(crate) created_at_unix_milliseconds: u64,
}

/// Bans `user_id` from the guild (a repeated ban replaces the reason) and removes their
/// membership, in one transaction. Returns whether they were a member.
///
/// # Errors
/// Fails on database errors, including a user that does not exist. Nothing is written then.
pub(crate) async fn ban_and_remove_member(
    database: &Database,
    guild_id: Snowflake,
    user_id: Snowflake,
    reason: &str,
    banned_by: Snowflake,
    now_unix_milliseconds: u64,
    audit: &NewAuditEntry,
) -> Result<bool, GuildStorageError> {
    let guild_id_integer = to_sqlite_integer(guild_id.raw())?;
    let user_id_integer = to_sqlite_integer(user_id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    sqlx::query(
        "INSERT INTO bans (guild_id, user_id, reason, banned_by, created_at_unix_milliseconds) \
         VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT (guild_id, user_id) DO UPDATE SET reason = excluded.reason, \
         banned_by = excluded.banned_by",
    )
    .bind(guild_id_integer)
    .bind(user_id_integer)
    .bind(reason)
    .bind(to_sqlite_integer(banned_by.raw())?)
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .execute(&mut *transaction)
    .await?;
    let removed = sqlx::query("DELETE FROM guild_members WHERE guild_id = ? AND user_id = ?")
        .bind(guild_id_integer)
        .bind(user_id_integer)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
    insert_audit_entry(&mut transaction, audit).await?;
    transaction.commit().await?;
    Ok(removed > 0)
}

/// Returns whether a ban was lifted.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn delete_ban(
    database: &Database,
    guild_id: Snowflake,
    user_id: Snowflake,
    audit: &NewAuditEntry,
) -> Result<bool, GuildStorageError> {
    let (Some(guild_id), Some(user_id)) = (lookup_integer(guild_id), lookup_integer(user_id))
    else {
        return Ok(false);
    };
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let deleted = sqlx::query("DELETE FROM bans WHERE guild_id = ? AND user_id = ?")
        .bind(guild_id)
        .bind(user_id)
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

/// At most `maximum_bans` bans, newest first.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn list_bans(
    database: &Database,
    guild_id: Snowflake,
    maximum_bans: u32,
) -> Result<Vec<StoredBan>, GuildStorageError> {
    let rows: Vec<(i64, String, String, String, i64)> = sqlx::query_as(
        "SELECT users.id, users.username, users.display_name, bans.reason, \
         bans.created_at_unix_milliseconds FROM bans JOIN users ON users.id = bans.user_id \
         WHERE bans.guild_id = ? ORDER BY bans.created_at_unix_milliseconds DESC, users.id LIMIT ?",
    )
    .bind(to_sqlite_integer(guild_id.raw())?)
    .bind(to_count(maximum_bans))
    .fetch_all(database.readers())
    .await?;
    rows.into_iter()
        .map(|(user_id, username, display_name, reason, created_at)| {
            Ok(StoredBan {
                user_id: to_snowflake(user_id)?,
                username,
                display_name,
                reason,
                created_at_unix_milliseconds: convert_integer::<_, u64>(created_at)?,
            })
        })
        .collect()
}
