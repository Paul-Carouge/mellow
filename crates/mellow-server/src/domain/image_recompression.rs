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
use crate::storage::image_recompression as storage;

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
    /// there was none, or nothing could be read. The request is dropped once none is left.
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
            Ok(true) => true,
            Ok(false) => {
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
    async fn recompress_next_waiting(&self) -> Result<bool, MessageError> {
        let outcome = if let Some(image) = storage::claim_next_profile_image(&self.database).await?
        {
            self.profile_images.recompress(image).await
        } else if let Some(icon) = storage::claim_next_icon(&self.database).await? {
            self.guilds
                .recompress_icon(&self.profile_images, icon)
                .await
        } else if let Some(attachment_id) = storage::claim_next_attachment(&self.database).await? {
            // The same job as a new upload's: smaller stored form and a preview.
            self.attachments.optimize(attachment_id).await
        } else {
            return Ok(false);
        };
        // Claimed before the work: a failed picture keeps its file and is not tried again.
        if let Err(error) = outcome {
            tracing::warn!(%error, "cannot recompress a picture");
        }
        Ok(true)
    }
}
