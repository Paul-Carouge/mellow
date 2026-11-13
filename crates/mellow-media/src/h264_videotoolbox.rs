//! H.264 decoding on macOS through `VideoToolbox`: Annex B access units turned into AVCC samples
//! (4-byte lengths) in a reused buffer, a decompression session per sequence and picture
//! parameter set pair, and planar 4:2:0 pictures read in place while the buffer is locked.

use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::Mutex;

use objc2_core_foundation::{CFDictionary, CFRetained, kCFAllocatorNull};
use objc2_core_media::{
    CMBlockBuffer, CMFormatDescription, CMSampleBuffer, CMTime,
    CMVideoFormatDescriptionCreateFromH264ParameterSets, CMVideoFormatDescriptionGetDimensions,
};
use objc2_core_video::{
    CVImageBuffer, CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane,
    CVPixelBufferGetHeight, CVPixelBufferGetHeightOfPlane, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetPlaneCount, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress, kCVPixelBufferPixelFormatTypeKey,
    kCVPixelFormatType_420YpCbCr8Planar, kCVReturnSuccess,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString};
use objc2_video_toolbox::{
    VTDecodeFrameFlags, VTDecodeInfoFlags, VTDecompressionOutputCallbackRecord,
    VTDecompressionSession,
};

use crate::h264::{
    H264Error, PICTURE_PARAMETER_SET, SEQUENCE_PARAMETER_SET, is_within_maximum_size,
    nal_unit_type, nal_units,
};
use crate::video_frame::I420View;

/// AVCC's NAL unit length field.
const NAL_LENGTH_BYTES: i32 = 4;
/// `noErr`.
const NO_ERROR: i32 = 0;

/// Where the output callback leaves the picture of the frame being decoded.
type PictureSlot = Mutex<Option<CFRetained<CVImageBuffer>>>;

/// The decompression session of one SPS and PPS pair.
struct Session {
    sequence_parameter_set: Vec<u8>,
    picture_parameter_set: Vec<u8>,
    format: CFRetained<CMFormatDescription>,
    decompression: CFRetained<VTDecompressionSession>,
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: a session this decoder created; invalidating it twice is harmless.
        unsafe { self.decompression.invalidate() };
    }
}

pub(crate) struct H264Decoder {
    session: Option<Session>,
    /// Boxed: the session's callback holds its address.
    picture: Box<PictureSlot>,
    /// The picture handed out last, locked until the next decode.
    locked: Option<CFRetained<CVImageBuffer>>,
    /// The AVCC sample, reused across frames.
    sample: Vec<u8>,
}

impl std::fmt::Debug for H264Decoder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("H264Decoder")
            .finish_non_exhaustive()
    }
}

/// `VideoToolbox`'s output callback: keeps the decoded picture in the slot `slot` points to.
unsafe extern "C-unwind" fn keep_picture(
    slot: *mut c_void,
    _source_frame: *mut c_void,
    status: i32,
    _flags: VTDecodeInfoFlags,
    image: *mut CVImageBuffer,
    _presentation_time: CMTime,
    _duration: CMTime,
) {
    // SAFETY: `slot` is the boxed slot of the decoder that owns the session, which outlives it
    // (the session is invalidated before the box is freed).
    let slot = unsafe { &*slot.cast::<PictureSlot>() };
    let picture = NonNull::new(image)
        .filter(|_| status == NO_ERROR)
        // SAFETY: a valid image buffer for the callback's duration; retained to keep it.
        .map(|image| unsafe { CFRetained::retain(image) });
    *slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = picture;
}

impl H264Decoder {
    // The same signature as the FFmpeg decoder, which can fail.
    #[allow(clippy::unnecessary_wraps)]
    pub(crate) fn new() -> Result<Self, H264Error> {
        Ok(Self {
            session: None,
            picture: Box::new(Mutex::new(None)),
            locked: None,
            sample: Vec::new(),
        })
    }

