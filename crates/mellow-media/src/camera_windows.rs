//! Windows cameras through Media Foundation: the video capture devices `MFEnumDeviceSources`
//! lists, read by a synchronous source reader on a thread of its own, which converts whatever
//! the camera sends (YUY2, MJPEG, NV12) to NV12 and copies it, packed, into a reused buffer for
//! the session's [`VideoInput`]. The camera mode closest to 1280 × 720 at 30 frames per second
//! is chosen, as the web client asks.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;

use windows::Win32::Foundation::E_ACCESSDENIED;
use windows::Win32::Media::MediaFoundation::{
    IMF2DBuffer2, IMFActivate, IMFAttributes, IMFMediaSource, IMFMediaType, IMFSample,
    IMFSourceReader, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, MF_E_HW_MFT_FAILED_START_STREAMING,
    MF_E_VIDEO_RECORDING_DEVICE_PREEMPTED, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
    MF_MT_SUBTYPE, MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR, MF2DBuffer_LockFlags_Read,
    MFCreateAttributes, MFCreateMediaType, MFCreateSourceReaderFromMediaSource,
    MFEnumDeviceSources, MFMediaType_Video, MFVideoFormat_NV12,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::core::{Interface, PWSTR};

use crate::camera::{
    CameraDescription, CameraError, Cameras, PREFERRED_CAMERA_SIZE, RunningCapture, SystemCameras,
};
use crate::simulcast::FRAMES_PER_SECOND;
use crate::video_frame::{CapturedFrame, CapturedPixelFormat};
use crate::video_sender::VideoInput;
use crate::windows_media_foundation::ensure_media_foundation;

/// The source reader's stream index for the first video stream (a negative constant).
const FIRST_VIDEO_STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0.cast_unsigned();

fn system_error(error: &windows::core::Error) -> CameraError {
    match error.code() {
        // The camera privacy setting refuses Mellow (or every desktop application).
        code if code == E_ACCESSDENIED => CameraError::PermissionDenied,
        code if code == MF_E_VIDEO_RECORDING_DEVICE_PREEMPTED
            || code == MF_E_HW_MFT_FAILED_START_STREAMING =>
        {
            CameraError::Busy
        }
        _ => CameraError::System(error.message()),
    }
}

/// A string attribute (a name, a symbolic link) of a device.
fn string_attribute(attributes: &IMFAttributes, key: &windows::core::GUID) -> Option<String> {
    let mut value = PWSTR::null();
    let mut length = 0;
    // SAFETY: Media Foundation allocates the string; it is copied, then freed once.
    unsafe {
        attributes
            .GetAllocatedString(key, &raw mut value, &raw mut length)
            .ok()?;
        let text = value.to_string().ok();
        CoTaskMemFree(Some(value.0.cast_const().cast()));
        text
    }
}

/// The video capture devices, in the system's order (the first is the default).
fn capture_devices() -> Result<Vec<IMFActivate>, CameraError> {
    ensure_media_foundation().map_err(CameraError::System)?;
    // SAFETY: the attributes are created here; the returned array holds `count` owned
    // activation objects, each moved out once, then the array itself is freed once.
    unsafe {
        let mut attributes = None;
        MFCreateAttributes(&raw mut attributes, 1).map_err(|error| system_error(&error))?;
        let attributes = attributes.ok_or(CameraError::NoCamera)?;
        attributes
            .SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )
            .map_err(|error| system_error(&error))?;
        let mut array: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0_u32;
        MFEnumDeviceSources(&attributes, &raw mut array, &raw mut count)
            .map_err(|error| system_error(&error))?;
        if array.is_null() {
            return Ok(Vec::new());
        }
        let devices = (0..count as usize)
            .filter_map(|index| array.add(index).replace(None))
            .collect();
        CoTaskMemFree(Some(array.cast_const().cast()));
        Ok(devices)
    }
}

