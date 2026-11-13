//! Cameras behind a trait: the system's (`AVFoundation` on macOS, PipeWire or the xdg Camera
//! portal on Linux, none on Windows) or a synthetic one for tests and headless use. An open
//! camera pushes its pictures into a session's [`VideoInput`] (or a [`CameraPreview`]'s) from its
//! own thread and stops when dropped or when the input closes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::simulcast::{FRAMES_PER_SECOND, VideoSourceKind};
use crate::video_frame::{CapturedFrame, CapturedPixelFormat};
use crate::video_sender::{VideoEncoderControl, VideoInput};

/// The web client's camera request (`camera.ts`): 1280 × 720 at 30 frames per second.
pub(crate) const PREFERRED_CAMERA_SIZE: (u32, u32) = (1_280, 720);

/// One camera as the settings list it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraDescription {
    /// Stable across runs where the system allows it: store this one in the settings.
    pub id: String,
    pub name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CameraError {
    #[error("no camera is available")]
    NoCamera,
    #[error("the system refused access to the camera")]
    PermissionDenied,
    #[error("another application is using the camera")]
    Busy,
    #[error("the camera system failed: {0}")]
    System(String),
}

/// Keeps a camera running; dropping it stops the camera.
pub trait RunningCapture {
    /// The camera stopped by itself (unplugged, taken by the system): no picture follows.
    fn has_ended(&self) -> bool {
        false
    }
}

/// Lists and opens cameras.
pub trait Cameras {
    /// # Errors
    /// The camera system cannot list devices.
    fn cameras(&self) -> Result<Vec<CameraDescription>, CameraError>;

    /// Opens camera `device_id`, or the default one when `None` or no longer there, sending its
    /// pictures to `frames`. Can block while the system starts the device (hundreds of
    /// milliseconds): call it off the interface's thread.
    ///
    /// # Errors
    /// No camera, no permission, or the system failed.
    fn open(
        &self,
        device_id: Option<&str>,
        frames: VideoInput,
    ) -> Result<Box<dyn RunningCapture>, CameraError>;
}

/// A camera filming a moving test pattern, paced at 30 frames per second.
#[derive(Debug, Clone, Copy)]
pub struct SyntheticCameras {
    pub width: u32,
    pub height: u32,
}

impl Default for SyntheticCameras {
    fn default() -> Self {
        Self {
            width: PREFERRED_CAMERA_SIZE.0,
            height: PREFERRED_CAMERA_SIZE.1,
        }
    }
}

/// The synthetic camera's id and name.
const SYNTHETIC_CAMERA_ID: &str = "synthetic";

struct SyntheticCapture {
    should_stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RunningCapture for SyntheticCapture {}

impl Drop for SyntheticCapture {
    fn drop(&mut self) {
        self.should_stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the synthetic camera's thread panicked");
        }
    }
}

/// A frame of the test pattern: diagonal bands that move with `index`, so every frame differs
/// and the encoder spends real bits on it.
pub(crate) fn synthetic_frame(width: u32, height: u32, index: u32) -> CapturedFrame {
    let mut bytes = Vec::with_capacity(CapturedFrame::expected_length(
        width,
        height,
        CapturedPixelFormat::I420,
    ));
    for y in 0..height {
        bytes.extend((0..width).map(|x| {
            // Always below 220, so the sum stays within limited-range luma.
            let band = (x + y + index * 8) % 220;
            u8::try_from(band + 16).unwrap_or(u8::MAX)
        }));
    }
    let chroma = (width.div_ceil(2) * height.div_ceil(2)) as usize;
    bytes.extend(std::iter::repeat_n(96, chroma));
    bytes.extend(std::iter::repeat_n(160, chroma));
    CapturedFrame {
        width,
        height,
        format: CapturedPixelFormat::I420,
        bytes,
    }
}

impl Cameras for SyntheticCameras {
    fn cameras(&self) -> Result<Vec<CameraDescription>, CameraError> {
        Ok(vec![CameraDescription {
            id: SYNTHETIC_CAMERA_ID.into(),
            name: "Synthetic camera".into(),
        }])
    }

