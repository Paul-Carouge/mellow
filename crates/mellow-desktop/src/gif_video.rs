//! Klipy's MP4 GIFs (ADR-0042), played as the web plays them: muted, looping, by themselves
//! unless motion is reduced (the first picture then, until the "GIF ▶" key plays it). An MP4 is
//! a few times lighter than the same animated GIF, downloaded and decoded as it plays rather than
//! held whole as pictures. Each shown GIF owns its decoder for as long as its element is drawn
//! (`use_gif_video`): scrolled away, its thread stops and its picture leaves the atlas. The
//! downloaded files stay in a small memory cache, so scrolling back does not fetch them again.

use std::sync::Arc;

use gpui::{App, Context, ElementId, Entity, Global, SharedString, Task, Window};
use mellow_media::{VideoStream, VideoStreamPhase, VideoTiming, plays_video_type};

use crate::pictures::klipy_client;
use crate::pictures::lru::ByteBoundedLru;
use crate::tokio_bridge::spawn_on_network_runtime;
use crate::voice::video_view::VideoPicture;

/// Klipy's clips are a few hundred kilobytes: this keeps a screenful or two.
const GIF_VIDEO_CACHE_BYTES: usize = 16 * 1024 * 1024;

/// The content type of a Klipy clip address, from its extension (the files themselves are
/// checked again before any decoder sees them).
#[must_use]
pub(crate) fn klipy_video_type(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    if path.to_ascii_lowercase().ends_with(".webm") {
        "video/webm"
    } else {
        "video/mp4"
    }
}

/// Whether this system plays the Klipy clip at `url`.
#[must_use]
pub(crate) fn plays_klipy_video(url: &str) -> bool {
    plays_video_type(klipy_video_type(url))
}

struct GifVideoFiles(ByteBoundedLru<SharedString, Arc<[u8]>>);

impl Global for GifVideoFiles {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GifVideoPhase {
    Loading,
    Showing,
    /// Klipy did not serve it, or it does not decode here: the caller falls back.
    Failed,
}

/// One shown GIF's decoder and newest picture.
pub(crate) struct GifVideo {
    phase: GifVideoPhase,
    is_playing: bool,
    stream: Option<VideoStream>,
    picture: Option<VideoPicture>,
    /// The download, then the picture pump; dropped with the GIF.
    work: Option<Task<()>>,
}

impl std::fmt::Debug for GifVideo {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GifVideo")
            .field("phase", &self.phase)
            .field("is_playing", &self.is_playing)
            .finish_non_exhaustive()
    }
}

impl GifVideo {
    fn new(url: SharedString, is_playing: bool, cx: &mut Context<Self>) -> Self {
        cx.on_release(|video: &mut Self, cx| {
            if let Some(picture) = video.picture.take() {
                picture.free(cx);
            }
        })
        .detach();
        let cached = cx.default_global::<GifVideoFiles>().0.get(&url).cloned();
        let mut video = Self {
            phase: GifVideoPhase::Loading,
            is_playing,
            stream: None,
            picture: None,
            work: None,
        };
        if let Some(bytes) = cached {
            video.start(&url, bytes, cx);
            return video;
        }
        let Some(klipy) = klipy_client(cx) else {
            video.phase = GifVideoPhase::Failed;
            return video;
        };
        let address = url.to_string();
        let download = spawn_on_network_runtime(cx, async move {
            klipy.media(&address).await.map(Arc::<[u8]>::from)
        });
        video.work = Some(cx.spawn(async move |video, cx| {
            let bytes = match download.await {
                Ok(Ok(bytes)) => bytes,
                Ok(Err(error)) => {
                    tracing::warn!(%error, "cannot download a GIF clip");
                    let _ = video.update(cx, GifVideo::fail);
                    return;
                }
                Err(_) => return,
            };
            let _ = video.update(cx, |video, cx| {
                let byte_count = bytes.len();
                cx.default_global::<GifVideoFiles>().0.insert(
                    url.clone(),
                    Arc::clone(&bytes),
                    byte_count,
                );
                video.start(&url, bytes, cx);
            });
        }));
        video
    }

