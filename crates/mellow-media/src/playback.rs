//! An output device as the engine's threads see it: 10 ms frames at 48 kHz in, resampled to the
//! device's rate and channel count, queued a little ahead in the device's ring buffer.

use std::collections::VecDeque;

use crate::audio_format::{Frame, SAMPLES_PER_FRAME, VOICE_SAMPLE_RATE};
use crate::devices::{AudioDevices, PlaybackStream, has_failed};
use crate::resampling::{Interpolation, ResamplingError, StreamResampler};

/// How much mixed audio waits in the output ring buffer: enough to ride over the thread's
/// scheduling, little enough to keep the delay low.
const OUTPUT_QUEUE_MILLISECONDS: u32 = 30;

/// The open output, if any.
#[derive(Debug, Default)]
pub(crate) struct Playback {
    stream: Option<PlaybackStream>,
    /// 48 kHz to the device rate, for the open stream.
    resampler: Option<StreamResampler>,
    at_device_rate: VecDeque<f32>,
}

impl Playback {
    /// Opens `device_id` (or the default), closing what was open.
    pub(crate) fn open(
        &mut self,
        devices: &dyn AudioDevices,
        device_id: Option<&str>,
    ) -> Result<(), String> {
        self.close();
        let stream = devices
            .open_playback(device_id)
            .map_err(|error| error.to_string())?;
        let resampler = StreamResampler::new(
            VOICE_SAMPLE_RATE,
            stream.format.sample_rate,
            SAMPLES_PER_FRAME,
            Interpolation::Sinc,
            1.0,
        )
        .map_err(|error| error.to_string())?;
        self.stream = Some(stream);
        self.resampler = Some(resampler);
        Ok(())
    }

    pub(crate) fn close(&mut self) {
        self.stream = None;
        self.resampler = None;
        self.at_device_rate.clear();
    }

    pub(crate) fn is_open(&self) -> bool {
        self.stream.is_some()
    }

    /// The device stopped (unplugged): open it again, which falls back to the default.
    pub(crate) fn has_failed(&self) -> bool {
        self.stream
            .as_ref()
            .is_some_and(|stream| has_failed(&stream.has_failed))
    }

    /// Milliseconds of audio queued for the device.
    pub(crate) fn queued_milliseconds(&self) -> u32 {
        let Some(stream) = self.stream.as_ref() else {
            return 0;
        };
        let queued = stream.samples.buffer().capacity() - stream.samples.slots();
        let per_millisecond =
            u64::from(stream.format.sample_rate) * u64::from(stream.format.channels) / 1_000;
        u32::try_from(u64::try_from(queued).unwrap_or(u64::MAX) / per_millisecond.max(1))
            .unwrap_or(u32::MAX)
    }

    /// Whether the device wants another frame now.
    pub(crate) fn wants_frame(&self) -> bool {
        self.is_open() && self.queued_milliseconds() < OUTPUT_QUEUE_MILLISECONDS
    }

    /// Queues one frame. Returns whether the ring buffer still has room for more.
    pub(crate) fn write(&mut self, frame: &Frame) -> Result<bool, ResamplingError> {
        let (Some(stream), Some(resampler)) = (self.stream.as_mut(), self.resampler.as_mut())
        else {
            return Ok(false);
        };
        resampler.process(frame, &mut self.at_device_rate)?;
        let channels = usize::from(stream.format.channels).max(1);
        while stream.samples.slots() >= channels {
            let Some(sample) = self.at_device_rate.pop_front() else {
                break;
            };
            for _ in 0..channels {
                // Room for `channels` samples was checked just above.
                let _ = stream.samples.push(sample);
            }
        }
        Ok(stream.samples.slots() >= channels)
    }
}
