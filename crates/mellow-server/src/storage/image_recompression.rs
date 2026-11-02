//! Pictures stored before ADR-0033 and their recompression (migration 0031).

use mellow_protocol::Snowflake;

use super::Database;

use crate::storage::guild_images::StoredGuildImage;
use crate::storage::guilds::BEGIN_WRITE_TRANSACTION;
use crate::storage::messages::MessageStorageError;
use crate::storage::profile_images::{NewProfileImage, ProfileImageSlot, StoredProfileImage};
use crate::storage::sqlite_integers::{convert_integer, to_snowflake, to_sqlite_integer};

/// How many pictures of one kind wait, and the bytes their main files take (still frames apart).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WaitingPictures {
    pub(crate) count: u64,
    pub(crate) stored_bytes: u64,
}

/// Every picture stored before ADR-0033 that the recompression would take, by kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PicturesToRecompress {
    pub(crate) attachments: WaitingPictures,
    pub(crate) profile_images: WaitingPictures,
    pub(crate) icons: WaitingPictures,
}

impl PicturesToRecompress {
    pub(crate) const fn count(&self) -> u64 {
        self.attachments.count + self.profile_images.count + self.icons.count
    }
}

async fn waiting(
    database: &Database,
    query: &'static str,
) -> Result<WaitingPictures, MessageStorageError> {
    let (count, stored_bytes): (i64, i64) =
        sqlx::query_as(query).fetch_one(database.readers()).await?;
    Ok(WaitingPictures {
        count: convert_integer(count)?,
        stored_bytes: convert_integer(stored_bytes)?,
    })
}

pub(crate) async fn pictures_to_recompress(
    database: &Database,
) -> Result<PicturesToRecompress, MessageStorageError> {
    Ok(PicturesToRecompress {
        attachments: waiting(
            database,
            "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM attachments \
             WHERE compression_version = 0",
        )
        .await?,
        profile_images: waiting(
            database,
            "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM profile_images \
             WHERE compression_version = 0",
        )
        .await?,
        icons: waiting(
            database,
            "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM guild_images \
             WHERE compression_version = 0 AND id NOT IN (SELECT id FROM guild_emojis)",
        )
        .await?,
    })
}

/// Marks the newest attachment stored before ADR-0033 as recompressed and answers it: the mark
/// comes before the work, so a picture that kills the process is not tried again.
pub(crate) async fn claim_next_attachment(
    database: &Database,
) -> Result<Option<Snowflake>, MessageStorageError> {
    let id: Option<i64> = sqlx::query_scalar(
        "UPDATE attachments SET compression_version = 1 WHERE id = \
         (SELECT id FROM attachments WHERE compression_version = 0 ORDER BY id DESC LIMIT 1) \
         RETURNING id",
    )
    .fetch_optional(database.writer())
    .await?;
    Ok(id.map(to_snowflake).transpose()?)
}

/// As `claim_next_attachment`, for avatars and banners.
pub(crate) async fn claim_next_profile_image(
    database: &Database,
) -> Result<Option<StoredProfileImage>, MessageStorageError> {
    let row: Option<(i64, i64, Option<i64>)> = sqlx::query_as(
        "UPDATE profile_images SET compression_version = 1 WHERE id = \
         (SELECT id FROM profile_images WHERE compression_version = 0 ORDER BY id DESC LIMIT 1) \
         RETURNING id, user_id, still_id",
    )
    .fetch_optional(database.writer())
    .await?;
    row.map(|(id, user_id, still_id)| {
        Ok(StoredProfileImage {
            id: to_snowflake(id)?,
            user_id: to_snowflake(user_id)?,
            still_id: still_id.map(to_snowflake).transpose()?,
        })
    })
    .transpose()
}

/// As `claim_next_attachment`, for guild and channel icons.
pub(crate) async fn claim_next_icon(
    database: &Database,
) -> Result<Option<StoredGuildImage>, MessageStorageError> {
    // Custom emoji are left out: an emoji's id is its picture's id, and messages and reactions
    // name it, so it cannot get a new one (ADR-0033, existing pictures).
    let row: Option<(i64, i64, Option<i64>, Option<i64>)> = sqlx::query_as(
        "UPDATE guild_images SET compression_version = 1 WHERE id = \
         (SELECT id FROM guild_images WHERE compression_version = 0 \
         AND id NOT IN (SELECT id FROM guild_emojis) ORDER BY id DESC LIMIT 1) \
         RETURNING id, guild_id, channel_id, still_id",
    )
    .fetch_optional(database.writer())
    .await?;
    row.map(|(id, guild_id, channel_id, still_id)| {
        Ok(StoredGuildImage {
            id: to_snowflake(id)?,
            guild_id: to_snowflake(guild_id)?,
            channel_id: channel_id.map(to_snowflake).transpose()?,
            still_id: still_id.map(to_snowflake).transpose()?,
        })
    })
    .transpose()
}

