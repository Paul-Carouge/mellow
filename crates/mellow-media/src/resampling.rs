//! Mono sample-rate conversion on a stream: device rates (44.1 kHz, 16 kHz headsets...) to and
//! from the engine's 48 kHz, and the small ratio changes of drift steering. Buffers are reserved
//! up front; steady-state processing does not allocate.

use std::collections::VecDeque;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Adjustable as _, Async, FixedAsync, PolynomialDegree, Resampler as _,
    SincInterpolationParameters,
};

/// The resampler cannot be built for these rates, or failed on a buffer it sized itself.
#[derive(Debug, thiserror::Error)]
#[error("audio resampling failed")]
pub struct ResamplingError;

/// How the resampler interpolates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Interpolation {
    /// Windowed sinc: device rate conversion, where quality is audible.
    Sinc,
    /// Cubic polynomial: drift steering of each received voice around a ratio of 1, cheap.
    Cubic,
}

/// Converts a mono stream from one rate to another, chunk by chunk.
pub(crate) struct StreamResampler {
    /// `None` passes samples through: same rates and no steering.
    resampler: Option<Async<f32>>,
    pending_input: Vec<f32>,
    chunk_output: Vec<f32>,
}

impl std::fmt::Debug for StreamResampler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StreamResampler")
            .field("is_passthrough", &self.resampler.is_none())
            .field("pending_input", &self.pending_input.len())
            .finish_non_exhaustive()
    }
}

impl StreamResampler {
    /// `chunk_frames` input frames are converted at a time; `maximum_relative_ratio` above 1
    /// allows steering by that factor either way.
    pub(crate) fn new(
        input_rate: u32,
        output_rate: u32,
        chunk_frames: usize,
        interpolation: Interpolation,
        maximum_relative_ratio: f64,
    ) -> Result<Self, ResamplingError> {
        let is_steerable = maximum_relative_ratio > 1.0;
        if input_rate == output_rate && !is_steerable {
            return Ok(Self {
                resampler: None,
                pending_input: Vec::new(),
                chunk_output: Vec::new(),
            });
        }
        let ratio = f64::from(output_rate) / f64::from(input_rate);
        let maximum_relative_ratio = maximum_relative_ratio.max(1.0);
        let resampler = match interpolation {
            Interpolation::Sinc => Async::new_sinc(
                ratio,
                maximum_relative_ratio,
                &SincInterpolationParameters::default(),
                chunk_frames,
                1,
                FixedAsync::Input,
            ),
            Interpolation::Cubic => Async::new_poly(
                ratio,
                maximum_relative_ratio,
                PolynomialDegree::Cubic,
                chunk_frames,
                1,
                FixedAsync::Input,
            ),
        }
        .map_err(|_| ResamplingError)?;
        let chunk_output = vec![0.0; resampler.output_frames_max()];
        Ok(Self {
            resampler: Some(resampler),
            pending_input: Vec::with_capacity(chunk_frames * 4),
            chunk_output,
        })
    }

    /// Changes the ratio by `relative_ratio` (1 = nominal), ramped over the next chunk.
    pub(crate) fn set_relative_ratio(
        &mut self,
        relative_ratio: f64,
    ) -> Result<(), ResamplingError> {
        match self.resampler.as_mut() {
            Some(resampler) => resampler
                .set_resample_ratio_relative(relative_ratio, true)
                .map_err(|_| ResamplingError),
            None => Ok(()),
        }
    }

    /// Converts `input` and appends what whole chunks give to `output`.
    pub(crate) fn process(
        &mut self,
        input: &[f32],
        output: &mut VecDeque<f32>,
    ) -> Result<(), ResamplingError> {
        let Some(resampler) = self.resampler.as_mut() else {
            output.extend(input);
            return Ok(());
        };
        self.pending_input.extend_from_slice(input);
        let mut consumed_frames = 0;
        loop {
            let needed = resampler.input_frames_next();
            let available = &self.pending_input[consumed_frames..];
            if available.len() < needed {
                break;
            }
            let chunk_input = InterleavedSlice::new(available, 1, available.len())
                .map_err(|_| ResamplingError)?;
            let capacity = self.chunk_output.len();
            let mut chunk_output = InterleavedSlice::new_mut(&mut self.chunk_output, 1, capacity)
                .map_err(|_| ResamplingError)?;
            let (consumed, produced) = resampler
                .process_into_buffer(&chunk_input, &mut chunk_output, None)
                .map_err(|_| ResamplingError)?;
            output.extend(&self.chunk_output[..produced]);
            consumed_frames += consumed;
        }
        // One move of what is left per call, not per chunk.
        self.pending_input.drain(..consumed_frames);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn converted_length(input_rate: u32, output_rate: u32, input_frames: usize) -> usize {
        let mut resampler =
            StreamResampler::new(input_rate, output_rate, 441, Interpolation::Sinc, 1.0).unwrap();
        let mut output = VecDeque::new();
        for _ in 0..input_frames / 441 {
            resampler.process(&[0.1; 441], &mut output).unwrap();
        }
        output.len()
    }

    #[test]
    fn a_second_at_44_1_kilohertz_becomes_about_a_second_at_48() {
        let produced = converted_length(44_100, 48_000, 44_100);
        assert!(
            produced.abs_diff(48_000) < 600,
            "{produced} frames, short only by the filter's delay"
        );
    }

    #[test]
    fn same_rates_pass_samples_through_unchanged() {
        let mut resampler =
            StreamResampler::new(48_000, 48_000, 480, Interpolation::Sinc, 1.0).unwrap();
        let mut output = VecDeque::new();
        resampler.process(&[0.25, -0.5], &mut output).unwrap();
        assert_eq!(output, [0.25, -0.5]);
    }

    #[test]
    fn a_relative_ratio_above_one_produces_more_samples() {
        let mut nominal =
            StreamResampler::new(48_000, 48_000, 960, Interpolation::Cubic, 1.01).unwrap();
        let mut slowed =
            StreamResampler::new(48_000, 48_000, 960, Interpolation::Cubic, 1.01).unwrap();
        slowed.set_relative_ratio(1.005).unwrap();
        let (mut nominal_output, mut slowed_output) = (VecDeque::new(), VecDeque::new());
        for _ in 0..100 {
            nominal.process(&[0.1; 960], &mut nominal_output).unwrap();
            slowed.process(&[0.1; 960], &mut slowed_output).unwrap();
        }
        let extra = slowed_output.len() - nominal_output.len();
        assert!(
            (400..=560).contains(&extra),
            "{extra} extra frames for 0.5 % of 96 000"
        );
    }
}
