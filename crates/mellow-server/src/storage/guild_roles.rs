//! Guild roles and who holds them (migration 0002).

use mellow_protocol::{Permissions, ProfileImage, Role, Snowflake};

use super::Database;
use super::guilds::{
    BEGIN_WRITE_TRANSACTION, GuildStorageError, permissions_from_integer, permissions_to_integer,
    to_count,
};
use crate::storage::audit_log::{NewAuditEntry, insert_audit_entry};
use crate::storage::sqlite_integers::{
    convert_integer, lookup_integer, to_snowflake, to_sqlite_integer,
};

pub(super) type RoleRow = (
    i64,
    String,
    i64,
    i64,
    i64,
    bool,
    bool,
    Option<i64>,
    bool,
    Option<String>,
);

/// The columns `RoleRow` reads, in its order, for `concat!` into a query.
macro_rules! role_columns {
    () => {
        "id, name, color, position, permissions, is_displayed_separately, is_mentionable, \
         icon_id, icon_is_animated, unicode_emoji"
    };
}
pub(super) use role_columns;

pub(super) fn role_from_row(row: RoleRow) -> Result<Role, GuildStorageError> {
    let (
        id,
        name,
        color,
        position,
        permissions,
        is_displayed_separately,
        is_mentionable,
        icon_id,
        icon_is_animated,
        unicode_emoji,
    ) = row;
    Ok(Role {
        id: to_snowflake(id)?,
        name,
        color: convert_integer::<_, u32>(color)?,
        position: convert_integer::<_, u32>(position)?,
        permissions: permissions_from_integer(permissions),
        is_displayed_separately,
        is_mentionable,
        icon: icon_id
            .map(|id| {
                Ok::<_, GuildStorageError>(ProfileImage {
                    id: to_snowflake(id)?,
                    is_animated: icon_is_animated,
                })
            })
            .transpose()?,
        unicode_emoji,
    })
}

/// The guild's roles by position, `@everyone` first.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn list_roles(
    database: &Database,
    guild_id: Snowflake,
) -> Result<Vec<Role>, GuildStorageError> {
    roles_of_guild(database.readers(), guild_id).await
}

pub(super) async fn roles_of_guild(
    executor: impl sqlx::SqliteExecutor<'_>,
    guild_id: Snowflake,
) -> Result<Vec<Role>, GuildStorageError> {
    let rows: Vec<RoleRow> = sqlx::query_as(concat!(
        "SELECT ",
        role_columns!(),
        " FROM roles WHERE guild_id = ? ORDER BY position, id"
    ))
    .bind(to_sqlite_integer(guild_id.raw())?)
    .fetch_all(executor)
    .await?;
    rows.into_iter().map(role_from_row).collect()
}

/// Inserts `role` at position 1, just above `@everyone`, and moves every other role up by one,
/// provided the guild has fewer than `maximum_roles` roles (`@everyone` included). Returns the
/// guild's new role list.
///
/// # Errors
/// `LimitReached` at the quota; database errors. Nothing is written on error.
pub(crate) async fn insert_role_above_everyone(
    database: &Database,
    guild_id: Snowflake,
    role: &Role,
    maximum_roles: u32,
    audit: &NewAuditEntry,
) -> Result<Vec<Role>, GuildStorageError> {
    let guild_id_integer = to_sqlite_integer(guild_id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let role_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roles WHERE guild_id = ?")
        .bind(guild_id_integer)
        .fetch_one(&mut *transaction)
        .await?;
    if role_count >= to_count(maximum_roles) {
        return Err(GuildStorageError::LimitReached);
    }
    sqlx::query("UPDATE roles SET position = position + 1 WHERE guild_id = ? AND position >= 1")
        .bind(guild_id_integer)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO roles (id, guild_id, name, color, permissions, is_displayed_separately, \
         is_mentionable, position) VALUES (?, ?, ?, ?, ?, ?, ?, 1)",
    )
    .bind(to_sqlite_integer(role.id.raw())?)
    .bind(guild_id_integer)
    .bind(&role.name)
    .bind(i64::from(role.color))
    .bind(permissions_to_integer(role.permissions))
    .bind(role.is_displayed_separately)
    .bind(role.is_mentionable)
    .execute(&mut *transaction)
    .await?;
    insert_audit_entry(&mut transaction, audit).await?;
    transaction.commit().await?;
    list_roles(database, guild_id).await
}

