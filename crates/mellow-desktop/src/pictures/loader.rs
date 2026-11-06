//! Picture bytes from the instance, on the network runtime: one authenticated client, one request
//! per file at a time however many views ask, within the server's download budget, retried once
//! like the web client's `image-retry.ts`, and cached by kind (disk or memory, see `disk_cache`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use mellow_client_core::{ClientError, Download, DownloadableFile, RestClient};
use tokio::sync::oneshot;

use super::PictureLoadError;
use super::disk_cache::{DiskCache, disk_file_name};
use super::limiter::{BUDGET_WINDOW, CONCURRENT_DOWNLOADS, DOWNLOADS_PER_WINDOW, DownloadLimiter};
use super::lru::ByteBoundedLru;

/// The web client's `IMAGE_RETRY_DELAY_MILLISECONDS`: long enough for a busy instance to free a
/// download slot, short enough to go unnoticed.
const RETRY_DELAY: Duration = Duration::from_secs(2);
/// Stored avatars, banners, icons and emoji are a few hundred kilobytes at most (ADR-0033).
const MAXIMUM_PROFILE_PICTURE_BYTES: usize = 16 * 1024 * 1024;
/// Attachments over 16 MP keep their uploaded file (ADR-0033), which can be a large PNG.
const MAXIMUM_ATTACHMENT_PICTURE_BYTES: usize = 128 * 1024 * 1024;
/// Conversation pictures kept in memory, as files, so a revalidation answered `304` costs no
/// transfer. Decoded pictures have their own cache in the store.
pub(crate) const FILES_IN_MEMORY_CAPACITY_BYTES: usize = 64 * 1024 * 1024;

pub(crate) type PictureBytes = Arc<[u8]>;
type FetchResult = Result<PictureBytes, PictureLoadError>;

#[derive(Debug, Clone)]
struct FileInMemory {
    bytes: PictureBytes,
    entity_tag: Option<String>,
}

fn maximum_bytes(file: DownloadableFile) -> usize {
    match file {
        DownloadableFile::ProfileImage(_)
        | DownloadableFile::ProfileImageStill(_)
        | DownloadableFile::Icon(_)
        | DownloadableFile::IconStill(_)
        | DownloadableFile::LinkIcon(_) => MAXIMUM_PROFILE_PICTURE_BYTES,
        DownloadableFile::Attachment(_)
        | DownloadableFile::AttachmentPreview(_)
        | DownloadableFile::LinkPreviewImage { .. }
        | DownloadableFile::Sound(_) => MAXIMUM_ATTACHMENT_PICTURE_BYTES,
    }
}

impl From<ClientError> for PictureLoadError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::FileTooLarge => Self::TooLarge,
            error if error.is_worth_retrying() => Self::Unavailable,
            _ => Self::Refused,
        }
    }
}

#[derive(Debug)]
pub(crate) struct PictureLoader {
    rest: RestClient,
    limiter: DownloadLimiter,
    disk_cache: Option<Arc<DiskCache>>,
    files_in_memory: Mutex<ByteBoundedLru<DownloadableFile, FileInMemory>>,
    /// Files being downloaded, with the callers waiting for the first caller's answer.
    in_flight: Mutex<HashMap<DownloadableFile, Vec<oneshot::Sender<FetchResult>>>>,
    retry_delay: Duration,
}

/// Lock a map that holds no invariant a panicking holder could break halfway.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Ends a file's in-flight entry even when the download future is dropped: waiters then get
/// `Unavailable` from their closed channel instead of waiting forever.
struct InFlightEntry<'a> {
    loader: &'a PictureLoader,
    file: DownloadableFile,
}

impl InFlightEntry<'_> {
    fn finish(self) -> Vec<oneshot::Sender<FetchResult>> {
        let waiters = lock(&self.loader.in_flight)
            .remove(&self.file)
            .unwrap_or_default();
        // `Drop` must not run: by then another download of the file may own the entry.
        std::mem::forget(self);
        waiters
    }
}

impl Drop for InFlightEntry<'_> {
    fn drop(&mut self) {
        lock(&self.loader.in_flight).remove(&self.file);
    }
}

impl PictureLoader {
    /// `rest` carries the session's token; `disk_cache` is `None` when the user has no cache
    /// directory, and profile pictures then stay in memory like the rest.
    pub(crate) fn new(rest: RestClient, disk_cache: Option<Arc<DiskCache>>) -> Self {
        Self {
            rest,
            limiter: DownloadLimiter::new(
                CONCURRENT_DOWNLOADS,
                DOWNLOADS_PER_WINDOW,
                BUDGET_WINDOW,
            ),
            disk_cache,
            files_in_memory: Mutex::new(ByteBoundedLru::new(FILES_IN_MEMORY_CAPACITY_BYTES)),
            in_flight: Mutex::new(HashMap::new()),
            retry_delay: RETRY_DELAY,
        }
    }

