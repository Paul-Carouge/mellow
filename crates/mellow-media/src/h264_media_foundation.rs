//! H.264 decoding on Windows through Media Foundation's own decoder (`CMSH264DecoderMFT`, part
//! of every Windows edition with the media features): Annex B access units in, planar I420
//! pictures out, in low-latency mode so each access unit gives its picture at once.

use std::mem::ManuallyDrop;

use windows::Win32::Media::MediaFoundation::{
    CLSID_MSH264DecoderMFT, CODECAPI_AVLowLatencyMode, IMFMediaBuffer, IMFMediaType, IMFSample,
    IMFTransform, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_MINIMUM_DISPLAY_APERTURE,
    MF_MT_SUBTYPE, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video,
    MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFVideoArea, MFVideoFormat_H264,
    MFVideoFormat_I420, MFVideoFormat_IYUV,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};

use crate::h264::{H264Error, is_within_maximum_size};
use crate::video_frame::I420View;
use crate::windows_media_foundation::ensure_media_foundation;

/// Media Foundation counts time in 100 ns units; pictures are stamped 1/30 s apart (the decoder
/// wants increasing times, never shows them).
const SAMPLE_DURATION_HUNDRED_NANOSECONDS: i64 = 10_000_000 / 30;

fn failed(what: &str) -> impl FnOnce(windows::core::Error) -> H264Error + '_ {
    move |error| H264Error(format!("{what}: {error}"))
}

/// How the decoder lays its pictures out, read from its output type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PictureLayout {
    /// The picture as shown (the display aperture, without the rows coding added).
    width: u32,
    height: u32,
    /// The decoded buffer: luma rows of `stride` bytes, `rows` of them, then each chroma plane
    /// at half the stride and half the rows.
    stride: usize,
    rows: usize,
}

impl PictureLayout {
    fn buffer_length(self) -> Option<usize> {
        let luma = self.stride.checked_mul(self.rows)?;
        luma.checked_add(2 * (self.stride / 2) * self.rows.div_ceil(2))
    }

    /// The I420 planes of `bytes`, a whole buffer of this layout.
    fn view(self, bytes: &[u8]) -> Option<I420View<'_>> {
        let chroma_stride = self.stride / 2;
        let luma_length = self.stride.checked_mul(self.rows)?;
        let chroma_length = chroma_stride.checked_mul(self.rows.div_ceil(2))?;
        let luma = bytes.get(..luma_length)?;
        let blue = bytes.get(luma_length..luma_length + chroma_length)?;
        let red = bytes.get(luma_length + chroma_length..luma_length + 2 * chroma_length)?;
        I420View::new(
            self.width,
            self.height,
            (luma, self.stride),
            (blue, red, chroma_stride),
        )
    }
}

pub(crate) struct H264Decoder {
    transform: IMFTransform,
    layout: Option<PictureLayout>,
    /// The caller's output sample when the decoder does not provide its own, kept across
    /// pictures (re-made when the layout grows).
    output_sample: Option<(IMFSample, u32)>,
    /// The buffer behind the last picture, locked while the picture is borrowed.
    locked: Option<IMFMediaBuffer>,
    next_sample_time: i64,
}

