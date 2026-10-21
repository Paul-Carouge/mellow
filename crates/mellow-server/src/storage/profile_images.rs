//! Avatars and banners (migration 0011); their sealed files are in `attachment_files`.

use std::collections::HashMap;

use mellow_protocol::{ProfileImage, Snowflake};
use sqlx::SqlitePool;

use crate::storage::guilds::BEGIN_WRITE_TRANSACTION;
use crate::storage::messages::{MessageStorageError, lookup_integer, to_id, to_integer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileImageSlot {
    Avatar,
    Banner,
}

/// A user's current avatar and banner, as user payloads carry them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct UserProfileImages {
    pub(crate) avatar: Option<ProfileImage>,
    pub(crate) banner: Option<ProfileImage>,
}

/// A version to store: its file, its still frame's file when animated, its size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NewProfileImage {
    pub(crate) id: Snowflake,
    pub(crate) still_id: Option<Snowflake>,
    pub(crate) size_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StoredProfileImage {
    pub(crate) id: Snowflake,
    pub(crate) user_id: Snowflake,
    pub(crate) still_id: Option<Snowflake>,
}

fn profile_image(
    id: Option<i64>,
    is_animated: i64,
) -> Result<Option<ProfileImage>, MessageStorageError> {
    id.map(|id| {
        Ok(ProfileImage {
            id: to_id(id)?,
            is_animated: is_animated != 0,
        })
    })
    .transpose()
}

/// Stores a new version and makes it the user's current one, in one transaction; answers the
/// version it replaces, whose files the caller deletes.
pub(crate) async fn replace_profile_image(
    pool: &SqlitePool,
    user_id: Snowflake,
    slot: ProfileImageSlot,
    new_image: Option<NewProfileImage>,
    now_unix_milliseconds: u64,
) -> Result<Option<StoredProfileImage>, MessageStorageError> {
    let user = to_integer(user_id.raw())?;
    let mut transaction = pool.begin_with(BEGIN_WRITE_TRANSACTION).await?;
    let (previous,): (Option<i64>,) = match slot {
        ProfileImageSlot::Avatar => sqlx::query_as("SELECT avatar_id FROM users WHERE id = ?"),
        ProfileImageSlot::Banner => sqlx::query_as("SELECT banner_id FROM users WHERE id = ?"),
    }
    .bind(user)
    .fetch_one(&mut *transaction)
    .await?;
    let (id, is_animated) = match &new_image {
        Some(image) => {
            sqlx::query(
                "INSERT INTO profile_images (id, user_id, still_id, size_bytes, \
                 created_at_unix_milliseconds) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(to_integer(image.id.raw())?)
            .bind(user)
            .bind(image.still_id.map(|id| to_integer(id.raw())).transpose()?)
            .bind(to_integer(image.size_bytes)?)
            .bind(to_integer(now_unix_milliseconds)?)
            .execute(&mut *transaction)
            .await?;
            (
                Some(to_integer(image.id.raw())?),
                i64::from(image.still_id.is_some()),
            )
        }
        None => (None, 0),
    };
    match slot {
        ProfileImageSlot::Avatar => {
            sqlx::query("UPDATE users SET avatar_id = ?, avatar_is_animated = ? WHERE id = ?")
        }
        ProfileImageSlot::Banner => {
            sqlx::query("UPDATE users SET banner_id = ?, banner_is_animated = ? WHERE id = ?")
        }
    }
    .bind(id)
    .bind(is_animated)
    .bind(user)
    .execute(&mut *transaction)
    .await?;
    let replaced = match previous {
        Some(previous) => {
            let row: Option<(i64, i64, Option<i64>)> = sqlx::query_as(
                "DELETE FROM profile_images WHERE id = ? RETURNING id, user_id, still_id",
            )
            .bind(previous)
            .fetch_optional(&mut *transaction)
            .await?;
            row.map(|(id, user_id, still_id)| {
                Ok::<_, MessageStorageError>(StoredProfileImage {
                    id: to_id(id)?,
                    user_id: to_id(user_id)?,
                    still_id: still_id.map(to_id).transpose()?,
                })
            })
            .transpose()?
        }
        None => None,
    };
    transaction.commit().await?;
    Ok(replaced)
}

pub(crate) async fn find_profile_image(
    pool: &SqlitePool,
    id: Snowflake,
) -> Result<Option<StoredProfileImage>, MessageStorageError> {
    let Some(id) = lookup_integer(id) else {
        return Ok(None);
    };
    let row: Option<(i64, i64, Option<i64>)> =
        sqlx::query_as("SELECT id, user_id, still_id FROM profile_images WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    row.map(|(id, user_id, still_id)| {
        Ok(StoredProfileImage {
            id: to_id(id)?,
            user_id: to_id(user_id)?,
            still_id: still_id.map(to_id).transpose()?,
        })
    })
    .transpose()
}

type ProfileRow = (i64, Option<i64>, i64, Option<i64>, i64);

/// The avatars and banners of these users, in one query whatever their number.
pub(crate) async fn profile_images_of_users(
    pool: &SqlitePool,
    user_ids: &[Snowflake],
) -> Result<HashMap<Snowflake, UserProfileImages>, MessageStorageError> {
    if user_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let ids: Vec<u64> = user_ids.iter().map(|id| id.raw()).collect();
    // The ids as a JSON array: one bound value, read by SQLite's json_each.
    let ids = serde_json::to_string(&ids).map_err(|_| MessageStorageError::IntegerOutOfRange)?;
    let rows: Vec<ProfileRow> = sqlx::query_as(
        "SELECT id, avatar_id, avatar_is_animated, banner_id, banner_is_animated FROM users \
         WHERE id IN (SELECT value FROM json_each(?)) \
         AND (avatar_id IS NOT NULL OR banner_id IS NOT NULL)",
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(
            |(id, avatar_id, avatar_is_animated, banner_id, banner_is_animated)| {
                Ok((
                    to_id(id)?,
                    UserProfileImages {
                        avatar: profile_image(avatar_id, avatar_is_animated)?,
                        banner: profile_image(banner_id, banner_is_animated)?,
                    },
                ))
            },
        )
        .collect()
}

/// Every file id of every avatar and banner, still frames included, for the orphan sweep.
pub(crate) async fn all_profile_image_file_ids(
    pool: &SqlitePool,
) -> Result<Vec<Snowflake>, MessageStorageError> {
    let rows: Vec<(i64, Option<i64>)> = sqlx::query_as("SELECT id, still_id FROM profile_images")
        .fetch_all(pool)
        .await?;
    let mut ids = Vec::with_capacity(rows.len() * 2);
    for (id, still_id) in rows {
        ids.push(to_id(id)?);
        if let Some(still_id) = still_id {
            ids.push(to_id(still_id)?);
        }
    }
    Ok(ids)
}