    #[cfg(test)]
    fn with_retry_delay(mut self, retry_delay: Duration) -> Self {
        self.retry_delay = retry_delay;
        self
    }

    fn disk_cache_and_name(&self, file: DownloadableFile) -> Option<(Arc<DiskCache>, String)> {
        Some((Arc::clone(self.disk_cache.as_ref()?), disk_file_name(file)?))
    }

    /// The file's bytes, from the disk cache, or from the instance (revalidating a copy kept in
    /// memory with its entity tag).
    pub(crate) async fn fetch(&self, file: DownloadableFile) -> FetchResult {
        if let Some((disk_cache, name)) = self.disk_cache_and_name(file) {
            let cached = tokio::task::spawn_blocking(move || disk_cache.read(&name)).await;
            if let Ok(Some(bytes)) = cached {
                return Ok(bytes.into());
            }
        }
        let waiter = {
            let mut in_flight = lock(&self.in_flight);
            if let Some(waiters) = in_flight.get_mut(&file) {
                let (sender, receiver) = oneshot::channel();
                waiters.push(sender);
                Some(receiver)
            } else {
                in_flight.insert(file, Vec::new());
                None
            }
        };
        if let Some(receiver) = waiter {
            return receiver.await.unwrap_or(Err(PictureLoadError::Unavailable));
        }
        let entry = InFlightEntry { loader: self, file };
        let result = self.download_retrying_once(file).await;
        for waiter in entry.finish() {
            let _ = waiter.send(result.clone());
        }
        result
    }

    async fn download_retrying_once(&self, file: DownloadableFile) -> FetchResult {
        match self.download(file).await {
            Err(error) if error.is_worth_retrying() => {
                tokio::time::sleep(self.retry_delay).await;
                Ok(self.download(file).await?)
            }
            result => Ok(result?),
        }
    }

    async fn download(&self, file: DownloadableFile) -> Result<PictureBytes, ClientError> {
        let in_memory = lock(&self.files_in_memory).get(&file).cloned();
        let answer = {
            let _permit = self.limiter.start_download().await;
            self.rest
                .download(
                    file,
                    in_memory
                        .as_ref()
                        .and_then(|copy| copy.entity_tag.as_deref()),
                    maximum_bytes(file),
                )
                .await?
        };
        let downloaded = match answer {
            Download::NotModified => {
                return in_memory.map(|copy| copy.bytes).ok_or_else(|| {
                    ClientError::UnexpectedResponse("not modified without a cached copy".into())
                });
            }
            Download::Fetched(downloaded) => downloaded,
        };
        let bytes: PictureBytes = downloaded.bytes.into();
        if let Some((disk_cache, name)) = self.disk_cache_and_name(file) {
            let bytes_to_write = Arc::clone(&bytes);
            let written =
                tokio::task::spawn_blocking(move || disk_cache.write(&name, &bytes_to_write)).await;
            if !matches!(written, Ok(Ok(()))) {
                // The picture still shows; it is downloaded again next launch.
                tracing::warn!("cannot keep a picture in the disk cache");
            }
        } else {
            let byte_count = bytes.len();
            lock(&self.files_in_memory).insert(
                file,
                FileInMemory {
                    bytes: Arc::clone(&bytes),
                    entity_tag: downloaded.entity_tag,
                },
                byte_count,
            );
        }
        Ok(bytes)
    }

