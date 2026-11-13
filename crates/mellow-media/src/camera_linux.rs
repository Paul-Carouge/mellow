//! Linux cameras through Vesper's PipeWire capture (`vesper_capture::camera`): the camera nodes
//! `WirePlumber` makes of every V4L2 (and libcamera) device, opened by their stable `node.name`.
//! When the session's PipeWire shows none (a sandbox, or an access policy that hides cameras from
//! plain clients) the xdg Camera portal hands a PipeWire connection that sees them.
//!
//! No direct V4L2 path: PipeWire already reads V4L2 underneath on every desktop the client
//! targets, and opening `/dev/video*` beside it would fight `WirePlumber` for the device (busy).
//!
//! Pictures stay in system memory (MJPEG decoded by FFmpeg on the CPU, YUY2 and NV12 copied by
//! Vesper's PipeWire thread), at most 1280 × 720. A thread of ours takes the newest one (Vesper
//! hands over latest-wins) and converts it to limited-range I420 for the encoder, so PipeWire's
//! callback never converts.

use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use vesper_capture::camera::{
    Camera, CameraFrame, CameraInfo, CpuFrame, CpuLayout, OpenOptions, Range, cameras, cameras_on,
    portal_remote,
};

use crate::camera::{
    CameraDescription, CameraError, Cameras, PREFERRED_CAMERA_SIZE, RunningCapture, SystemCameras,
};
use crate::camera_conversion::{SampleRange, i420_from_nv12, i420_from_yuy2};
use crate::video_frame::{CapturedFrame, CapturedPixelFormat};
use crate::video_sender::VideoInput;

/// How long `open` waits for the first picture. Vesper gives up by itself after 10 s when the
/// camera never starts; this only bounds a camera that starts and then sends nothing.
const FIRST_PICTURE_TIMEOUT: Duration = Duration::from_secs(12);
/// How often the conversion thread looks whether it must stop while no picture comes.
const STOP_CHECK_PERIOD: Duration = Duration::from_millis(100);

#[allow(clippy::needless_pass_by_value)] // the shape `map_err` hands over
fn system_error(error: vesper_capture::Error) -> CameraError {
    CameraError::System(error.to_string())
}

/// What a camera stream's end says, in our terms: the kernel's "Device or resource busy" when
/// another application holds the device.
fn error_of_ended_stream(reason: &str) -> CameraError {
    if reason.to_ascii_lowercase().contains("busy") {
        CameraError::Busy
    } else {
        CameraError::System(reason.to_owned())
    }
}

fn description_of(camera: CameraInfo) -> CameraDescription {
    CameraDescription {
        id: camera.node_name,
        name: camera.description,
    }
}

/// The node of `device_id` when listed, else the first one.
fn chosen_node(listed: &[CameraInfo], device_id: Option<&str>) -> Option<String> {
    listed
        .iter()
        .find(|camera| Some(camera.node_name.as_str()) == device_id)
        .or_else(|| listed.first())
        .map(|camera| camera.node_name.clone())
}

/// The Camera portal's PipeWire connection and the cameras it shows; `None` when the portal has
/// none. A refusal is the user's.
fn cameras_through_the_portal() -> Result<Option<(OwnedFd, Vec<CameraInfo>)>, CameraError> {
    let remote = match portal_remote() {
        Ok(Some(remote)) => remote,
        Ok(None) => return Ok(None),
        Err(error)
            if vesper_capture::is_refused(&error) || vesper_capture::is_cancelled(&error) =>
        {
            return Err(CameraError::PermissionDenied);
        }
        Err(error) => {
            // No portal (a bare window manager): the session's empty list stands.
            tracing::debug!(%error, "the camera portal is unavailable");
            return Ok(None);
        }
    };
    let listing_remote = remote
        .try_clone()
        .map_err(|error| CameraError::System(error.to_string()))?;
    let listed = cameras_on(Some(listing_remote)).map_err(system_error)?;
    Ok(Some((remote, listed)))
}

/// The packed I420 picture of a Vesper picture in system memory.
fn captured_frame_of(frame: &CpuFrame) -> Option<CapturedFrame> {
    // shortcut: the BT.709 matrix a few raw cameras declare is read as BT.601 (a faint hue
    // shift); convert the matrix if a camera shows it.
    let range = match frame.range {
        Range::Limited => SampleRange::Limited,
        Range::Full => SampleRange::Full,
    };
    let plane = |index: usize| {
        let plane = frame.planes.get(index)?;
        Some((frame.data.get(plane.offset..)?, plane.stride))
    };
    let bytes = match frame.layout {
        CpuLayout::Yuy2 => i420_from_yuy2(frame.width, frame.height, plane(0)?, range),
        CpuLayout::Nv12 => i420_from_nv12(frame.width, frame.height, plane(0)?, plane(1)?, range),
    }?;
    Some(CapturedFrame {
        width: frame.width,
        height: frame.height,
        format: CapturedPixelFormat::I420,
        bytes,
    })
}

fn push_picture(frame: &CameraFrame, frames: &VideoInput) {
    match frame {
        CameraFrame::Cpu(frame) => {
            if let Some(captured) = captured_frame_of(frame) {
                frames.push(captured);
            } else {
                tracing::warn!("a camera picture does not match its size; dropped");
            }
        }
        // Never asked for (`decode_on_gpu: false`).
        CameraFrame::Gpu(_) => {}
    }
}