impl std::fmt::Debug for H264Decoder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("H264Decoder")
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl H264Decoder {
    pub(crate) fn new() -> Result<Self, H264Error> {
        ensure_media_foundation().map_err(H264Error)?;
        // SAFETY: COM is initialised on this thread; the calls configure a decoder this function
        // owns, with types it creates.
        unsafe {
            let transform: IMFTransform =
                CoCreateInstance(&CLSID_MSH264DecoderMFT, None, CLSCTX_INPROC_SERVER)
                    .map_err(failed("create the H.264 decoder"))?;
            // Without it the decoder keeps pictures for reordering, which Constrained Baseline
            // never needs.
            if let Ok(attributes) = transform.GetAttributes() {
                let _ = attributes.SetUINT32(&CODECAPI_AVLowLatencyMode, 1);
            }
            let input = MFCreateMediaType().map_err(failed("describe H.264"))?;
            input
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .and_then(|()| input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264))
                .map_err(failed("describe H.264"))?;
            transform
                .SetInputType(0, &input, 0)
                .map_err(failed("set the H.264 input"))?;
            let mut decoder = Self {
                transform,
                layout: None,
                output_sample: None,
                locked: None,
                next_sample_time: 0,
            };
            decoder.choose_planar_output()?;
            Ok(decoder)
        }
    }

    /// Picks the decoder's I420 output (offered under two names) and reads its layout.
    fn choose_planar_output(&mut self) -> Result<(), H264Error> {
        // SAFETY: plain calls on the decoder this value owns.
        unsafe {
            let mut index = 0;
            let chosen = loop {
                let offered = self
                    .transform
                    .GetOutputAvailableType(0, index)
                    .map_err(failed("find a planar output"))?;
                let subtype = offered.GetGUID(&MF_MT_SUBTYPE).unwrap_or_default();
                if subtype == MFVideoFormat_I420 || subtype == MFVideoFormat_IYUV {
                    break offered;
                }
                index += 1;
            };
            self.transform
                .SetOutputType(0, &chosen, 0)
                .map_err(failed("set the planar output"))?;
            self.layout = layout_of(&chosen);
        }
        Ok(())
    }

    fn unlock(&mut self) {
        if let Some(buffer) = self.locked.take() {
            // SAFETY: locked by `decode`, once.
            let _ = unsafe { buffer.Unlock() };
        }
    }

    /// The sample the decoder writes into, when it wants the caller's.
    fn output_sample(&mut self) -> Result<Option<IMFSample>, H264Error> {
        // SAFETY: plain calls on objects this decoder owns.
        unsafe {
            let info = self
                .transform
                .GetOutputStreamInfo(0)
                .map_err(failed("read the output's needs"))?;
            if info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0.cast_unsigned() != 0 {
                return Ok(None);
            }
            let needed = info.cbSize.max(
                self.layout
                    .and_then(PictureLayout::buffer_length)
                    .and_then(|length| u32::try_from(length).ok())
                    .unwrap_or(0),
            );
            if let Some((sample, size)) = &self.output_sample
                && *size >= needed
            {
                return Ok(Some(sample.clone()));
            }
            let sample = MFCreateSample().map_err(failed("allocate a picture"))?;
            let buffer = MFCreateMemoryBuffer(needed).map_err(failed("allocate a picture"))?;
            sample
                .AddBuffer(&buffer)
                .map_err(failed("allocate a picture"))?;
            self.output_sample = Some((sample.clone(), needed));
            Ok(Some(sample))
        }
    }

    /// Decodes one access unit; `None` when it holds no picture.
    pub(crate) fn decode(&mut self, frame: &[u8]) -> Result<Option<I420View<'_>>, H264Error> {
        self.unlock();
        let length =
            u32::try_from(frame.len()).map_err(|_| H264Error("the frame is too large".into()))?;
        // SAFETY: the buffer is locked for `length` bytes while they are copied, then unlocked;
        // the sample is handed to the decoder, which keeps its own reference.
        unsafe {
            // A new input buffer per frame: the decoder may keep the sample after `ProcessInput`.
            let buffer = MFCreateMemoryBuffer(length.max(1)).map_err(failed("copy a frame"))?;
            let mut bytes = std::ptr::null_mut();
            buffer
                .Lock(&raw mut bytes, None, None)
                .map_err(failed("copy a frame"))?;
            std::ptr::copy_nonoverlapping(frame.as_ptr(), bytes, frame.len());
            buffer.Unlock().map_err(failed("copy a frame"))?;
            buffer
                .SetCurrentLength(length)
                .map_err(failed("copy a frame"))?;
            let sample = MFCreateSample().map_err(failed("copy a frame"))?;
            sample.AddBuffer(&buffer).map_err(failed("copy a frame"))?;
            let _ = sample.SetSampleTime(self.next_sample_time);
            let _ = sample.SetSampleDuration(SAMPLE_DURATION_HUNDRED_NANOSECONDS);
            self.next_sample_time += SAMPLE_DURATION_HUNDRED_NANOSECONDS;
            self.transform
                .ProcessInput(0, &sample, 0)
                .map_err(failed("decode an H.264 frame"))?;
        }
        let Some(picture) = self.take_picture()? else {
            return Ok(None);
        };
        self.view_of(&picture)
    }

    /// The next decoded picture, following the decoder's stream changes (the first SPS, a new
    /// size); `None` when it needs more input.
    fn take_picture(&mut self) -> Result<Option<IMFSample>, H264Error> {
        // A stream change is answered once per picture; two in a row means the decoder is stuck.
        for _ in 0..2 {
            let sample = self.output_sample()?;
            let mut output = [MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: ManuallyDrop::new(sample),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            }];
            let mut status = 0;
            // SAFETY: one output buffer for stream 0; its sample and events are released below
            // whatever the result.
            let result = unsafe {
                self.transform
                    .ProcessOutput(0, &mut output, &raw mut status)
            };
            let [output] = output;
            let sample = ManuallyDrop::into_inner(output.pSample);
            drop(ManuallyDrop::into_inner(output.pEvents));
            match result {
                Ok(()) => return Ok(sample),
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
                Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    self.choose_planar_output()?;
                }
                Err(error) => return Err(failed("decode an H.264 frame")(error)),
            }
        }
        Err(H264Error("the decoder keeps changing its output".into()))
    }

    fn view_of(&mut self, picture: &IMFSample) -> Result<Option<I420View<'_>>, H264Error> {
        let Some(layout) = self.layout else {
            return Ok(None);
        };
        if !is_within_maximum_size(layout.width, layout.height) {
            return Err(H264Error("the picture is too large".into()));
        }
        // SAFETY: the buffer stays locked, and so its bytes valid, until the next `decode` or
        // the drop, both of which need `&mut self` and unlock it first.
        unsafe {
            let buffer = picture
                .ConvertToContiguousBuffer()
                .map_err(failed("read a picture"))?;
            let mut bytes = std::ptr::null_mut();
            let mut length = 0;
            buffer
                .Lock(&raw mut bytes, None, Some(&raw mut length))
                .map_err(failed("read a picture"))?;
            self.locked = Some(buffer);
            if bytes.is_null() {
                return Ok(None);
            }
            let bytes = std::slice::from_raw_parts(bytes, length as usize);
            Ok(layout.view(bytes))
        }
    }
}