fn describe(device: &IMFActivate) -> Option<CameraDescription> {
    Some(CameraDescription {
        id: string_attribute(
            device,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
        )?,
        name: string_attribute(device, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME).unwrap_or_default(),
    })
}

/// Two 32-bit halves of a Media Foundation size or ratio attribute.
fn halves(value: u64) -> (u32, u32) {
    // Each half is 32 bits by construction.
    #[allow(clippy::cast_possible_truncation)]
    ((value >> 32) as u32, value as u32)
}

/// How far a camera mode is from the wanted 1280 × 720 at 30 frames per second: the size first
/// (in pixels, larger modes cost more than smaller ones lose), then a rate below 30.
fn mode_distance(width: u32, height: u32, frames_per_second: f64) -> (u64, bool) {
    let wanted = u64::from(PREFERRED_CAMERA_SIZE.0) * u64::from(PREFERRED_CAMERA_SIZE.1);
    let area = u64::from(width) * u64::from(height);
    let size_distance = if area > wanted {
        (area - wanted) * 2
    } else {
        wanted - area
    };
    (
        size_distance,
        frames_per_second < f64::from(FRAMES_PER_SECOND) - 1.0,
    )
}

/// The native mode closest to the wanted one.
fn closest_native_mode(reader: &IMFSourceReader) -> Option<IMFMediaType> {
    let mut best: Option<((u64, bool), IMFMediaType)> = None;
    for index in 0.. {
        // SAFETY: plain calls on a valid reader; the loop ends at the first missing index.
        let Ok(mode) = (unsafe { reader.GetNativeMediaType(FIRST_VIDEO_STREAM, index) }) else {
            break;
        };
        // SAFETY: plain attribute reads.
        let (size, rate) = unsafe {
            (
                mode.GetUINT64(&MF_MT_FRAME_SIZE),
                mode.GetUINT64(&MF_MT_FRAME_RATE),
            )
        };
        let Ok(size) = size else { continue };
        let (width, height) = halves(size);
        let frames_per_second = rate.ok().map_or(0.0, |rate| {
            let (numerator, denominator) = halves(rate);
            f64::from(numerator) / f64::from(denominator.max(1))
        });
        let distance = mode_distance(width, height, frames_per_second);
        if best.as_ref().is_none_or(|(best, _)| distance < *best) {
            best = Some((distance, mode));
        }
    }
    best.map(|(_, mode)| mode)
}

/// The pictures' layout: width, height and bytes per luma row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Nv12Layout {
    width: u32,
    height: u32,
    stride: usize,
}

/// The current pictures' width and height.
fn current_size(reader: &IMFSourceReader) -> Option<(u32, u32)> {
    // SAFETY: plain reads on a valid reader's current type.
    unsafe {
        let current = reader.GetCurrentMediaType(FIRST_VIDEO_STREAM).ok()?;
        Some(halves(current.GetUINT64(&MF_MT_FRAME_SIZE).ok()?))
    }
}

/// Packs an NV12 picture with `layout.stride` bytes per row (luma rows, then chroma rows) into
/// `bytes` (reused), as [`CapturedPixelFormat::Nv12`] wants it. `None` when `source` is too
/// short for the layout.
fn pack_nv12(mut bytes: Vec<u8>, layout: Nv12Layout, source: &[u8]) -> Option<CapturedFrame> {
    let width = usize::try_from(layout.width).ok()?;
    let height = usize::try_from(layout.height).ok()?;
    let chroma_row = 2 * width.div_ceil(2);
    let chroma_rows = height.div_ceil(2);
    if width == 0 || height == 0 || layout.stride < chroma_row {
        return None;
    }
    let chroma_start = layout.stride.checked_mul(height)?;
    let needed = chroma_start
        .checked_add(layout.stride.checked_mul(chroma_rows - 1)?)?
        .checked_add(chroma_row)?;
    if source.len() < needed {
        return None;
    }
    bytes.clear();
    bytes.reserve(CapturedFrame::expected_length(
        layout.width,
        layout.height,
        CapturedPixelFormat::Nv12,
    ));
    for row in source[..chroma_start].chunks(layout.stride) {
        bytes.extend_from_slice(row.get(..width)?);
    }
    for row in source[chroma_start..]
        .chunks(layout.stride)
        .take(chroma_rows)
    {
        bytes.extend_from_slice(row.get(..chroma_row)?);
    }
    Some(CapturedFrame {
        width: layout.width,
        height: layout.height,
        format: CapturedPixelFormat::Nv12,
        bytes,
    })
}

