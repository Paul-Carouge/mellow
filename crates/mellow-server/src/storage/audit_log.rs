//! The guild audit log (migration 0008). Entries are written by the storage function of the action
//! they record, inside its transaction.

use mellow_protocol::{AuditAction, AuditLogEntry, Snowflake};
use sqlx::{SqliteConnection, SqlitePool};

use crate::storage::guilds::GuildStorageError;

/// An action about to be committed, as the audit log records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewAuditEntry {
    pub(crate) id: Snowflake,
    pub(crate) guild_id: Snowflake,
    pub(crate) actor_id: Snowflake,
    pub(crate) action: AuditAction,
    pub(crate) target_id: Option<Snowflake>,
    pub(crate) context_id: Option<Snowflake>,
    pub(crate) reason: String,
}

fn action_to_text(action: AuditAction) -> &'static str {
    match action {
        AuditAction::ChannelCreate => "channel_create",
        AuditAction::ChannelUpdate => "channel_update",
        AuditAction::ChannelDelete => "channel_delete",
        AuditAction::OverwriteSet => "overwrite_set",
        AuditAction::OverwriteDelete => "overwrite_delete",
        AuditAction::RoleCreate => "role_create",
        AuditAction::RoleUpdate => "role_update",
        AuditAction::RoleDelete => "role_delete",
        AuditAction::RolesReorder => "roles_reorder",
        AuditAction::MemberRoleAdd => "member_role_add",
        AuditAction::MemberRoleRemove => "member_role_remove",
        AuditAction::MemberKick => "member_kick",
        AuditAction::MemberBanAdd => "member_ban_add",
        AuditAction::MemberBanRemove => "member_ban_remove",
    }
}

fn action_from_text(text: &str) -> Result<AuditAction, GuildStorageError> {
    [
        AuditAction::ChannelCreate,
        AuditAction::ChannelUpdate,
        AuditAction::ChannelDelete,
        AuditAction::OverwriteSet,
        AuditAction::OverwriteDelete,
        AuditAction::RoleCreate,
        AuditAction::RoleUpdate,
        AuditAction::RoleDelete,
        AuditAction::RolesReorder,
        AuditAction::MemberRoleAdd,
        AuditAction::MemberRoleRemove,
        AuditAction::MemberKick,
        AuditAction::MemberBanAdd,
        AuditAction::MemberBanRemove,
    ]
    .into_iter()
    .find(|action| action_to_text(*action) == text)
    .ok_or_else(|| GuildStorageError::UnknownAuditAction(text.to_owned()))
}

fn to_integer(id: Snowflake) -> Result<i64, GuildStorageError> {
    i64::try_from(id.raw()).map_err(|_| GuildStorageError::IntegerOutOfRange)
}

fn to_id(value: i64) -> Result<Snowflake, GuildStorageError> {
    u64::try_from(value)
        .map(Snowflake::from_raw)
        .map_err(|_| GuildStorageError::IntegerOutOfRange)
}

/// Records `entry` on `connection`, which must be the transaction of the action it describes.
///
/// # Errors
/// Fails on database errors; the caller's transaction is then rolled back with the action.
pub(crate) async fn insert_audit_entry(
    connection: &mut SqliteConnection,
    entry: &NewAuditEntry,
) -> Result<(), GuildStorageError> {
    sqlx::query(
        "INSERT INTO audit_log (id, guild_id, actor_id, action, target_id, context_id, reason) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(to_integer(entry.id)?)
    .bind(to_integer(entry.guild_id)?)
    .bind(to_integer(entry.actor_id)?)
    .bind(action_to_text(entry.action))
    .bind(entry.target_id.map(to_integer).transpose()?)
    .bind(entry.context_id.map(to_integer).transpose()?)
    .bind(&entry.reason)
    .execute(connection)
    .await?;
    Ok(())
}

type AuditRow = (i64, Option<i64>, String, Option<i64>, Option<i64>, String);

/// At most `limit` entries of the guild, newest first, strictly older than `before` if given.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn list_audit_entries(
    pool: &SqlitePool,
    guild_id: Snowflake,
    before: Option<Snowflake>,
    limit: u32,
) -> Result<Vec<AuditLogEntry>, GuildStorageError> {
    let rows: Vec<AuditRow> = sqlx::query_as(
        "SELECT id, actor_id, action, target_id, context_id, reason FROM audit_log \
         WHERE guild_id = ? AND (? IS NULL OR id < ?) ORDER BY id DESC LIMIT ?",
    )
    .bind(to_integer(guild_id)?)
    // Every id SQLite holds is below one it cannot hold.
    .bind(before.and_then(|before| i64::try_from(before.raw()).ok()))
    .bind(before.and_then(|before| i64::try_from(before.raw()).ok()))
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(id, actor_id, action, target_id, context_id, reason)| {
            Ok(AuditLogEntry {
                id: to_id(id)?,
                actor_id: actor_id.map(to_id).transpose()?,
                action: action_from_text(&action)?,
                target_id: target_id.map(to_id).transpose()?,
                context_id: context_id.map(to_id).transpose()?,
                reason,
            })
        })
        .collect()
}
