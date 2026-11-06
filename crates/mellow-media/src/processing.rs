//! Microphone processing, in the web client's order (ADR-0031): echo cancellation against what
//! the speakers play, then noise suppression (WebRTC's, or `RNNoise` for "advanced"), automatic
//! gain or the voice enhancement (which replaces it), then the input volume. WebRTC's processing is `sonora`, a pure Rust port of its APM.

use nnnoiseless::DenoiseState;
use sonora::config::{
    AdaptiveDigital, EchoCanceller, GainController2, HighPassFilter, NoiseSuppression,
};
use sonora::{AudioProcessing, Config, StreamConfig};

use crate::audio_format::{Frame, SAMPLES_PER_FRAME, VOICE_SAMPLE_RATE};
use crate::enhancement::{EqualizerDecibels, VoiceEnhancer};
use crate::mixer::clamped_volume;

/// `RNNoise` works on 16-bit sample values held in `f32`.
const RNNOISE_SCALE: f32 = 32_768.0;

/// Which noise suppression runs (`noiseSuppression` in the web's voice settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoiseSuppressionMode {
    Off,
    /// WebRTC's suppressor, what browsers apply.
    Standard,
    /// `RNNoise`, the web client's default.
    #[default]
    Advanced,
}

/// The microphone processing settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessingSettings {
    pub is_echo_cancellation_on: bool,
    pub noise_suppression: NoiseSuppressionMode,
    /// Ignored while the voice enhancement is on: its compressor sets the loudness.
    pub is_automatic_gain_on: bool,
    /// The voice enhancement's equaliser; `None` while it is off (the web's default).
    pub voice_enhancement: Option<EqualizerDecibels>,
    /// 0 to 2 (200 %), applied last.
    pub input_volume: f32,
}

impl Default for ProcessingSettings {
    fn default() -> Self {
        Self {
            is_echo_cancellation_on: true,
            noise_suppression: NoiseSuppressionMode::default(),
            is_automatic_gain_on: true,
            voice_enhancement: None,
            input_volume: 1.0,
        }
    }
}

/// The capture chain, plus the render path the echo canceller learns the speakers from.
pub(crate) struct VoiceProcessor {
    settings: ProcessingSettings,
    audio_processing: AudioProcessing,
    rnnoise: Option<Box<DenoiseState<'static>>>,
    enhancer: Option<VoiceEnhancer>,
    scratch: Frame,
}

impl std::fmt::Debug for VoiceProcessor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VoiceProcessor")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl VoiceProcessor {
    pub(crate) fn new(settings: ProcessingSettings) -> Self {
        let mono_48_kilohertz = StreamConfig::new(VOICE_SAMPLE_RATE, 1);
        let audio_processing = AudioProcessing::builder()
            .config(sonora_config(settings))
            .capture_config(mono_48_kilohertz)
            .render_config(mono_48_kilohertz)
            .build();
        Self {
            settings,
            audio_processing,
            rnnoise: (settings.noise_suppression == NoiseSuppressionMode::Advanced)
                .then(DenoiseState::new),
            enhancer: settings.voice_enhancement.map(VoiceEnhancer::new),
            scratch: [0.0; SAMPLES_PER_FRAME],
        }
    }

    /// Moving a slider (equaliser, input volume) keeps the echo canceller's learned state; any
    /// other change starts the chain over.
    pub(crate) fn apply(&mut self, settings: ProcessingSettings) {
        let is_slider_change = ProcessingSettings {
            voice_enhancement: self.settings.voice_enhancement,
            input_volume: self.settings.input_volume,
            ..settings
        } == self.settings
            && settings.voice_enhancement.is_some() == self.settings.voice_enhancement.is_some();
        if !is_slider_change {
            *self = Self::new(settings);
            return;
        }
        if let (Some(enhancer), Some(equalizer)) = (&mut self.enhancer, settings.voice_enhancement)
        {
            enhancer.set_equalizer(equalizer);
        }
        self.settings = settings;
    }

    /// The echo canceller's far end: the 10 ms the speakers are about to play.
    pub(crate) fn analyze_playback(&mut self, frame: &Frame) {
        if !self.settings.is_echo_cancellation_on {
            return;
        }
        self.scratch.copy_from_slice(frame);
        // The render output is not used: only the analysis matters.
        if self
            .audio_processing
            .process_render_f32(&[&frame[..]], &mut [&mut self.scratch[..]])
            .is_err()
        {
            tracing::debug!("echo canceller refused a playback frame");
        }
    }

    /// Processes one 10 ms microphone frame in place. `playback_delay_milliseconds` is how long
    /// a played frame takes to come back through the microphone, as far as the engine knows.
    pub(crate) fn process_capture(&mut self, frame: &mut Frame, playback_delay_milliseconds: i32) {
        if self.settings.is_echo_cancellation_on {
            // Out-of-range delays are clamped by the canceller, which then re-estimates.
            let _ = self
                .audio_processing
                .set_stream_delay_ms(playback_delay_milliseconds);
        }
        self.scratch.copy_from_slice(frame);
        if self
            .audio_processing
            .process_capture_f32(&[&self.scratch[..]], &mut [&mut frame[..]])
            .is_err()
        {
            // The configuration is fixed and valid: this does not happen; pass the frame as is.
            frame.copy_from_slice(&self.scratch);
        }
        if let Some(rnnoise) = self.rnnoise.as_mut() {
            for (scaled, sample) in self.scratch.iter_mut().zip(frame.iter()) {
                *scaled = sample * RNNOISE_SCALE;
            }
            rnnoise.process_frame(&mut frame[..], &self.scratch);
            for sample in frame.iter_mut() {
                *sample /= RNNOISE_SCALE;
            }
        }
        if let Some(enhancer) = self.enhancer.as_mut() {
            enhancer.process(frame);
        }
        let input_volume = clamped_volume(self.settings.input_volume);
        if (input_volume - 1.0).abs() > f32::EPSILON {
            for sample in frame.iter_mut() {
                *sample *= input_volume;
            }
        }
    }
}

