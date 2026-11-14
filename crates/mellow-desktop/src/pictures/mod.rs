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
pub(crate) mod disk_cache;
mod limiter;
mod loader;
pub(crate) mod lru;
mod playback;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    App, EntityId, Global, ImageCacheError, ImageSource, RenderImage, SharedString, Task, Window,
};
use mellow_client_core::{DownloadableFile, KlipyClient, RestClient};
use mellow_protocol::ProfileImage;
use tokio::sync::oneshot;

pub(crate) use self::decode::decode_frames;
use self::disk_cache::{DISK_CACHE_CAPACITY_BYTES, DiskCache, instance_directory_name};
use self::loader::PictureLoader;
use self::lru::ByteBoundedLru;
use self::playback::{DecodedPicture, on_frame_change_beat};
use crate::preferences::{PictureAnimationChoice, preferences};
use crate::tokio_bridge::spawn_on_network_runtime;

/// Decoded pictures kept for views to draw again; each also holds a slot in GPUI's atlas until it
/// is evicted from here.
const DECODED_PICTURES_CAPACITY_BYTES: usize = 192 * 1024 * 1024;
/// A picture that failed because the instance was unreachable or busy is asked again after this,
/// when a view draws it: long enough that repaints do not hammer the instance.
const FAILED_PICTURE_RETRY_AFTER: Duration = Duration::from_secs(30);
/// Display sizes are rounded up to one of eight steps per doubling (at most 1/8 of the next power
/// of two above), so resizing a window reuses decoded pictures instead of decoding at every pixel.
const DISPLAY_SIZE_STEPS_PER_DOUBLING: u32 = 8;

/// Whether animated pictures play: off follows the reduced-motion and picture-animation settings,
/// and fetches the instance's still frame where it has one (`/still`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Animation {
    Animate,
    StillFrame,
}

/// Whether a picture plays (`shouldPictureAnimate` on the web, DESIGN.md v0.34).
/// `is_playing_without_choice` is what the picture does by itself: true for those that always
/// play (custom emoji, banners, the profile card, GIFs, Melli), hover or focus of their row for
/// avatars and server icons. "Always animate" is the reader's own decision, so it wins over the
/// system's reduced motion; otherwise reduced motion keeps every picture still, hovered or not.
#[must_use]
pub fn should_picture_animate(
    choice: PictureAnimationChoice,
    is_reduced_motion_preferred: bool,
    is_playing_without_choice: bool,
) -> bool {
    choice == PictureAnimationChoice::Always
        || (!is_reduced_motion_preferred && is_playing_without_choice)
}

/// [`should_picture_animate`] with the reader's stored choice and the system's reduced motion.
#[must_use]
pub fn is_picture_animation_wanted(is_playing_without_choice: bool, cx: &App) -> bool {
    let choice = preferences(cx)
        .map(|preferences| preferences.picture_animation)
        .unwrap_or_default();
    should_picture_animate(choice, cx.reduce_motion(), is_playing_without_choice)
}

/// The animation to ask for a picture: see [`is_picture_animation_wanted`].
#[must_use]
pub fn chosen_picture_animation(is_playing_without_choice: bool, cx: &App) -> Animation {
    if is_picture_animation_wanted(is_playing_without_choice, cx) {
        Animation::Animate
    } else {
        Animation::StillFrame
    }
}

/// [`chosen_picture_animation`] for an avatar or an icon: one that is not animated is always asked
/// still, so pointing at it never downloads and decodes the same picture a second time.
#[must_use]
pub fn chosen_profile_image_animation(
    image: &ProfileImage,
    is_playing_without_choice: bool,
    cx: &App,
) -> Animation {
    if image.is_animated {
        chosen_picture_animation(is_playing_without_choice, cx)
    } else {
        Animation::StillFrame
    }
}

/// The box a picture is drawn in, in device pixels (the layout size times the scale factor): the
/// picture is scaled down to cover it and never kept larger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DisplaySize {
    pub width: u32,
    pub height: u32,
}

/// `side` rounded up to the next step of its doubling.
fn rounded_up_to_display_step(side: u32) -> u32 {
    let Some(next_power_of_two) = side.checked_next_power_of_two() else {
        return side;
    };
    let step = (next_power_of_two / DISPLAY_SIZE_STEPS_PER_DOUBLING).max(1);
    side.div_ceil(step).saturating_mul(step)
}

