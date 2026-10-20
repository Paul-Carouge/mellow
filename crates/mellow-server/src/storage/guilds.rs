//! Guilds, their roles, members, channels and invites (migrations 0002 and 0005).

use std::collections::HashMap;

use mellow_protocol::{ChannelKind, Permissions, Role, Snowflake};
use sqlx::SqlitePool;

/// Takes the write lock at once, so a read-then-write transaction waits for `busy_timeout`
/// instead of failing with `SQLITE_BUSY_SNAPSHOT` when another connection wrote in between.
const BEGIN_WRITE_TRANSACTION: &str = "BEGIN IMMEDIATE";

#[derive(Debug, thiserror::Error)]
pub(crate) enum GuildStorageError {
    #[error("value does not fit SQLite's signed 64-bit integer")]
    IntegerOutOfRange,
    #[error("the channel kind stored in the database is unknown: {0}")]
    UnknownChannelKind(String),
    /// A quota checked atomically with the insert was reached.
    #[error("limit reached")]
    LimitReached,
    /// The referenced parent channel no longer exists.
    #[error("referenced row is missing")]
    ReferenceMissing,
    #[error("database error")]
    Database(#[from] sqlx::Error),
}

fn to_integer(value: u64) -> Result<i64, GuildStorageError> {
    i64::try_from(value).map_err(|_| GuildStorageError::IntegerOutOfRange)
}

/// For lookups: an id SQLite cannot hold cannot exist, so it is simply not found.
fn lookup_integer(id: Snowflake) -> Option<i64> {
    i64::try_from(id.raw()).ok()
}

fn to_id(value: i64) -> Result<Snowflake, GuildStorageError> {
    u64::try_from(value)
        .map(Snowflake::from_raw)
        .map_err(|_| GuildStorageError::IntegerOutOfRange)
}

fn to_unsigned(value: i64) -> Result<u64, GuildStorageError> {
    u64::try_from(value).map_err(|_| GuildStorageError::IntegerOutOfRange)
}

fn to_count(value: u32) -> i64 {
    i64::from(value)
}

/// Permission bitfields use all 64 bits; SQLite stores them as the same bits in a signed integer.
fn permissions_to_integer(permissions: Permissions) -> i64 {
    i64::from_ne_bytes(permissions.bits().to_ne_bytes())
}

fn permissions_from_integer(value: i64) -> Permissions {
    Permissions::from_bits_retain(u64::from_ne_bytes(value.to_ne_bytes()))
}

fn channel_kind_to_text(kind: ChannelKind) -> &'static str {
    match kind {
        ChannelKind::Category => "category",
        ChannelKind::Text => "text",
    }
}