/// The fields of a role to change; `None` keeps the stored value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RoleFieldChanges<'a> {
    pub(crate) name: Option<&'a str>,
    pub(crate) color: Option<u32>,
    pub(crate) permissions: Option<Permissions>,
    pub(crate) is_displayed_separately: Option<bool>,
    pub(crate) is_mentionable: Option<bool>,
}

/// Writes only the given fields of a role of `guild_id`. Only the `changeable_permissions` bits
/// take the new value; the others keep what the row holds now, so a permission granted
/// meanwhile by someone holding it is not erased. Returns the updated role, or `None` if the
/// guild has no such role.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn update_role_fields(
    database: &Database,
    guild_id: Snowflake,
    role_id: Snowflake,
    changes: &RoleFieldChanges<'_>,
    changeable_permissions: Permissions,
    audit: &NewAuditEntry,
) -> Result<Option<Role>, GuildStorageError> {
    let (Some(guild_id), Some(role_id)) = (lookup_integer(guild_id), lookup_integer(role_id))
    else {
        return Ok(None);
    };
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let row: Option<RoleRow> = sqlx::query_as(concat!(
        "UPDATE roles SET name = COALESCE(?, name), color = COALESCE(?, color), \
         permissions = COALESCE((permissions & ~?) | (? & ?), permissions), \
         is_displayed_separately = COALESCE(?, is_displayed_separately), \
         is_mentionable = COALESCE(?, is_mentionable) \
         WHERE id = ? AND guild_id = ? RETURNING ",
        role_columns!()
    ))
    .bind(changes.name)
    .bind(changes.color.map(i64::from))
    .bind(permissions_to_integer(changeable_permissions))
    .bind(changes.permissions.map(permissions_to_integer))
    .bind(permissions_to_integer(changeable_permissions))
    .bind(changes.is_displayed_separately)
    .bind(changes.is_mentionable)
    .bind(role_id)
    .bind(guild_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    insert_audit_entry(&mut transaction, audit).await?;
    transaction.commit().await?;
    role_from_row(row).map(Some)
}

/// Gives the named roles of `guild_id` the positions 1, 2, … in `new_order`, provided they are
/// still in `expected_order` (what the permission check saw). Returns the guild's new role
/// list, or `None` if the order changed meanwhile.
///
/// # Errors
/// Fails on database errors. Nothing is written on error.
pub(crate) async fn reorder_roles(
    database: &Database,
    guild_id: Snowflake,
    expected_order: &[Snowflake],
    new_order: &[Snowflake],
    audit: &NewAuditEntry,
) -> Result<Option<Vec<Role>>, GuildStorageError> {
    let guild_id_integer = to_sqlite_integer(guild_id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let stored_order: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM roles WHERE guild_id = ? AND id != guild_id ORDER BY position, id",
    )
    .bind(guild_id_integer)
    .fetch_all(&mut *transaction)
    .await?;
    let stored_order: Vec<Snowflake> = stored_order
        .into_iter()
        .map(to_snowflake)
        .collect::<Result<_, _>>()?;
    if stored_order != expected_order {
        return Ok(None);
    }
    for (position, role_id) in (1_i64..).zip(new_order) {
        sqlx::query("UPDATE roles SET position = ? WHERE id = ? AND guild_id = ?")
            .bind(position)
            .bind(to_sqlite_integer(role_id.raw())?)
            .bind(guild_id_integer)
            .execute(&mut *transaction)
            .await?;
    }
    insert_audit_entry(&mut transaction, audit).await?;
    transaction.commit().await?;
    list_roles(database, guild_id).await.map(Some)
}

/// Deletes a named role of `guild_id` (never `@everyone`) and closes the gap in positions.
/// Returns whether a role was deleted.
///
/// # Errors
/// Fails on database errors. Nothing is written on error.
pub(crate) async fn delete_role(
    database: &Database,
    guild_id: Snowflake,
    role_id: Snowflake,
    audit: &NewAuditEntry,
) -> Result<bool, GuildStorageError> {
    let (Some(guild_id), Some(role_id)) = (lookup_integer(guild_id), lookup_integer(role_id))
    else {
        return Ok(false);
    };
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let deleted_position: Option<(i64, Option<i64>)> = sqlx::query_as(
        "DELETE FROM roles WHERE id = ? AND guild_id = ? AND id != guild_id \
         RETURNING position, icon_id",
    )
    .bind(role_id)
    .bind(guild_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some((deleted_position, icon_id)) = deleted_position else {
        return Ok(false);
    };
    // Its picture's file goes with the next sweep of files no row names.
    sqlx::query("DELETE FROM guild_images WHERE id = ?")
        .bind(icon_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("UPDATE roles SET position = position - 1 WHERE guild_id = ? AND position > ?")
        .bind(guild_id)
        .bind(deleted_position)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "DELETE FROM channel_permission_overwrites WHERE target_id = ? AND channel_id IN \
         (SELECT id FROM channels WHERE guild_id = ?)",
    )
    .bind(role_id)
    .bind(guild_id)
    .execute(&mut *transaction)
    .await?;
    insert_audit_entry(&mut transaction, audit).await?;
    transaction.commit().await?;
    Ok(true)
}

/// Gives a named role of `guild_id` to one of its members (nothing changes if they hold it),
/// provided every permission of the role is in `assignable_permissions`, checked in the same
/// statement so a permission added to the role meanwhile cannot slip through. Returns the
/// member's role ids afterwards, or `None` if the role is not a named role of this guild holding
/// only assignable permissions, or the user is not a member.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn add_member_role(
    database: &Database,
    guild_id: Snowflake,
    user_id: Snowflake,
    role_id: Snowflake,
    assignable_permissions: Permissions,
    audit: &NewAuditEntry,
) -> Result<Option<Vec<Snowflake>>, GuildStorageError> {
    let (Some(guild_id_integer), Some(user_id_integer), Some(role_id_integer)) = (
        lookup_integer(guild_id),
        lookup_integer(user_id),
        lookup_integer(role_id),
    ) else {
        return Ok(None);
    };
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO member_roles (guild_id, user_id, role_id) \
         SELECT guild_members.guild_id, guild_members.user_id, roles.id \
         FROM guild_members JOIN roles ON roles.guild_id = guild_members.guild_id \
         WHERE guild_members.guild_id = ? AND guild_members.user_id = ? \
         AND roles.id = ? AND roles.id != roles.guild_id AND (roles.permissions & ~?) = 0",
    )
    .bind(guild_id_integer)
    .bind(user_id_integer)
    .bind(role_id_integer)
    .bind(permissions_to_integer(assignable_permissions))
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    // Recorded only when the member did not hold it already.
    if inserted > 0 {
        insert_audit_entry(&mut transaction, audit).await?;
    }
    transaction.commit().await?;
    let role_ids = member_role_ids(database, guild_id, user_id).await?;
    // Already held: only reachable when the role passed the same permission check earlier.
    Ok((inserted > 0 || role_ids.contains(&role_id)).then_some(role_ids))
}

