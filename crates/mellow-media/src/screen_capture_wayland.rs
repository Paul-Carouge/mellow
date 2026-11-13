//! Screen sharing under Wayland (ADR-0054): the ScreenCast portal's dialog is the picker, and
//! PipeWire streams the chosen screen or window through Vesper's `vesper-capture`. When the GPU
//! encodes Constrained Baseline H.264 through VA-API, pictures come as DMA-BUFs and stay on the
//! GPU ([`crate::h264_encoder_linux`]); otherwise they come in shared memory and go through the
//! VP8 path. The restore token is never kept: the dialog shows at every share, as the web's.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use mellow_protocol::VideoCodec;
use vesper_capture::{
    Cancel, Capture, FrameMemory, MemoryFrame, ScreenFrame, SourceType, StartOptions, is_cancelled,
    is_refused,
};

use crate::h264_encoder_linux::{H264PictureSlot, H264ScreenEncoder};
use crate::screen_capture::{
    RunningScreenCapture, ScreenCaptureError, ScreenShareContext, ShareSound, ShareSoundState,
    pack_bgra_rows,
};
use crate::screen_capture_linux::{RunningShareSound, start_sound_for};
use crate::session::ScreenShareEnd;
use crate::simulcast::FRAMES_PER_SECOND;
use crate::video_sender::VideoInput;

/// DRM fourccs of the 8-bit RGB layouts shared memory may carry: blue first (`BGRx`, `BGRA`
/// in memory), or red first (`RGBx`, `RGBA`), which is swapped on copy.
const XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
const ARGB8888: u32 = u32::from_le_bytes(*b"AR24");
const XBGR8888: u32 = u32::from_le_bytes(*b"XB24");
const ABGR8888: u32 = u32::from_le_bytes(*b"AB24");

/// Where the PipeWire thread sends each picture, set once the portal answered and the share is
/// on: pictures before that are dropped.
enum PictureSink {
    Gpu(H264PictureSlot),
    Memory(VideoInput),
}

struct PortalCapture {
    // Dropped first: the stream stops before the sinks it feeds.
    _capture: Capture,
    _encoder: Option<H264ScreenEncoder>,
    _sound: Option<RunningShareSound>,
    sound: ShareSoundState,
}

impl RunningScreenCapture for PortalCapture {
    fn sound(&self) -> ShareSoundState {
        self.sound
    }
}

/// Copies a shared-memory picture into the session's recycled buffer and hands it over.
fn push_memory_frame(pictures: &VideoInput, frame: &MemoryFrame<'_>) {
    let is_red_first = match frame.fourcc {
        XRGB8888 | ARGB8888 => false,
        XBGR8888 | ABGR8888 => true,
        _ => return,
    };
    let Ok(stride) = usize::try_from(frame.stride) else {
        return;
    };
    if let Some(picture) = pack_bgra_rows(
        pictures.recycled_bytes(),
        frame.width,
        frame.height,
        stride,
        frame.pixels,
        is_red_first,
    ) {
        pictures.push(picture);
    }
}

fn error_of(error: &vesper_capture::Error) -> ScreenCaptureError {
    if is_cancelled(error) {
        ScreenCaptureError::Cancelled
    } else if is_refused(error) {
        ScreenCaptureError::PermissionDenied
    } else {
        ScreenCaptureError::System(error.to_string())
    }
}

/// Asks the portal for a screen or window and starts sharing it.
pub(crate) fn start(
    sound: &ShareSound,
    share: &ScreenShareContext,
) -> Result<Box<dyn RunningScreenCapture>, ScreenCaptureError> {
    let memory = if crate::h264_encoder_linux::can_encode_h264() {
        FrameMemory::DmaBuf
    } else {
        FrameMemory::Shared
    };
    let sink: Arc<OnceLock<PictureSink>> = Arc::default();
    // The stream may end before the share is on (no format agreed): `share.end` cannot tell it.
    let has_ended_early = Arc::new(AtomicBool::new(false));
    let options = StartOptions {
        sources: SourceType::Monitor | SourceType::Window,
        restore_token: None,
        persist: false,
        memory,
        max_fps: FRAMES_PER_SECOND,
    };
    let capture = {
        let sink = Arc::clone(&sink);
        let share = share.clone();
        let has_ended_early = Arc::clone(&has_ended_early);
        Capture::start_with_options(&options, &Cancel::default(), move |frame| match frame {
            Ok(ScreenFrame::DmaBuf(frame)) => {
                if let Some(PictureSink::Gpu(slot)) = sink.get() {
                    slot.offer(frame);
                }
            }
            Ok(ScreenFrame::Memory(frame)) => {
                if let Some(PictureSink::Memory(pictures)) = sink.get() {
                    push_memory_frame(pictures, &frame);
                }
            }
            Err(error) => {
                if sink.get().is_none() {
                    has_ended_early.store(true, Ordering::SeqCst);
                }
                // The window closed, the person stopped it from the compositor, or it failed.
                let reason = if error.to_string().contains("source closed") {
                    ScreenShareEnd::SourceClosed
                } else {
                    ScreenShareEnd::Failed(error.to_string())
                };
                share.end(reason);
            }
        })
        .map_err(|error| error_of(&error))?
    };
    let is_whole_screen = capture.source().source_type == Some(SourceType::Monitor);
    let (picture_sink, encoder) = match memory {
        FrameMemory::DmaBuf => {
            let input = share.start_encoded(VideoCodec::H264);
            let (encoder, slot) =
                crate::h264_encoder_linux::start(input, share.clone()).map_err(|error| {
                    share.stop();
                    ScreenCaptureError::System(error.to_string())
                })?;
            (PictureSink::Gpu(slot), Some(encoder))
        }
        FrameMemory::Shared => (PictureSink::Memory(share.start_pictures()), None),
    };
    // Set once, here: the PipeWire thread only reads it.
    let _ = sink.set(picture_sink);
    if has_ended_early.load(Ordering::SeqCst) {
        share.stop();
        return Err(ScreenCaptureError::System(
            "the screen's stream ended before its first picture".into(),
        ));
    }
    // The portal names no application for a window: its sound is not known.
    let (running_sound, sound_state) = start_sound_for(sound, is_whole_screen, None, share);
    Ok(Box::new(PortalCapture {
        _capture: capture,
        _encoder: encoder,
        _sound: running_sound,
        sound: sound_state,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulcast::{ScreenShareQuality, VideoSourceKind};
    use crate::video_sender::VideoEncoderControl;

    #[test]
    fn shared_memory_pictures_of_either_order_become_bgra_and_others_are_skipped() {
        let control = Arc::new(VideoEncoderControl::default());
        let pictures = control.start(VideoSourceKind::Screen(ScreenShareQuality::High));
        let pixels = [10, 20, 30, 255, 40, 50, 60, 255];
        let frame = |fourcc| MemoryFrame {
            width: 2,
            height: 1,
            stride: 8,
            fourcc,
            pixels: &pixels,
            pts: std::time::Duration::ZERO,
        };
        push_memory_frame(&pictures, &frame(XBGR8888));
        push_memory_frame(&pictures, &frame(u32::from_le_bytes(*b"AR30")));
        // The red-first picture is waiting, swapped; the 10-bit one never arrived.
        let waiting = control.take_waiting_picture_for_tests().unwrap();
        assert_eq!(waiting.bytes, [30, 20, 10, 255, 60, 50, 40, 255]);
    }
}
