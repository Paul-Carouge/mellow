//! Pictures from the instance, shown by GPUI (ADR-0051 "Pictures and files").
//!
//! A view asks for a picture with [`picture_source`] and draws it with `img`. The store answers
//! from its memory cache of decoded pictures; otherwise the [`loader`] fetches the bytes on the
//! network runtime (disk cache, request limits, retry), `decode` turns them into BGRA frames
//! scaled to the displayed size on GPUI's background executor (AVIF with `rav1d` and `moxcms`,
//! the rest with `image`), and the waiting views are notified. Nothing here runs on the main
//! thread but map lookups.

// rav1d publishes only dav1d's C interface; the module's guards keep its `unsafe` contained.
#[allow(unsafe_code)]
mod av1;
mod avif;
mod colour;
mod decode;
mod disk_cache;
mod limiter;
mod loader;
mod lru;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{App, EntityId, Global, ImageCacheError, ImageSource, RenderImage, Window};
use mellow_client_core::{DownloadableFile, RestClient};

use self::disk_cache::{DISK_CACHE_CAPACITY_BYTES, DiskCache, instance_directory_name};
use self::loader::PictureLoader;
use self::lru::ByteBoundedLru;
use crate::tokio_bridge::spawn_on_network_runtime;

/// Decoded pictures kept for views to draw again; each also holds a slot in GPUI's atlas until it
/// is evicted from here.
const DECODED_PICTURES_CAPACITY_BYTES: usize = 192 * 1024 * 1024;

/// Whether animated pictures play: off follows the reduced-motion and picture-animation settings,
/// and fetches the instance's still frame where it has one (`/still`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Animation {
    Animate,
    StillFrame,
}

/// The box a picture is drawn in, in device pixels (the layout size times the scale factor): the
/// picture is scaled down to cover it and never kept larger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DisplaySize {
    pub width: u32,
    pub height: u32,
}

/// One picture as one place shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PictureRequest {
    pub file: DownloadableFile,
    pub animation: Animation,
    pub display_size: DisplaySize,
}