fn sonora_config(settings: ProcessingSettings) -> Config {
    Config {
        high_pass_filter: Some(HighPassFilter::default()),
        echo_canceller: settings
            .is_echo_cancellation_on
            .then(EchoCanceller::default),
        noise_suppression: (settings.noise_suppression == NoiseSuppressionMode::Standard)
            .then(NoiseSuppression::default),
        gain_controller2: (settings.is_automatic_gain_on && settings.voice_enhancement.is_none())
            .then(|| GainController2 {
                adaptive_digital: Some(AdaptiveDigital::default()),
                ..GainController2::default()
            }),
        ..Config::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::root_mean_square;

    /// Deterministic white noise in `[-amplitude, amplitude]` (a linear congruential generator).
    fn noise_frames(count: usize, amplitude: f32) -> Vec<Frame> {
        let mut state: u32 = 0x1234_5678;
        (0..count)
            .map(|_| {
                let mut frame = [0.0; SAMPLES_PER_FRAME];
                for sample in &mut frame {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    #[allow(clippy::cast_precision_loss)]
                    let unit = (state >> 8) as f32 / (1 << 24) as f32;
                    *sample = (unit * 2.0 - 1.0) * amplitude;
                }
                frame
            })
            .collect()
    }

    fn output_rms(settings: ProcessingSettings, frames: &[Frame]) -> f32 {
        let mut processor = VoiceProcessor::new(settings);
        let mut last_second = Vec::new();
        for (index, input) in frames.iter().enumerate() {
            let mut frame = *input;
            processor.analyze_playback(&[0.0; SAMPLES_PER_FRAME]);
            processor.process_capture(&mut frame, 0);
            if index >= frames.len().saturating_sub(100) {
                last_second.extend_from_slice(&frame);
            }
        }
        root_mean_square(&last_second)
    }

    const PLAIN: ProcessingSettings = ProcessingSettings {
        is_echo_cancellation_on: false,
        noise_suppression: NoiseSuppressionMode::Off,
        is_automatic_gain_on: false,
        voice_enhancement: None,
        input_volume: 1.0,
    };

    #[test]
    fn both_noise_suppressors_attenuate_steady_noise() {
        let frames = noise_frames(300, 0.1);
        let untouched = output_rms(PLAIN, &frames);
        for mode in [
            NoiseSuppressionMode::Standard,
            NoiseSuppressionMode::Advanced,
        ] {
            let suppressed = output_rms(
                ProcessingSettings {
                    noise_suppression: mode,
                    ..PLAIN
                },
                &frames,
            );
            assert!(
                suppressed < untouched / 2.0,
                "{mode:?}: {suppressed} against {untouched}"
            );
        }
    }

    #[test]
    fn input_volume_scales_the_output() {
        let frames = noise_frames(50, 0.1);
        let normal = output_rms(PLAIN, &frames);
        let doubled = output_rms(
            ProcessingSettings {
                input_volume: 2.0,
                ..PLAIN
            },
            &frames,
        );
        assert!((doubled / normal - 2.0).abs() < 0.01);
    }

    #[test]
    fn the_voice_enhancement_lifts_a_quiet_voice() {
        let frames = noise_frames(200, 0.01);
        let untouched = output_rms(PLAIN, &frames);
        let enhanced = output_rms(
            ProcessingSettings {
                voice_enhancement: Some(crate::VoiceEnhancementPreset::Natural.equalizer()),
                ..PLAIN
            },
            &frames,
        );
        assert!(enhanced > untouched * 2.0, "{enhanced} against {untouched}");
    }

    #[test]
    fn the_full_chain_runs_with_echo_cancellation_on() {
        let frames = noise_frames(100, 0.1);
        let rms = output_rms(ProcessingSettings::default(), &frames);
        assert!(rms.is_finite());
    }
}