    /// Decodes one access unit; `None` when it holds no picture.
    pub(crate) fn decode(&mut self, frame: &[u8]) -> Result<Option<I420View<'_>>, H264Error> {
        self.unlock();
        let mut sequence_parameter_set = None;
        let mut picture_parameter_set = None;
        self.sample.clear();
        for nal_unit in nal_units(frame) {
            match nal_unit_type(nal_unit) {
                Some(SEQUENCE_PARAMETER_SET) => sequence_parameter_set = Some(nal_unit),
                Some(PICTURE_PARAMETER_SET) => picture_parameter_set = Some(nal_unit),
                _ => {
                    let length = u32::try_from(nal_unit.len())
                        .map_err(|_| H264Error("a NAL unit is too large".into()))?;
                    self.sample.extend_from_slice(&length.to_be_bytes());
                    self.sample.extend_from_slice(nal_unit);
                }
            }
        }
        if let (Some(sequence), Some(picture)) = (sequence_parameter_set, picture_parameter_set) {
            self.prepare_session(sequence, picture)?;
        }
        if self.sample.is_empty() {
            return Ok(None);
        }
        let Some(session) = self.session.as_ref() else {
            return Err(H264Error(
                "no parameter sets before the first picture".into(),
            ));
        };
        let sample = sample_buffer(&mut self.sample, &session.format)?;
        let mut flags = VTDecodeInfoFlags(0);
        // SAFETY: a valid session and sample (whose memory, `self.sample`, outlives the
        // synchronous decode); the callback writes the boxed slot only.
        let status = unsafe {
            session.decompression.decode_frame(
                &sample,
                VTDecodeFrameFlags(0),
                ptr::null_mut(),
                &raw mut flags,
            )
        };
        // SAFETY: a valid session; returns once the callback ran.
        unsafe { session.decompression.wait_for_asynchronous_frames() };
        drop(sample);
        if status != NO_ERROR {
            return Err(H264Error(format!(
                "VideoToolbox refused a frame ({status})"
            )));
        }
        let picture = self
            .picture
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(picture) = picture else {
            return Ok(None);
        };
        // SAFETY: a valid pixel buffer; unlocked in `unlock`, before the next decode or drop.
        if unsafe { CVPixelBufferLockBaseAddress(&picture, CVPixelBufferLockFlags::ReadOnly) }
            != kCVReturnSuccess
        {
            return Err(H264Error("the decoded picture could not be read".into()));
        }
        self.locked = Some(picture);
        Ok(self.locked.as_deref().and_then(planar_view))
    }

    /// A new session when the parameter sets change (a new sender, size or layer).
    fn prepare_session(&mut self, sequence: &[u8], picture: &[u8]) -> Result<(), H264Error> {
        if self.session.as_ref().is_some_and(|session| {
            session.sequence_parameter_set == sequence && session.picture_parameter_set == picture
        }) {
            return Ok(());
        }
        self.session = None;
        let format = format_description(sequence, picture)?;
        // SAFETY: a valid format description.
        let dimensions = unsafe { CMVideoFormatDescriptionGetDimensions(&format) };
        let width = u32::try_from(dimensions.width).unwrap_or(0);
        let height = u32::try_from(dimensions.height).unwrap_or(0);
        if !is_within_maximum_size(width, height) {
            return Err(H264Error(format!(
                "a {width} × {height} picture is too large"
            )));
        }
        let callback = VTDecompressionOutputCallbackRecord {
            decompressionOutputCallback: Some(keep_picture),
            decompressionOutputRefCon: ptr::from_ref::<PictureSlot>(&self.picture)
                .cast_mut()
                .cast(),
        };
        // `kCVPixelBufferPixelFormatTypeKey` is a CFString, toll-free bridged to NSString.
        // SAFETY: a CoreVideo constant, valid for the process.
        let key: &NSString =
            unsafe { &*std::ptr::from_ref(kCVPixelBufferPixelFormatTypeKey).cast::<NSString>() };
        let pixel_format = NSNumber::numberWithUnsignedInt(kCVPixelFormatType_420YpCbCr8Planar);
        let attributes = NSDictionary::from_slices(&[key], &[&*pixel_format]);
        // NSDictionary is toll-free bridged to CFDictionary.
        // SAFETY: the same object, viewed as its CoreFoundation type, alive for the call.
        let attributes: &CFDictionary =
            unsafe { &*objc2::rc::Retained::as_ptr(&attributes).cast::<CFDictionary>() };
        let mut session_out: *mut VTDecompressionSession = ptr::null_mut();
        // SAFETY: valid format and attributes; the callback record is copied by the call and
        // its context (the boxed slot) outlives the session (see `Session`'s drop order).
        let status = unsafe {
            VTDecompressionSession::create(
                None,
                &format,
                None,
                Some(attributes),
                &raw const callback,
                NonNull::from(&mut session_out),
            )
        };
        let session = NonNull::new(session_out)
            .filter(|_| status == NO_ERROR)
            // SAFETY: a session created just above with a +1 retain count.
            .map(|session| unsafe { CFRetained::from_raw(session) })
            .ok_or_else(|| H264Error(format!("no VideoToolbox session ({status})")))?;
        self.session = Some(Session {
            sequence_parameter_set: sequence.to_vec(),
            picture_parameter_set: picture.to_vec(),
            format,
            decompression: session,
        });
        Ok(())
    }

    fn unlock(&mut self) {
        if let Some(picture) = self.locked.take() {
            // SAFETY: locked in `decode` with the same flags.
            unsafe { CVPixelBufferUnlockBaseAddress(&picture, CVPixelBufferLockFlags::ReadOnly) };
        }
    }
}