impl Drop for H264Decoder {
    fn drop(&mut self) {
        self.unlock();
    }
}

/// The picture layout an output type describes; `None` while the decoder has not seen a
/// sequence parameter set (no size yet).
fn layout_of(media_type: &IMFMediaType) -> Option<PictureLayout> {
    // SAFETY: plain attribute reads into owned values.
    unsafe {
        let frame_size = media_type.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
        let coded_width = u32::try_from(frame_size >> 32).ok()?;
        let coded_height = u32::try_from(frame_size & u64::from(u32::MAX)).ok()?;
        // A negative stride is a bottom-up picture, which planar YUV never is.
        let stride = media_type
            .GetUINT32(&MF_MT_DEFAULT_STRIDE)
            .ok()
            .and_then(|stride| usize::try_from(stride.cast_signed()).ok())
            .unwrap_or(usize::try_from(coded_width).ok()?);
        let mut aperture = MFVideoArea::default();
        let aperture_bytes = std::slice::from_raw_parts_mut(
            (&raw mut aperture).cast::<u8>(),
            size_of::<MFVideoArea>(),
        );
        let (width, height) =
            match media_type.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, aperture_bytes, None) {
                Ok(()) => (
                    u32::try_from(aperture.Area.cx).ok()?,
                    u32::try_from(aperture.Area.cy).ok()?,
                ),
                Err(_) => (coded_width, coded_height),
            };
        let layout = PictureLayout {
            width: width.min(coded_width),
            height: height.min(coded_height),
            stride,
            rows: usize::try_from(coded_height).ok()?,
        };
        (layout.width > 0 && layout.height > 0).then_some(layout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_coded_picture_taller_than_shown_gives_its_shown_planes() {
        // 4 × 2 shown out of 4 × 4 coded (rows added to reach whole macroblocks).
        let layout = PictureLayout {
            width: 4,
            height: 2,
            stride: 4,
            rows: 4,
        };
        assert_eq!(layout.buffer_length(), Some(16 + 4 + 4));
        let mut bytes: Vec<u8> = (0..16).collect();
        bytes.extend([100, 101, 102, 103, 200, 201, 202, 203]);
        let view = layout.view(&bytes).unwrap();
        assert_eq!((view.width(), view.height()), (4, 2));
        assert_eq!(&view.luma()[..4], &[0, 1, 2, 3]);
        assert!(layout.view(&bytes[..23]).is_none(), "a short buffer");
    }
}
