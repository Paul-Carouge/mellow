//! Richer profiles (ADR-0050): an "about me" text and up to five links, each shown with its
//! site's icon. The server fetches each site's icon once per host, in the background, through
//! the link preview fetcher's guards, and stores it through the picture pipeline; browsers only
//! ever load it from the instance.

mod site_icons;
#[cfg(test)]
pub(crate) use site_icons::tests as site_icons_tests;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mellow_protocol::{ProfileLink, Snowflake, UpdateUserProfileRequest, UserProfile};
use tokio::sync::{OwnedSemaphorePermit, mpsc};

use crate::domain::clock::now_unix_milliseconds;
use crate::domain::image_processing::{IconKind, ImageProcessingError, process_icon};
use crate::domain::link_previews::fetcher::LinkFetcher;
use crate::domain::messages::MessageError;
use crate::domain::profile_images::ProfileImages;
use crate::domain::profile_rules::{ProfileRuleViolation, normalize_about_me, normalize_links};
use crate::storage::Database;
use crate::storage::messages::MessageStorageError;
use crate::storage::user_profiles::{self as storage, IconRecord, StoredLink, StoredProfile};

/// A site's icon is fetched again this long after it was.
const LINK_ICON_LIFETIME_MILLISECONDS: u64 = 7 * 24 * 60 * 60 * 1_000;
/// A site that did not answer is asked again after this.
const LINK_ICON_RETRY_MILLISECONDS: u64 = 60 * 60 * 1_000;
/// Hosts waiting for their icon; past it a host is simply asked for on a later profile read.
const LINK_ICON_QUEUE_CAPACITY: usize = 64;
/// A processed site icon larger than this is not kept: it is shown at 16 px.
const MAXIMUM_LINK_ICON_STORED_BYTES: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ProfileError {
    #[error(transparent)]
    Rule(#[from] ProfileRuleViolation),
    /// An unknown user, or one the reader shares nothing with.
    #[error("profile not found")]
    NotFound,
    #[error(transparent)]
    Message(#[from] MessageError),
}

impl From<MessageStorageError> for ProfileError {
    fn from(error: MessageStorageError) -> Self {
        Self::Message(error.into())
    }
}

/// Hosts queued or being fetched, each once.
#[derive(Debug)]
struct LinkIconJobs {
    is_enabled: AtomicBool,
    sender: mpsc::Sender<String>,
    receiver: Mutex<Option<mpsc::Receiver<String>>>,
    hosts: Mutex<HashSet<String>>,
}

impl LinkIconJobs {
    fn hosts(&self) -> MutexGuard<'_, HashSet<String>> {
        // Each change is one insert or removal: a panic elsewhere cannot leave it half written.
        self.hosts.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Debug)]
pub(crate) struct Profiles {
    database: Database,
    images: Arc<ProfileImages>,
    link_icons: LinkIconJobs,
}

impl Profiles {
    pub(crate) fn new(database: Database, images: Arc<ProfileImages>) -> Self {
        let (sender, receiver) = mpsc::channel(LINK_ICON_QUEUE_CAPACITY);
        Self {
            database,
            images,
            link_icons: LinkIconJobs {
                is_enabled: AtomicBool::new(false),
                sender,
                receiver: Mutex::new(Some(receiver)),
                hosts: Mutex::default(),
            },
        }
    }

    /// Turns site icons on: `serve` calls it when link previews are on, since both fetch from
    /// the sites members name. Off, links show without icons.
    pub(crate) fn enable_link_icons(&self) {
        self.link_icons.is_enabled.store(true, Ordering::Relaxed);
    }

    /// `user_id`'s profile, for themselves or whoever shares a server, a conversation or a
    /// relationship with them; `NotFound` for anyone else. Icons missing or older than a week are
    /// queued for a fetch.
    pub(crate) async fn profile(
        &self,
        viewer_id: Snowflake,
        user_id: Snowflake,
    ) -> Result<UserProfile, ProfileError> {
        if !storage::may_view_profile(&self.database, viewer_id, user_id).await? {
            return Err(ProfileError::NotFound);
        }
        let stored = storage::profile_of_user(&self.database, user_id).await?;
        self.queue_stale_icons(&stored.links, now_unix_milliseconds());
        Ok(user_profile(user_id, stored))
    }

    /// Replaces the user's text and links. Sites no profile links any more lose their icon.
    pub(crate) async fn update(
        &self,
        user_id: Snowflake,
        request: &UpdateUserProfileRequest,
    ) -> Result<UserProfile, ProfileError> {
        let about_me = normalize_about_me(&request.about_me)?;
        let links = normalize_links(&request.links)?;
        let orphaned_files =
            storage::replace_profile(&self.database, user_id, &about_me, &links).await?;
        for file_id in orphaned_files {
            self.images.delete_files(file_id, None).await;
        }
        let stored = storage::profile_of_user(&self.database, user_id).await?;
        self.queue_stale_icons(&stored.links, now_unix_milliseconds());
        Ok(user_profile(user_id, stored))
    }

    /// A site icon's bytes, with the read slot the response keeps until sent; any signed-in user
    /// may read one, as it is the site's public icon.
    pub(crate) async fn link_icon(
        &self,
        file_id: Snowflake,
    ) -> Result<(Vec<u8>, OwnedSemaphorePermit), ProfileError> {
        if !storage::is_link_icon_file(&self.database, file_id).await? {
            return Err(ProfileError::NotFound);
        }
        Ok(self.images.read_file(file_id).await?)
    }

    fn queue_stale_icons(&self, links: &[StoredLink], now_unix_milliseconds: u64) {
        for link in links {
            let is_stale = link
                .icon_refresh_after_unix_milliseconds
                .is_none_or(|refresh_after| refresh_after <= now_unix_milliseconds);
            if is_stale {
                self.queue_link_icon(&link.host);
            }
        }
    }

    /// Queues `host` unless it is already queued or fetched, or the queue is full.
    fn queue_link_icon(&self, host: &str) {
        if !self.link_icons.is_enabled.load(Ordering::Relaxed) {
            return;
        }
        let mut hosts = self.link_icons.hosts();
        if hosts.contains(host) {
            return;
        }
        if self.link_icons.sender.try_send(host.to_owned()).is_ok() {
            hosts.insert(host.to_owned());
        }
    }

    /// Fetches `host`'s icon and records what was found.
    async fn refresh_link_icon(
        &self,
        fetcher: &LinkFetcher,
        host: &str,
    ) -> Result<(), ProfileError> {
        let Ok(bytes) = site_icons::fetch_site_icon(fetcher, host).await else {
            storage::postpone_link_icon_refresh(
                &self.database,
                host,
                now_unix_milliseconds() + LINK_ICON_RETRY_MILLISECONDS,
            )
            .await?;
            return Ok(());
        };
        let file = match bytes {
            Some(bytes) => self.store_link_icon(bytes).await?,
            None => None,
        };
        let recorded = storage::record_link_icon(
            &self.database,
            host,
            file,
            now_unix_milliseconds() + LINK_ICON_LIFETIME_MILLISECONDS,
        )
        .await;
        let unused_file = match recorded {
            Ok(IconRecord::Stored { replaced_file_id }) => replaced_file_id,
            Ok(IconRecord::HostUnlinked) => file.map(|(file_id, _)| file_id),
            Err(error) => {
                if let Some((file_id, _)) = file {
                    self.images.delete_files(file_id, None).await;
                }
                return Err(error.into());
            }
        };
        if let Some(file_id) = unused_file {
            self.images.delete_files(file_id, None).await;
        }
        Ok(())
    }

    /// Processes a fetched icon and writes it under a random id; `None` when it is not a picture
    /// the pipeline reads or comes out too large.
    async fn store_link_icon(
        &self,
        bytes: Vec<u8>,
    ) -> Result<Option<(Snowflake, u64)>, ProfileError> {
        let permit = self.images.wait_for_image_processing_permit().await?;
        let processed = tokio::task::spawn_blocking(move || {
            // Held until the work really ends.
            let _permit = permit;
            process_icon(&bytes, IconKind::LinkIcon, None)
        })
        .await
        .map_err(|error| MessageError::Internal(Box::new(error)))?;
        let picture = match processed {
            Ok(processed) => processed.picture.bytes,
            Err(ImageProcessingError::Unsupported | ImageProcessingError::TooLarge) => {
                return Ok(None);
            }
            Err(error @ ImageProcessingError::Encoding) => {
                return Err(MessageError::Internal(Box::new(error)).into());
            }
        };
        if picture.len() > MAXIMUM_LINK_ICON_STORED_BYTES {
            return Ok(None);
        }
        let size_bytes = u64::try_from(picture.len()).unwrap_or(u64::MAX);
        let file_id = random_file_id()?;
        self.images.write_file(file_id, picture).await?;
        Ok(Some((file_id, size_bytes)))
    }
}

/// A site icon's id, random rather than a Snowflake: icons are shared by every profile linking
/// the host, and an id carrying its fetch time would tell when someone first linked the site.
fn random_file_id() -> Result<Snowflake, MessageError> {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes).map_err(|error| MessageError::Internal(Box::new(error)))?;
    // Positive and never 0, as SQLite integer keys and ids are.
    Ok(Snowflake::from_raw((u64::from_le_bytes(bytes) >> 1).max(1)))
}

