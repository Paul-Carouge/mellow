//! The guild audit log (migration 0008). Entries are written by the storage function of the action
//! they record, inside its transaction.

use mellow_protocol::{AuditAction, AuditChange, AuditLogEntry, Snowflake};
use sqlx::SqliteConnection;

use super::Database;

use crate::storage::guilds::{GuildStorageError, to_id, to_integer};

/// An action about to be committed, as the audit log records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewAuditEntry {
    pub(crate) id: Snowflake,
    pub(crate) guild_id: Snowflake,
    pub(crate) actor_id: Snowflake,
    pub(crate) action: AuditAction,
    pub(crate) target_id: Option<Snowflake>,
    pub(crate) context_id: Option<Snowflake>,
    /// The channel the entry is about: readers who cannot view it never see the entry.
    pub(crate) channel_id: Option<Snowflake>,
    pub(crate) reason: String,
    pub(crate) changes: Vec<AuditChange>,
}

/// Which entries about channels a reader may see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChannelEntryFilter {
    /// The owner and administrators, who see every channel, deleted ones included.
    All,
    /// Entries about these channels only; entries about no channel are always shown.
    Only(Vec<Snowflake>),
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
        AuditAction::MessageDelete => "message_delete",
        AuditAction::MessagePin => "message_pin",
        AuditAction::MessageUnpin => "message_unpin",
        AuditAction::MessageReactionRemove => "message_reaction_remove",
        AuditAction::GuildUpdate => "guild_update",
        AuditAction::InviteCreate => "invite_create",
        AuditAction::InviteDelete => "invite_delete",
        // Only clients produce it, for actions of a newer server; the server never records it.
        AuditAction::Unknown => "unknown",
    }
}

fn action_from_text(text: &str) -> Result<AuditAction, GuildStorageError> {
    match text {
        "channel_create" => Ok(AuditAction::ChannelCreate),
        "channel_update" => Ok(AuditAction::ChannelUpdate),
        "channel_delete" => Ok(AuditAction::ChannelDelete),
        "overwrite_set" => Ok(AuditAction::OverwriteSet),
        "overwrite_delete" => Ok(AuditAction::OverwriteDelete),
        "role_create" => Ok(AuditAction::RoleCreate),
        "role_update" => Ok(AuditAction::RoleUpdate),
        "role_delete" => Ok(AuditAction::RoleDelete),
        "roles_reorder" => Ok(AuditAction::RolesReorder),
        "member_role_add" => Ok(AuditAction::MemberRoleAdd),
        "member_role_remove" => Ok(AuditAction::MemberRoleRemove),
        "member_kick" => Ok(AuditAction::MemberKick),
        "member_ban_add" => Ok(AuditAction::MemberBanAdd),
        "member_ban_remove" => Ok(AuditAction::MemberBanRemove),
        "message_delete" => Ok(AuditAction::MessageDelete),
        "message_pin" => Ok(AuditAction::MessagePin),
        "message_unpin" => Ok(AuditAction::MessageUnpin),
        "message_reaction_remove" => Ok(AuditAction::MessageReactionRemove),
        "guild_update" => Ok(AuditAction::GuildUpdate),
        "invite_create" => Ok(AuditAction::InviteCreate),
        "invite_delete" => Ok(AuditAction::InviteDelete),
        other => Err(GuildStorageError::UnknownAuditAction(other.to_owned())),
    }
}

fn optional_integer(id: Option<Snowflake>) -> Result<Option<i64>, GuildStorageError> {
    id.map(|id| to_integer(id.raw())).transpose()
}

/// Records `entry` on `connection`, which must be the transaction of the action it describes.
///
/// # Errors
/// Fails on database errors; the caller's transaction is then rolled back with the action.
pub(crate) async fn insert_audit_entry(
    connection: &mut SqliteConnection,
    entry: &NewAuditEntry,
) -> Result<(), GuildStorageError> {
    let changes = serde_json::to_string(&entry.changes)
        .map_err(|_| GuildStorageError::UnreadableAuditChanges)?;
    sqlx::query(
        "INSERT INTO audit_log (id, guild_id, actor_id, action, target_id, context_id, \
         channel_id, reason, changes) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(to_integer(entry.id.raw())?)
    .bind(to_integer(entry.guild_id.raw())?)
    .bind(to_integer(entry.actor_id.raw())?)
    .bind(action_to_text(entry.action))
    .bind(optional_integer(entry.target_id)?)
    .bind(optional_integer(entry.context_id)?)
    .bind(optional_integer(entry.channel_id)?)
    .bind(&entry.reason)
    .bind(changes)
    .execute(connection)
    .await?;
    Ok(())
}

type AuditRow = (
    i64,
    Option<i64>,
    String,
    Option<i64>,
    Option<i64>,
    String,
    String,
);

/// At most `limit` entries of the guild, newest first, strictly older than `before` if given,
/// and only those about channels `filter` lets through (filtered before the limit, so a page
/// is never cut short by hidden entries).
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn list_audit_entries(
    database: &Database,
    guild_id: Snowflake,
    before: Option<Snowflake>,
    limit: u32,
    filter: &ChannelEntryFilter,
) -> Result<Vec<AuditLogEntry>, GuildStorageError> {
    // An id SQLite cannot hold is above every stored one.
    let before = before.map_or(i64::MAX, |before| {
        i64::try_from(before.raw()).unwrap_or(i64::MAX)
    });
    let visible_channel_ids = match filter {
        ChannelEntryFilter::All => None,
        ChannelEntryFilter::Only(channel_ids) => Some(
            serde_json::to_string(
                &channel_ids
                    .iter()
                    .map(|id| to_integer(id.raw()))
                    .collect::<Result<Vec<_>, _>>()?,
            )
            .map_err(|_| GuildStorageError::UnreadableAuditChanges)?,
        ),
    };
    let rows: Vec<AuditRow> = sqlx::query_as(
        "SELECT id, actor_id, action, target_id, context_id, reason, changes FROM audit_log \
         WHERE guild_id = ? AND id < ? AND (? IS NULL OR channel_id IS NULL \
         OR channel_id IN (SELECT value FROM json_each(?))) ORDER BY id DESC LIMIT ?",
    )
    .bind(to_integer(guild_id.raw())?)
    .bind(before)
    .bind(visible_channel_ids.as_deref())
    .bind(visible_channel_ids.as_deref())
    .bind(i64::from(limit))
    .fetch_all(database.readers())
    .await?;
    rows.into_iter()
        .map(
            |(id, actor_id, action, target_id, context_id, reason, changes)| {
                Ok(AuditLogEntry {
                    id: to_id(id)?,
                    actor_id: actor_id.map(to_id).transpose()?,
                    action: action_from_text(&action)?,
                    target_id: target_id.map(to_id).transpose()?,
                    context_id: context_id.map(to_id).transpose()?,
                    reason,
                    changes: serde_json::from_str(&changes)
                        .map_err(|_| GuildStorageError::UnreadableAuditChanges)?,
                })
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_recorded_action_reads_back() {
        let recorded = [
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
            AuditAction::MessageDelete,
        ];
        for action in recorded {
            assert_eq!(action_from_text(action_to_text(action)).unwrap(), action);
        }
        assert!(action_from_text("unknown").is_err(), "never recorded");
    }
}