/// Opens `device` (or the first camera) as an NV12 source reader.
fn open_reader(device_id: Option<&str>) -> Result<IMFSourceReader, CameraError> {
    let devices = capture_devices()?;
    let device = device_id
        .and_then(|id| {
            devices
                .iter()
                .find(|device| describe(device).is_some_and(|described| described.id == id))
        })
        .or_else(|| devices.first())
        .ok_or(CameraError::NoCamera)?;
    // SAFETY: plain Media Foundation calls on objects created here, on this thread.
    unsafe {
        let source: IMFMediaSource = device
            .ActivateObject()
            .map_err(|error| system_error(&error))?;
        let mut attributes = None;
        MFCreateAttributes(&raw mut attributes, 1).map_err(|error| system_error(&error))?;
        let attributes = attributes.ok_or(CameraError::NoCamera)?;
        // Lets the reader decode MJPEG and convert YUY2 to the NV12 asked below.
        attributes
            .SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)
            .map_err(|error| system_error(&error))?;
        let reader = MFCreateSourceReaderFromMediaSource(&source, &attributes)
            .map_err(|error| system_error(&error))?;
        let native = closest_native_mode(&reader).ok_or(CameraError::NoCamera)?;
        reader
            .SetCurrentMediaType(FIRST_VIDEO_STREAM, None, &native)
            .map_err(|error| system_error(&error))?;
        // A fresh type, not a copy of the native one: the native stride and sample size (a YUY2
        // mode's, twice the width) would not describe NV12 pictures.
        let output = MFCreateMediaType().map_err(|error| system_error(&error))?;
        output
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .and_then(|()| output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12))
            .and_then(|()| {
                output.SetUINT64(&MF_MT_FRAME_SIZE, native.GetUINT64(&MF_MT_FRAME_SIZE)?)
            })
            .map_err(|error| system_error(&error))?;
        // A mode without a rate leaves it to the reader.
        if let Ok(rate) = native.GetUINT64(&MF_MT_FRAME_RATE) {
            output
                .SetUINT64(&MF_MT_FRAME_RATE, rate)
                .map_err(|error| system_error(&error))?;
        }
        reader
            .SetCurrentMediaType(FIRST_VIDEO_STREAM, None, &output)
            .map_err(|error| system_error(&error))?;
        Ok(reader)
    }
}

/// What a running camera's thread shares with it.
struct CaptureState {
    should_stop: AtomicBool,
    has_ended: AtomicBool,
}

/// One synchronous read: the stream's flags and the picture, if any.
fn read_sample(reader: &IMFSourceReader) -> windows::core::Result<(i32, Option<IMFSample>)> {
    let mut flags = 0_u32;
    let mut sample = None;
    // SAFETY: a synchronous read on a valid reader into owned out values.
    unsafe {
        reader.ReadSample(
            FIRST_VIDEO_STREAM,
            0,
            None,
            Some(&raw mut flags),
            None,
            Some(&raw mut sample),
        )?;
    }
    Ok((flags.cast_signed(), sample))
}

fn has_stream_stopped(flags: i32) -> bool {
    flags & (MF_SOURCE_READERF_ERROR.0 | MF_SOURCE_READERF_ENDOFSTREAM.0) != 0
}