fn channel_kind_from_text(text: &str) -> Result<ChannelKind, GuildStorageError> {
    match text {
        "category" => Ok(ChannelKind::Category),
        "text" => Ok(ChannelKind::Text),
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
}

type ChannelRow = (i64, i64, String, String, String, Option<i64>, i64);

fn channel_from_row(row: ChannelRow) -> Result<StoredChannel, GuildStorageError> {
    let (id, guild_id, kind, name, topic, parent_id, position) = row;
    Ok(StoredChannel {
        id: to_id(id)?,
        guild_id: to_id(guild_id)?,
        kind: channel_kind_from_text(&kind)?,
        name,
        topic,
        parent_id: parent_id.map(to_id).transpose()?,
        position: u32::try_from(position).map_err(|_| GuildStorageError::IntegerOutOfRange)?,
    })
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
    pool: &SqlitePool,
    guild: &NewGuild,
    maximum_owned_guilds: u32,
) -> Result<(), GuildStorageError> {
    let mut transaction = pool.begin_with(BEGIN_WRITE_TRANSACTION).await?;
    let guild_id = to_integer(guild.id.raw())?;
    let owner_id = to_integer(guild.owner_id.raw())?;
    let created_at = to_integer(guild.created_at_unix_milliseconds)?;
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
        .bind(to_integer(channel.id.raw())?)
        .bind(guild_id)
        .bind(channel_kind_to_text(channel.kind))
        .bind(&channel.name)
        .bind(&channel.topic)
        .bind(
            channel
                .parent_id
                .map(|id| to_integer(id.raw()))
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

type RoleRow = (i64, String, i64, i64, i64);

fn role_from_row(row: RoleRow) -> Result<Role, GuildStorageError> {
    let (id, name, color, position, permissions) = row;
    Ok(Role {
        id: to_id(id)?,
        name,
        color: u32::try_from(color).map_err(|_| GuildStorageError::IntegerOutOfRange)?,
        position: u32::try_from(position).map_err(|_| GuildStorageError::IntegerOutOfRange)?,
        permissions: permissions_from_integer(permissions),
    })
}

/// `None` if the guild does not exist or the user is not a member: callers answer both the same
/// way, so outsiders cannot probe which guilds exist.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn find_membership(
    pool: &SqlitePool,
    guild_id: Snowflake,
    user_id: Snowflake,
) -> Result<Option<Membership>, GuildStorageError> {
    let (Some(guild_id), Some(user_id)) = (lookup_integer(guild_id), lookup_integer(user_id))
    else {
        return Ok(None);
    };
    let row: Option<(String, i64)> = sqlx::query_as(
        "SELECT guilds.name, guilds.owner_id FROM guild_members \
         JOIN guilds ON guilds.id = guild_members.guild_id \
         WHERE guild_members.guild_id = ? AND guild_members.user_id = ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let Some((guild_name, owner_id)) = row else {
        return Ok(None);
    };
    let rows: Vec<RoleRow> = sqlx::query_as(
        "SELECT id, name, color, position, permissions FROM roles WHERE id = ? OR id IN \
         (SELECT role_id FROM member_roles WHERE guild_id = ? AND user_id = ?) \
         ORDER BY position, id",
    )
    .bind(guild_id)
    .bind(guild_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(Some(Membership {
        guild_name,
        owner_id: to_id(owner_id)?,
        member_roles: rows
            .into_iter()
            .map(role_from_row)
            .collect::<Result<_, _>>()?,
    }))
}

/// The guild's roles by position, `@everyone` first.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn list_roles(
    pool: &SqlitePool,
    guild_id: Snowflake,
) -> Result<Vec<Role>, GuildStorageError> {
    let rows: Vec<RoleRow> = sqlx::query_as(
        "SELECT id, name, color, position, permissions FROM roles WHERE guild_id = ? \
         ORDER BY position, id",
    )
    .bind(to_integer(guild_id.raw())?)
    .fetch_all(pool)
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
    pool: &SqlitePool,
    guild_id: Snowflake,
    role: &Role,
    maximum_roles: u32,
) -> Result<Vec<Role>, GuildStorageError> {
    let guild_id_integer = to_integer(guild_id.raw())?;
    let mut transaction = pool.begin_with(BEGIN_WRITE_TRANSACTION).await?;
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
        "INSERT INTO roles (id, guild_id, name, color, permissions, position) VALUES (?, ?, ?, ?, ?, 1)",
    )
    .bind(to_integer(role.id.raw())?)
    .bind(guild_id_integer)
    .bind(&role.name)
    .bind(i64::from(role.color))
    .bind(permissions_to_integer(role.permissions))
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    list_roles(pool, guild_id).await
}

/// Writes only the given fields of a role of `guild_id`. Returns the updated role, or `None` if
/// the guild has no such role.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn update_role_fields(
    pool: &SqlitePool,
    guild_id: Snowflake,
    role_id: Snowflake,
    name: Option<&str>,
    color: Option<u32>,
    permissions: Option<Permissions>,
) -> Result<Option<Role>, GuildStorageError> {
    let (Some(guild_id), Some(role_id)) = (lookup_integer(guild_id), lookup_integer(role_id))
    else {
        return Ok(None);
    };
    let row: Option<RoleRow> = sqlx::query_as(
        "UPDATE roles SET name = COALESCE(?, name), color = COALESCE(?, color), \
         permissions = COALESCE(?, permissions) WHERE id = ? AND guild_id = ? \
         RETURNING id, name, color, position, permissions",
    )
    .bind(name)
    .bind(color.map(i64::from))
    .bind(permissions.map(permissions_to_integer))
    .bind(role_id)
    .bind(guild_id)
    .fetch_optional(pool)
    .await?;
    row.map(role_from_row).transpose()
}

