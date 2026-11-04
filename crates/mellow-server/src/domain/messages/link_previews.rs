//! Link previews (ADR-0041): queued after a send or an edit, fetched in the background, shared
//! by every message linking the same address for a day, published with
//! `MESSAGE_LINK_PREVIEWS_UPDATE`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mellow_protocol::{GatewayEvent, LinkPreview, LinkPreviewImage, Snowflake};
use sha2::{Digest as _, Sha256};
use tokio::sync::{Semaphore, mpsc};
use url::Url;

use super::{MessageAudience, MessageError, Messages, publish};
use crate::domain::attachments::Attachments;
use crate::domain::link_previews::fetcher::{Fetched, LinkFetcher};
use crate::domain::link_previews::links::links_to_preview;
use crate::domain::link_previews::page::PageDetails;
use crate::storage::link_previews::{self as storage, NewLinkPreview, StoredLinkPreview};

/// Messages waiting for their previews; a full queue drops the newest, which simply get none.
const LINK_PREVIEW_QUEUE_CAPACITY: usize = 256;
/// Links fetched at once for the whole instance.
const CONCURRENT_LINK_PREVIEW_JOBS: usize = 2;
/// The most one message's previews may take, its three links and their pictures together.
const LINK_PREVIEW_JOB_BUDGET: std::time::Duration = std::time::Duration::from_secs(30);
/// A fetched preview is reused this long by every message linking the same address.
const PREVIEW_LIFETIME_MILLISECONDS: u64 = 24 * 60 * 60 * 1_000;
/// A link that gave nothing to show is not fetched again before this.
const EMPTY_PREVIEW_LIFETIME_MILLISECONDS: u64 = 60 * 60 * 1_000;

/// One message whose previews to fetch, as it was when queued.
pub(super) struct PreviewJob {
    guild_id: Option<Snowflake>,
    channel_id: Snowflake,
    message_id: Snowflake,
    /// Previews are recorded only if the message still says this (`set_message_previews`).
    content: String,
    links: Vec<Url>,
    audience: MessageAudience,
}

/// The queue between message writes and the background fetcher, off until the instance turns
/// previews on.
#[derive(Debug)]
pub(super) struct LinkPreviewQueue {
    is_enabled: AtomicBool,
    sender: mpsc::Sender<PreviewJob>,
    receiver: std::sync::Mutex<Option<mpsc::Receiver<PreviewJob>>>,
}

impl Default for LinkPreviewQueue {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel(LINK_PREVIEW_QUEUE_CAPACITY);
        Self {
            is_enabled: AtomicBool::new(false),
            sender,
            receiver: std::sync::Mutex::new(Some(receiver)),
        }
    }
}

impl std::fmt::Debug for PreviewJob {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the content or the links: they are message content (logs rule).
        formatter
            .debug_struct("PreviewJob")
            .field("message_id", &self.message_id)
            .finish_non_exhaustive()
    }
}

impl PreviewJob {
    /// The job for an edited message: always one, even without links, so previews of links the
    /// edit removed go away (a job fetching the old text may have recorded them while the edit
    /// was being written).
    pub(super) fn for_edit(
        guild_id: Option<Snowflake>,
        message: &mellow_protocol::Message,
        audience: MessageAudience,
    ) -> Self {
        Self {
            guild_id,
            channel_id: message.channel_id,
            message_id: message.id,
            content: message.content.clone(),
            links: links_to_preview(&message.content),
            audience,
        }
    }
}

/// What a new message needs to queue its previews once it is stored: its links, found before
/// the send lock so a long text never holds up other sends.
pub(super) struct LinkPreviewTicket {
    sender: mpsc::Sender<PreviewJob>,
    guild_id: Option<Snowflake>,
    links: Vec<Url>,
}

impl LinkPreviewTicket {
    /// Queues the new message's previews.
    pub(super) fn queue_for(self, message: &mellow_protocol::Message, audience: MessageAudience) {
        queue(
            Some(&self.sender),
            Some(PreviewJob {
                guild_id: self.guild_id,
                channel_id: message.channel_id,
                message_id: message.id,
                content: message.content.clone(),
                links: self.links,
                audience,
            }),
        );
    }
}

