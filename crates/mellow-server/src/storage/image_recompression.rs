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

async fn count_waiting_pictures_and_their_bytes(
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
        attachments: count_waiting_pictures_and_their_bytes(
            database,
            "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM attachments \
             WHERE compression_version = 0",
        )
        .await?,
        profile_images: count_waiting_pictures_and_their_bytes(
            database,
            "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM profile_images \
             WHERE compression_version = 0",
        )
        .await?,
        icons: count_waiting_pictures_and_their_bytes(
            database,
            "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM guild_images \
             WHERE compression_version = 0 AND id NOT IN (SELECT id FROM guild_emojis)",
        )
        .await?,
    })
}

/// An attachment claimed for recompression, with the file it held when claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClaimedAttachment {
    pub(crate) id: Snowflake,
    pub(crate) file_id: Snowflake,
}

/// Marks the newest attachment stored before ADR-0033 as recompressed and answers it: the mark
/// comes before the work, so a picture that kills the process is not tried again.
pub(crate) async fn claim_next_attachment(
    database: &Database,
) -> Result<Option<ClaimedAttachment>, MessageStorageError> {
    let row: Option<(i64, i64)> = sqlx::query_as(
        "UPDATE attachments SET compression_version = 1 WHERE id = \
         (SELECT id FROM attachments WHERE compression_version = 0 ORDER BY id DESC LIMIT 1) \
         RETURNING id, COALESCE(file_id, id)",
    )
    .fetch_optional(database.writer())
    .await?;
    row.map(|(id, file_id)| {
        Ok(ClaimedAttachment {
            id: to_snowflake(id)?,
            file_id: to_snowflake(file_id)?,
        })
    })
    .transpose()
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

/// The table a claimed picture is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClaimedPictureKind {
    /// With the file it held when claimed: a forwarded copy holds its original's (ADR-0039).
    Attachment {
        claimed_file_id: Snowflake,
    },
    ProfileImage,
    Icon,
}

/// Puts a claimed picture back in the queue after a failure that says nothing about the picture.
/// A picture already swapped for its new version stays done: the avatar or icon row is gone, and
/// an attachment points at its new file.
pub(crate) async fn release_claim(
    database: &Database,
    kind: ClaimedPictureKind,
    id: Snowflake,
) -> Result<(), MessageStorageError> {
    let query = match kind {
        ClaimedPictureKind::Attachment { claimed_file_id } => {
            sqlx::query(
                "UPDATE attachments SET compression_version = 0 \
                 WHERE id = ? AND compression_version = 1 AND COALESCE(file_id, id) = ?",
            )
            .bind(to_sqlite_integer(id.raw())?)
            .bind(to_sqlite_integer(claimed_file_id.raw())?)
            .execute(database.writer())
            .await?;
            return Ok(());
        }
        ClaimedPictureKind::ProfileImage => {
            "UPDATE profile_images SET compression_version = 0 \
             WHERE id = ? AND compression_version = 1"
        }
        ClaimedPictureKind::Icon => {
            "UPDATE guild_images SET compression_version = 0 WHERE id = ? AND compression_version = 1"
        }
    };
    sqlx::query(query)
        .bind(to_sqlite_integer(id.raw())?)
        .execute(database.writer())
        .await?;
    Ok(())
}

/// SQLite's primary result codes for a database another connection holds.
const SQLITE_BUSY: i64 = 5;
const SQLITE_LOCKED: i64 = 6;