/// Puts `new_image` in the place of `previous` as the user's avatar or banner, in one transaction;
/// `false`, and nothing written, when `previous` is no longer in that place (replaced meanwhile).
pub(crate) async fn swap_profile_image(
    database: &Database,
    previous: &StoredProfileImage,
    slot: ProfileImageSlot,
    new_image: NewProfileImage,
    now_unix_milliseconds: u64,
) -> Result<bool, MessageStorageError> {
    let user = to_sqlite_integer(previous.user_id.raw())?;
    let previous_id = to_sqlite_integer(previous.id.raw())?;
    let new_id = to_sqlite_integer(new_image.id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    sqlx::query(
        "INSERT INTO profile_images (id, user_id, still_id, size_bytes, \
         created_at_unix_milliseconds) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(new_id)
    .bind(user)
    .bind(
        new_image
            .still_id
            .map(|id| to_sqlite_integer(id.raw()))
            .transpose()?,
    )
    .bind(to_sqlite_integer(new_image.size_bytes)?)
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .execute(&mut *transaction)
    .await?;
    let updated = match slot {
        ProfileImageSlot::Avatar => sqlx::query(
            "UPDATE users SET avatar_id = ?, avatar_is_animated = ? WHERE id = ? AND avatar_id = ?",
        ),
        ProfileImageSlot::Banner => sqlx::query(
            "UPDATE users SET banner_id = ?, banner_is_animated = ? WHERE id = ? AND banner_id = ?",
        ),
    }
    .bind(new_id)
    .bind(i64::from(new_image.still_id.is_some()))
    .bind(user)
    .bind(previous_id)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() == 0 {
        return Ok(false);
    }
    sqlx::query("DELETE FROM profile_images WHERE id = ?")
        .bind(previous_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(true)
}

/// Puts `new_image` in the place of `previous` as its guild's or channel's icon, in one
/// transaction; `false`, and nothing written, when `previous` is no longer that icon.
pub(crate) async fn swap_icon(
    database: &Database,
    previous: &StoredGuildImage,
    new_image: NewProfileImage,
    now_unix_milliseconds: u64,
) -> Result<bool, MessageStorageError> {
    let guild = to_sqlite_integer(previous.guild_id.raw())?;
    let channel = previous
        .channel_id
        .map(|id| to_sqlite_integer(id.raw()))
        .transpose()?;
    let previous_id = to_sqlite_integer(previous.id.raw())?;
    let new_id = to_sqlite_integer(new_image.id.raw())?;
    let mut transaction = database
        .writer()
        .begin_with(BEGIN_WRITE_TRANSACTION)
        .await?;
    sqlx::query(
        "INSERT INTO guild_images (id, guild_id, channel_id, still_id, size_bytes, \
         created_at_unix_milliseconds) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(new_id)
    .bind(guild)
    .bind(channel)
    .bind(
        new_image
            .still_id
            .map(|id| to_sqlite_integer(id.raw()))
            .transpose()?,
    )
    .bind(to_sqlite_integer(new_image.size_bytes)?)
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .execute(&mut *transaction)
    .await?;
    let updated =
        match channel {
            Some(channel) => {
                sqlx::query("UPDATE channels SET icon_id = ? WHERE id = ? AND icon_id = ?")
                    .bind(new_id)
                    .bind(channel)
                    .bind(previous_id)
                    .execute(&mut *transaction)
                    .await?
            }
            None => sqlx::query(
                "UPDATE guilds SET icon_id = ?, icon_is_animated = ? WHERE id = ? AND icon_id = ?",
            )
            .bind(new_id)
            .bind(i64::from(new_image.still_id.is_some()))
            .bind(guild)
            .bind(previous_id)
            .execute(&mut *transaction)
            .await?,
        };
    if updated.rows_affected() == 0 {
        return Ok(false);
    }
    sqlx::query("DELETE FROM guild_images WHERE id = ?")
        .bind(previous_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(true)
}

/// Asks the running server, or the next one to start, to recompress what waits.
pub(crate) async fn request_recompression(
    database: &Database,
    now_unix_milliseconds: u64,
) -> Result<(), MessageStorageError> {
    sqlx::query(
        "INSERT INTO image_recompression_requests (requested_at_unix_milliseconds) VALUES (?)",
    )
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .execute(database.writer())
    .await?;
    Ok(())
}

pub(crate) async fn is_recompression_requested(
    database: &Database,
) -> Result<bool, MessageStorageError> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM image_recompression_requests)")
            .fetch_one(database.readers())
            .await?,
    )
}

/// Once nothing waits any more.
pub(crate) async fn clear_recompression_requests(
    database: &Database,
) -> Result<(), MessageStorageError> {
    sqlx::query("DELETE FROM image_recompression_requests")
        .execute(database.writer())
        .await?;
    Ok(())
}