impl DisplaySize {
    fn rounded_up_to_display_step(self) -> Self {
        Self {
            width: rounded_up_to_display_step(self.width),
            height: rounded_up_to_display_step(self.height),
        }
    }
}

/// Where a picture comes from: the instance, or Klipy's media hosts (ADR-0042), loaded from the
/// address Klipy returned and kept in memory only, since Klipy's terms forbid copies and caches
/// beyond what a browser keeps.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PictureFile {
    Instance(DownloadableFile),
    /// An address `klipy_media_url` accepted.
    Klipy(SharedString),
}

impl From<DownloadableFile> for PictureFile {
    fn from(file: DownloadableFile) -> Self {
        Self::Instance(file)
    }
}

/// One picture as one place shows it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PictureRequest {
    pub file: PictureFile,
    pub animation: Animation,
    pub display_size: DisplaySize,
}

impl PictureRequest {
    /// The file to download: the still route of a profile picture or icon when animation is off.
    /// Other pictures have no still route; their first frame is decoded instead.
    fn file_to_download(&self) -> PictureFile {
        let PictureFile::Instance(file) = self.file else {
            return self.file.clone();
        };
        PictureFile::Instance(match (self.animation, file) {
            (Animation::StillFrame, DownloadableFile::ProfileImage(id)) => {
                DownloadableFile::ProfileImageStill(id)
            }
            (Animation::StillFrame, DownloadableFile::Icon(id)) => DownloadableFile::IconStill(id),
            (_, file) => file,
        })
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

/// How long a failed picture stays failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PictureFailure {
    /// Refused, too large or undecodable: asking again changes nothing this session.
    Permanent,
    /// The instance could not be reached or was busy: asked again after a while.
    Transient { failed_at: Instant },
}

impl From<PictureLoadError> for PictureFailure {
    fn from(error: PictureLoadError) -> Self {
        match error {
            PictureLoadError::Unavailable => Self::Transient {
                failed_at: Instant::now(),
            },
            PictureLoadError::Refused | PictureLoadError::TooLarge => Self::Permanent,
        }
    }
}

/// The pictures of the signed-in session, as a GPUI global.
#[derive(Default)]
pub struct PictureStore {
    loader: Option<Arc<PictureLoader>>,
    /// Bumped when a session starts or ends, so answers for the previous one are dropped.
    session: u64,
    pictures: Option<ByteBoundedLru<PictureRequest, DecodedPicture>>,
    /// Pictures being fetched or decoded, with the views to notify when they are ready.
    loading: HashMap<PictureRequest, Vec<EntityId>>,
    /// Pictures that failed after their retry, shown as failed until the failure expires.
    failed: HashMap<PictureRequest, PictureFailure>,
    /// Resolves once the last sign-out's deletion of the disk cache is over, so a new session
    /// never opens its directory before an older deletion runs.
    disk_cache_removal: Option<oneshot::Receiver<()>>,
    /// Evicted pictures whose atlas slots the next window that draws a picture frees.
    released: Vec<Arc<RenderImage>>,
    /// Where the shared beat of frame changes is counted from (`on_frame_change_beat`).
    frame_beat_origin: Option<Instant>,
    /// The views showing an animation, each with its next frame change, on the beat.
    frame_wake_ups: Vec<(EntityId, Instant)>,
    /// The one timer that draws those views again, with when it fires; replacing it cancels the
    /// previous one.
    frame_timer: Option<(Instant, Task<()>)>,
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

impl PictureStore {
    fn pictures(&mut self) -> &mut ByteBoundedLru<PictureRequest, DecodedPicture> {
        self.pictures
            .get_or_insert_with(|| ByteBoundedLru::new(DECODED_PICTURES_CAPACITY_BYTES))
    }

    /// Keeps a decoded picture; the ones it pushes out wait for a window to free their slots.
    fn keep_decoded(&mut self, request: PictureRequest, picture: DecodedPicture) {
        let byte_count = picture.byte_count();
        let evicted = self.pictures().insert(request, picture, byte_count);
        self.release(evicted.into_iter().map(|(_, picture)| picture));
    }

    fn release(&mut self, pictures: impl IntoIterator<Item = DecodedPicture>) {
        for picture in pictures {
            self.released.extend(picture.frames().iter().cloned());
        }
    }