    fn fail(&mut self, cx: &mut Context<Self>) {
        self.phase = GifVideoPhase::Failed;
        self.stream = None;
        self.work = None;
        cx.notify();
    }

    fn start(&mut self, url: &str, bytes: Arc<[u8]>, cx: &mut Context<Self>) {
        let stream = match VideoStream::start(
            bytes,
            klipy_video_type(url),
            VideoTiming::Loop {
                is_playing: self.is_playing,
            },
        ) {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(error = %error.kind(), "cannot start a GIF clip's decoder");
                self.fail(cx);
                return;
            }
        };
        let wake = Arc::new(tokio::sync::Notify::new());
        let waker = Arc::clone(&wake);
        stream.set_waker(move || waker.notify_one());
        self.stream = Some(stream);
        self.work = Some(cx.spawn(async move |video, cx| {
            loop {
                wake.notified().await;
                if video
                    .update(cx, GifVideo::take_picture)
                    .is_err()
                {
                    return;
                }
            }
        }));
        cx.notify();
    }

    fn take_picture(&mut self, cx: &mut Context<Self>) {
        let Some(stream) = self.stream.as_ref() else {
            return;
        };
        if let VideoStreamPhase::Failed(error) = stream.status().phase {
            tracing::warn!(%error, "cannot play a GIF clip here");
            self.fail(cx);
            return;
        }
        let Some(picture) = stream
            .take_new_picture()
            .and_then(VideoPicture::from_decoded)
        else {
            return;
        };
        self.phase = GifVideoPhase::Showing;
        if let Some(previous) = self.picture.replace(picture) {
            previous.free(cx);
        }
        cx.notify();
    }

    fn set_playing(&mut self, is_playing: bool) {
        if self.is_playing != is_playing {
            self.is_playing = is_playing;
            if let Some(stream) = self.stream.as_ref() {
                stream.set_loop_playing(is_playing);
            }
        }
    }

    #[must_use]
    pub(crate) fn phase(&self) -> GifVideoPhase {
        self.phase
    }

    #[must_use]
    pub(crate) fn picture(&self) -> Option<&VideoPicture> {
        self.picture.as_ref()
    }
}

/// The clip at `url` for the element `id`, kept while that element is drawn; it loops while
/// `is_playing`, else holds its picture.
pub(crate) fn use_gif_video(
    id: &ElementId,
    url: &SharedString,
    is_playing: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<GifVideo> {
    let url = url.clone();
    let video = window.use_keyed_state(id.clone(), cx, |_, cx| GifVideo::new(url, is_playing, cx));
    video.update(cx, |video, _| video.set_playing(is_playing));
    video
}

impl Default for GifVideoFiles {
    fn default() -> Self {
        Self(ByteBoundedLru::new(GIF_VIDEO_CACHE_BYTES))
    }
}

#[cfg(test)]
mod tests {
    use gpui::AppContext as _;

    use super::*;

    #[test]
    fn a_clips_type_comes_from_its_extension() {
        assert_eq!(
            klipy_video_type("https://static.klipy.com/ii/a/b/md.mp4"),
            "video/mp4"
        );
        assert_eq!(
            klipy_video_type("https://static.klipy.com/ii/a/b/md.WEBM?x=1"),
            "video/webm"
        );
        assert_eq!(
            plays_klipy_video("https://static.klipy.com/ii/a/b/md.mp4"),
            cfg!(any(target_os = "linux", target_os = "macos"))
        );
    }

    #[gpui::test]
    fn without_klipy_a_clip_fails_so_its_link_shows(cx: &mut gpui::TestAppContext) {
        let video =
            cx.new(|cx| GifVideo::new("https://static.klipy.com/ii/a/b/md.mp4".into(), true, cx));
        video.read_with(cx, |video, _| {
            assert_eq!(video.phase(), GifVideoPhase::Failed);
            assert!(video.picture().is_none());
        });
    }
    // The decoding itself is tested in `mellow-media` (`video_file`): GPUI's test scheduler
    // refuses wakes from the decoder's own thread.
}
