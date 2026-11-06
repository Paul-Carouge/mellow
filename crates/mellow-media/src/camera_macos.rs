//! macOS cameras through `AVFoundation` (`objc2` bindings, the same family as GPUI's own): a
//! capture session delivering NV12 (video range) pictures on a dispatch queue, copied into the
//! session's [`VideoInput`]. The system asks the user for camera access the first time.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AllocAnyThread as _, DefinedClass as _, define_class, msg_send};
use objc2_av_foundation::{
    AVAuthorizationStatus, AVCaptureConnection, AVCaptureDevice, AVCaptureDeviceInput,
    AVCaptureOutput, AVCaptureSession, AVCaptureSessionPreset1280x720, AVCaptureVideoDataOutput,
    AVCaptureVideoDataOutputSampleBufferDelegate, AVMediaTypeVideo,
};
use objc2_core_media::CMSampleBuffer;
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane,
    CVPixelBufferGetHeightOfPlane, CVPixelBufferGetPixelFormatType, CVPixelBufferGetPlaneCount,
    CVPixelBufferGetWidthOfPlane, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress, kCVPixelBufferPixelFormatTypeKey,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVReturnSuccess,
};
use objc2_foundation::{NSDictionary, NSNumber, NSObject, NSObjectProtocol, NSString};

use crate::camera::{CameraDescription, CameraError, Cameras, RunningCapture, SystemCameras};
use crate::video_frame::{CapturedFrame, CapturedPixelFormat};
use crate::video_sender::VideoInput;