impl LinkPreviewQueue {
    /// For a message about to be sent with `content`; none without links, and none for a
    /// forward, which copies its original's words, not its previews (ADR-0041).
    pub(super) fn ticket(
        &self,
        guild_id: Option<Snowflake>,
        is_forward: bool,
        content: &str,
    ) -> Option<LinkPreviewTicket> {
        let sender = self.sender_if_enabled().filter(|_| !is_forward)?;
        let links = links_to_preview(content);
        (!links.is_empty()).then_some(LinkPreviewTicket {
            sender,
            guild_id,
            links,
        })
    }

    pub(super) fn sender_if_enabled(&self) -> Option<mpsc::Sender<PreviewJob>> {
        self.is_enabled
            .load(Ordering::Relaxed)
            .then(|| self.sender.clone())
    }
}

/// Queues the job without waiting; a full queue drops it.
pub(super) fn queue(sender: Option<&mpsc::Sender<PreviewJob>>, job: Option<PreviewJob>) {
    if let (Some(sender), Some(job)) = (sender, job)
        && sender.try_send(job).is_err()
    {
        tracing::debug!("link preview queue full: a message gets no preview");
    }
}

pub(crate) fn link_preview_from_stored(preview: StoredLinkPreview) -> LinkPreview {
    LinkPreview {
        id: preview.id,
        url: preview.url,
        title: preview.title,
        description: preview.description,
        site_name: preview.site_name,
        theme_color: preview.theme_color,
        image: preview.image.map(|image| LinkPreviewImage {
            width: image.width,
            height: image.height,
        }),
    }
}

impl Messages {
    /// Turns link previews on: messages sent or edited from now on queue theirs.
    pub(crate) fn enable_link_previews(&self) {
        self.link_previews.is_enabled.store(true, Ordering::Relaxed);
    }
}

/// One message's previews, read before a write whose event must carry them.
pub(super) async fn previews_of(
    database: &crate::storage::Database,
    message_id: Snowflake,
) -> Result<Vec<LinkPreview>, MessageError> {
    Ok(storage::previews_of_messages(database, &[message_id])
        .await?
        .remove(&message_id)
        .unwrap_or_default()
        .into_iter()
        .map(link_preview_from_stored)
        .collect())
}

/// Fills `link_previews` of these messages from storage, in one query.
pub(super) async fn fill_link_previews(
    database: &crate::storage::Database,
    messages: &mut [mellow_protocol::Message],
) -> Result<(), MessageError> {
    let ids: Vec<Snowflake> = messages.iter().map(|message| message.id).collect();
    let mut previews = storage::previews_of_messages(database, &ids).await?;
    for message in messages {
        if let Some(stored) = previews.remove(&message.id) {
            message.link_previews = stored.into_iter().map(link_preview_from_stored).collect();
        }
    }
    Ok(())
}

/// Runs until the process stops: takes queued messages, fetches their links a few at a time and
/// publishes their previews.
pub(crate) async fn fetch_link_previews_in_background(
    messages: Arc<Messages>,
    attachments: Arc<Attachments>,
    fetcher: LinkFetcher,
) {
    let receiver = messages
        .link_previews
        .receiver
        .lock()
        .ok()
        .and_then(|mut receiver| receiver.take());
    let Some(mut receiver) = receiver else {
        return;
    };
    let slots = Arc::new(Semaphore::new(CONCURRENT_LINK_PREVIEW_JOBS));
    // Dropped with this task when the server stops, which aborts the jobs still running before
    // the database closes.
    let mut jobs = tokio::task::JoinSet::new();
    while let Some(job) = receiver.recv().await {
        while jobs.try_join_next().is_some() {}
        let Ok(slot) = slots.clone().acquire_owned().await else {
            return;
        };
        let worker = PreviewWorker {
            messages: messages.clone(),
            attachments: attachments.clone(),
            fetcher: fetcher.clone(),
        };
        jobs.spawn(async move {
            let _slot = slot;
            // One job's links cannot keep a slot longer than this, whatever the sites do.
            match tokio::time::timeout(LINK_PREVIEW_JOB_BUDGET, worker.run(job)).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::warn!(%error, "cannot record link previews"),
                Err(_) => tracing::debug!("link preview job out of time"),
            }
        });
    }
}