/// Deletes a named role of `guild_id` (never `@everyone`) and closes the gap in positions.
/// Returns whether a role was deleted.
///
/// # Errors
/// Fails on database errors. Nothing is written on error.
pub(crate) async fn delete_role(
    pool: &SqlitePool,
    guild_id: Snowflake,
    role_id: Snowflake,
) -> Result<bool, GuildStorageError> {
    let (Some(guild_id), Some(role_id)) = (lookup_integer(guild_id), lookup_integer(role_id))
    else {
        return Ok(false);
    };
    let mut transaction = pool.begin_with(BEGIN_WRITE_TRANSACTION).await?;
    let deleted_position: Option<i64> = sqlx::query_scalar(
        "DELETE FROM roles WHERE id = ? AND guild_id = ? AND id != guild_id RETURNING position",
    )
    .bind(role_id)
    .bind(guild_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(deleted_position) = deleted_position else {
        return Ok(false);
    };
    sqlx::query("UPDATE roles SET position = position - 1 WHERE guild_id = ? AND position > ?")
        .bind(guild_id)
        .bind(deleted_position)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(true)
}

/// Gives a named role of `guild_id` to one of its members (nothing changes if they hold it).
/// Returns the member's role ids afterwards, or `None` if the role is not a named role of this
/// guild or the user is not a member.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn add_member_role(
    pool: &SqlitePool,
    guild_id: Snowflake,
    user_id: Snowflake,
    role_id: Snowflake,
) -> Result<Option<Vec<Snowflake>>, GuildStorageError> {
    let (Some(guild), Some(user), Some(role)) = (
        lookup_integer(guild_id),
        lookup_integer(user_id),
        lookup_integer(role_id),
    ) else {
        return Ok(None);
    };
    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO member_roles (guild_id, user_id, role_id) \
         SELECT guild_members.guild_id, guild_members.user_id, roles.id \
         FROM guild_members JOIN roles ON roles.guild_id = guild_members.guild_id \
         WHERE guild_members.guild_id = ? AND guild_members.user_id = ? \
         AND roles.id = ? AND roles.id != roles.guild_id",
    )
    .bind(guild)
    .bind(user)
    .bind(role)
    .execute(pool)
    .await?
    .rows_affected();
    let role_ids = member_role_ids(pool, guild_id, user_id).await?;
    Ok((inserted > 0 || role_ids.contains(&role_id)).then_some(role_ids))
}

/// Takes a role away from a member. Returns the member's role ids afterwards, or `None` if they
/// did not hold it.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn remove_member_role(
    pool: &SqlitePool,
    guild_id: Snowflake,
    user_id: Snowflake,
    role_id: Snowflake,
) -> Result<Option<Vec<Snowflake>>, GuildStorageError> {
    let (Some(guild), Some(user), Some(role)) = (
        lookup_integer(guild_id),
        lookup_integer(user_id),
        lookup_integer(role_id),
    ) else {
        return Ok(None);
    };
    let removed =
        sqlx::query("DELETE FROM member_roles WHERE guild_id = ? AND user_id = ? AND role_id = ?")
            .bind(guild)
            .bind(user)
            .bind(role)
            .execute(pool)
            .await?
            .rows_affected();
    if removed == 0 {
        return Ok(None);
    }
    member_role_ids(pool, guild_id, user_id).await.map(Some)
}

