//! Link previews (ADR-0041): queued after a send or an edit, fetched in the background, shared
//! by every message linking the same address for a day, published with
//! `MESSAGE_LINK_PREVIEWS_UPDATE`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mellow_protocol::{GatewayEvent, LinkPreview, LinkPreviewImage, Snowflake};
use sha2::{Digest as _, Sha256};
use tokio::sync::{Semaphore, mpsc};
use url::Url;

use super::{
    MessageAudience, MessageChannel, MessageError, Messages, direct_message_storage, publish,
};
use crate::domain::attachments::{Attachments, LinkPreviewPicture};
use crate::domain::guilds::GuildError;
use crate::domain::link_previews::fetcher::{Fetched, LinkFetcher};
use crate::domain::link_previews::links::links_to_preview;
use crate::domain::link_previews::page::PageDetails;
use crate::domain::link_previews::providers::provider_preview;
use crate::storage::link_previews::{self as storage, NewLinkPreview, StoredLinkPreview};

/// Messages waiting for their previews; a full queue drops the newest, which simply get none.
const LINK_PREVIEW_QUEUE_CAPACITY: usize = 256;
/// Links fetched at once for the whole instance.
const CONCURRENT_LINK_PREVIEW_JOBS: usize = 2;
/// The most one message's previews may take, its three links and their pictures together: one
/// slow page (8 s per fetch) and a picture, so a member pasting slow links holds a slot for this
/// at most. The previews found before it runs out are kept.
const LINK_PREVIEW_JOB_BUDGET: std::time::Duration = std::time::Duration::from_secs(12);
/// Jobs one author may have queued or running at once: past it their new messages get no
/// preview, so one member pasting slow links cannot fill the queue in front of everyone else.
const MAXIMUM_LINK_PREVIEW_JOBS_PER_AUTHOR: usize = 4;
/// A fetched preview is reused this long by every message linking the same address.
const PREVIEW_LIFETIME_MILLISECONDS: u64 = 24 * 60 * 60 * 1_000;
/// A link that gave nothing to show is not fetched again before this.
const EMPTY_PREVIEW_LIFETIME_MILLISECONDS: u64 = 60 * 60 * 1_000;

/// One message whose previews to fetch, as it was when queued.
pub(super) struct PreviewJob {
    /// Who receives the previews is decided again from it just before publishing.
    channel: MessageChannel,
    message_id: Snowflake,
    /// Previews are recorded only if the message still says this (`add_message_previews`).
    content: String,
    links: Vec<Url>,
    /// Counts the job against its author until it ends, however it ends.
    _author_slot: AuthorJobSlot,
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

/// Queued and running jobs of each author with any; an author leaves the map with their last job.
#[derive(Debug, Default)]
struct JobsPerAuthor(Mutex<HashMap<Snowflake, usize>>);

impl JobsPerAuthor {
    fn counts(&self) -> MutexGuard<'_, HashMap<Snowflake, usize>> {
        // A count is updated in one step, so a panic elsewhere cannot leave it half written.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A slot for one more job of `author_id`, or `None` at the cap.
    fn take_slot(self: &Arc<Self>, author_id: Snowflake) -> Option<AuthorJobSlot> {
        let mut counts = self.counts();
        let count = counts.entry(author_id).or_default();
        if *count >= MAXIMUM_LINK_PREVIEW_JOBS_PER_AUTHOR {
            // No id: the count is enough to see the cap at work (logs rule).
            tracing::debug!(
                "link preview jobs of one author at the cap: a message gets no preview"
            );
            return None;
        }
        *count += 1;
        Some(AuthorJobSlot {
            jobs_per_author: self.clone(),
            author_id,
        })
    }
}

/// One job counted against its author, given back when dropped.
#[derive(Debug)]
struct AuthorJobSlot {
    jobs_per_author: Arc<JobsPerAuthor>,
    author_id: Snowflake,
}

impl Drop for AuthorJobSlot {
    fn drop(&mut self) {
        let mut counts = self.jobs_per_author.counts();
        if let Some(count) = counts.get_mut(&self.author_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                counts.remove(&self.author_id);
            }
        }
    }
}

/// The queue between message writes and the background fetcher, off until the instance turns
/// previews on.
#[derive(Debug)]
pub(super) struct LinkPreviewQueue {
    is_enabled: AtomicBool,
    sender: mpsc::Sender<PreviewJob>,
    receiver: Mutex<Option<mpsc::Receiver<PreviewJob>>>,
    jobs_per_author: Arc<JobsPerAuthor>,
}

impl Default for LinkPreviewQueue {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel(LINK_PREVIEW_QUEUE_CAPACITY);
        Self {
            is_enabled: AtomicBool::new(false),
            sender,
            receiver: Mutex::new(Some(receiver)),
            jobs_per_author: Arc::default(),
        }
    }
}

