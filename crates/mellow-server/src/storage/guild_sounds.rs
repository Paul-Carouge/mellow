//! Soundboard sounds and members' join sounds (migration 0023, ADR-0032). Each sound's samples
//! are the sealed file of the same id.

use std::collections::HashMap;

use mellow_protocol::{AuditChange, AuditChangeKey, Snowflake, SoundboardSound};
use sqlx::SqliteConnection;

use super::Database;

use crate::storage::audit_log::{NewAuditEntry, insert_audit_entry};
use crate::storage::guilds::{BEGIN_WRITE_TRANSACTION, GuildStorageError};
use crate::storage::sqlite_integers::{
    convert_integer, lookup_integer, to_snowflake, to_sqlite_integer,
};

/// A sound to store, its file already written under `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewGuildSound {
    pub(crate) id: Snowflake,
    pub(crate) name: String,
    pub(crate) emoji: Option<String>,
    /// Set when `emoji` is a custom emoji's key: it must be one of the guild's.
    pub(crate) custom_emoji_id: Option<Snowflake>,
    pub(crate) duration_milliseconds: u32,
    pub(crate) size_bytes: u64,
}

/// A stored sound and the guild it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredGuildSound {
    pub(crate) guild_id: Snowflake,
    pub(crate) sound: SoundboardSound,
}

/// What creating or editing a sound did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SoundWriteOutcome {
    /// Written: the guild's sounds afterwards, as `GUILD_SOUNDS_UPDATE` carries them.
    Written(Vec<SoundboardSound>),
    /// An edit to the values it already has: nothing written or recorded.
    Unchanged(SoundboardSound),
    /// No such guild, or no such sound in it.
    NotFound,
    /// The emoji is a custom emoji of no such guild's.
    EmojiNotFound,
    LimitReached,
}

/// What deleting a sound did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SoundDeleteOutcome {
    /// `sounds` are the guild's afterwards; `members_without_join_sound` had it as join sound.
    Deleted {
        sounds: Vec<SoundboardSound>,
        members_without_join_sound: Vec<Snowflake>,
    },
    NotFound,
}

/// What choosing a join sound did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JoinSoundOutcome {
    Changed,
    Unchanged,
    /// Not a member, or no such sound in the guild.
    NotFound,
}

type SoundRow = (i64, String, Option<String>, i64);

fn sound_from_row(
    (id, name, emoji, duration_milliseconds): SoundRow,
) -> Result<SoundboardSound, GuildStorageError> {
    Ok(SoundboardSound {
        id: to_snowflake(id)?,
        name,
        emoji,
        duration_milliseconds: convert_integer::<_, u32>(duration_milliseconds)?,
    })
}

const SELECT_SOUNDS_OF_GUILD: &str = "SELECT id, name, emoji, duration_milliseconds \
     FROM guild_sounds WHERE guild_id = ? ORDER BY id";

async fn sounds_of_guild(
    connection: &mut SqliteConnection,
    guild_id: i64,
) -> Result<Vec<SoundboardSound>, GuildStorageError> {
    let rows: Vec<SoundRow> = sqlx::query_as(SELECT_SOUNDS_OF_GUILD)
        .bind(guild_id)
        .fetch_all(&mut *connection)
        .await?;
    rows.into_iter().map(sound_from_row).collect()
}

/// The guild's sounds, oldest first.
pub(crate) async fn list_guild_sounds(
    database: &Database,
    guild_id: Snowflake,
) -> Result<Vec<SoundboardSound>, GuildStorageError> {
    let Some(guild_id) = lookup_integer(guild_id) else {
        return Ok(Vec::new());
    };
    let rows: Vec<SoundRow> = sqlx::query_as(SELECT_SOUNDS_OF_GUILD)
        .bind(guild_id)
        .fetch_all(database.readers())
        .await?;
    rows.into_iter().map(sound_from_row).collect()
}