/// The member's named roles, by position.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn member_role_ids(
    pool: &SqlitePool,
    guild_id: Snowflake,
    user_id: Snowflake,
) -> Result<Vec<Snowflake>, GuildStorageError> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT member_roles.role_id FROM member_roles \
         JOIN roles ON roles.id = member_roles.role_id \
         WHERE member_roles.guild_id = ? AND member_roles.user_id = ? \
         ORDER BY roles.position, roles.id",
    )
    .bind(to_integer(guild_id.raw())?)
    .bind(to_integer(user_id.raw())?)
    .fetch_all(pool)
    .await?;
    ids.into_iter().map(to_id).collect()
}

/// # Errors
/// Fails on database errors.
pub(crate) async fn list_guild_channels(
    pool: &SqlitePool,
    guild_id: Snowflake,
) -> Result<Vec<StoredChannel>, GuildStorageError> {
    let rows: Vec<ChannelRow> = sqlx::query_as(
        "SELECT id, guild_id, kind, name, topic, parent_id, position FROM channels \
         WHERE guild_id = ? ORDER BY position, id",
    )
    .bind(to_integer(guild_id.raw())?)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(channel_from_row).collect()
}

/// # Errors
/// Fails on database errors.
pub(crate) async fn find_guild_channel(
    pool: &SqlitePool,
    channel_id: Snowflake,
) -> Result<Option<StoredChannel>, GuildStorageError> {
    let Some(channel_id) = lookup_integer(channel_id) else {
        return Ok(None);
    };
    let row: Option<ChannelRow> = sqlx::query_as(
        "SELECT id, guild_id, kind, name, topic, parent_id, position FROM channels \
         WHERE id = ? AND guild_id IS NOT NULL",
    )
    .bind(channel_id)
    .fetch_optional(pool)
    .await?;
    row.map(channel_from_row).transpose()
}

/// Inserts the channel at the end of its guild (position = last + 1) and returns it, provided
/// the guild has fewer than `maximum_channels` channels (checked in the same statement).
///
/// # Errors
/// `LimitReached` at the quota, `ReferenceMissing` if the parent was deleted meanwhile.
pub(crate) async fn insert_channel_last(
    pool: &SqlitePool,
    channel: &StoredChannel,
    maximum_channels: u32,
) -> Result<StoredChannel, GuildStorageError> {
    let guild_id = to_integer(channel.guild_id.raw())?;
    let result: Result<Option<i64>, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO channels (id, guild_id, kind, name, topic, parent_id, position) \
         SELECT ?, ?, ?, ?, ?, ?, COALESCE(MAX(position) + 1, 0) FROM channels WHERE guild_id = ? \
         HAVING COUNT(*) < ? \
         RETURNING position",
    )
    .bind(to_integer(channel.id.raw())?)
    .bind(guild_id)
    .bind(channel_kind_to_text(channel.kind))
    .bind(&channel.name)
    .bind(&channel.topic)
    .bind(
        channel
            .parent_id
            .map(|id| to_integer(id.raw()))
            .transpose()?,
    )
    .bind(guild_id)
    .bind(to_count(maximum_channels))
    .fetch_optional(pool)
    .await;
    let position = match result {
        Ok(Some(position)) => position,
        Ok(None) => return Err(GuildStorageError::LimitReached),
        Err(sqlx::Error::Database(error)) if error.is_foreign_key_violation() => {
            return Err(GuildStorageError::ReferenceMissing);
        }
        Err(error) => return Err(error.into()),
    };
    Ok(StoredChannel {
        position: u32::try_from(position).map_err(|_| GuildStorageError::IntegerOutOfRange)?,
        ..channel.clone()
    })
}

/// Writes only the given fields, so concurrent edits of different fields do not overwrite each
/// other. Returns the updated channel, or `None` if it no longer exists.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn update_channel_fields(
    pool: &SqlitePool,
    channel_id: Snowflake,
    name: Option<&str>,
    topic: Option<&str>,
) -> Result<Option<StoredChannel>, GuildStorageError> {
    let row: Option<ChannelRow> = sqlx::query_as(
        "UPDATE channels SET name = COALESCE(?, name), topic = COALESCE(?, topic) \
         WHERE id = ? AND guild_id IS NOT NULL \
         RETURNING id, guild_id, kind, name, topic, parent_id, position",
    )
    .bind(name)
    .bind(topic)
    .bind(to_integer(channel_id.raw())?)
    .fetch_optional(pool)
    .await?;
    row.map(channel_from_row).transpose()
}