/// What a message needs to queue its previews once it is written: its links, found before the
/// send lock so a long text never holds up other sends, and its author's job slot.
pub(super) struct LinkPreviewTicket {
    sender: mpsc::Sender<PreviewJob>,
    channel: MessageChannel,
    links: Vec<Url>,
    author_slot: AuthorJobSlot,
}

impl LinkPreviewTicket {
    /// Queues the message's previews without waiting; a full queue drops them.
    pub(super) fn queue_for(self, message: &mellow_protocol::Message) {
        let job = PreviewJob {
            channel: self.channel,
            message_id: message.id,
            content: message.content.clone(),
            links: self.links,
            _author_slot: self.author_slot,
        };
        if self.sender.try_send(job).is_err() {
            tracing::debug!("link preview queue full: a message gets no preview");
        }
    }
}

impl LinkPreviewQueue {
    /// For a message about to be sent or edited to `content` by `author_id`; none without links,
    /// none past the author's job cap, and none for a forward, which copies its original's words,
    /// not its previews (ADR-0041).
    pub(super) fn ticket(
        &self,
        channel: &MessageChannel,
        author_id: Snowflake,
        is_forward: bool,
        content: &str,
    ) -> Option<LinkPreviewTicket> {
        if is_forward || !self.is_enabled.load(Ordering::Relaxed) {
            return None;
        }
        let links = links_to_preview(content);
        if links.is_empty() {
            return None;
        }
        Some(LinkPreviewTicket {
            sender: self.sender.clone(),
            channel: channel.clone(),
            links,
            author_slot: self.jobs_per_author.take_slot(author_id)?,
        })
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
        embed: preview.embed,
    }
}