/// Fetches every queued message's previews now, one after the other: tests wait for exactly
/// this work instead of a background task.
#[cfg(test)]
pub(crate) async fn fetch_queued_link_previews(
    messages: &Arc<Messages>,
    attachments: &Arc<Attachments>,
    fetcher: &LinkFetcher,
) {
    let worker = PreviewWorker {
        messages: messages.clone(),
        attachments: attachments.clone(),
        fetcher: fetcher.clone(),
    };
    loop {
        let job = {
            let Ok(mut receiver) = messages.link_previews.receiver.lock() else {
                return;
            };
            let Some(job) = receiver
                .as_mut()
                .and_then(|receiver| receiver.try_recv().ok())
            else {
                return;
            };
            job
        };
        worker.run(job).await.unwrap();
    }
}

/// A preview's id, random rather than a Snowflake: the cache is shared by every channel, and an id
/// carrying its fetch time would tell a member that someone elsewhere posted the same link, and
/// when (security audit of ADR-0041).
fn random_preview_id() -> Result<Snowflake, MessageError> {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes).map_err(|error| MessageError::Internal(Box::new(error)))?;
    // Positive and never 0, as SQLite integer keys and ids are.
    Ok(Snowflake::from_raw((u64::from_le_bytes(bytes) >> 1).max(1)))
}

struct PreviewWorker {
    messages: Arc<Messages>,
    attachments: Arc<Attachments>,
    fetcher: LinkFetcher,
}

impl PreviewWorker {
    async fn run(&self, job: PreviewJob) -> Result<(), MessageError> {
        let mut previews = Vec::with_capacity(job.links.len());
        for link in &job.links {
            let preview = self.preview_of(link).await?;
            if !preview.is_empty {
                previews.push(preview);
            }
        }
        // Counts only: links are message content (logs rule).
        tracing::debug!(
            links = job.links.len(),
            shown = previews.len(),
            "link previews ready"
        );
        let preview_ids: Vec<Snowflake> = previews.iter().map(|preview| preview.id).collect();
        let replaced = storage::set_message_previews(
            &self.messages.database,
            job.message_id,
            &job.content,
            &preview_ids,
        )
        .await?;
        // Published when there is something to show or something shown went away.
        if replaced.is_some_and(|removed| !previews.is_empty() || removed > 0) {
            publish(
                &self.messages.guilds,
                &job.audience,
                &GatewayEvent::MessageLinkPreviewsUpdate {
                    guild_id: job.guild_id,
                    channel_id: job.channel_id,
                    message_id: job.message_id,
                    link_previews: previews.into_iter().map(link_preview_from_stored).collect(),
                },
            );
        }
        Ok(())
    }

    /// The link's fresh preview, fetched and recorded when there is none.
    async fn preview_of(&self, link: &Url) -> Result<StoredLinkPreview, MessageError> {
        let url_sha256: [u8; 32] = Sha256::digest(link.as_str().as_bytes()).into();
        let now = crate::domain::clock::now_unix_milliseconds();
        if let Some(preview) =
            storage::find_fresh_preview(&self.messages.database, &url_sha256, now).await?
        {
            return Ok(preview);
        }
        let (details, picture) = match self.fetcher.fetch_page_or_picture(link.clone()).await {
            Ok(Fetched::Page(details)) => {
                let picture = match &details.image_url {
                    Some(image_url) => self.fetcher.fetch_picture(image_url.clone()).await.ok(),
                    None => None,
                };
                (details, picture)
            }
            Ok(Fetched::Picture(bytes)) => (PageDetails::default(), Some(bytes)),
            Err(_) => (PageDetails::default(), None),
        };
        let image = match picture {
            Some(bytes) => self.attachments.store_link_preview_picture(bytes).await?,
            None => None,
        };
        let is_empty = details.title.is_none() && details.description.is_none() && image.is_none();
        let fetched_at = crate::domain::clock::now_unix_milliseconds();
        let lifetime = if is_empty {
            EMPTY_PREVIEW_LIFETIME_MILLISECONDS
        } else {
            PREVIEW_LIFETIME_MILLISECONDS
        };
        let preview = StoredLinkPreview {
            id: random_preview_id()?,
            url: link.to_string(),
            title: details.title,
            description: details.description,
            site_name: details.site_name,
            theme_color: details.theme_color,
            image,
            is_empty,
        };
        storage::insert_preview(
            &self.messages.database,
            &NewLinkPreview {
                preview: preview.clone(),
                url_sha256,
                fetched_at_unix_milliseconds: fetched_at,
                expires_at_unix_milliseconds: fetched_at.saturating_add(lifetime),
            },
        )
        .await?;
        Ok(preview)
    }
}