/// # Errors
/// Fails on database errors.
pub(crate) async fn delete_channel(
    pool: &SqlitePool,
    channel_id: Snowflake,
) -> Result<(), GuildStorageError> {
    sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(to_integer(channel_id.raw())?)
        .execute(pool)
        .await?;
    Ok(())
}

/// # Errors
/// Fails on database errors.
pub(crate) async fn list_user_guilds(
    pool: &SqlitePool,
    user_id: Snowflake,
) -> Result<Vec<(Snowflake, String)>, GuildStorageError> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT guilds.id, guilds.name FROM guild_members \
         JOIN guilds ON guilds.id = guild_members.guild_id \
         WHERE guild_members.user_id = ? ORDER BY guild_members.joined_at_unix_milliseconds, guilds.id",
    )
    .bind(to_integer(user_id.raw())?)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(id, name)| Ok((to_id(id)?, name)))
        .collect()
}

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
    pool: &SqlitePool,
    guild_id: Snowflake,
    maximum_members: u32,
) -> Result<Vec<StoredMember>, GuildStorageError> {
    let guild = to_integer(guild_id.raw())?;
    let rows: Vec<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT users.id, users.username, users.display_name, guild_members.joined_at_unix_milliseconds \
         FROM guild_members JOIN users ON users.id = guild_members.user_id \
         WHERE guild_members.guild_id = ? ORDER BY users.username LIMIT ?",
    )
    .bind(guild)
    .bind(to_count(maximum_members))
    .fetch_all(pool)
    .await?;
    let assignments: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT member_roles.user_id, member_roles.role_id FROM member_roles \
         JOIN roles ON roles.id = member_roles.role_id \
         WHERE member_roles.guild_id = ? ORDER BY roles.position, roles.id",
    )
    .bind(guild)
    .fetch_all(pool)
    .await?;
    let mut role_ids_by_user: HashMap<i64, Vec<Snowflake>> = HashMap::new();
    for (user_id, role_id) in assignments {
        role_ids_by_user
            .entry(user_id)
            .or_default()
            .push(to_id(role_id)?);
    }
    rows.into_iter()
        .map(|(user_id, username, display_name, joined_at)| {
            Ok(StoredMember {
                role_ids: role_ids_by_user.remove(&user_id).unwrap_or_default(),
                user_id: to_id(user_id)?,
                username,
                display_name,
                joined_at_unix_milliseconds: to_unsigned(joined_at)?,
            })
        })
        .collect()
}