/// Converts and hands over the newest picture until stopped, the input closes or the camera ends.
fn pump_pictures(
    camera: &Camera,
    first: &CameraFrame,
    frames: &VideoInput,
    should_stop: &AtomicBool,
    has_ended: &AtomicBool,
) {
    push_picture(first, frames);
    while !should_stop.load(Ordering::Relaxed) && frames.is_open() {
        match camera.wait_take(STOP_CHECK_PERIOD) {
            Ok(Some(frame)) => push_picture(&frame, frames),
            Ok(None) => {}
            Err(reason) => {
                tracing::warn!(%reason, "the camera stopped");
                has_ended.store(true, Ordering::Relaxed);
                return;
            }
        }
    }
}

struct PipeWireCapture {
    should_stop: Arc<AtomicBool>,
    has_ended: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RunningCapture for PipeWireCapture {
    fn has_ended(&self) -> bool {
        self.has_ended.load(Ordering::Relaxed)
    }
}

impl Drop for PipeWireCapture {
    fn drop(&mut self) {
        self.should_stop.store(true, Ordering::Relaxed);
        // Waits at most one stop check; dropping the camera there never waits on PipeWire.
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the camera's conversion thread panicked");
        }
    }
}

impl Cameras for SystemCameras {
    fn cameras(&self) -> Result<Vec<CameraDescription>, CameraError> {
        let listed = cameras().map_err(system_error)?;
        if !listed.is_empty() {
            return Ok(listed.into_iter().map(description_of).collect());
        }
        Ok(cameras_through_the_portal()?
            .map(|(_, listed)| listed.into_iter().map(description_of).collect())
            .unwrap_or_default())
    }

    fn open(
        &self,
        device_id: Option<&str>,
        frames: VideoInput,
    ) -> Result<Box<dyn RunningCapture>, CameraError> {
        let listed = cameras().map_err(system_error)?;
        let (node, remote) = if let Some(node) = chosen_node(&listed, device_id) {
            (node, None)
        } else {
            let (remote, listed) = cameras_through_the_portal()?.ok_or(CameraError::NoCamera)?;
            let node = chosen_node(&listed, device_id).ok_or(CameraError::NoCamera)?;
            (node, Some(remote))
        };
        let options = OpenOptions {
            remote,
            max_size: PREFERRED_CAMERA_SIZE,
            decode_on_gpu: false,
        };
        let camera = Camera::open_with(&node, options).map_err(system_error)?;
        // The first picture says the camera works: a busy or broken one fails here, where the
        // caller can tell the user, rather than leaving a camera that never sends.
        let deadline = Instant::now() + FIRST_PICTURE_TIMEOUT;
        let first = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match camera.wait_take(left) {
                Ok(Some(frame)) => break frame,
                Ok(None) if !left.is_zero() => {}
                Ok(None) => return Err(CameraError::System("the camera sent no picture".into())),
                Err(reason) => return Err(error_of_ended_stream(&reason.to_string())),
            }
        };
        let should_stop = Arc::new(AtomicBool::new(false));
        let has_ended = Arc::new(AtomicBool::new(false));
        let thread = {
            let (should_stop, has_ended) = (Arc::clone(&should_stop), Arc::clone(&has_ended));
            std::thread::Builder::new()
                .name("mellow-camera".into())
                .spawn(move || pump_pictures(&camera, &first, &frames, &should_stop, &has_ended))
                .map_err(|error| CameraError::System(error.to_string()))?
        };
        Ok(Box::new(PipeWireCapture {
            should_stop,
            has_ended,
            thread: Some(thread),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vesper_capture::camera::{CpuPlane, Matrix};

    fn camera(node_name: &str) -> CameraInfo {
        CameraInfo {
            node_name: node_name.into(),
            description: format!("{node_name} camera"),
        }
    }

    #[test]
    fn the_chosen_camera_opens_else_the_first_listed() {
        let listed = [camera("front"), camera("desk")];
        assert_eq!(chosen_node(&listed, Some("desk")).as_deref(), Some("desk"));
        assert_eq!(chosen_node(&listed, Some("gone")).as_deref(), Some("front"));
        assert_eq!(chosen_node(&listed, None).as_deref(), Some("front"));
        assert_eq!(chosen_node(&[], None), None);
    }

    #[test]
    fn a_busy_device_reads_as_busy() {
        assert!(matches!(
            error_of_ended_stream("camera stream error: Device or resource busy"),
            CameraError::Busy
        ));
        assert!(matches!(
            error_of_ended_stream("camera unplugged"),
            CameraError::System(_)
        ));
    }

    #[test]
    fn vesper_pictures_become_packed_i420() {
        // A 2 × 2 MJPEG picture as the CPU decoder repacks it: NV12, full range.
        let frame = CpuFrame {
            width: 2,
            height: 2,
            layout: CpuLayout::Nv12,
            planes: vec![
                CpuPlane {
                    offset: 0,
                    stride: 2,
                },
                CpuPlane {
                    offset: 4,
                    stride: 2,
                },
            ],
            data: vec![0, 255, 0, 255, 128, 128],
            matrix: Matrix::Bt601,
            range: Range::Full,
            pts: Duration::ZERO,
        };
        let captured = captured_frame_of(&frame).unwrap();
        assert_eq!(captured.format, CapturedPixelFormat::I420);
        assert_eq!(captured.bytes, [16, 235, 16, 235, 128, 128]);
        // A plane past the data is refused, not read.
        let broken = CpuFrame {
            planes: vec![
                CpuPlane {
                    offset: 0,
                    stride: 2,
                },
                CpuPlane {
                    offset: 9,
                    stride: 2,
                },
            ],
            ..frame
        };
        assert!(captured_frame_of(&broken).is_none());
    }
}