pub(crate) async fn count_guild_sounds(
    database: &Database,
    guild_id: Snowflake,
) -> Result<u32, GuildStorageError> {
    let Some(guild_id) = lookup_integer(guild_id) else {
        return Ok(0);
    };
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM guild_sounds WHERE guild_id = ?")
        .bind(guild_id)
        .fetch_one(database.readers())
        .await?;
    Ok(convert_integer::<_, u32>(count)?)
}

pub(crate) async fn find_guild_sound(
    database: &Database,
    id: Snowflake,
) -> Result<Option<StoredGuildSound>, GuildStorageError> {
    let Some(id) = lookup_integer(id) else {
        return Ok(None);
    };
    let row: Option<(i64, i64, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT guild_id, id, name, emoji, duration_milliseconds FROM guild_sounds WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(database.readers())
    .await?;
    row.map(|(guild_id, id, name, emoji, duration)| {
        Ok(StoredGuildSound {
            guild_id: to_snowflake(guild_id)?,
            sound: sound_from_row((id, name, emoji, duration))?,
        })
    })
    .transpose()
}

/// Whether `custom_emoji_id` is none, or a custom emoji of the guild.
async fn is_emoji_usable_in_guild(
    connection: &mut SqliteConnection,
    guild_id: i64,
    custom_emoji_id: Option<Snowflake>,
) -> Result<bool, GuildStorageError> {
    let Some(emoji_id) = custom_emoji_id else {
        return Ok(true);
    };
    let Some(emoji_id) = lookup_integer(emoji_id) else {
        return Ok(false);
    };
    let row: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM guild_emojis WHERE id = ? AND guild_id = ?")
            .bind(emoji_id)
            .bind(guild_id)
            .fetch_optional(&mut *connection)
            .await?;
    Ok(row.is_some())
}

fn change(key: AuditChangeKey, old_value: &str, new_value: &str) -> AuditChange {
    AuditChange {
        key,
        old_value: old_value.to_owned(),
        new_value: new_value.to_owned(),
    }
}

/// Stores `sound` in the guild, provided it holds fewer than `maximum` sounds, recording `audit`
/// in the same transaction.
///
/// # Errors
/// Fails on database errors; nothing is written then.
pub(crate) async fn insert_guild_sound(
    database: &Database,
    guild_id: Snowflake,
    sound: &NewGuildSound,
    maximum: u32,
    audit: &NewAuditEntry,
    now_unix_milliseconds: u64,
) -> Result<SoundWriteOutcome, GuildStorageError> {
    let guild = to_sqlite_integer(guild_id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let guild_row: Option<i64> = sqlx::query_scalar("SELECT 1 FROM guilds WHERE id = ?")
        .bind(guild)
        .fetch_optional(&mut *transaction)
        .await?;
    if guild_row.is_none() {
        return Ok(SoundWriteOutcome::NotFound);
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM guild_sounds WHERE guild_id = ?")
        .bind(guild)
        .fetch_one(&mut *transaction)
        .await?;
    if count >= i64::from(maximum) {
        return Ok(SoundWriteOutcome::LimitReached);
    }
    if !is_emoji_usable_in_guild(&mut transaction, guild, sound.custom_emoji_id).await? {
        return Ok(SoundWriteOutcome::EmojiNotFound);
    }
    sqlx::query(
        "INSERT INTO guild_sounds (id, guild_id, name, emoji, duration_milliseconds, \
         size_bytes, created_at_unix_milliseconds) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(to_sqlite_integer(sound.id.raw())?)
    .bind(guild)
    .bind(&sound.name)
    .bind(&sound.emoji)
    .bind(i64::from(sound.duration_milliseconds))
    .bind(to_sqlite_integer(sound.size_bytes)?)
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .execute(&mut *transaction)
    .await?;
    let audit = NewAuditEntry {
        changes: vec![change(AuditChangeKey::Name, "", &sound.name)],
        ..audit.clone()
    };
    insert_audit_entry(&mut transaction, &audit).await?;
    // Read before the commit: once it is done, nothing may fail before the caller announces it.
    let sounds = sounds_of_guild(&mut transaction, guild).await?;
    transaction.commit().await?;
    Ok(SoundWriteOutcome::Written(sounds))
}

/// Replaces the name and emoji of the guild's sound, recording `audit` with what changed in the
/// same transaction. `custom_emoji_id` is set when `emoji` is a custom emoji's key.
///
/// # Errors
/// Fails on database errors; nothing is written then.
pub(crate) async fn update_guild_sound(
    database: &Database,
    guild_id: Snowflake,
    sound_id: Snowflake,
    name: &str,
    emoji: Option<&str>,
    custom_emoji_id: Option<Snowflake>,
    audit: &NewAuditEntry,
) -> Result<SoundWriteOutcome, GuildStorageError> {
    let (Some(guild), Some(id)) = (lookup_integer(guild_id), lookup_integer(sound_id)) else {
        return Ok(SoundWriteOutcome::NotFound);
    };
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let row: Option<SoundRow> = sqlx::query_as(
        "SELECT id, name, emoji, duration_milliseconds FROM guild_sounds \
         WHERE id = ? AND guild_id = ?",
    )
    .bind(id)
    .bind(guild)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(old) = row.map(sound_from_row).transpose()? else {
        return Ok(SoundWriteOutcome::NotFound);
    };
    let mut changes = Vec::new();
    if old.name != name {
        changes.push(change(AuditChangeKey::Name, &old.name, name));
    }
    if old.emoji.as_deref() != emoji {
        changes.push(change(
            AuditChangeKey::Emoji,
            old.emoji.as_deref().unwrap_or_default(),
            emoji.unwrap_or_default(),
        ));
    }
    if changes.is_empty() {
        return Ok(SoundWriteOutcome::Unchanged(old));
    }
    // An emoji kept as it was is not checked again: it may have been deleted meanwhile.
    if old.emoji.as_deref() != emoji
        && !is_emoji_usable_in_guild(&mut transaction, guild, custom_emoji_id).await?
    {
        return Ok(SoundWriteOutcome::EmojiNotFound);
    }
    sqlx::query("UPDATE guild_sounds SET name = ?, emoji = ? WHERE id = ?")
        .bind(name)
        .bind(emoji)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    insert_audit_entry(
        &mut transaction,
        &NewAuditEntry {
            changes,
            ..audit.clone()
        },
    )
    .await?;
    let sounds = sounds_of_guild(&mut transaction, guild).await?;
    transaction.commit().await?;
    Ok(SoundWriteOutcome::Written(sounds))
}

/// Deletes the guild's sound, clearing it as join sound, recording `audit` in the same
/// transaction. The caller deletes its file.
///
/// # Errors
/// Fails on database errors; nothing is written then.
pub(crate) async fn delete_guild_sound(
    database: &Database,
    guild_id: Snowflake,
    sound_id: Snowflake,
    audit: &NewAuditEntry,
) -> Result<SoundDeleteOutcome, GuildStorageError> {
    let (Some(guild), Some(id)) = (lookup_integer(guild_id), lookup_integer(sound_id)) else {
        return Ok(SoundDeleteOutcome::NotFound);
    };
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let name: Option<String> =
        sqlx::query_scalar("SELECT name FROM guild_sounds WHERE id = ? AND guild_id = ?")
            .bind(id)
            .bind(guild)
            .fetch_optional(&mut *transaction)
            .await?;
    let Some(name) = name else {
        return Ok(SoundDeleteOutcome::NotFound);
    };
    let members: Vec<i64> = sqlx::query_scalar(
        "SELECT user_id FROM guild_members WHERE guild_id = ? AND join_sound_id = ?",
    )
    .bind(guild)
    .bind(id)
    .fetch_all(&mut *transaction)
    .await?;
    // Members' join_sound_id is cleared by ON DELETE SET NULL.
    sqlx::query("DELETE FROM guild_sounds WHERE id = ?")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    let audit = NewAuditEntry {
        changes: vec![change(AuditChangeKey::Name, &name, "")],
        ..audit.clone()
    };
    insert_audit_entry(&mut transaction, &audit).await?;
    let sounds = sounds_of_guild(&mut transaction, guild).await?;
    let members_without_join_sound = members
        .into_iter()
        .map(to_snowflake)
        .collect::<Result<_, _>>()?;
    transaction.commit().await?;
    Ok(SoundDeleteOutcome::Deleted {
        sounds,
        members_without_join_sound,
    })
}

/// Sets the member's join sound in the guild to one of its sounds, or clears it.
///
/// # Errors
/// Fails on database errors; nothing is written then.
pub(crate) async fn set_member_join_sound(
    database: &Database,
    guild_id: Snowflake,
    user_id: Snowflake,
    sound_id: Option<Snowflake>,
) -> Result<JoinSoundOutcome, GuildStorageError> {
    let (Some(guild), Some(user)) = (lookup_integer(guild_id), lookup_integer(user_id)) else {
        return Ok(JoinSoundOutcome::NotFound);
    };
    let sound = match sound_id {
        Some(sound_id) => match lookup_integer(sound_id) {
            Some(sound) => Some(sound),
            None => return Ok(JoinSoundOutcome::NotFound),
        },
        None => None,
    };
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let current: Option<Option<i64>> = sqlx::query_scalar(
        "SELECT join_sound_id FROM guild_members WHERE guild_id = ? AND user_id = ?",
    )
    .bind(guild)
    .bind(user)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(current) = current else {
        return Ok(JoinSoundOutcome::NotFound);
    };
    if current == sound {
        return Ok(JoinSoundOutcome::Unchanged);
    }
    if let Some(sound) = sound {
        let row: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM guild_sounds WHERE id = ? AND guild_id = ?")
                .bind(sound)
                .bind(guild)
                .fetch_optional(&mut *transaction)
                .await?;
        if row.is_none() {
            return Ok(JoinSoundOutcome::NotFound);
        }
    }
    sqlx::query("UPDATE guild_members SET join_sound_id = ? WHERE guild_id = ? AND user_id = ?")
        .bind(sound)
        .bind(guild)
        .bind(user)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(JoinSoundOutcome::Changed)
}

/// The member's join sound in the guild, if any.
pub(crate) async fn member_join_sound(
    database: &Database,
    guild_id: Snowflake,
    user_id: Snowflake,
) -> Result<Option<Snowflake>, GuildStorageError> {
    let (Some(guild), Some(user)) = (lookup_integer(guild_id), lookup_integer(user_id)) else {
        return Ok(None);
    };
    let sound: Option<Option<i64>> = sqlx::query_scalar(
        "SELECT join_sound_id FROM guild_members WHERE guild_id = ? AND user_id = ?",
    )
    .bind(guild)
    .bind(user)
    .fetch_optional(database.readers())
    .await?;
    Ok(sound.flatten().map(to_snowflake).transpose()?)
}

/// Each member's join sound in the guild, for those who chose one.
pub(crate) async fn join_sounds_of_guild(
    database: &Database,
    guild_id: Snowflake,
) -> Result<HashMap<Snowflake, Snowflake>, GuildStorageError> {
    let Some(guild) = lookup_integer(guild_id) else {
        return Ok(HashMap::new());
    };
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT user_id, join_sound_id FROM guild_members \
         WHERE guild_id = ? AND join_sound_id IS NOT NULL",
    )
    .bind(guild)
    .fetch_all(database.readers())
    .await?;
    rows.into_iter()
        .map(|(user, sound)| Ok((to_snowflake(user)?, to_snowflake(sound)?)))
        .collect()
}

/// Every sound's id, which is also its file's, for the orphan sweep.
pub(crate) async fn all_guild_sound_ids(
    database: &Database,
) -> Result<Vec<Snowflake>, GuildStorageError> {
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM guild_sounds")
        .fetch_all(database.readers())
        .await?;
    Ok(ids
        .into_iter()
        .map(to_snowflake)
        .collect::<Result<_, _>>()?)
}