impl PictureRequest {
    /// The file to download: the still route of a profile picture or icon when animation is off.
    /// Other pictures have no still route; their first frame is decoded instead.
    fn file_to_download(&self) -> DownloadableFile {
        match (self.animation, self.file) {
            (Animation::StillFrame, DownloadableFile::ProfileImage(id)) => {
                DownloadableFile::ProfileImageStill(id)
            }
            (Animation::StillFrame, DownloadableFile::Icon(id)) => DownloadableFile::IconStill(id),
            (_, file) => file,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PictureDecodeError {
    #[error("the picture cannot be decoded")]
    Undecodable,
    #[error("the picture is too large to decode")]
    TooLarge,
    #[error("the picture's colours cannot be converted")]
    ColourConversion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PictureLoadError {
    /// The instance could not be reached or was busy, twice.
    #[error("the picture is unavailable for now")]
    Unavailable,
    /// The instance refused it (gone, or not visible to this user).
    #[error("the picture is not available")]
    Refused,
    #[error("the picture is too large")]
    TooLarge,
}

/// The pictures of the signed-in session, as a GPUI global.
#[derive(Default)]
pub struct PictureStore {
    loader: Option<Arc<PictureLoader>>,
    /// Bumped when a session starts or ends, so answers for the previous one are dropped.
    session: u64,
    pictures: Option<ByteBoundedLru<PictureRequest, Arc<RenderImage>>>,
    /// Pictures being fetched or decoded, with the views to notify when they are ready.
    loading: HashMap<PictureRequest, Vec<EntityId>>,
    /// Pictures that failed after their retry: shown as failed until the next session.
    failed: HashSet<PictureRequest>,
    /// Evicted pictures whose atlas slots the next window that draws a picture frees.
    released: Vec<Arc<RenderImage>>,
}

impl std::fmt::Debug for PictureStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PictureStore")
            .field("session", &self.session)
            .field("loading", &self.loading.len())
            .field("failed", &self.failed.len())
            .finish_non_exhaustive()
    }
}

impl Global for PictureStore {}

fn frames_byte_count(picture: &RenderImage) -> usize {
    (0..picture.frame_count())
        .filter_map(|frame_index| picture.as_bytes(frame_index))
        .map(<[u8]>::len)
        .sum()
}

impl PictureStore {
    fn pictures(&mut self) -> &mut ByteBoundedLru<PictureRequest, Arc<RenderImage>> {
        self.pictures
            .get_or_insert_with(|| ByteBoundedLru::new(DECODED_PICTURES_CAPACITY_BYTES))
    }

    /// Keeps a decoded picture; the ones it pushes out wait for a window to free their slots.
    fn keep_decoded(&mut self, request: PictureRequest, picture: Arc<RenderImage>) {
        let byte_count = frames_byte_count(&picture);
        let evicted = self.pictures().insert(request, picture, byte_count);
        self.released
            .extend(evicted.into_iter().map(|(_, picture)| picture));
    }

    /// Forgets the session's pictures and returns the views that waited for some.
    fn reset(&mut self) -> Vec<EntityId> {
        self.session += 1;
        self.loader = None;
        self.failed.clear();
        let released = self.pictures().take_all();
        self.released
            .extend(released.into_iter().map(|(_, picture)| picture));
        self.loading.drain().flat_map(|(_, views)| views).collect()
    }
}

/// Starts loading pictures for a signed-in session; `rest` carries its token. The disk cache is
/// opened on the network runtime, and views asking before it is ready draw once it is.
pub fn start_picture_session(rest: RestClient, cx: &mut App) {
    let waiting_views = cx.default_global::<PictureStore>().reset();
    let session = cx.global::<PictureStore>().session;
    let cache_directory = disk_cache::pictures_directory()
        .map(|directory| directory.join(instance_directory_name(rest.instance())));
    let opening = spawn_on_network_runtime(cx, async move {
        let disk_cache = match cache_directory {
            Some(directory) => tokio::task::spawn_blocking(move || {
                DiskCache::open(directory, DISK_CACHE_CAPACITY_BYTES)
            })
            .await
            .ok()
            .and_then(|opened| {
                opened
                    .inspect_err(|_| {
                        tracing::warn!("cannot open the picture cache; pictures stay in memory");
                    })
                    .ok()
            }),
            None => None,
        };
        Arc::new(PictureLoader::new(rest, disk_cache.map(Arc::new)))
    });
    cx.spawn(async move |cx| {
        let Ok(loader) = opening.await else {
            return;
        };
        cx.update(|cx| {
            let store = cx.global_mut::<PictureStore>();
            if store.session == session {
                store.loader = Some(loader);
                // Views drew placeholders while the loader was missing; let them ask again.
                cx.refresh_windows();
            }
        });
    })
    .detach();
    for view in waiting_views {
        cx.notify(view);
    }
}

/// Signs the pictures out: memory caches emptied, and the disk cache deleted (every instance's)
/// so the computer keeps no one's pictures.
pub fn end_picture_session(cx: &mut App) {
    let waiting_views = cx.default_global::<PictureStore>().reset();
    for view in waiting_views {
        cx.notify(view);
    }
    if let Some(directory) = disk_cache::pictures_directory() {
        spawn_on_network_runtime(cx, async move {
            let removed = tokio::task::spawn_blocking(move || {
                disk_cache::remove_pictures_directory(&directory)
            })
            .await;
            if !matches!(removed, Ok(Ok(()))) {
                tracing::warn!("cannot delete the picture cache after signing out");
            }
        })
        .detach();
    }
}

/// The source for `img(...)`: the picture once it is ready, nothing while it loads (the element
/// shows its loading state), an error once it failed (its fallback).
#[must_use]
pub fn picture_source(request: PictureRequest) -> ImageSource {
    ImageSource::Custom(Arc::new(move |window, cx| picture(request, window, cx)))
}

fn picture(
    request: PictureRequest,
    window: &mut Window,
    cx: &mut App,
) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
    let store = cx.default_global::<PictureStore>();
    for released in store.released.drain(..) {
        // Only fails for a window without an atlas, which holds nothing to free.
        let _ = window.drop_image(released);
    }
    if let Some(picture) = store.pictures().get(&request) {
        return Some(Ok(Arc::clone(picture)));
    }
    if store.failed.contains(&request) {
        return Some(Err(ImageCacheError::Asset("picture unavailable".into())));
    }
    let view = window.current_view();
    if let Some(waiting_views) = store.loading.get_mut(&request) {
        if !waiting_views.contains(&view) {
            waiting_views.push(view);
        }
        return None;
    }
    let loader = Arc::clone(store.loader.as_ref()?);
    store.loading.insert(request, vec![view]);
    let session = store.session;
    load(request, loader, session, cx);
    None
}

fn load(request: PictureRequest, loader: Arc<PictureLoader>, session: u64, cx: &mut App) {
    let file = request.file_to_download();
    let fetch_loader = Arc::clone(&loader);
    let download = spawn_on_network_runtime(cx, async move { fetch_loader.fetch(file).await });
    cx.spawn(async move |cx| {
        let decoded = match download.await {
            Ok(Ok(bytes)) => {
                let decoding = cx.background_executor().spawn(async move {
                    decode::decode_frames(&bytes, request.animation, request.display_size)
                });
                let decoded = decoding.await;
                if decoded.is_err() {
                    // A cached copy that does not decode is fetched again next time.
                    cx.update(|cx| {
                        spawn_on_network_runtime(cx, async move { loader.forget(file).await })
                            .detach();
                    });
                }
                decoded.ok()
            }
            Ok(Err(_)) | Err(_) => None,
        };
        cx.update(|cx| {
            let store = cx.default_global::<PictureStore>();
            if store.session != session {
                return;
            }
            let waiting_views = store.loading.remove(&request).unwrap_or_default();
            match decoded {
                Some(frames) => store.keep_decoded(request, Arc::new(RenderImage::new(frames))),
                None => {
                    store.failed.insert(request);
                }
            }
            for view in waiting_views {
                cx.notify(view);
            }
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use image::{Delay, Frame, RgbaImage};
    use mellow_protocol::Snowflake;

    use super::*;

    fn request(id: u64) -> PictureRequest {
        PictureRequest {
            file: DownloadableFile::ProfileImage(Snowflake::from_raw(id)),
            animation: Animation::Animate,
            display_size: DisplaySize {
                width: 64,
                height: 64,
            },
        }
    }

    fn picture_of_bytes(byte_count: u32) -> Arc<RenderImage> {
        let frame = Frame::from_parts(
            RgbaImage::new(byte_count / 4, 1),
            0,
            0,
            Delay::from_numer_denom_ms(0, 1),
        );
        Arc::new(RenderImage::new(vec![frame]))
    }

    #[test]
    fn the_still_mode_asks_for_the_still_routes_where_they_exist() {
        let id = Snowflake::from_raw(5);
        let still = |file| {
            PictureRequest {
                file,
                animation: Animation::StillFrame,
                ..request(1)
            }
            .file_to_download()
        };
        assert_eq!(
            still(DownloadableFile::ProfileImage(id)),
            DownloadableFile::ProfileImageStill(id)
        );
        assert_eq!(
            still(DownloadableFile::Icon(id)),
            DownloadableFile::IconStill(id)
        );
        assert_eq!(
            still(DownloadableFile::Attachment(id)),
            DownloadableFile::Attachment(id)
        );
        assert_eq!(
            request(5).file_to_download(),
            DownloadableFile::ProfileImage(id)
        );
    }

    #[test]
    fn evicted_pictures_wait_for_a_window_to_free_their_atlas_slots() {
        let mut store = PictureStore {
            pictures: Some(ByteBoundedLru::new(1_000)),
            ..PictureStore::default()
        };
        let first = picture_of_bytes(600);
        store.keep_decoded(request(1), Arc::clone(&first));
        assert_eq!(store.released.len(), 0);
        store.keep_decoded(request(2), picture_of_bytes(600));
        assert_eq!(store.released.len(), 1);
        assert_eq!(store.released[0].id, first.id);
        assert!(store.pictures().get(&request(1)).is_none());
        assert!(store.pictures().get(&request(2)).is_some());
    }

    #[test]
    fn a_new_session_drops_everything_of_the_previous_one() {
        let mut store = PictureStore::default();
        store.keep_decoded(request(1), picture_of_bytes(400));
        store.failed.insert(request(2));
        let view = EntityId::from(7_u64);
        store.loading.insert(request(3), vec![view]);
        let session = store.session;

        assert_eq!(store.reset(), [view]);
        assert_ne!(
            store.session, session,
            "late answers of the old session are dropped"
        );
        assert!(store.pictures().get(&request(1)).is_none());
        assert_eq!(store.released.len(), 1);
        assert!(store.failed.is_empty());
        assert!(store.loading.is_empty());
    }
}