    /// Drops a cached copy that did not decode, so the next view downloads it again.
    pub(crate) async fn forget(&self, file: DownloadableFile) {
        lock(&self.files_in_memory).remove(&file);
        if let Some((disk_cache, name)) = self.disk_cache_and_name(file) {
            let _ = tokio::task::spawn_blocking(move || disk_cache.remove(&name)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use mellow_client_core::InstanceAddress;
    use mellow_protocol::Snowflake;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;
    use zeroize::Zeroizing;

    use super::*;

    const PICTURE: &[u8] = b"picture bytes";

    #[derive(Clone, Copy)]
    enum Answer {
        Picture,
        Unavailable,
        NotModifiedWhenRevalidated,
    }

    /// A tiny HTTP server answering each request from `answers` in turn (the last one repeats),
    /// counting requests and recording whether they carried `If-None-Match`.
    struct FakeInstance {
        address: String,
        request_count: Arc<AtomicUsize>,
        revalidation_count: Arc<AtomicUsize>,
    }

    async fn fake_instance(answers: Vec<Answer>) -> FakeInstance {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let request_count = Arc::new(AtomicUsize::new(0));
        let revalidation_count = Arc::new(AtomicUsize::new(0));
        let (requests, revalidations) =
            (Arc::clone(&request_count), Arc::clone(&revalidation_count));
        tokio::spawn(async move {
            loop {
                let (mut connection, _) = listener.accept().await.unwrap();
                let mut head = Vec::new();
                let mut buffer = [0_u8; 1024];
                while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = connection.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    head.extend_from_slice(&buffer[..read]);
                }
                let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
                let index = requests.fetch_add(1, Ordering::SeqCst);
                let is_revalidation = head.contains("if-none-match: \"v1\"");
                if is_revalidation {
                    revalidations.fetch_add(1, Ordering::SeqCst);
                }
                let answer = answers[index.min(answers.len() - 1)];
                let response = match answer {
                    Answer::NotModifiedWhenRevalidated if is_revalidation => {
                        "HTTP/1.1 304 Not Modified\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n"
                            .to_owned()
                    }
                    Answer::Picture | Answer::NotModifiedWhenRevalidated => format!(
                        "HTTP/1.1 200 OK\r\nETag: \"v1\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        PICTURE.len(),
                        String::from_utf8_lossy(PICTURE)
                    ),
                    Answer::Unavailable => {
                        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_owned()
                    }
                };
                connection.write_all(response.as_bytes()).await.unwrap();
                connection.shutdown().await.unwrap();
            }
        });
        FakeInstance {
            address,
            request_count,
            revalidation_count,
        }
    }

    fn loader_for(instance: &FakeInstance, disk_cache: Option<Arc<DiskCache>>) -> PictureLoader {
        let mut rest = RestClient::new(InstanceAddress::parse(&instance.address).unwrap()).unwrap();
        rest.set_token(Zeroizing::new("test-token".to_owned()));
        PictureLoader::new(rest, disk_cache).with_retry_delay(Duration::ZERO)
    }

    const ATTACHMENT: DownloadableFile = DownloadableFile::Attachment(Snowflake::from_raw(41));
    const AVATAR: DownloadableFile = DownloadableFile::ProfileImage(Snowflake::from_raw(42));

    #[tokio::test]
    async fn views_asking_for_the_same_file_at_once_share_one_request() {
        let instance = fake_instance(vec![Answer::Picture]).await;
        let loader = loader_for(&instance, None);
        // `join!` polls every fetch before any can finish: the first starts the download, the
        // others find it in flight.
        let answers = tokio::join!(
            loader.fetch(ATTACHMENT),
            loader.fetch(ATTACHMENT),
            loader.fetch(ATTACHMENT),
            loader.fetch(ATTACHMENT),
            loader.fetch(ATTACHMENT),
        );
        for answer in [answers.0, answers.1, answers.2, answers.3, answers.4] {
            assert_eq!(&*answer.unwrap(), PICTURE);
        }
        assert_eq!(instance.request_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_busy_instance_is_asked_once_more_then_the_failure_stands() {
        let instance = fake_instance(vec![Answer::Unavailable, Answer::Picture]).await;
        let loader = loader_for(&instance, None);
        assert_eq!(&*loader.fetch(ATTACHMENT).await.unwrap(), PICTURE);
        assert_eq!(instance.request_count.load(Ordering::SeqCst), 2);

        let instance = fake_instance(vec![Answer::Unavailable]).await;
        let loader = loader_for(&instance, None);
        assert_eq!(
            loader.fetch(ATTACHMENT).await,
            Err(PictureLoadError::Unavailable)
        );
        assert_eq!(instance.request_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn an_attachment_kept_in_memory_is_revalidated_with_its_entity_tag() {
        let instance = fake_instance(vec![Answer::NotModifiedWhenRevalidated]).await;
        let loader = loader_for(&instance, None);
        assert_eq!(&*loader.fetch(ATTACHMENT).await.unwrap(), PICTURE);
        assert_eq!(&*loader.fetch(ATTACHMENT).await.unwrap(), PICTURE);
        assert_eq!(instance.request_count.load(Ordering::SeqCst), 2);
        assert_eq!(instance.revalidation_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_profile_picture_comes_from_disk_the_second_time() {
        let directory = tempfile::tempdir().unwrap();
        let disk_cache =
            Arc::new(DiskCache::open(directory.path().join("instance"), 1_000).unwrap());
        let instance = fake_instance(vec![Answer::Picture]).await;
        let loader = loader_for(&instance, Some(Arc::clone(&disk_cache)));
        assert_eq!(&*loader.fetch(AVATAR).await.unwrap(), PICTURE);
        assert_eq!(&*loader.fetch(AVATAR).await.unwrap(), PICTURE);
        assert_eq!(instance.request_count.load(Ordering::SeqCst), 1);
        // A new session (a new loader) finds it on disk too.
        let loader = loader_for(&instance, Some(disk_cache));
        assert_eq!(&*loader.fetch(AVATAR).await.unwrap(), PICTURE);
        assert_eq!(instance.request_count.load(Ordering::SeqCst), 1);
        // Attachments never reach the disk.
        loader.fetch(ATTACHMENT).await.unwrap();
        assert_eq!(
            std::fs::read_dir(directory.path().join("instance"))
                .unwrap()
                .count(),
            1
        );
    }
}
