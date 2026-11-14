//! An output device as the engine's threads see it: 10 ms frames at 48 kHz in, resampled to the
//! device's rate and channel count, queued a little ahead in the device's ring buffer.

use std::collections::VecDeque;

use crate::audio_format::{Frame, SAMPLES_PER_FRAME, VOICE_SAMPLE_RATE};
use crate::devices::{AudioDevices, PlaybackStream, has_failed};
use crate::resampling::{Interpolation, ResamplingError, StreamResampler};

/// How much mixed audio waits in the output ring buffer: enough to ride over the thread's
/// scheduling, little enough to keep the delay low.
/// Longer when the device's period is (see [`PlaybackStream::queue_target_milliseconds`]).
const OUTPUT_QUEUE_MILLISECONDS: u64 = 30;

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
        self.stream.as_ref().is_some_and(|stream| {
            u64::from(self.queued_milliseconds())
                < stream.queue_target_milliseconds(OUTPUT_QUEUE_MILLISECONDS)
        })
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    use super::*;
    use crate::devices::{RunningStream, StreamFormat, ring_buffer_capacity};

    struct IdleStream;
    impl RunningStream for IdleStream {}

    /// A 48 kHz stereo output whose callbacks take `period_frames` frames each.
    fn playback_with_period(period_frames: usize) -> Playback {
        let format = StreamFormat {
            sample_rate: VOICE_SAMPLE_RATE,
            channels: 2,
        };
        // The consumer is dropped unread: the queue only grows, as between two callbacks.
        let (samples, _) = rtrb::RingBuffer::new(ring_buffer_capacity(format));
        let resampler = StreamResampler::new(
            VOICE_SAMPLE_RATE,
            format.sample_rate,
            SAMPLES_PER_FRAME,
            Interpolation::Sinc,
            1.0,
        )
        .unwrap();
        Playback {
            stream: Some(PlaybackStream {
                format,
                samples,
                has_failed: Arc::new(AtomicBool::new(false)),
                period_samples: Arc::new(AtomicUsize::new(period_frames * 2)),
                running: Box::new(IdleStream),
            }),
            resampler: Some(resampler),
            at_device_rate: VecDeque::new(),
        }
    }

    fn fill(playback: &mut Playback) -> u32 {
        let frame = [0.25; SAMPLES_PER_FRAME];
        for _ in 0..100 {
            if !playback.wants_frame() {
                break;
            }
            playback.write(&frame).unwrap();
        }
        playback.queued_milliseconds()
    }

    #[test]
    fn a_device_period_longer_than_the_queue_gets_a_whole_period_queued() {
        // PipeWire's largest quantum: 2048 frames, 42.7 ms. A 30 ms queue left 12.7 ms of
        // silence in every callback (measured on a PipeWire graph forced to that quantum).
        let queued = fill(&mut playback_with_period(2_048));
        assert!(queued >= 43, "{queued} ms queued for a 42.7 ms period");
        // A short period keeps the short queue and its low delay.
        let queued = fill(&mut playback_with_period(256));
        assert!(
            (30..=40).contains(&queued),
            "{queued} ms queued for a 5 ms period"
        );
    }

    #[test]
    fn a_period_longer_than_the_ring_buffer_still_lets_the_queue_fill() {
        // 250 ms, a driver's wrong value: the target stops below the ring buffer's 250 ms, so
        // the engine stops writing instead of pushing into a full ring forever.
        let mut playback = playback_with_period(12_000);
        let queued = fill(&mut playback);
        assert!(!playback.wants_frame(), "{queued} ms queued");
        assert!((240..250).contains(&queued), "{queued} ms queued");
    }
}