impl Messages {
    /// Turns link previews on: messages sent or edited from now on queue theirs.
    pub(crate) fn enable_link_previews(&self) {
        self.link_previews.is_enabled.store(true, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn disable_link_previews(&self) {
        self.link_previews
            .is_enabled
            .store(false, Ordering::Relaxed);
    }

    /// Jobs waiting in the queue, for tests counting what a flood of links left in it.
    #[cfg(test)]
    pub(crate) fn queued_link_preview_jobs(&self) -> usize {
        LINK_PREVIEW_QUEUE_CAPACITY - self.link_previews.sender.capacity()
    }

    /// Fills the queue with jobs of authors who never send, as a busy instance would.
    #[cfg(test)]
    pub(crate) fn fill_link_preview_queue(&self) {
        for index in 0..LINK_PREVIEW_QUEUE_CAPACITY {
            // Ids no account or message has.
            let id = Snowflake::from_raw((1_u64 << 40) + u64::try_from(index).unwrap());
            let author_slot = self.link_previews.jobs_per_author.take_slot(id).unwrap();
            let job = PreviewJob {
                channel: MessageChannel::Direct {
                    id,
                    participant_ids: Vec::new(),
                    is_group: false,
                },
                message_id: id,
                content: String::new(),
                links: Vec::new(),
                _author_slot: author_slot,
            };
            self.link_previews.sender.try_send(job).unwrap();
        }
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
            let deadline = std::pin::pin!(tokio::time::sleep(LINK_PREVIEW_JOB_BUDGET));
            if let Err(error) = worker.run(job, deadline).await {
                tracing::warn!(%error, "cannot record link previews");
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
    let results =
        run_queued_link_preview_jobs(messages, attachments, fetcher, std::future::pending()).await;
    for result in results {
        result.unwrap();
    }
}

/// Runs every queued job now, one after the other, until `deadline`, which stands for the job
/// budget: a test ends it on an event rather than after some time. It ends the first job it
/// interrupts only; the jobs after it run to their end.
#[cfg(test)]
pub(crate) async fn run_queued_link_preview_jobs(
    messages: &Arc<Messages>,
    attachments: &Arc<Attachments>,
    fetcher: &LinkFetcher,
    deadline: impl Future<Output = ()>,
) -> Vec<Result<(), MessageError>> {
    use futures_util::FutureExt as _;

    let worker = PreviewWorker {
        messages: messages.clone(),
        attachments: attachments.clone(),
        fetcher: fetcher.clone(),
    };
    let mut deadline = std::pin::pin!(deadline.fuse());
    let mut results = Vec::new();
    loop {
        let job = {
            let Ok(mut receiver) = messages.link_previews.receiver.lock() else {
                return results;
            };
            let Some(job) = receiver
                .as_mut()
                .and_then(|receiver| receiver.try_recv().ok())
            else {
                return results;
            };
            job
        };
        results.push(worker.run(job, deadline.as_mut()).await);
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

/// Who can see the channel now: a member who lost access while the links were fetched does not
/// get the previews.
async fn audience_now(
    messages: &Messages,
    channel: &MessageChannel,
) -> Result<MessageAudience, MessageError> {
    match channel {
        MessageChannel::Guild(_) => messages.audience(channel).await,
        MessageChannel::Direct { id, .. } => Ok(MessageAudience::Participants(
            direct_message_storage::list_participant_ids(&messages.database, *id)
                .await
                .map_err(GuildError::from)?,
        )),
    }
}

struct PreviewWorker {
    messages: Arc<Messages>,
    attachments: Arc<Attachments>,
    fetcher: LinkFetcher,
}

impl PreviewWorker {
    /// Fetches the job's links in order until `deadline` or the first error, then records and
    /// publishes what it found by then; that error is returned afterwards.
    async fn run<D: Future<Output = ()>>(
        &self,
        job: PreviewJob,
        mut deadline: Pin<&mut D>,
    ) -> Result<(), MessageError> {
        let mut found = Vec::with_capacity(job.links.len());
        let mut link_error = None;
        for link in &job.links {
            tokio::select! {
                biased;
                preview = self.preview_of(link) => match preview {
                    Ok(preview) => found.push(preview),
                    Err(error) => {
                        link_error = Some(error);
                        break;
                    }
                },
                () = deadline.as_mut() => {
                    tracing::debug!("link preview job out of time: what it found is kept");
                    break;
                }
            }
        }
        let links: Vec<(&str, Option<Snowflake>)> = job
            .links
            .iter()
            .enumerate()
            .map(|(index, link)| {
                let shown = found.get(index).filter(|preview| !preview.is_empty);
                (link.as_str(), shown.map(|preview| preview.id))
            })
            .collect();
        // Counts only: links are message content (logs rule).
        tracing::debug!(
            links = links.len(),
            found = links.iter().filter(|(_, id)| id.is_some()).count(),
            "link previews ready"
        );
        let changed = storage::add_message_previews(
            &self.messages.database,
            job.message_id,
            &job.content,
            &links,
        )
        .await?;
        if changed == Some(true) {
            let link_previews = previews_of(&self.messages.database, job.message_id).await?;
            let audience = audience_now(&self.messages, &job.channel).await?;
            publish(
                &self.messages.guilds,
                &audience,
                &GatewayEvent::MessageLinkPreviewsUpdate {
                    guild_id: job.channel.guild_id(),
                    channel_id: job.channel.id(),
                    message_id: job.message_id,
                    link_previews,
                },
            );
        }
        link_error.map_or(Ok(()), Err)
    }

    /// The link's fresh preview, fetched and recorded when there is none; `link` is already in
    /// its canonical form (ADR-0044).
    async fn preview_of(&self, link: &Url) -> Result<StoredLinkPreview, MessageError> {
        let url_sha256: [u8; 32] = Sha256::digest(link.as_str().as_bytes()).into();
        let now = crate::domain::clock::now_unix_milliseconds();
        if let Some(preview) =
            storage::find_fresh_preview(&self.messages.database, &url_sha256, now).await?
        {
            return Ok(preview);
        }
        let (details, picture, embed) =
            if let Some(provider) = provider_preview(&self.fetcher, link).await {
                (provider.details, provider.picture, Some(provider.embed))
            } else {
                let (details, picture) = self.page_preview(link).await;
                (details, picture, None)
            };
        let picture = match picture {
            Some(bytes) => self.attachments.store_link_preview_picture(bytes).await?,
            None => LinkPreviewPicture::NotShown,
        };
        let is_picture_skipped = matches!(picture, LinkPreviewPicture::SlotBusy);
        let image = match picture {
            LinkPreviewPicture::Stored(image) => Some(image),
            LinkPreviewPicture::NotShown | LinkPreviewPicture::SlotBusy => None,
        };
        let is_empty = details.title.is_none() && details.description.is_none() && image.is_none();
        let fetched_at = crate::domain::clock::now_unix_milliseconds();
        // A card whose picture was skipped for a busy slot is fetched again soon, so one member
        // keeping the slot busy cannot leave everyone's cards without pictures for a day.
        let lifetime = if is_empty || is_picture_skipped {
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
            embed,
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

    /// What the link's page says and its picture's bytes, or the picture the link points at.
    async fn page_preview(&self, link: &Url) -> (PageDetails, Option<Vec<u8>>) {
        match self.fetcher.fetch_page_or_picture(link.clone()).await {
            Ok(Fetched::Page(details)) => {
                let picture = match &details.image_url {
                    Some(image_url) => self.fetcher.fetch_picture(image_url.clone()).await.ok(),
                    None => None,
                };
                (details, picture)
            }
            Ok(Fetched::Picture(bytes)) => (PageDetails::default(), Some(bytes)),
            Err(_) => (PageDetails::default(), None),
        }
    }
}
