//! Received pictures on screen (ADR-0051, "Receiving video"): the engine's decoding threads hand
//! each watched person's newest picture, already scaled to its tile and converted to BGRA, to
//! [`VideoPictures`], an entity only the call view reads. GPUI marks a notified view's ancestors
//! dirty too, so each new picture also re-renders the shell around the call view (its children
//! are cheap element trees; the call view in full screen draws the overlay alone).
//! [`video_view`] draws one.
//!
//! Drawing: each picture becomes a GPUI `RenderImage` around the decoding thread's own buffer
//! (no copy on the UI thread), uploaded once to the window's atlas; the previous picture of that
//! person leaves the atlas at once. `surface` (a `CVPixelBuffer` drawn without upload) exists on
//! macOS only, ignores corner radii and wants NV12 buffers from a pool: the image path is the
//! one every platform shares, and the decoder's per-picture buffer is the only allocation.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, Entity, ImageSource, IntoElement, ObjectFit, RenderImage,
    Styled as _, StyledImage as _, Task, img,
};
use image::{Frame, RgbaImage};
use mellow_media::{DecodedPixelFormat, DecodedVideoFrame, VideoFrames};
use mellow_protocol::Snowflake;

/// One received picture, ready to draw.
#[derive(Debug, Clone)]
pub struct VideoPicture {
    image: Arc<RenderImage>,
    pub width: u32,
    pub height: u32,
}

impl VideoPicture {
    /// Wraps a decoded BGRA picture; `None` for another format or a size its bytes do not
    /// match.
    #[must_use]
    pub fn from_decoded(frame: DecodedVideoFrame) -> Option<Self> {
        if frame.format != DecodedPixelFormat::Bgra {
            return None;
        }
        Self::from_bgra(frame.width, frame.height, frame.pixels)
    }

    /// Wraps packed BGRA rows (a screen's thumbnail); `None` for a size `pixels` does not match.
    #[must_use]
    pub fn from_bgra(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        // GPUI's images hold BGRA in an RGBA buffer type: the bytes go in as they are.
        let pixels = RgbaImage::from_raw(width, height, pixels)?;
        Some(Self {
            image: Arc::new(RenderImage::new(vec![Frame::new(pixels)])),
            width,
            height,
        })
    }

    /// Frees its tiles in the windows' atlases: a picture no longer drawn (a closed picker's
    /// thumbnail) would otherwise keep its texture for the window's life.
    pub fn free(self, cx: &mut App) {
        free_atlas_tiles(self.image, cx);
    }

    /// The image itself, for a view that frees its atlas slot on its own (the settings preview).
    #[must_use]
    pub fn into_render_image(self) -> Arc<RenderImage> {
        self.image
    }
}

/// How a picture fills its tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoFit {
    /// Cropped to fill: cameras.
    Cover,
    /// Letterboxed: screens, whose edges matter.
    Contain,
}

/// Draws `picture` filling its parent (which clips and rounds it).
#[must_use]
pub fn video_view(picture: &VideoPicture, fit: VideoFit) -> impl IntoElement + use<> {
    img(ImageSource::Render(Arc::clone(&picture.image)))
        .size_full()
        .object_fit(match fit {
            VideoFit::Cover => ObjectFit::Cover,
            VideoFit::Contain => ObjectFit::Contain,
        })
}

/// The newest picture of each watched person in the call.
#[derive(Debug, Default)]
pub struct VideoPictures {
    pictures: HashMap<Snowflake, VideoPicture>,
}

impl VideoPictures {
    #[must_use]
    pub fn picture(&self, user_id: Snowflake) -> Option<&VideoPicture> {
        self.pictures.get(&user_id)
    }

    /// Shows the new pictures, freeing the atlas tiles of the ones they replace.
    pub fn show(&mut self, frames: Vec<(Snowflake, DecodedVideoFrame)>, cx: &mut Context<Self>) {
        if frames.is_empty() {
            return;
        }
        for (user_id, frame) in frames {
            let Some(picture) = VideoPicture::from_decoded(frame) else {
                continue;
            };
            if let Some(previous) = self.pictures.insert(user_id, picture) {
                free_atlas_tiles(previous.image, cx);
            }
        }
        cx.notify();
    }