/// Takes a role away from a member. Returns the member's role ids afterwards, or `None` if they
/// did not hold it.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn remove_member_role(
    database: &Database,
    guild_id: Snowflake,
    user_id: Snowflake,
    role_id: Snowflake,
    audit: &NewAuditEntry,
) -> Result<Option<Vec<Snowflake>>, GuildStorageError> {
    let (Some(guild_id_integer), Some(user_id_integer), Some(role_id_integer)) = (
        lookup_integer(guild_id),
        lookup_integer(user_id),
        lookup_integer(role_id),
    ) else {
        return Ok(None);
    };
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let removed =
        sqlx::query("DELETE FROM member_roles WHERE guild_id = ? AND user_id = ? AND role_id = ?")
            .bind(guild_id_integer)
            .bind(user_id_integer)
            .bind(role_id_integer)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
    if removed == 0 {
        return Ok(None);
    }
    insert_audit_entry(&mut transaction, audit).await?;
    transaction.commit().await?;
    member_role_ids(database, guild_id, user_id).await.map(Some)
}

/// The member's named roles, by position.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn member_role_ids(
    database: &Database,
    guild_id: Snowflake,
    user_id: Snowflake,
) -> Result<Vec<Snowflake>, GuildStorageError> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT member_roles.role_id FROM member_roles \
         JOIN roles ON roles.id = member_roles.role_id \
         WHERE member_roles.guild_id = ? AND member_roles.user_id = ? \
         ORDER BY roles.position, roles.id",
    )
    .bind(to_sqlite_integer(guild_id.raw())?)
    .bind(to_sqlite_integer(user_id.raw())?)
    .fetch_all(database.readers())
    .await?;
    Ok(ids
        .into_iter()
        .map(to_snowflake)
        .collect::<Result<_, _>>()?)
}
