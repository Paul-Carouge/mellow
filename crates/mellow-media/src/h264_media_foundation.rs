//! H.264 decoding on Windows through Media Foundation's own decoder (`CMSH264DecoderMFT`, part
//! of every Windows edition with the media features): Annex B access units in, planar I420
//! pictures out, in low-latency mode so each access unit gives its picture at once.

use std::mem::ManuallyDrop;

use windows::Win32::Media::MediaFoundation::{
    CLSID_MSH264DecoderMFT, CODECAPI_AVLowLatencyMode, IMFMediaBuffer, IMFMediaType, IMFSample,
    IMFTransform, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_MINIMUM_DISPLAY_APERTURE,
    MF_MT_SUBTYPE, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video,
    MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES,
    MFVideoArea, MFVideoFormat_H264, MFVideoFormat_I420, MFVideoFormat_IYUV,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};

use crate::h264::{H264Error, is_within_maximum_size};
use crate::video_frame::I420View;
use crate::windows_media_foundation::ensure_media_foundation;

/// Media Foundation counts time in 100 ns units; pictures are stamped 1/30 s apart (the decoder
/// wants increasing times, never shows them).
const SAMPLE_DURATION_HUNDRED_NANOSECONDS: i64 = 10_000_000 / 30;
/// The caller's output samples kept: the one holding the newest picture and one to read on into.
const OUTPUT_SAMPLES: usize = 2;
/// The most pictures read out after one access unit (a sender may pack several in one); the
/// newest is shown.
const MAXIMUM_OUTPUTS_PER_ACCESS_UNIT: usize = 8;

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

    /// The I420 planes of `bytes` as the decoder wrote them: with the coded rows (whole
    /// macroblocks) when the buffer holds them, else with the shown rows only, which the decoder
    /// writes for a cropped picture (640 × 360, coded 368 rows high) when the buffer holds
    /// exactly that.
    fn written_view(self, bytes: &[u8]) -> Option<I420View<'_>> {
        if bytes.len() >= self.buffer_length()? {
            return self.view(bytes);
        }
        let shown = Self {
            rows: usize::try_from(self.height).ok()?.next_multiple_of(2),
            ..self
        };
        (bytes.len() >= shown.buffer_length()?)
            .then(|| shown.view(bytes))
            .flatten()
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
    /// The caller's output samples when the decoder does not provide its own, each with its one
    /// buffer and that buffer's size, kept across pictures (re-made when the layout grows). Two:
    /// reading out a second picture of one access unit must not empty the first.
    output_samples: Vec<(IMFSample, IMFMediaBuffer, u32)>,
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
            // Without low latency the decoder keeps pictures for reordering, which Constrained
            // Baseline never needs. Even with it, its worker threads may finish a picture after
            // `ProcessOutput` asked, which then comes out at the next access unit (seen on
            // Windows CI with one thread as with several): a picture at most one frame late,
            // which a live share's next frame (the Linux sender repeats a still screen at least
            // once a second) brings out. Its default threads are kept for 1440p at 60 frames.
            let attributes = transform
                .GetAttributes()
                .map_err(failed("configure the H.264 decoder"))?;
            if let Err(error) = attributes.SetUINT32(&CODECAPI_AVLowLatencyMode, 1) {
                tracing::warn!(%error, "the H.264 decoder refused low latency");
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
                output_samples: Vec::with_capacity(OUTPUT_SAMPLES),
                locked: None,
                next_sample_time: 0,
            };
            decoder.choose_planar_output()?;
            // What a pipeline tells a decoder before its first sample; optional for this one,
            // sent so it never waits for them.
            for message in [
                MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
                MFT_MESSAGE_NOTIFY_START_OF_STREAM,
            ] {
                if let Err(error) = decoder.transform.ProcessMessage(message, 0) {
                    tracing::debug!(%error, "the H.264 decoder refused a streaming message");
                }
            }
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
            let layout = layout_of(&chosen);
            // Refused before any output buffer is allocated for it.
            if layout.is_some_and(|layout| !is_within_maximum_size(layout.width, layout.height)) {
                return Err(H264Error("the picture is too large".into()));
            }
            self.transform
                .SetOutputType(0, &chosen, 0)
                .map_err(failed("set the planar output"))?;
            self.layout = layout;
        }
        Ok(())
    }

    /// Drops whatever the decoder holds after an error, so the next keyframe starts afresh
    /// instead of every later input being refused.
    fn flush(&self) {
        // SAFETY: a plain message to the decoder this value owns.
        if let Err(error) = unsafe { self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0) } {
            tracing::debug!(%error, "the H.264 decoder could not be flushed");
        }
    }

    fn unlock(&mut self) {
        if let Some(buffer) = self.locked.take() {
            // SAFETY: locked by `decode`, once.
            let _ = unsafe { buffer.Unlock() };
        }
    }

    /// The sample the decoder writes into, when it wants the caller's: never `holding`, the one
    /// with the newest picture of this access unit.
    fn output_sample(
        &mut self,
        holding: Option<&IMFSample>,
    ) -> Result<Option<IMFSample>, H264Error> {
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
            let is_free = |sample: &IMFSample| holding != Some(sample);
            if let Some((sample, buffer, _)) = self
                .output_samples
                .iter()
                .find(|(sample, _, size)| *size >= needed && is_free(sample))
            {
                // The decoder writes after the buffer's current length: a reused buffer still
                // holding a picture fails ("CopyDecodedFrame failed", E_FAIL) or gets the new
                // picture after the old one, which is then read again (a frozen picture).
                buffer
                    .SetCurrentLength(0)
                    .map_err(failed("reuse a picture"))?;
                return Ok(Some(sample.clone()));
            }
            let sample = MFCreateSample().map_err(failed("allocate a picture"))?;
            let buffer = MFCreateMemoryBuffer(needed).map_err(failed("allocate a picture"))?;
            sample
                .AddBuffer(&buffer)
                .map_err(failed("allocate a picture"))?;
            // Replaces a free one too small (or any free one) once two are kept.
            if self.output_samples.len() >= OUTPUT_SAMPLES
                && let Some(index) = self
                    .output_samples
                    .iter()
                    .position(|(kept, _, _)| is_free(kept))
            {
                self.output_samples.remove(index);
            }
            self.output_samples.push((sample.clone(), buffer, needed));
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
            if let Err(error) = self.transform.ProcessInput(0, &sample, 0) {
                self.flush();
                return Err(failed("decode an H.264 frame")(error));
            }
        }
        let picture = match self.take_picture() {
            Ok(Some(picture)) => picture,
            Ok(None) => return Ok(None),
            Err(error) => {
                self.flush();
                return Err(error);
            }
        };
        self.view_of(&picture)
    }

    /// The newest decoded picture, every waiting one read out and the decoder's stream changes
    /// (the first SPS, a new size) followed; `None` when the access unit gave none.
    fn take_picture(&mut self) -> Result<Option<IMFSample>, H264Error> {
        let mut newest = None;
        let mut has_just_changed = false;
        for _ in 0..MAXIMUM_OUTPUTS_PER_ACCESS_UNIT {
            let sample = self.output_sample(newest.as_ref())?;
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
                Ok(()) => {
                    newest = sample;
                    has_just_changed = false;
                }
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(newest),
                // Two changes in a row means the decoder is stuck.
                Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE && !has_just_changed => {
                    has_just_changed = true;
                    // A picture taken before the change has the old layout.
                    newest = None;
                    self.choose_planar_output()?;
                }
                Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    return Err(H264Error("the decoder keeps changing its output".into()));
                }
                Err(error) => return Err(failed("decode an H.264 frame")(error)),
            }
        }
        Ok(newest)
    }

    fn view_of(&mut self, picture: &IMFSample) -> Result<Option<I420View<'_>>, H264Error> {
        // Within the maximum size: `choose_planar_output` refuses any other layout.
        let Some(layout) = self.layout else {
            return Err(H264Error("a picture came before its size".into()));
        };
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
                return Err(H264Error("a picture without bytes".into()));
            }
            let bytes = std::slice::from_raw_parts(bytes, length as usize);
            layout.written_view(bytes).map(Some).ok_or_else(|| {
                H264Error(format!(
                    "a picture of {length} bytes does not fit its layout {layout:?}"
                ))
            })
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

    #[test]
    fn a_cropped_picture_written_without_its_coded_rows_is_read_at_its_shown_rows() {
        // 4 × 2 shown out of 4 × 4 coded, written as 4 × 2: luma 8, then two chroma planes of 2.
        let layout = PictureLayout {
            width: 4,
            height: 2,
            stride: 4,
            rows: 4,
        };
        let mut bytes: Vec<u8> = (0..8).collect();
        bytes.extend([100, 101, 200, 201]);
        let view = layout.written_view(&bytes).unwrap();
        assert_eq!(&view.luma()[..8], &[0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(&view.blue_chroma()[..2], &[100, 101]);
        assert_eq!(&view.red_chroma()[..2], &[200, 201]);
        assert!(
            layout.written_view(&bytes[..11]).is_none(),
            "a short buffer"
        );
        let mut coded: Vec<u8> = (0..16).collect();
        coded.extend([100, 101, 102, 103, 200, 201, 202, 203]);
        assert_eq!(
            &layout.written_view(&coded).unwrap().red_chroma()[..2],
            &[200, 201],
            "the coded rows when the buffer holds them"
        );
    }
}