/// Copies `sample`'s NV12 picture of `size` into `bytes` (reused), with the row pitch its buffer
/// really has: a 2-D buffer's own, else the width (`Lock` gives packed rows).
fn copy_picture(
    sample: &IMFSample,
    (width, height): (u32, u32),
    bytes: Vec<u8>,
) -> Option<CapturedFrame> {
    // SAFETY: each buffer is locked while its bytes are copied, then unlocked; the slice read
    // from a 2-D lock ends at the end of the buffer Windows reports.
    unsafe {
        let buffer = sample.ConvertToContiguousBuffer().ok()?;
        if let Ok(planes) = buffer.cast::<IMF2DBuffer2>() {
            let mut first_row = std::ptr::null_mut();
            let mut pitch = 0_i32;
            let mut start = std::ptr::null_mut();
            let mut length = 0_u32;
            if planes
                .Lock2DSize(
                    MF2DBuffer_LockFlags_Read,
                    &raw mut first_row,
                    &raw mut pitch,
                    &raw mut start,
                    &raw mut length,
                )
                .is_ok()
            {
                // A negative pitch (bottom-up) never comes with NV12: refused.
                let picture = usize::try_from(pitch)
                    .ok()
                    .filter(|_| !first_row.is_null())
                    .and_then(|stride| {
                        let offset = first_row.addr().checked_sub(start.addr())?;
                        let available = (length as usize).checked_sub(offset)?;
                        let source = std::slice::from_raw_parts(first_row, available);
                        pack_nv12(
                            bytes,
                            Nv12Layout {
                                width,
                                height,
                                stride,
                            },
                            source,
                        )
                    });
                let _ = planes.Unlock2D();
                return picture;
            }
        }
        let mut source = std::ptr::null_mut();
        let mut length = 0_u32;
        buffer
            .Lock(&raw mut source, None, Some(&raw mut length))
            .ok()?;
        let layout = usize::try_from(width).ok().map(|stride| Nv12Layout {
            width,
            height,
            stride,
        });
        let picture = layout.filter(|_| !source.is_null()).and_then(|layout| {
            pack_nv12(
                bytes,
                layout,
                std::slice::from_raw_parts(source, length as usize),
            )
        });
        let _ = buffer.Unlock();
        picture
    }
}

/// Sends pictures, starting with the `first` read, until stopped, the input closes, or the
/// camera goes away.
fn read_pictures(
    reader: &IMFSourceReader,
    first: (i32, Option<IMFSample>),
    frames: &VideoInput,
    state: &CaptureState,
) {
    let mut size = current_size(reader);
    let mut has_logged_a_refused_picture = false;
    let mut read = first;
    loop {
        let (flags, sample) = read;
        if has_stream_stopped(flags) {
            // Unplugged, taken by the system, or turned off by its privacy switch.
            state.has_ended.store(true, Ordering::Relaxed);
            return;
        }
        if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 != 0 {
            size = current_size(reader);
        }
        if let (Some(sample), Some(size)) = (sample, size) {
            match copy_picture(&sample, size, frames.recycled_bytes()) {
                Some(picture) => frames.push(picture),
                None if !has_logged_a_refused_picture => {
                    has_logged_a_refused_picture = true;
                    tracing::debug!(?size, "a camera picture did not match its NV12 layout");
                }
                None => {}
            }
        }
        if state.should_stop.load(Ordering::Relaxed) || !frames.is_open() {
            return;
        }
        let Ok(next) = read_sample(reader) else {
            state.has_ended.store(true, Ordering::Relaxed);
            return;
        };
        read = next;
    }
}

/// A reader and its first read.
type StartedReader = (IMFSourceReader, (i32, Option<IMFSample>));

