//! "About me" texts, profile links and their sites' icons (migration 0043, ADR-0050). Icon files
//! are sealed in `attachment_files`.

use mellow_protocol::Snowflake;

use super::Database;
use crate::storage::guilds::BEGIN_WRITE_TRANSACTION;
use crate::storage::messages::MessageStorageError;
use crate::storage::sqlite_integers::{lookup_integer, to_snowflake, to_sqlite_integer};

/// A link to store, already checked (`profile_rules`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewStoredLink {
    pub(crate) url: String,
    pub(crate) label: Option<String>,
    pub(crate) host: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredLink {
    pub(crate) url: String,
    pub(crate) label: Option<String>,
    pub(crate) host: String,
    /// The host's icon as last fetched; `None` before the first fetch and for sites without one.
    pub(crate) icon_file_id: Option<Snowflake>,
    /// `None` when the host was never fetched.
    pub(crate) icon_refresh_after_unix_milliseconds: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StoredProfile {
    pub(crate) about_me: String,
    pub(crate) links: Vec<StoredLink>,
}

/// Whether `viewer_id` may read `user_id`'s profile: their own, or someone they share a server,
/// a conversation or a relationship with. A block does not count: it is written on the blocker's
/// side only, without telling the user blocked, so anyone could otherwise read any profile.
/// `false` for an unknown user.
pub(crate) async fn may_view_profile(
    database: &Database,
    viewer_id: Snowflake,
    user_id: Snowflake,
) -> Result<bool, MessageStorageError> {
    let (Some(viewer), Some(user)) = (lookup_integer(viewer_id), lookup_integer(user_id)) else {
        return Ok(false);
    };
    let may_view: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE id = ?2) AND (?1 = ?2 \
         OR EXISTS (SELECT 1 FROM guild_members AS mine JOIN guild_members AS theirs \
             ON theirs.guild_id = mine.guild_id AND theirs.user_id = ?2 WHERE mine.user_id = ?1) \
         OR EXISTS (SELECT 1 FROM direct_message_participants AS mine \
             JOIN direct_message_participants AS theirs \
             ON theirs.channel_id = mine.channel_id AND theirs.user_id = ?2 WHERE mine.user_id = ?1) \
         OR EXISTS (SELECT 1 FROM relationships WHERE user_id = ?1 AND other_user_id = ?2 \
             AND kind != 'blocked'))",
    )
    .bind(viewer)
    .bind(user)
    .fetch_one(database.readers())
    .await?;
    Ok(may_view)
}

type LinkRow = (String, Option<String>, String, Option<i64>, Option<i64>);

/// The user's text and links with their icons, in two reads.
pub(crate) async fn profile_of_user(
    database: &Database,
    user_id: Snowflake,
) -> Result<StoredProfile, MessageStorageError> {
    let user = to_sqlite_integer(user_id.raw())?;
    let about_me: Option<String> =
        sqlx::query_scalar("SELECT about_me FROM user_profiles WHERE user_id = ?")
            .bind(user)
            .fetch_optional(database.readers())
            .await?;
    let rows: Vec<LinkRow> = sqlx::query_as(
        "SELECT user_profile_links.url, user_profile_links.label, user_profile_links.host, \
         profile_link_icons.file_id, profile_link_icons.refresh_after_unix_milliseconds \
         FROM user_profile_links \
         LEFT JOIN profile_link_icons ON profile_link_icons.host = user_profile_links.host \
         WHERE user_profile_links.user_id = ? ORDER BY user_profile_links.position",
    )
    .bind(user)
    .fetch_all(database.readers())
    .await?;
    let links = rows
        .into_iter()
        .map(|(url, label, host, file_id, refresh_after)| {
            Ok(StoredLink {
                url,
                label,
                host,
                icon_file_id: file_id.map(to_snowflake).transpose()?,
                icon_refresh_after_unix_milliseconds: refresh_after
                    .map(|value| u64::try_from(value).unwrap_or(0)),
            })
        })
        .collect::<Result<_, MessageStorageError>>()?;
    Ok(StoredProfile {
        about_me: about_me.unwrap_or_default(),
        links,
    })
}