    /// The frame of `request` to draw at `now`, once decoded, and when the next one is due
    /// when the picture is animated.
    fn frame_shown_at(
        &mut self,
        request: &PictureRequest,
        now: Instant,
    ) -> Option<(Arc<RenderImage>, Option<Instant>)> {
        let picture = self.pictures().get(request)?;
        let (frame_index, next_due) = picture.frame_drawn_at(now);
        let frame = picture
            .frames()
            .get(frame_index)
            .or_else(|| picture.first_frame())?;
        Some((Arc::clone(frame), next_due))
    }

    /// Whether `request` still shows as failed at `now`; an expired failure is forgotten so the
    /// picture loads again.
    fn has_standing_failure(&mut self, request: &PictureRequest, now: Instant) -> bool {
        match self.failed.get(request) {
            None => false,
            Some(PictureFailure::Permanent) => true,
            Some(PictureFailure::Transient { failed_at })
                if now.saturating_duration_since(*failed_at) < FAILED_PICTURE_RETRY_AFTER =>
            {
                true
            }
            Some(PictureFailure::Transient { .. }) => {
                self.failed.remove(request);
                false
            }
        }
    }

    /// Forgets the session's pictures; returns the views that waited for some and the session's
    /// loader, whose downloads may still be running.
    fn reset(&mut self) -> (Vec<EntityId>, Option<Arc<PictureLoader>>) {
        self.session += 1;
        let loader = self.loader.take();
        self.failed.clear();
        let released = self.pictures().take_all();
        self.release(released.into_iter().map(|(_, picture)| picture));
        self.frame_wake_ups.clear();
        self.frame_timer = None;
        let waiting_views = self.loading.drain().flat_map(|(_, views)| views).collect();
        (waiting_views, loader)
    }
}

/// Starts loading pictures for a signed-in session; `rest` carries its token. The disk cache is
/// opened on the network runtime, and views asking before it is ready draw once it is.
pub fn start_picture_session(rest: RestClient, cx: &mut App) {
    let store = cx.default_global::<PictureStore>();
    let (waiting_views, previous_loader) = store.reset();
    let session = store.session;
    let disk_cache_removal = store.disk_cache_removal.take();
    let cache_directory = disk_cache::pictures_directory()
        .map(|directory| directory.join(instance_directory_name(rest.instance())));
    let opening = spawn_on_network_runtime(cx, async move {
        if let Some(previous_loader) = previous_loader {
            let _ = tokio::task::spawn_blocking(move || previous_loader.close_disk_cache()).await;
        }
        if let Some(disk_cache_removal) = disk_cache_removal {
            // A closed channel means the deletion was cancelled with the runtime: nothing to wait.
            let _ = disk_cache_removal.await;
        }
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

/// The picture loader's Klipy client, so the GIF picker shares its connections; `None` before the
/// session's loader is ready.
pub(crate) fn klipy_client(cx: &App) -> Option<KlipyClient> {
    cx.try_global::<PictureStore>()?
        .loader
        .as_ref()?
        .klipy()
        .cloned()
}

/// Signs the pictures out: memory caches emptied, and the disk cache deleted (every instance's)
/// so the computer keeps no one's pictures.
pub fn end_picture_session(cx: &mut App) {
    let store = cx.default_global::<PictureStore>();
    let (waiting_views, loader) = store.reset();
    let previous_removal = store.disk_cache_removal.take();
    let (removal_done, removal_done_receiver) = oneshot::channel();
    store.disk_cache_removal = Some(removal_done_receiver);
    for view in waiting_views {
        cx.notify(view);
    }
    let directory = disk_cache::pictures_directory();
    spawn_on_network_runtime(cx, async move {
        if let Some(previous_removal) = previous_removal {
            let _ = previous_removal.await;
        }
        let removed = tokio::task::spawn_blocking(move || {
            // The old session's downloads run a moment longer: none may write after the deletion.
            if let Some(loader) = loader {
                loader.close_disk_cache();
            }
            directory.map_or(Ok(()), |directory| {
                disk_cache::remove_pictures_directory(&directory)
            })
        })
        .await;
        if !matches!(removed, Ok(Ok(()))) {
            tracing::warn!("cannot delete the picture cache after signing out");
        }
        let _ = removal_done.send(());
    })
    .detach();
}

/// The source for `img(...)`: the picture once it is ready, nothing while it loads (the element
/// shows its loading state), an error once it failed (its fallback).
///
/// Give the element an explicit size: the decoded picture's natural size is in device pixels
/// (GPUI cannot be told its scale factor) and covers a display size rounded up a little, so an
/// unsized `img` would draw too large on a high-density screen.
#[must_use]
pub fn picture_source(request: PictureRequest) -> ImageSource {
    let request = request.rounded_up_to_display_step();
    ImageSource::Custom(Arc::new(move |window, cx| picture(&request, window, cx)))
}

impl PictureRequest {
    fn rounded_up_to_display_step(self) -> Self {
        Self {
            display_size: self.display_size.rounded_up_to_display_step(),
            ..self
        }
    }
}

/// How far a picture has come, for a view that sizes or replaces itself by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureStatus {
    Loading,
    /// Decoded, `width` × `height` device pixels (scaled down, in the picture's own shape).
    Ready {
        width: u32,
        height: u32,
    },
    Failed,
}

/// Where `request` stands, without asking for it: the view's `img` does that.
pub fn picture_status(request: PictureRequest, cx: &mut App) -> PictureStatus {
    let request = request.rounded_up_to_display_step();
    let store = cx.default_global::<PictureStore>();
    if let Some(picture) = store
        .pictures()
        .get(&request)
        .and_then(DecodedPicture::first_frame)
    {
        let size = picture.size(0);
        return PictureStatus::Ready {
            width: u32::try_from(size.width.0).unwrap_or(0),
            height: u32::try_from(size.height.0).unwrap_or(0),
        };
    }
    if store.has_standing_failure(&request, Instant::now()) {
        PictureStatus::Failed
    } else {
        PictureStatus::Loading
    }
}

fn picture(
    request: &PictureRequest,
    window: &mut Window,
    cx: &mut App,
) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
    let store = cx.default_global::<PictureStore>();
    for released in store.released.drain(..) {
        // Only fails for a window without an atlas, which holds nothing to free.
        let _ = window.drop_image(released);
    }
    // The executor's clock, which tests can drive.
    let now = cx.background_executor().now();
    let store = cx.default_global::<PictureStore>();
    if let Some((frame, next_due)) = store.frame_shown_at(request, now) {
        if let Some(next_due) = next_due {
            draw_again_for_next_frame(window.current_view(), next_due, cx);
        }
        return Some(Ok(frame));
    }
    // An avatar pointed at starts playing: its still frame stays while the animation loads, or
    // if it fails, rather than a blank or the initial.
    let still_meanwhile = (request.animation == Animation::Animate)
        .then(|| PictureRequest {
            animation: Animation::StillFrame,
            ..request.clone()
        })
        .and_then(|still| store.frame_shown_at(&still, now))
        .map(|(frame, _)| Ok(frame));
    if store.has_standing_failure(request, Instant::now()) {
        return still_meanwhile
            .or_else(|| Some(Err(ImageCacheError::Asset("picture unavailable".into()))));
    }
    let view = window.current_view();
    if let Some(waiting_views) = store.loading.get_mut(request) {
        if !waiting_views.contains(&view) {
            waiting_views.push(view);
        }
        return still_meanwhile;
    }
    let Some(loader) = store.loader.as_ref().map(Arc::clone) else {
        return still_meanwhile;
    };
    store.loading.insert(request.clone(), vec![view]);
    let session = store.session;
    load(request.clone(), loader, session, cx);
    still_meanwhile
}

/// Draws `view` again at its next frame change, on the shared beat so every animation changes in
/// the same redraw. One timer serves every view, as `MelliTicker` does; a view that stops drawing
/// its animations is not woken again.
fn draw_again_for_next_frame(view: EntityId, due: Instant, cx: &mut App) {
    let store = cx.default_global::<PictureStore>();
    let origin = *store.frame_beat_origin.get_or_insert(due);
    let due = on_frame_change_beat(due, origin);
    match store
        .frame_wake_ups
        .iter_mut()
        .find(|(waiting, _)| *waiting == view)
    {
        Some((_, wake_up)) if *wake_up <= due => {}
        Some((_, wake_up)) => *wake_up = due,
        None => store.frame_wake_ups.push((view, due)),
    }
    schedule_frame_timer(cx);
}

/// Keeps the one timer set for the earliest wake-up; none once nothing waits.
fn schedule_frame_timer(cx: &mut App) {
    let store = cx.default_global::<PictureStore>();
    let Some(next) = store.frame_wake_ups.iter().map(|(_, due)| *due).min() else {
        store.frame_timer = None;
        return;
    };
    if store
        .frame_timer
        .as_ref()
        .is_some_and(|(fires_at, _)| *fires_at <= next)
    {
        return;
    }
    let executor = cx.background_executor().clone();
    let wait = next.saturating_duration_since(executor.now());
    let timer = cx.spawn(async move |cx| {
        executor.timer(wait).await;
        cx.update(|cx| {
            let now = cx.background_executor().now();
            let store = cx.default_global::<PictureStore>();
            store.frame_timer = None;
            let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut store.frame_wake_ups)
                .into_iter()
                .partition(|(_, due)| *due <= now);
            store.frame_wake_ups = later;
            for (view, _) in due {
                // Drawn again, the view asks for its next frame change if it still shows one.
                cx.notify(view);
            }
            schedule_frame_timer(cx);
        });
    });
    cx.default_global::<PictureStore>().frame_timer = Some((next, timer));
}

fn load(request: PictureRequest, loader: Arc<PictureLoader>, session: u64, cx: &mut App) {
    let file = request.file_to_download();
    let fetch_loader = Arc::clone(&loader);
    let fetched_file = file.clone();
    let download = spawn_on_network_runtime(cx, async move {
        match fetched_file {
            PictureFile::Instance(file) => fetch_loader.fetch(file).await,
            PictureFile::Klipy(url) => fetch_loader.fetch_klipy(url).await,
        }
    });
    let (animation, display_size) = (request.animation, request.display_size);
    cx.spawn(async move |cx| {
        let decoded: Result<_, PictureFailure> = match download.await {
            Ok(Ok(bytes)) => {
                let decoding = cx.background_executor().spawn(async move {
                    decode::decode_animation(&bytes, animation, display_size)
                });
                let decoded = decoding.await;
                if decoded.is_err() {
                    // A cached copy that does not decode is fetched again next time.
                    cx.update(|cx| {
                        spawn_on_network_runtime(cx, async move { loader.forget(file).await })
                            .detach();
                    });
                }
                decoded.map_err(|_| PictureFailure::Permanent)
            }
            Ok(Err(error)) => Err(error.into()),
            // The download was cancelled with the runtime; ask again later like any outage.
            Err(_) => Err(PictureLoadError::Unavailable.into()),
        };
        cx.update(|cx| {
            let store = cx.default_global::<PictureStore>();
            if store.session != session {
                return;
            }
            let waiting_views = store.loading.remove(&request).unwrap_or_default();
            match decoded {
                Ok((frames, play_count)) => {
                    store.keep_decoded(request, DecodedPicture::new(frames, play_count));
                }
                Err(failure) => {
                    store.failed.insert(request, failure);
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
            file: DownloadableFile::ProfileImage(Snowflake::from_raw(id)).into(),
            animation: Animation::Animate,
            display_size: DisplaySize {
                width: 64,
                height: 64,
            },
        }
    }

    fn picture_of_bytes(byte_count: u32) -> DecodedPicture {
        let frame = Frame::from_parts(
            RgbaImage::new(byte_count / 4, 1),
            0,
            0,
            Delay::from_numer_denom_ms(0, 1),
        );
        DecodedPicture::new(vec![frame], None)
    }

    /// Three frames of 100 ms, each its own colour.
    fn animated_picture() -> DecodedPicture {
        DecodedPicture::new(
            [10_u8, 20, 30]
                .into_iter()
                .map(|shade| {
                    Frame::from_parts(
                        RgbaImage::from_pixel(1, 1, image::Rgba([shade, shade, shade, 255])),
                        0,
                        0,
                        Delay::from_numer_denom_ms(100, 1),
                    )
                })
                .collect(),
            None,
        )
    }

    /// A window that draws one picture while `is_showing`, as `img` asks for it, and remembers
    /// which image it was handed.
    struct PictureViewer {
        source: ImageSource,
        is_showing: bool,
        render_count: usize,
        drawn_image: Option<gpui::ImageId>,
    }

    impl gpui::Render for PictureViewer {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            self.render_count += 1;
            if self.is_showing
                && let ImageSource::Custom(load) = &self.source
            {
                self.drawn_image = load(window, cx).and_then(Result::ok).map(|image| image.id);
            }
            gpui::div()
        }
    }

    #[gpui::test]
    fn the_view_is_drawn_again_for_each_frame_and_no_more_once_it_hides_the_picture(
        cx: &mut gpui::TestAppContext,
    ) {
        let decoded = animated_picture();
        let frame_ids: Vec<gpui::ImageId> = decoded.frames().iter().map(|frame| frame.id).collect();
        cx.update(|cx| {
            cx.default_global::<PictureStore>()
                .keep_decoded(request(1), decoded);
        });
        let (viewer, cx) = cx.add_window_view(|_, _| PictureViewer {
            source: picture_source(request(1)),
            is_showing: true,
            render_count: 0,
            drawn_image: None,
        });
        cx.run_until_parked();
        let drawn =
            |cx: &mut gpui::VisualTestContext| viewer.read_with(cx, |viewer, _| viewer.drawn_image);
        assert_eq!(drawn(cx), Some(frame_ids[0]));

        cx.executor().advance_clock(Duration::from_millis(110));
        cx.run_until_parked();
        assert_eq!(drawn(cx), Some(frame_ids[1]), "woken for its next frame");
        // Its change at 200 ms waits for the shared beat (33 ms steps from the first one, 100 ms).
        cx.executor().advance_clock(Duration::from_millis(130));
        cx.run_until_parked();
        assert_eq!(drawn(cx), Some(frame_ids[2]));

        viewer.update(cx, |viewer, cx| {
            viewer.is_showing = false;
            cx.notify();
        });
        cx.run_until_parked();
        // The wake-up already asked for runs once more, then nothing.
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        let renders = viewer.read_with(cx, |viewer, _| viewer.render_count);
        cx.executor().advance_clock(Duration::from_secs(5));
        cx.run_until_parked();
        assert_eq!(
            viewer.read_with(cx, |viewer, _| viewer.render_count),
            renders
        );
        assert!(
            cx.update(|_, cx| cx.global::<PictureStore>().frame_timer.is_none()),
            "the timer stops by itself"
        );
    }

    // The owner's report: animated avatars, emoji and GIFs stayed on their first frame, as GPUI's
    // `img` advanced frames only for an element with an id. The store now picks the frame.
    #[test]
    fn an_animated_picture_shows_the_frame_due_at_each_moment() {
        let mut store = PictureStore::default();
        store.keep_decoded(request(1), animated_picture());
        let start = Instant::now();
        let at = |milliseconds| start + Duration::from_millis(milliseconds);
        let shade_at = |store: &mut PictureStore, milliseconds| {
            let (frame, next_due) = store.frame_shown_at(&request(1), at(milliseconds)).unwrap();
            assert_eq!(frame.frame_count(), 1, "GPUI never plays it on its own");
            (frame.as_bytes(0).unwrap()[0], next_due)
        };
        assert_eq!(shade_at(&mut store, 0), (10, Some(at(100))));
        assert_eq!(shade_at(&mut store, 150), (20, Some(at(200))));
        assert_eq!(shade_at(&mut store, 250), (30, Some(at(300))));
        assert_eq!(shade_at(&mut store, 300), (10, Some(at(400))));

        store.keep_decoded(request(2), picture_of_bytes(400));
        let (_, next_due) = store.frame_shown_at(&request(2), start).unwrap();
        assert_eq!(next_due, None, "a still picture asks for no other frame");
    }

    #[test]
    fn always_animate_wins_over_reduced_motion_and_hover_only_without_it() {
        use PictureAnimationChoice::{Always, OnHover};
        // (choice, reduced motion, playing without a choice) -> plays
        for (choice, is_reduced, is_playing_by_itself, expected) in [
            (OnHover, false, true, true),
            (OnHover, false, false, false),
            (OnHover, true, true, false),
            (OnHover, true, false, false),
            (Always, false, true, true),
            (Always, false, false, true),
            (Always, true, true, true),
            (Always, true, false, true),
        ] {
            assert_eq!(
                should_picture_animate(choice, is_reduced, is_playing_by_itself),
                expected,
                "{choice:?}, reduced motion {is_reduced}, by itself {is_playing_by_itself}"
            );
        }
    }

    #[gpui::test]
    fn the_stored_choice_and_the_system_decide_the_animation(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            assert_eq!(chosen_picture_animation(true, cx), Animation::Animate);
            assert_eq!(chosen_picture_animation(false, cx), Animation::StillFrame);
            cx.set_reduce_motion(true);
            assert_eq!(chosen_picture_animation(true, cx), Animation::StillFrame);
            crate::preferences::change_preferences(cx, |preferences| {
                preferences.picture_animation = PictureAnimationChoice::Always;
            });
            assert_eq!(chosen_picture_animation(false, cx), Animation::Animate);
        });
    }

    #[test]
    fn the_still_mode_asks_for_the_still_routes_where_they_exist() {
        let id = Snowflake::from_raw(5);
        let still = |file: DownloadableFile| {
            PictureRequest {
                file: file.into(),
                animation: Animation::StillFrame,
                ..request(1)
            }
            .file_to_download()
        };
        let instance = PictureFile::Instance;
        assert_eq!(
            still(DownloadableFile::ProfileImage(id)),
            instance(DownloadableFile::ProfileImageStill(id))
        );
        assert_eq!(
            still(DownloadableFile::Icon(id)),
            instance(DownloadableFile::IconStill(id))
        );
        assert_eq!(
            still(DownloadableFile::Attachment(id)),
            instance(DownloadableFile::Attachment(id))
        );
        assert_eq!(
            request(5).file_to_download(),
            instance(DownloadableFile::ProfileImage(id))
        );
        let klipy = PictureFile::Klipy("https://static.klipy.com/ii/a/md.webp".into());
        let klipy_still = PictureRequest {
            file: klipy.clone(),
            animation: Animation::StillFrame,
            ..request(1)
        };
        assert_eq!(
            klipy_still.file_to_download(),
            klipy,
            "its first frame is decoded"
        );
    }

    #[test]
    fn evicted_pictures_wait_for_a_window_to_free_their_atlas_slots() {
        let mut store = PictureStore {
            pictures: Some(ByteBoundedLru::new(1_000)),
            ..PictureStore::default()
        };
        let first = picture_of_bytes(600);
        let first_id = first.frames()[0].id;
        store.keep_decoded(request(1), first);
        assert_eq!(store.released.len(), 0);
        store.keep_decoded(request(2), picture_of_bytes(600));
        assert_eq!(store.released.len(), 1);
        assert_eq!(store.released[0].id, first_id);
        assert!(store.pictures().get(&request(1)).is_none());
        assert!(store.pictures().get(&request(2)).is_some());
    }

    #[test]
    fn an_unavailable_picture_loads_again_once_the_retry_delay_has_passed() {
        let mut store = PictureStore::default();
        let failed_at = Instant::now();
        store
            .failed
            .insert(request(1), PictureFailure::Transient { failed_at });
        store.failed.insert(request(2), PictureFailure::Permanent);
        let soon = failed_at + FAILED_PICTURE_RETRY_AFTER.saturating_sub(Duration::from_secs(1));
        let later = failed_at + FAILED_PICTURE_RETRY_AFTER;

        assert!(store.has_standing_failure(&request(1), soon));
        assert!(!store.has_standing_failure(&request(1), later));
        assert!(
            !store.failed.contains_key(&request(1)),
            "the next draw loads it"
        );
        assert!(store.has_standing_failure(&request(2), later));
        assert!(!store.has_standing_failure(&request(3), later));
    }

    #[test]
    fn display_sizes_round_up_to_a_few_steps_per_doubling() {
        assert_eq!(rounded_up_to_display_step(0), 0);
        assert_eq!(rounded_up_to_display_step(1), 1);
        assert_eq!(rounded_up_to_display_step(7), 7);
        assert_eq!(rounded_up_to_display_step(48), 48);
        assert_eq!(rounded_up_to_display_step(1_000), 1_024);
        assert_eq!(rounded_up_to_display_step(1_025), 1_280);
        for side in [1_100, 1_150, 1_200, 1_280] {
            assert_eq!(
                rounded_up_to_display_step(side),
                1_280,
                "one key while resizing"
            );
        }
        assert_eq!(rounded_up_to_display_step(u32::MAX), u32::MAX);
        for side in 1..5_000 {
            let rounded = rounded_up_to_display_step(side);
            assert!(
                rounded >= side && rounded <= side + side / 4 + 1,
                "{side} -> {rounded}"
            );
        }
    }

    #[test]
    fn a_new_session_drops_everything_of_the_previous_one() {
        let mut store = PictureStore::default();
        store.keep_decoded(request(1), picture_of_bytes(400));
        store.failed.insert(request(2), PictureFailure::Permanent);
        let view = EntityId::from(7_u64);
        store.loading.insert(request(3), vec![view]);
        let session = store.session;

        let (waiting_views, _) = store.reset();
        assert_eq!(waiting_views, [view]);
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
