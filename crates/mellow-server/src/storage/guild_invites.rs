//! Guild invites and joining through them.

use mellow_protocol::Snowflake;

use super::Database;
use super::guilds::{BEGIN_WRITE_TRANSACTION, GuildStorageError, to_count};
use crate::storage::audit_log::{NewAuditEntry, insert_audit_entry};
use crate::storage::sqlite_integers::{convert_integer, to_snowflake, to_sqlite_integer};

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
/// (checked in the same statement), and records `audit` with it.
///
/// # Errors
/// `LimitReached` at the quota; database errors, including the (astronomically unlikely)
/// duplicate code.
pub(crate) async fn insert_invite(
    database: &Database,
    invite: &StoredInvite,
    maximum_active_invites: u32,
    audit: &NewAuditEntry,
) -> Result<(), GuildStorageError> {
    let guild_id = to_sqlite_integer(invite.guild_id.raw())?;
    let created_at = to_sqlite_integer(invite.created_at_unix_milliseconds)?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
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
    .bind(to_sqlite_integer(invite.creator_id.raw())?)
    .bind(invite.max_uses.map(i64::from))
    .bind(invite.expires_at_unix_milliseconds.map(to_sqlite_integer).transpose()?)
    .bind(created_at)
    .bind(guild_id)
    .bind(created_at)
    .bind(to_count(maximum_active_invites))
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if inserted == 0 {
        return Err(GuildStorageError::LimitReached);
    }
    insert_audit_entry(&mut transaction, audit).await?;
    transaction.commit().await?;
    Ok(())
}

/// A usable invite with its creator's names, as the invites settings list it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListedInvite {
    pub(crate) invite: StoredInvite,
    pub(crate) creator_username: String,
    pub(crate) creator_display_name: String,
}

type ListedInviteRow = (
    String,
    i64,
    Option<i64>,
    i64,
    Option<i64>,
    i64,
    String,
    String,
);

fn to_u32(value: i64) -> Result<u32, GuildStorageError> {
    Ok(convert_integer::<_, u32>(value)?)
}

/// At most `maximum_invites` invites of the guild that can still be used at `now`, newest first.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn list_usable_invites(
    database: &Database,
    guild_id: Snowflake,
    now_unix_milliseconds: u64,
    maximum_invites: u32,
) -> Result<Vec<ListedInvite>, GuildStorageError> {
    let rows: Vec<ListedInviteRow> = sqlx::query_as(
        "SELECT invites.code, invites.creator_id, invites.max_uses, invites.uses, \
         invites.expires_at_unix_milliseconds, invites.created_at_unix_milliseconds, \
         users.username, users.display_name \
         FROM invites JOIN users ON users.id = invites.creator_id \
         WHERE invites.guild_id = ? \
         AND (invites.max_uses IS NULL OR invites.uses < invites.max_uses) \
         AND (invites.expires_at_unix_milliseconds IS NULL \
              OR invites.expires_at_unix_milliseconds > ?) \
         ORDER BY invites.created_at_unix_milliseconds DESC, invites.code LIMIT ?",
    )
    .bind(to_sqlite_integer(guild_id.raw())?)
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .bind(to_count(maximum_invites))
    .fetch_all(database.readers())
    .await?;
    rows.into_iter()
        .map(
            |(code, creator_id, max_uses, uses, expires_at, created_at, username, display_name)| {
                Ok(ListedInvite {
                    invite: StoredInvite {
                        code,
                        guild_id,
                        creator_id: to_snowflake(creator_id)?,
                        max_uses: max_uses.map(to_u32).transpose()?,
                        uses: to_u32(uses)?,
                        expires_at_unix_milliseconds: expires_at
                            .map(convert_integer::<_, u64>)
                            .transpose()?,
                        created_at_unix_milliseconds: convert_integer::<_, u64>(created_at)?,
                    },
                    creator_username: username,
                    creator_display_name: display_name,
                })
            },
        )
        .collect()
}

/// The guild an invite belongs to, used up or expired included; `None` for an unknown code.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn find_invite_guild_id(
    database: &Database,
    code: &str,
) -> Result<Option<Snowflake>, GuildStorageError> {
    let guild_id: Option<i64> = sqlx::query_scalar("SELECT guild_id FROM invites WHERE code = ?")
        .bind(code)
        .fetch_optional(database.readers())
        .await?;
    Ok(guild_id.map(to_snowflake).transpose()?)
}

/// Deletes the invite and records `audit`, in one transaction. Returns whether it existed in
/// that guild.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn delete_invite(
    database: &Database,
    guild_id: Snowflake,
    code: &str,
    audit: &NewAuditEntry,
) -> Result<bool, GuildStorageError> {
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let deleted = sqlx::query("DELETE FROM invites WHERE code = ? AND guild_id = ?")
        .bind(code)
        .bind(to_sqlite_integer(guild_id.raw())?)
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

/// What happened to an invite use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JoinOutcome {
    /// Unknown, expired or used-up code.
    InvalidInvite,
    /// The user is banned from the invite's guild; nothing was consumed.
    Banned,
    Joined {
        guild_id: Snowflake,
        is_new_member: bool,
    },
}

/// Joins the guild through `code`, atomically: a valid invite consumes one use and adds the
/// member; an existing member consumes nothing; a banned user is refused in the same
/// transaction, so a ban committed meanwhile cannot be slipped past.
///
/// # Errors
/// Fails on database errors; nothing is written then.
pub(crate) async fn join_with_invite(
    database: &Database,
    code: &str,
    user_id: Snowflake,
    now_unix_milliseconds: u64,
) -> Result<JoinOutcome, GuildStorageError> {
    let user_id = to_sqlite_integer(user_id.raw())?;
    let now = to_sqlite_integer(now_unix_milliseconds)?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
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
        return Ok(JoinOutcome::InvalidInvite);
    };
    let is_banned: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM bans WHERE guild_id = ? AND user_id = ?)")
            .bind(guild_id)
            .bind(user_id)
            .fetch_one(&mut *transaction)
            .await?;
    if is_banned {
        return Ok(JoinOutcome::Banned);
    }
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
    Ok(JoinOutcome::Joined {
        guild_id: to_snowflake(guild_id)?,
        is_new_member: added > 0,
    })
}