fn user_profile(user_id: Snowflake, stored: StoredProfile) -> UserProfile {
    UserProfile {
        user_id,
        about_me: stored.about_me,
        links: stored
            .links
            .into_iter()
            .map(|link| ProfileLink {
                url: link.url,
                label: link.label,
                icon_id: link.icon_file_id,
            })
            .collect(),
    }
}

/// Runs until the process stops: fetches queued hosts' icons one at a time.
pub(crate) async fn fetch_link_icons_in_background(profiles: Arc<Profiles>, fetcher: LinkFetcher) {
    let receiver = profiles
        .link_icons
        .receiver
        .lock()
        .ok()
        .and_then(|mut receiver| receiver.take());
    let Some(mut receiver) = receiver else {
        return;
    };
    while let Some(host) = receiver.recv().await {
        if let Err(error) = profiles.refresh_link_icon(&fetcher, &host).await {
            // Never the host: it is part of what a member wrote (logs rule).
            tracing::warn!(%error, "cannot record a profile link's site icon");
        }
        profiles.link_icons.hosts().remove(&host);
    }
}

/// Fetches every queued host's icon now, one after the other: tests wait for exactly this work.
#[cfg(test)]
pub(crate) async fn fetch_queued_link_icons(profiles: &Profiles, fetcher: &LinkFetcher) {
    loop {
        let host = {
            let mut receiver = profiles.link_icons.receiver.lock().unwrap();
            match receiver
                .as_mut()
                .and_then(|receiver| receiver.try_recv().ok())
            {
                Some(host) => host,
                None => return,
            }
        };
        profiles.refresh_link_icon(fetcher, &host).await.unwrap();
        profiles.link_icons.hosts().remove(&host);
    }
}