/// Opens the reader and reads its first picture: Media Foundation starts the stream there, and
/// a privacy refusal or a busy camera often shows only then.
fn start_reading(device_id: Option<&str>) -> Result<StartedReader, CameraError> {
    let reader = open_reader(device_id)?;
    let first = read_sample(&reader).map_err(|error| system_error(&error))?;
    if has_stream_stopped(first.0) {
        return Err(CameraError::System(
            "the camera stopped as it started".into(),
        ));
    }
    Ok((reader, first))
}

struct MediaFoundationCapture {
    state: Arc<CaptureState>,
    thread: Option<JoinHandle<()>>,
}

impl RunningCapture for MediaFoundationCapture {
    fn has_ended(&self) -> bool {
        self.state.has_ended.load(Ordering::Relaxed)
    }
}

impl Drop for MediaFoundationCapture {
    fn drop(&mut self) {
        self.state.should_stop.store(true, Ordering::Relaxed);
        // The reader returns within a picture's time (or at once once the camera is gone).
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the Windows camera's thread panicked");
        }
    }
}

impl Cameras for SystemCameras {
    fn cameras(&self) -> Result<Vec<CameraDescription>, CameraError> {
        Ok(capture_devices()?.iter().filter_map(describe).collect())
    }

    fn open(
        &self,
        device_id: Option<&str>,
        frames: VideoInput,
    ) -> Result<Box<dyn RunningCapture>, CameraError> {
        let state = Arc::new(CaptureState {
            should_stop: AtomicBool::new(false),
            has_ended: AtomicBool::new(false),
        });
        let (opened, has_opened) = mpsc::sync_channel(1);
        let device_id = device_id.map(str::to_owned);
        let thread = {
            let state = Arc::clone(&state);
            // The reader is made, used and released on this thread (COM's apartment rules).
            std::thread::Builder::new()
                .name("mellow-camera-windows".into())
                .spawn(move || match start_reading(device_id.as_deref()) {
                    Ok((reader, first)) => {
                        let _ = opened.send(Ok(()));
                        read_pictures(&reader, first, &frames, &state);
                    }
                    Err(error) => {
                        let _ = opened.send(Err(error));
                    }
                })
                .map_err(|error| CameraError::System(error.to_string()))?
        };
        let capture = MediaFoundationCapture {
            state,
            thread: Some(thread),
        };
        match has_opened.recv() {
            Ok(Ok(())) => Ok(Box::new(capture)),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(CameraError::System("the camera's thread stopped".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_nv12_rows_are_packed() {
        // 2 × 2 pixels, rows padded to 4 bytes.
        let source = [1, 2, 0, 0, 3, 4, 0, 0, 100, 200, 0, 0];
        let layout = Nv12Layout {
            width: 2,
            height: 2,
            stride: 4,
        };
        let picture = pack_nv12(Vec::new(), layout, &source).unwrap();
        assert_eq!(picture.bytes, [1, 2, 3, 4, 100, 200]);
        assert_eq!(
            picture.bytes.len(),
            CapturedFrame::expected_length(2, 2, CapturedPixelFormat::Nv12)
        );
        assert!(picture.to_i420().is_some());
        assert!(pack_nv12(Vec::new(), layout, &source[..9]).is_none());
    }

    #[test]
    fn the_mode_closest_to_720p_at_30_frames_wins() {
        let wanted = mode_distance(1_280, 720, 30.0);
        assert!(wanted < mode_distance(1_920, 1_080, 30.0));
        assert!(wanted < mode_distance(640, 480, 30.0));
        assert!(wanted < mode_distance(1_280, 720, 15.0));
        assert!(mode_distance(1_280, 720, 15.0) < mode_distance(1_920, 1_080, 30.0));
    }

    #[test]
    fn the_listing_answers_without_a_camera() {
        // The CI runner has no camera: the listing must still answer, empty or not.
        let cameras = SystemCameras.cameras();
        assert!(
            cameras.is_ok() || matches!(cameras, Err(CameraError::System(_))),
            "{cameras:?}"
        );
    }
}