    /// Forgets the pictures of people no longer watched (their last picture would show a frozen
    /// frame when they come back).
    pub fn keep_only(&mut self, is_watched: impl Fn(Snowflake) -> bool, cx: &mut Context<Self>) {
        let gone: Vec<Snowflake> = self
            .pictures
            .keys()
            .copied()
            .filter(|user_id| !is_watched(*user_id))
            .collect();
        if gone.is_empty() {
            return;
        }
        for user_id in gone {
            if let Some(picture) = self.pictures.remove(&user_id) {
                free_atlas_tiles(picture.image, cx);
            }
        }
        cx.notify();
    }
}

/// Frees `image`'s tiles in every window's atlas. Deferred: the window whose event handler is
/// running (closing the call view, watching a stream) is out of the app's window list until the
/// handler returns, and `drop_image` would miss it, leaking the texture for the window's life.
fn free_atlas_tiles(image: Arc<RenderImage>, cx: &mut App) {
    cx.defer(move |cx| cx.drop_image(image, None));
}

/// Brings the engine's pictures to `pictures` as they are decoded: the decoding threads wake
/// this task (a wake while it runs is kept, never queued twice), which takes the newest picture
/// of each person in one go.
pub(crate) fn pump_video_pictures(
    frames: VideoFrames,
    pictures: &Entity<VideoPictures>,
    cx: &mut App,
) -> Task<()> {
    let wake = Arc::new(tokio::sync::Notify::new());
    let waker = Arc::clone(&wake);
    frames.set_waker(move || waker.notify_one());
    let pictures = pictures.downgrade();
    cx.spawn(async move |cx| {
        loop {
            wake.notified().await;
            let taken = frames.take_new_frames();
            if pictures
                .update(cx, |pictures, cx| pictures.show(taken, cx))
                .is_err()
            {
                break;
            }
        }
    })
}

/// A new, empty set of pictures.
pub(crate) fn new_video_pictures(cx: &mut App) -> Entity<VideoPictures> {
    cx.new(|_| VideoPictures::default())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A grey BGRA picture.
    pub(crate) fn synthetic_frame(width: u32, height: u32) -> DecodedVideoFrame {
        DecodedVideoFrame {
            width,
            height,
            format: DecodedPixelFormat::Bgra,
            pixels: vec![128; (width * height * 4) as usize],
        }
    }

    #[test]
    fn only_a_bgra_picture_whose_bytes_match_its_size_is_drawn() {
        let picture = VideoPicture::from_decoded(synthetic_frame(4, 2)).unwrap();
        assert_eq!((picture.width, picture.height), (4, 2));
        let mut short = synthetic_frame(4, 2);
        short.pixels.pop();
        assert!(VideoPicture::from_decoded(short).is_none());
        let mut nv12 = synthetic_frame(4, 2);
        nv12.format = DecodedPixelFormat::Nv12;
        assert!(VideoPicture::from_decoded(nv12).is_none());
    }

    #[gpui::test]
    fn new_pictures_replace_old_ones_and_unwatched_people_are_forgotten(
        cx: &mut gpui::TestAppContext,
    ) {
        let pictures = cx.update(new_video_pictures);
        let (ana, ben) = (Snowflake::from_raw(1), Snowflake::from_raw(2));
        pictures.update(cx, |pictures, cx| {
            pictures.show(
                vec![(ana, synthetic_frame(8, 4)), (ben, synthetic_frame(4, 4))],
                cx,
            );
            pictures.show(vec![(ana, synthetic_frame(16, 8))], cx);
        });
        pictures.read_with(cx, |pictures, _| {
            assert_eq!(pictures.picture(ana).map(|picture| picture.width), Some(16));
            assert!(pictures.picture(ben).is_some());
        });
        pictures.update(cx, |pictures, cx| {
            pictures.keep_only(|user_id| user_id == ana, cx);
        });
        pictures.read_with(cx, |pictures, _| {
            assert!(pictures.picture(ana).is_some());
            assert!(pictures.picture(ben).is_none());
        });
    }
}