    fn open(
        &self,
        _device_id: Option<&str>,
        frames: VideoInput,
    ) -> Result<Box<dyn RunningCapture>, CameraError> {
        let should_stop = Arc::new(AtomicBool::new(false));
        let (width, height) = (self.width, self.height);
        let thread = {
            let should_stop = Arc::clone(&should_stop);
            std::thread::Builder::new()
                .name("mellow-synthetic-camera".into())
                .spawn(move || {
                    let interval = Duration::from_secs(1) / FRAMES_PER_SECOND;
                    let mut next_frame_at = Instant::now();
                    let mut index: u32 = 0;
                    while !should_stop.load(Ordering::Relaxed) && frames.is_open() {
                        frames.push(synthetic_frame(width, height, index));
                        index = index.wrapping_add(1);
                        next_frame_at += interval;
                        std::thread::sleep(next_frame_at.saturating_duration_since(Instant::now()));
                    }
                })
                .map_err(|error| CameraError::System(error.to_string()))?
        };
        Ok(Box::new(SyntheticCapture {
            should_stop,
            thread: Some(thread),
        }))
    }
}

/// A camera opened on its own, outside any call: its pictures come out one at a time, for
/// whoever shows them (the settings' preview) or forwards them into a call's [`VideoInput`] once
/// the camera works. Not `Send` (macOS captures are not): keep it on the thread that opened it.
pub struct CameraFeed {
    pictures: Arc<VideoEncoderControl>,
    capture: Box<dyn RunningCapture>,
}

impl std::fmt::Debug for CameraFeed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("CameraFeed").finish_non_exhaustive()
    }
}

impl CameraFeed {
    /// Opens `device_id` (the default when `None` or gone), blocking as [`Cameras::open`] does.
    ///
    /// # Errors
    /// The camera cannot be opened.
    pub fn open(cameras: &dyn Cameras, device_id: Option<&str>) -> Result<Self, CameraError> {
        let pictures = Arc::new(VideoEncoderControl::default());
        let input = pictures.start(VideoSourceKind::Camera);
        let capture = cameras.open(device_id, input)?;
        Ok(Self { pictures, capture })
    }

    /// The camera stopped by itself (unplugged, taken by the system).
    #[must_use]
    pub fn has_ended(&self) -> bool {
        self.capture.has_ended()
    }

    /// The newest picture since the last call, waiting up to `timeout` for one.
    #[must_use]
    pub fn wait_picture(&self, timeout: Duration) -> Option<CapturedFrame> {
        self.pictures.wait_for_picture(timeout)
    }
}

/// The system's cameras: `AVFoundation` on macOS, PipeWire on Linux (through the Camera portal
/// when PipeWire shows none). Windows lists none.
// shortcut: no Windows camera (Media Foundation is a capture graph of its own, not a few calls);
// add it when the desktop ships there.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemCameras;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl Cameras for SystemCameras {
    fn cameras(&self) -> Result<Vec<CameraDescription>, CameraError> {
        Ok(Vec::new())
    }

    fn open(
        &self,
        _device_id: Option<&str>,
        _frames: VideoInput,
    ) -> Result<Box<dyn RunningCapture>, CameraError> {
        Err(CameraError::NoCamera)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_pattern_is_a_whole_i420_picture_that_moves() {
        let first = synthetic_frame(64, 36, 0);
        let second = synthetic_frame(64, 36, 1);
        assert_eq!(
            first.bytes.len(),
            CapturedFrame::expected_length(64, 36, CapturedPixelFormat::I420)
        );
        assert_ne!(first, second);
        assert!(first.to_i420().is_some());
    }

    #[test]
    fn the_system_s_cameras_exist_on_every_platform() {
        let cameras: Box<dyn Cameras> = Box::new(SystemCameras);
        if cfg!(not(any(target_os = "macos", target_os = "linux"))) {
            assert_eq!(cameras.cameras().unwrap(), []);
        }
    }

    #[test]
    fn a_feed_hands_over_its_newest_picture() {
        let cameras = SyntheticCameras {
            width: 64,
            height: 36,
        };
        let feed = CameraFeed::open(&cameras, None).unwrap();
        // The synthetic camera's first picture comes at once; later ones every 33 ms.
        let picture = feed.wait_picture(Duration::from_secs(10)).unwrap();
        assert_eq!((picture.width, picture.height), (64, 36));
        assert!(!feed.has_ended());
    }
}