/// Replaces the user's text and links in one transaction. Answers the icon files of the sites no
/// profile links any more, whose rows it deleted: the caller deletes the files.
pub(crate) async fn replace_profile(
    database: &Database,
    user_id: Snowflake,
    about_me: &str,
    links: &[NewStoredLink],
) -> Result<Vec<Snowflake>, MessageStorageError> {
    let user = to_sqlite_integer(user_id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    sqlx::query(
        "INSERT INTO user_profiles (user_id, about_me) VALUES (?, ?) \
         ON CONFLICT (user_id) DO UPDATE SET about_me = excluded.about_me",
    )
    .bind(user)
    .bind(about_me)
    .execute(&mut *transaction)
    .await?;
    let previous_hosts: Vec<String> =
        sqlx::query_scalar("DELETE FROM user_profile_links WHERE user_id = ? RETURNING host")
            .bind(user)
            .fetch_all(&mut *transaction)
            .await?;
    for (position, link) in (0_i64..).zip(links) {
        sqlx::query(
            "INSERT INTO user_profile_links (user_id, position, url, label, host) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(user)
        .bind(position)
        .bind(&link.url)
        .bind(link.label.as_deref())
        .bind(&link.host)
        .execute(&mut *transaction)
        .await?;
    }
    let mut orphaned_files = Vec::new();
    for host in previous_hosts {
        let file_id: Option<Option<i64>> = sqlx::query_scalar(
            "DELETE FROM profile_link_icons WHERE host = ?1 \
             AND NOT EXISTS (SELECT 1 FROM user_profile_links WHERE host = ?1) RETURNING file_id",
        )
        .bind(&host)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(Some(file_id)) = file_id {
            orphaned_files.push(to_snowflake(file_id)?);
        }
    }
    transaction.commit().await?;
    Ok(orphaned_files)
}

/// What recording a fetched icon did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IconRecord {
    /// Stored; the file it replaced, if any, is the caller's to delete.
    Stored { replaced_file_id: Option<Snowflake> },
    /// No profile links the host any more: the new file is the caller's to delete.
    HostUnlinked,
}

/// Stores the outcome of a fetch of `host`'s icon: `file` (its id and size) or none, until
/// `refresh_after_unix_milliseconds`, if a profile still links the host.
pub(crate) async fn record_link_icon(
    database: &Database,
    host: &str,
    file: Option<(Snowflake, u64)>,
    refresh_after_unix_milliseconds: u64,
) -> Result<IconRecord, MessageStorageError> {
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    let is_linked: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_profile_links WHERE host = ?)")
            .bind(host)
            .fetch_one(&mut *transaction)
            .await?;
    if !is_linked {
        return Ok(IconRecord::HostUnlinked);
    }
    let previous: Option<Option<i64>> =
        sqlx::query_scalar("SELECT file_id FROM profile_link_icons WHERE host = ?")
            .bind(host)
            .fetch_optional(&mut *transaction)
            .await?;
    let (file_id, size_bytes) = match file {
        Some((id, size)) => (
            Some(to_sqlite_integer(id.raw())?),
            Some(to_sqlite_integer(size)?),
        ),
        None => (None, None),
    };
    sqlx::query(
        "INSERT INTO profile_link_icons (host, file_id, size_bytes, \
         refresh_after_unix_milliseconds) VALUES (?, ?, ?, ?) \
         ON CONFLICT (host) DO UPDATE SET file_id = excluded.file_id, \
         size_bytes = excluded.size_bytes, \
         refresh_after_unix_milliseconds = excluded.refresh_after_unix_milliseconds",
    )
    .bind(host)
    .bind(file_id)
    .bind(size_bytes)
    .bind(to_sqlite_integer(refresh_after_unix_milliseconds)?)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(IconRecord::Stored {
        replaced_file_id: previous.flatten().map(to_snowflake).transpose()?,
    })
}

/// After a fetch of `host`'s icon that may succeed later: the icon stored, if any, stays, and the
/// host is fetched again after `refresh_after_unix_milliseconds`.
pub(crate) async fn postpone_link_icon_refresh(
    database: &Database,
    host: &str,
    refresh_after_unix_milliseconds: u64,
) -> Result<(), MessageStorageError> {
    sqlx::query(
        "INSERT INTO profile_link_icons (host, refresh_after_unix_milliseconds) \
         SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM user_profile_links WHERE host = ?1) \
         ON CONFLICT (host) DO UPDATE SET \
         refresh_after_unix_milliseconds = excluded.refresh_after_unix_milliseconds",
    )
    .bind(host)
    .bind(to_sqlite_integer(refresh_after_unix_milliseconds)?)
    .execute(database.writer())
    .await?;
    Ok(())
}

/// Whether `file_id` is a site icon some profile shows.
pub(crate) async fn is_link_icon_file(
    database: &Database,
    file_id: Snowflake,
) -> Result<bool, MessageStorageError> {
    let Some(file_id) = lookup_integer(file_id) else {
        return Ok(false);
    };
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM profile_link_icons WHERE file_id = ?)")
            .bind(file_id)
            .fetch_one(database.readers())
            .await?,
    )
}

/// Every site icon file, for the orphan sweep.
pub(crate) async fn all_link_icon_file_ids(
    database: &Database,
) -> Result<Vec<Snowflake>, MessageStorageError> {
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT file_id FROM profile_link_icons WHERE file_id IS NOT NULL")
            .fetch_all(database.readers())
            .await?;
    Ok(ids
        .into_iter()
        .map(to_snowflake)
        .collect::<Result<_, _>>()?)
}