impl Drop for H264Decoder {
    fn drop(&mut self) {
        self.unlock();
        // The session goes before the slot its callback writes.
        self.session = None;
    }
}

fn format_description(
    sequence: &[u8],
    picture: &[u8],
) -> Result<CFRetained<CMFormatDescription>, H264Error> {
    let pointers = [
        NonNull::from(sequence).cast::<u8>(),
        NonNull::from(picture).cast::<u8>(),
    ];
    let sizes = [sequence.len(), picture.len()];
    let mut format: *const CMFormatDescription = ptr::null();
    // SAFETY: two valid parameter sets with their sizes; the description copies them.
    let status = unsafe {
        CMVideoFormatDescriptionCreateFromH264ParameterSets(
            None,
            pointers.len(),
            NonNull::from(&pointers).cast(),
            NonNull::from(&sizes).cast(),
            NAL_LENGTH_BYTES,
            NonNull::from(&mut format),
        )
    };
    NonNull::new(format.cast_mut())
        .filter(|_| status == NO_ERROR)
        // SAFETY: created just above with a +1 retain count.
        .map(|format| unsafe { CFRetained::from_raw(format) })
        .ok_or_else(|| H264Error(format!("unreadable parameter sets ({status})")))
}

/// A sample over `sample`'s bytes, without copying them: it must not outlive them.
fn sample_buffer(
    sample: &mut [u8],
    format: &CMFormatDescription,
) -> Result<CFRetained<CMSampleBuffer>, H264Error> {
    let length = sample.len();
    let mut block: *mut CMBlockBuffer = ptr::null_mut();
    // SAFETY: the block points at `sample` (never freed by it: `kCFAllocatorNull`), which the
    // caller keeps alive and unchanged until the sample is dropped.
    let status = unsafe {
        CMBlockBuffer::create_with_memory_block(
            None,
            sample.as_mut_ptr().cast(),
            length,
            kCFAllocatorNull,
            ptr::null(),
            0,
            length,
            0,
            NonNull::from(&mut block),
        )
    };
    let block = NonNull::new(block)
        .filter(|_| status == NO_ERROR)
        // SAFETY: created just above with a +1 retain count.
        .map(|block| unsafe { CFRetained::from_raw(block) })
        .ok_or_else(|| H264Error(format!("no block buffer ({status})")))?;
    let mut sample_out: *mut CMSampleBuffer = ptr::null_mut();
    // SAFETY: a valid block and format; one sample of `length` bytes, no timing.
    let status = unsafe {
        CMSampleBuffer::create_ready(
            None,
            Some(&block),
            Some(format),
            1,
            0,
            ptr::null(),
            1,
            &raw const length,
            NonNull::from(&mut sample_out),
        )
    };
    NonNull::new(sample_out)
        .filter(|_| status == NO_ERROR)
        // SAFETY: created just above with a +1 retain count.
        .map(|sample| unsafe { CFRetained::from_raw(sample) })
        .ok_or_else(|| H264Error(format!("no sample buffer ({status})")))
}

/// The locked planar picture's three planes. `None` for any other layout.
fn planar_view(picture: &CVImageBuffer) -> Option<I420View<'_>> {
    if CVPixelBufferGetPixelFormatType(picture) != kCVPixelFormatType_420YpCbCr8Planar
        || CVPixelBufferGetPlaneCount(picture) != 3
    {
        return None;
    }
    let plane = |index: usize| -> Option<(&[u8], usize)> {
        let base = CVPixelBufferGetBaseAddressOfPlane(picture, index).cast::<u8>();
        let stride = CVPixelBufferGetBytesPerRowOfPlane(picture, index);
        let rows = CVPixelBufferGetHeightOfPlane(picture, index);
        if base.is_null() {
            return None;
        }
        // SAFETY: the buffer is locked while the view lives (until the next decode); the
        // plane holds `rows` rows of `stride` bytes, as the buffer itself reports.
        Some((
            unsafe { std::slice::from_raw_parts(base, stride.checked_mul(rows)?) },
            stride,
        ))
    };
    let (luma, luma_stride) = plane(0)?;
    let (blue_chroma, blue_stride) = plane(1)?;
    let (red_chroma, red_stride) = plane(2)?;
    if blue_stride != red_stride {
        return None;
    }
    I420View::new(
        u32::try_from(CVPixelBufferGetWidth(picture)).ok()?,
        u32::try_from(CVPixelBufferGetHeight(picture)).ok()?,
        (luma, luma_stride),
        (blue_chroma, red_chroma, blue_stride),
    )
}