/// # Errors
/// Fails on database errors.
pub(crate) async fn delete_member(
    pool: &SqlitePool,
    guild_id: Snowflake,
    user_id: Snowflake,
) -> Result<(), GuildStorageError> {
    sqlx::query("DELETE FROM guild_members WHERE guild_id = ? AND user_id = ?")
        .bind(to_integer(guild_id.raw())?)
        .bind(to_integer(user_id.raw())?)
        .execute(pool)
        .await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredInvite {
    pub(crate) code: String,
    pub(crate) guild_id: Snowflake,
    pub(crate) creator_id: Snowflake,
    pub(crate) max_uses: Option<u32>,
    pub(crate) uses: u32,
    pub(crate) expires_at_unix_milliseconds: Option<u64>,
    pub(crate) created_at_unix_milliseconds: u64,
}

/// Inserts the invite if the guild has fewer than `maximum_active_invites` usable invites
/// (checked in the same statement).
///
/// # Errors
/// `LimitReached` at the quota; database errors, including the (astronomically unlikely)
/// duplicate code.
pub(crate) async fn insert_invite(
    pool: &SqlitePool,
    invite: &StoredInvite,
    maximum_active_invites: u32,
) -> Result<(), GuildStorageError> {
    let guild_id = to_integer(invite.guild_id.raw())?;
    let created_at = to_integer(invite.created_at_unix_milliseconds)?;
    let inserted = sqlx::query(
        "INSERT INTO invites (code, guild_id, creator_id, max_uses, uses, expires_at_unix_milliseconds, \
         created_at_unix_milliseconds) \
         SELECT ?, ?, ?, ?, 0, ?, ? \
         WHERE (SELECT COUNT(*) FROM invites WHERE guild_id = ? \
                AND (max_uses IS NULL OR uses < max_uses) \
                AND (expires_at_unix_milliseconds IS NULL OR expires_at_unix_milliseconds > ?)) < ?",
    )
    .bind(&invite.code)
    .bind(guild_id)
    .bind(to_integer(invite.creator_id.raw())?)
    .bind(invite.max_uses.map(i64::from))
    .bind(invite.expires_at_unix_milliseconds.map(to_integer).transpose()?)
    .bind(created_at)
    .bind(guild_id)
    .bind(created_at)
    .bind(to_count(maximum_active_invites))
    .execute(pool)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Err(GuildStorageError::LimitReached);
    }
    Ok(())
}

/// Joins the guild through `code`, atomically: a valid invite consumes one use and adds the
/// member; an existing member consumes nothing. Returns the guild id and whether the user is a
/// new member, or `None` if the code is unknown, expired or used up.
///
/// # Errors
/// Fails on database errors; nothing is written then.
pub(crate) async fn join_with_invite(
    pool: &SqlitePool,
    code: &str,
    user_id: Snowflake,
    now_unix_milliseconds: u64,
) -> Result<Option<(Snowflake, bool)>, GuildStorageError> {
    let user_id = to_integer(user_id.raw())?;
    let now = to_integer(now_unix_milliseconds)?;
    let mut transaction = pool.begin_with(BEGIN_WRITE_TRANSACTION).await?;
    let valid_invite_guild: Option<i64> = sqlx::query_scalar(
        "SELECT guild_id FROM invites WHERE code = ? \
         AND (max_uses IS NULL OR uses < max_uses) \
         AND (expires_at_unix_milliseconds IS NULL OR expires_at_unix_milliseconds > ?)",
    )
    .bind(code)
    .bind(now)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(guild_id) = valid_invite_guild else {
        return Ok(None);
    };
    let added = sqlx::query(
        "INSERT OR IGNORE INTO guild_members (guild_id, user_id, joined_at_unix_milliseconds) \
         VALUES (?, ?, ?)",
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(now)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if added > 0 {
        // The use-limit condition is repeated as defence in depth; the lock already serialises.
        sqlx::query(
            "UPDATE invites SET uses = uses + 1 WHERE code = ? AND (max_uses IS NULL OR uses < max_uses)",
        )
        .bind(code)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(Some((to_id(guild_id)?, added > 0)))
}

/// # Errors
/// Fails on database errors.
pub(crate) async fn find_member(
    pool: &SqlitePool,
    guild_id: Snowflake,
    user_id: Snowflake,
) -> Result<Option<StoredMember>, GuildStorageError> {
    let row: Option<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT users.id, users.username, users.display_name, guild_members.joined_at_unix_milliseconds \
         FROM guild_members JOIN users ON users.id = guild_members.user_id \
         WHERE guild_members.guild_id = ? AND guild_members.user_id = ?",
    )
    .bind(to_integer(guild_id.raw())?)
    .bind(to_integer(user_id.raw())?)
    .fetch_optional(pool)
    .await?;
    let Some((user_id, username, display_name, joined_at)) = row else {
        return Ok(None);
    };
    let user_id = to_id(user_id)?;
    Ok(Some(StoredMember {
        role_ids: member_role_ids(pool, guild_id, user_id).await?,
        user_id,
        username,
        display_name,
        joined_at_unix_milliseconds: to_unsigned(joined_at)?,
    }))
}
