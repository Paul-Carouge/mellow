//! Pictures stored before ADR-0033, recompressed once an administrator asked for it
//! (`mellow-server admin recompress-images --confirm`): one at a time, by the attachment worker
//! whenever it has nothing else to do, so new uploads come first and at most one large picture is
//! ever processed.

use std::sync::Arc;

use crate::domain::attachments::Attachments;
use crate::domain::guilds::Guilds;
use crate::domain::messages::MessageError;
use crate::domain::profile_images::ProfileImages;
use crate::storage::Database;
use crate::storage::image_recompression::{self as storage, ClaimedPictureKind};

#[derive(Debug, Clone)]
pub(crate) struct ImageRecompression {
    database: Database,
    attachments: Arc<Attachments>,
    profile_images: Arc<ProfileImages>,
    guilds: Arc<Guilds>,
}

impl ImageRecompression {
    pub(crate) const fn new(
        database: Database,
        attachments: Arc<Attachments>,
        profile_images: Arc<ProfileImages>,
        guilds: Arc<Guilds>,
    ) -> Self {
        Self {
            database,
            attachments,
            profile_images,
            guilds,
        }
    }

    /// Recompresses the next picture waiting when an administrator asked for it; `false` when
    /// there was none, nothing could be read, or the instance was too busy (the picture is tried
    /// again at the worker's next look). The request is dropped once none is left.
    pub(crate) async fn recompress_next(&self) -> bool {
        match storage::is_recompression_requested(&self.database).await {
            Ok(true) => {}
            Ok(false) => return false,
            Err(error) => {
                tracing::warn!(%error, "cannot read whether pictures are to be recompressed");
                return false;
            }
        }
        match self.recompress_next_waiting().await {
            Ok(Step::Tried) => true,
            Ok(Step::Postponed) => false,
            Ok(Step::NoneLeft) => {
                match storage::clear_recompression_requests(&self.database).await {
                    Ok(()) => {
                        tracing::info!("every picture stored before ADR-0033 was recompressed");
                    }
                    Err(error) => tracing::warn!(%error, "cannot end the picture recompression"),
                }
                false
            }
            Err(error) => {
                tracing::warn!(%error, "cannot pick the next picture to recompress");
                false
            }
        }
    }

    /// Avatars, banners and icons first: they are small and every member sees them.
    async fn recompress_next_waiting(&self) -> Result<Step, MessageError> {
        let (kind, id, outcome) =
            if let Some(image) = storage::claim_next_profile_image(&self.database).await? {
                let id = image.id;
                (
                    ClaimedPictureKind::ProfileImage,
                    id,
                    self.profile_images.recompress(image).await,
                )
            } else if let Some(icon) = storage::claim_next_icon(&self.database).await? {
                let id = icon.id;
                (
                    ClaimedPictureKind::Icon,
                    id,
                    self.guilds
                        .recompress_icon(&self.profile_images, icon)
                        .await,
                )
            } else if let Some(id) = storage::claim_next_attachment(&self.database).await? {
                // The same job as a new upload's: smaller stored form and a preview.
                (
                    ClaimedPictureKind::Attachment,
                    id,
                    self.attachments.optimize(id).await,
                )
            } else {
                return Ok(Step::NoneLeft);
            };
        match outcome {
            Ok(()) => Ok(Step::Tried),
            Err(error) if is_transient(&error) => {
                tracing::info!(%error, "picture recompression postponed: the instance is busy");
                if let Err(error) = storage::release_claim(&self.database, kind, id).await {
                    tracing::warn!(%error, "cannot put a postponed picture back in the queue");
                }
                Ok(Step::Postponed)
            }
            // Claimed before the work: a picture that fails on its own keeps its file and is not
            // tried again.
            Err(error) => {
                tracing::warn!(%error, "cannot recompress a picture");
                Ok(Step::Tried)
            }
        }
    }
}

/// What one look at the queue did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Tried,
    /// The instance was too busy: the picture waits again.
    Postponed,
    NoneLeft,
}

/// A failure caused by the load of the moment, not by the picture.
fn is_transient(error: &MessageError) -> bool {
    matches!(error, MessageError::Busy) || storage::is_database_contention(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage_error(error: sqlx::Error) -> MessageError {
        crate::storage::messages::MessageStorageError::from(error).into()
    }

    #[tokio::test]
    async fn a_database_held_by_another_connection_postpones_a_picture() {
        use sqlx::{ConnectOptions as _, Connection as _};
        let directory = tempfile::tempdir().unwrap();
        // No wait: the lock is reported at once.
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(directory.path().join("locked.db"))
            .create_if_missing(true)
            .busy_timeout(std::time::Duration::ZERO);
        let mut holder = options.connect().await.unwrap();
        sqlx::raw_sql("CREATE TABLE pictures (id INTEGER); BEGIN IMMEDIATE;")
            .execute(&mut holder)
            .await
            .unwrap();
        let mut writer = options.connect().await.unwrap();
        let locked = sqlx::query("INSERT INTO pictures VALUES (1)")
            .execute(&mut writer)
            .await
            .unwrap_err();
        writer.close().await.unwrap();
        holder.close().await.unwrap();

        assert!(is_transient(&storage_error(locked)));
    }

    #[test]
    fn only_the_load_of_the_moment_postpones_a_picture() {
        assert!(is_transient(&MessageError::Busy));
        assert!(is_transient(&storage_error(sqlx::Error::PoolTimedOut)));
        assert!(!is_transient(&storage_error(sqlx::Error::RowNotFound)));
        assert!(!is_transient(&MessageError::MessageNotFound));
        assert!(!is_transient(&MessageError::Internal(Box::new(
            std::io::Error::other("cannot decode")
        ))));
    }
}