/// What the capture callback needs.
struct DelegateState {
    frames: VideoInput,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; the class adds no `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "MellowCameraFrameDelegate"]
    #[ivars = DelegateState]
    struct FrameDelegate;

    // SAFETY: `NSObjectProtocol` has no extra requirements.
    unsafe impl NSObjectProtocol for FrameDelegate {}

    // SAFETY: the method's signature matches the protocol's.
    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for FrameDelegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        fn capture_output_did_output_sample_buffer(
            &self,
            _output: &AVCaptureOutput,
            sample_buffer: &CMSampleBuffer,
            _connection: &AVCaptureConnection,
        ) {
            let frames = &self.ivars().frames;
            if frames.is_open()
                && let Some(frame) = copy_nv12_picture(sample_buffer)
            {
                frames.push(frame);
            }
        }
    }
);

impl FrameDelegate {
    fn new(frames: VideoInput) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateState { frames });
        // SAFETY: `init` is `NSObject`'s designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

/// Copies the sample's NV12 planes, packed, into a frame. `None` for any other layout.
fn copy_nv12_picture(sample_buffer: &CMSampleBuffer) -> Option<CapturedFrame> {
    // SAFETY: the sample buffer is valid for this callback; the image buffer is retained.
    let image = unsafe { sample_buffer.image_buffer() }?;
    let pixels: &CVPixelBuffer = &image;
    if CVPixelBufferGetPixelFormatType(pixels) != kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        || CVPixelBufferGetPlaneCount(pixels) != 2
    {
        return None;
    }
    // SAFETY: a valid pixel buffer, locked read-only and unlocked below on every path.
    if unsafe { CVPixelBufferLockBaseAddress(pixels, CVPixelBufferLockFlags::ReadOnly) }
        != kCVReturnSuccess
    {
        return None;
    }
    let frame = copy_locked_planes(pixels);
    // SAFETY: locked just above with the same flags.
    unsafe { CVPixelBufferUnlockBaseAddress(pixels, CVPixelBufferLockFlags::ReadOnly) };
    frame
}

fn copy_locked_planes(pixels: &CVPixelBuffer) -> Option<CapturedFrame> {
    let width = CVPixelBufferGetWidthOfPlane(pixels, 0);
    let height = CVPixelBufferGetHeightOfPlane(pixels, 0);
    let frame_width = u32::try_from(width).ok()?;
    let frame_height = u32::try_from(height).ok()?;
    let mut bytes = Vec::with_capacity(CapturedFrame::expected_length(
        frame_width,
        frame_height,
        CapturedPixelFormat::Nv12,
    ));
    // Luma rows of `width` bytes, then interleaved chroma rows of `2 × ⌈width / 2⌉` bytes.
    for (plane, row_bytes, rows) in [
        (0, width, height),
        (1, 2 * width.div_ceil(2), height.div_ceil(2)),
    ] {
        let base = CVPixelBufferGetBaseAddressOfPlane(pixels, plane).cast::<u8>();
        let stride = CVPixelBufferGetBytesPerRowOfPlane(pixels, plane);
        if base.is_null()
            || stride < row_bytes
            || CVPixelBufferGetHeightOfPlane(pixels, plane) < rows
        {
            return None;
        }
        // SAFETY: the buffer is locked; the plane holds `rows` rows of `stride` bytes
        // (checked against the buffer's own plane height and stride above).
        let plane_bytes = unsafe { std::slice::from_raw_parts(base, stride * rows) };
        for row in plane_bytes.chunks(stride) {
            bytes.extend_from_slice(row.get(..row_bytes)?);
        }
    }
    Some(CapturedFrame {
        width: frame_width,
        height: frame_height,
        format: CapturedPixelFormat::Nv12,
        bytes,
    })
}

/// A running capture session; dropping it stops the camera.
struct AvFoundationCapture {
    session: Retained<AVCaptureSession>,
    output: Retained<AVCaptureVideoDataOutput>,
    // Kept alive while the output may still call it.
    _delegate: Retained<FrameDelegate>,
}

impl RunningCapture for AvFoundationCapture {}

impl Drop for AvFoundationCapture {
    fn drop(&mut self) {
        // SAFETY: plain AVFoundation calls on objects this capture owns.
        unsafe {
            self.session.stopRunning();
            self.output.setSampleBufferDelegate_queue(None, None);
        }
    }
}

fn video_media_type() -> Result<&'static objc2_av_foundation::AVMediaType, CameraError> {
    // SAFETY: an AVFoundation constant, present on every macOS AVFoundation runs on.
    unsafe { AVMediaTypeVideo }.ok_or(CameraError::NoCamera)
}

fn all_cameras() -> Result<Vec<Retained<AVCaptureDevice>>, CameraError> {
    let media_type = video_media_type()?;
    // `devicesWithMediaType` is deprecated for the discovery session, whose device types differ
    // per macOS version (`External` needs macOS 14); this call works on every version.
    // SAFETY: a plain class method with a valid media type.
    #[allow(deprecated)]
    let devices = unsafe { AVCaptureDevice::devicesWithMediaType(media_type) };
    Ok(devices.to_vec())
}

impl Cameras for SystemCameras {
    fn cameras(&self) -> Result<Vec<CameraDescription>, CameraError> {
        Ok(all_cameras()?
            .iter()
            .map(|device| CameraDescription {
                // SAFETY: plain property reads.
                id: unsafe { device.uniqueID() }.to_string(),
                name: unsafe { device.localizedName() }.to_string(),
            })
            .collect())
    }

    fn open(
        &self,
        device_id: Option<&str>,
        frames: VideoInput,
    ) -> Result<Box<dyn RunningCapture>, CameraError> {
        let media_type = video_media_type()?;
        // SAFETY: a plain class method with a valid media type.
        let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media_type) };
        if status == AVAuthorizationStatus::Denied || status == AVAuthorizationStatus::Restricted {
            return Err(CameraError::PermissionDenied);
        }
        // SAFETY: plain class methods; the id is a valid string.
        let device = device_id
            .and_then(|id| unsafe { AVCaptureDevice::deviceWithUniqueID(&NSString::from_str(id)) })
            .or_else(|| unsafe { AVCaptureDevice::defaultDeviceWithMediaType(media_type) })
            .ok_or(CameraError::NoCamera)?;
        // SAFETY: a valid device; failure is returned as an error.
        let input = unsafe { AVCaptureDeviceInput::deviceInputWithDevice_error(&device) }
            .map_err(|error| CameraError::System(error.localizedDescription().to_string()))?;
        let delegate = FrameDelegate::new(frames);
        // SAFETY: the session is configured between begin and commit, from this thread only,
        // with objects created here; the delegate outlives the output's use of it (dropped
        // after `setSampleBufferDelegate_queue(None, None)`).
        unsafe {
            let session = AVCaptureSession::new();
            session.beginConfiguration();
            if session.canSetSessionPreset(AVCaptureSessionPreset1280x720) {
                session.setSessionPreset(AVCaptureSessionPreset1280x720);
            }
            if !session.canAddInput(&input) {
                session.commitConfiguration();
                return Err(CameraError::System("the camera cannot be added".into()));
            }
            session.addInput(&input);
            let output = AVCaptureVideoDataOutput::new();
            // `kCVPixelBufferPixelFormatTypeKey` is a CFString, toll-free bridged to NSString.
            let key: &NSString =
                &*std::ptr::from_ref(kCVPixelBufferPixelFormatTypeKey).cast::<NSString>();
            let format =
                NSNumber::numberWithUnsignedInt(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange);
            let format: &AnyObject = &format;
            let settings = NSDictionary::from_slices(&[key], &[format]);
            output.setVideoSettings(Some(&settings));
            output.setAlwaysDiscardsLateVideoFrames(true);
            let queue = dispatch2::DispatchQueue::new("fr.mellow.camera", None);
            output.setSampleBufferDelegate_queue(
                Some(ProtocolObject::from_ref(&*delegate)),
                Some(&queue),
            );
            if !session.canAddOutput(&output) {
                session.commitConfiguration();
                return Err(CameraError::System(
                    "the camera output cannot be added".into(),
                ));
            }
            session.addOutput(&output);
            session.commitConfiguration();
            session.startRunning();
            Ok(Box::new(AvFoundationCapture {
                session,
                output,
                _delegate: delegate,
            }))
        }
    }
}