/// Whether `error` comes from a database too busy to answer in time (the writer taken past its
/// wait, a lock held): a later try may well succeed.
pub(crate) fn is_database_contention(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(error) = error.downcast_ref::<sqlx::Error>() {
            return match error {
                sqlx::Error::PoolTimedOut => true,
                sqlx::Error::Database(error) => error
                    .code()
                    .and_then(|code| code.parse::<i64>().ok())
                    // Extended result codes keep the primary one in their low byte.
                    .is_some_and(|code| matches!(code & 0xff, SQLITE_BUSY | SQLITE_LOCKED)),
                _ => false,
            };
        }
        current = error.source();
    }
    false
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_released_claim_waits_again_unless_the_attachment_already_has_its_new_file() {
        let (_directory, database) = crate::domain::accounts::tests::migrated_database().await;
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (20, 10, 'text', 'general', 0); \
             INSERT INTO messages (id, channel_id, author_id, content) VALUES (30, 20, 1, 'files'); \
             INSERT INTO attachments (id, channel_id, uploader_id, message_id, file_name, \
             content_type, size_bytes, created_at_unix_milliseconds, compression_version) \
             VALUES (40, 20, 1, 30, 'a.png', 'image/png', 300, 0, 0), \
             (41, 20, 1, 30, 'b.png', 'image/png', 300, 0, 0); \
             INSERT INTO profile_images (id, user_id, size_bytes, created_at_unix_milliseconds, \
             compression_version) VALUES (50, 1, 700, 0, 0);",
        )
        .execute(database.writer())
        .await
        .unwrap();

        let image = claim_next_profile_image(&database).await.unwrap().unwrap();
        let newest = claim_next_attachment(&database).await.unwrap().unwrap();
        let oldest = claim_next_attachment(&database).await.unwrap().unwrap();
        // The newest one was swapped for its new file before the failure.
        sqlx::raw_sql("UPDATE attachments SET file_id = 99 WHERE id = 41")
            .execute(database.writer())
            .await
            .unwrap();
        let waiting_while_claimed = pictures_to_recompress(&database).await.unwrap();
        release_claim(&database, ClaimedPictureKind::ProfileImage, image.id)
            .await
            .unwrap();
        for claimed in [newest, oldest] {
            let kind = ClaimedPictureKind::Attachment {
                claimed_file_id: claimed.file_id,
            };
            release_claim(&database, kind, claimed.id).await.unwrap();
        }
        let waiting_after_release = pictures_to_recompress(&database).await.unwrap();
        database.close().await;

        assert_eq!(waiting_while_claimed.count(), 0);
        assert_eq!(waiting_after_release.profile_images.count, 1);
        assert_eq!(
            waiting_after_release.attachments,
            WaitingPictures {
                count: 1,
                stored_bytes: 300
            },
            "only the attachment still on its old file"
        );
    }

    #[tokio::test]
    async fn a_copy_left_by_a_deleted_original_takes_its_place_in_both_queues_and_the_quota() {
        let (_directory, database) = crate::domain::accounts::tests::migrated_database().await;
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0), \
             (2, 'bob', 'Bob', 'bob@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (20, 10, 'text', 'general', 0); \
             INSERT INTO messages (id, channel_id, author_id, content) \
             VALUES (30, 20, 1, ''), (31, 20, 2, ''); \
             INSERT INTO attachments (id, channel_id, uploader_id, message_id, file_name, \
             content_type, size_bytes, created_at_unix_milliseconds, compression_version, \
             optimization_attempted_at_unix_milliseconds, file_id, is_forwarded_copy) \
             VALUES (40, 20, 1, 30, 'a.png', 'image/png', 300, 0, 0, NULL, NULL, 0), \
             (41, 20, 2, 31, 'a.png', 'image/png', 300, 0, 1, 0, 40, 1); \
             DELETE FROM messages WHERE id = 30;",
        )
        .execute(database.writer())
        .await
        .unwrap();

        let awaiting_optimization =
            crate::storage::attachments::newest_attachments_awaiting_optimization(&database, 10)
                .await
                .unwrap();
        let bob_bytes = crate::storage::attachments::stored_bytes_of_uploader(
            &database,
            Snowflake::from_raw(2),
        )
        .await
        .unwrap();
        let claimed = claim_next_attachment(&database).await.unwrap().unwrap();
        let kind = ClaimedPictureKind::Attachment {
            claimed_file_id: claimed.file_id,
        };
        release_claim(&database, kind, claimed.id).await.unwrap();
        let waiting_after_release = pictures_to_recompress(&database).await.unwrap();
        database.close().await;

        let copy = Snowflake::from_raw(41);
        assert_eq!(awaiting_optimization, [copy], "never tried: queued again");
        assert_eq!(bob_bytes, 300, "the copy now counts in its author's quota");
        assert_eq!(
            claimed,
            ClaimedAttachment {
                id: copy,
                file_id: Snowflake::from_raw(40)
            }
        );
        assert_eq!(waiting_after_release.attachments.count, 1, "released");
    }
}
