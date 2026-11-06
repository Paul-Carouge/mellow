//! The voice enhancement of the web client (`web/src/lib/voice-processing.ts`,
//! `docs/design/web-voice-enhancement.md`): a rumble high-pass, a three-band equaliser, a
//! compressor whose make-up gain replaces automatic gain, and a limiter. The filters use Web
//! Audio's exact `BiquadFilterNode` formulas and the dynamics port Chromium's
//! `DynamicsCompressorNode` static curve and make-up gain, so a voice sounds the same as from the
//! browser.

use crate::audio_format::VOICE_SAMPLE_RATE;

/// The sliders' range, in dB (`web/src/lib/voice-settings.ts`).
pub const MINIMUM_EQUALIZER_DECIBELS: f32 = -12.0;
pub const MAXIMUM_EQUALIZER_DECIBELS: f32 = 12.0;

/// Below the lowest voices: desk thumps, traffic and fan rumble.
const RUMBLE_CUTOFF_HERTZ: f32 = 90.0;
/// The web's `Math.SQRT1_2`, which Web Audio reads in dB for a high-pass.
const RUMBLE_FILTER_QUALITY: f32 = std::f32::consts::FRAC_1_SQRT_2;
const BASS_SHELF_HERTZ: f32 = 120.0;
/// Where a voice sounds boxy or muddy in a small room.
const MIDS_PEAK_HERTZ: f32 = 350.0;
/// About 1.5 octaves wide: a gentle bell, never a notch.
const MIDS_PEAK_QUALITY: f32 = 0.9;
/// Presence and air, above the sibilance-heavy 3 to 4 kHz.
const TREBLE_SHELF_HERTZ: f32 = 5_000.0;

/// About 3:1 above -24 dBFS: quiet words come up, shouts come down.
const COMPRESSOR: DynamicsSettings = DynamicsSettings {
    threshold_decibels: -24.0,
    knee_decibels: 10.0,
    ratio: 3.0,
    attack_seconds: 0.005,
    release_seconds: 0.2,
};
/// The hardest the compressor goes; with its make-up gain, peaks stop just under -1 dBFS.
const LIMITER: DynamicsSettings = DynamicsSettings {
    threshold_decibels: -2.0,
    knee_decibels: 0.0,
    ratio: 20.0,
    attack_seconds: 0.001,
    release_seconds: 0.05,
};
/// Chromium raises the make-up gain to this power: full compensation would sound pumped.
const MAKEUP_GAIN_EXPONENT: f32 = 0.6;

/// The equaliser's three sliders, in dB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqualizerDecibels {
    pub bass: f32,
    pub mids: f32,
    pub treble: f32,
}

impl EqualizerDecibels {
    /// Each band brought into the sliders' range; not a number counts as 0 dB.
    #[must_use]
    pub fn new(bass: f32, mids: f32, treble: f32) -> Self {
        let clamped = |decibels: f32| {
            if decibels.is_nan() {
                0.0
            } else {
                decibels.clamp(MINIMUM_EQUALIZER_DECIBELS, MAXIMUM_EQUALIZER_DECIBELS)
            }
        };
        Self {
            bass: clamped(bass),
            mids: clamped(mids),
            treble: clamped(treble),
        }
    }
}

/// The presets that fill the three sliders (`EQUALIZER_OF_PRESET` in the web client).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoiceEnhancementPreset {
    Natural,
    /// A little body and presence, the boxy mids down: the web's default.
    #[default]
    Streamer,
    Warm,
    Clear,
}

impl VoiceEnhancementPreset {
    pub const ALL: [Self; 4] = [Self::Natural, Self::Streamer, Self::Warm, Self::Clear];

    #[must_use]
    pub fn equalizer(self) -> EqualizerDecibels {
        match self {
            Self::Natural => EqualizerDecibels::new(0.0, 0.0, 0.0),
            Self::Streamer => EqualizerDecibels::new(3.0, -3.0, 4.0),
            Self::Warm => EqualizerDecibels::new(5.0, -1.0, -1.0),
            Self::Clear => EqualizerDecibels::new(-2.0, -2.0, 5.0),
        }
    }
}

/// The filter shapes the chain uses, with Web Audio's coefficient formulas.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FilterShape {
    /// Web Audio reads a high-pass's Q in dB.
    HighPass {
        quality_decibels: f32,
    },
    /// Web Audio's shelves ignore Q (slope 1).
    LowShelf {
        gain_decibels: f32,
    },
    Peaking {
        quality: f32,
        gain_decibels: f32,
    },
    HighShelf {
        gain_decibels: f32,
    },
}

/// A second-order filter in direct form I, normalised by `a0`.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    /// The Web Audio specification's "filters characteristics" formulas.
    fn new(shape: FilterShape, frequency_hertz: f32) -> Self {
        #[allow(clippy::cast_precision_loss)] // 48 000 is exact in f32
        let sample_rate = VOICE_SAMPLE_RATE as f32;
        let omega = std::f32::consts::TAU * frequency_hertz / sample_rate;
        let (sin, cos) = omega.sin_cos();
        let shelf = |gain_decibels: f32| {
            let amplitude = 10_f32.powf(gain_decibels / 40.0);
            // Slope 1: sqrt((A + 1/A)(1/S - 1) + 2) is sqrt(2).
            let alpha = sin / 2.0 * std::f32::consts::SQRT_2;
            (amplitude, 2.0 * alpha * amplitude.sqrt())
        };
        let (b0, b1, b2, a0, a1, a2) = match shape {
            FilterShape::HighPass { quality_decibels } => {
                let alpha = sin / (2.0 * 10_f32.powf(quality_decibels / 20.0));
                let side = f32::midpoint(1.0, cos);
                (
                    side,
                    -(1.0 + cos),
                    side,
                    1.0 + alpha,
                    -2.0 * cos,
                    1.0 - alpha,
                )
            }
            FilterShape::Peaking {
                quality,
                gain_decibels,
            } => {
                let amplitude = 10_f32.powf(gain_decibels / 40.0);
                let alpha = sin / (2.0 * quality);
                (
                    1.0 + alpha * amplitude,
                    -2.0 * cos,
                    1.0 - alpha * amplitude,
                    1.0 + alpha / amplitude,
                    -2.0 * cos,
                    1.0 - alpha / amplitude,
                )
            }
            FilterShape::LowShelf { gain_decibels } => {
                let (a, two_alpha_root) = shelf(gain_decibels);
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + two_alpha_root),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - two_alpha_root),
                    (a + 1.0) + (a - 1.0) * cos + two_alpha_root,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - two_alpha_root,
                )
            }
            FilterShape::HighShelf { gain_decibels } => {
                let (a, two_alpha_root) = shelf(gain_decibels);
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + two_alpha_root),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - two_alpha_root),
                    (a + 1.0) - (a - 1.0) * cos + two_alpha_root,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - two_alpha_root,
                )
            }
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Another filter's response with this one's history.
    fn take_coefficients_of(&mut self, retuned: Self) {
        *self = Self {
            x1: self.x1,
            x2: self.x2,
            y1: self.y1,
            y2: self.y2,
            ..retuned
        };
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.b0 * input + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = input;
        self.y2 = self.y1;
        // Flush denormals: a silent tail must not slow the audio thread down.
        self.y1 = if output.abs() < 1e-20 { 0.0 } else { output };
        self.y1
    }
}

/// A `DynamicsCompressorNode`'s parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
struct DynamicsSettings {
    threshold_decibels: f32,
    knee_decibels: f32,
    ratio: f32,
    attack_seconds: f32,
    release_seconds: f32,
}

fn decibels_to_linear(decibels: f32) -> f32 {
    10_f32.powf(decibels / 20.0)
}

fn linear_to_decibels(linear: f32) -> f32 {
    20.0 * linear.max(1e-9).log10()
}

/// Chromium's `DynamicsCompressorKernel` static curve: linear below the threshold, an exponential
/// knee whose end has the ratio's slope, then that slope in dB.
#[derive(Debug, Clone, Copy)]
struct StaticCurve {
    linear_threshold: f32,
    knee_threshold: f32,
    knee_threshold_decibels: f32,
    output_at_knee_threshold_decibels: f32,
    slope: f32,
    knee_sharpness: f32,
}

impl StaticCurve {
    fn new(settings: DynamicsSettings) -> Self {
        let knee_threshold_decibels = settings.threshold_decibels + settings.knee_decibels;
        let mut curve = Self {
            linear_threshold: decibels_to_linear(settings.threshold_decibels),
            knee_threshold: decibels_to_linear(knee_threshold_decibels),
            knee_threshold_decibels,
            output_at_knee_threshold_decibels: 0.0,
            slope: 1.0 / settings.ratio,
            knee_sharpness: 0.0,
        };
        curve.knee_sharpness = curve.sharpness_at_slope(curve.slope);
        curve.output_at_knee_threshold_decibels =
            linear_to_decibels(curve.knee(curve.knee_threshold));
        curve
    }

    fn knee_with(&self, input: f32, sharpness: f32) -> f32 {
        if input < self.linear_threshold {
            input
        } else {
            self.linear_threshold
                + (1.0 - (-sharpness * (input - self.linear_threshold)).exp()) / sharpness
        }
    }

    fn knee(&self, input: f32) -> f32 {
        self.knee_with(input, self.knee_sharpness)
    }

    /// The curve's slope in dB at `input` for a knee `sharpness`.
    fn slope_at(&self, input: f32, sharpness: f32) -> f32 {
        if input < self.linear_threshold {
            return 1.0;
        }
        let nudged = input * 1.001;
        let output = linear_to_decibels(self.knee_with(input, sharpness));
        let nudged_output = linear_to_decibels(self.knee_with(nudged, sharpness));
        (nudged_output - output) / (linear_to_decibels(nudged) - linear_to_decibels(input))
    }

    /// Chromium's `KAtSlope`: the knee sharpness whose slope at the knee's end is `slope`.
    fn sharpness_at_slope(&self, slope: f32) -> f32 {
        let (mut lowest, mut highest, mut sharpness) = (0.1_f32, 10_000.0_f32, 5.0_f32);
        for _ in 0..15 {
            if self.slope_at(self.knee_threshold, sharpness) < slope {
                highest = sharpness;
            } else {
                lowest = sharpness;
            }
            sharpness = (lowest * highest).sqrt();
        }
        sharpness
    }

    /// The output level for an input level, both linear.
    fn saturate(&self, input: f32) -> f32 {
        if input < self.knee_threshold {
            self.knee(input)
        } else {
            let output_decibels = self.output_at_knee_threshold_decibels
                + self.slope * (linear_to_decibels(input) - self.knee_threshold_decibels);
            decibels_to_linear(output_decibels)
        }
    }
}

/// A compressor or limiter with Web Audio's implicit make-up gain.
// shortcut: a peak envelope with one-pole attack and release, without Chromium's 6 ms look-ahead
// and adaptive release curve; port them if the enhanced voice is heard pumping.
#[derive(Debug, Clone, Copy)]
struct Dynamics {
    curve: StaticCurve,
    makeup_gain: f32,
    attack_coefficient: f32,
    release_coefficient: f32,
    envelope: f32,
}

impl Dynamics {
    fn new(settings: DynamicsSettings) -> Self {
        let curve = StaticCurve::new(settings);
        #[allow(clippy::cast_precision_loss)] // 48 000 is exact in f32
        let sample_rate = VOICE_SAMPLE_RATE as f32;
        let coefficient = |seconds: f32| (-1.0 / (seconds * sample_rate)).exp();
        Self {
            makeup_gain: (1.0 / curve.saturate(1.0)).powf(MAKEUP_GAIN_EXPONENT),
            curve,
            attack_coefficient: coefficient(settings.attack_seconds),
            release_coefficient: coefficient(settings.release_seconds),
            envelope: 0.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let level = input.abs();
        let coefficient = if level > self.envelope {
            self.attack_coefficient
        } else {
            self.release_coefficient
        };
        self.envelope = level + coefficient * (self.envelope - level);
        let gain = if self.envelope > self.curve.linear_threshold {
            self.curve.saturate(self.envelope) / self.envelope
        } else {
            1.0
        };
        input * gain * self.makeup_gain
    }
}

/// The enhancement stages after noise suppression, in the web's order.
#[derive(Debug, Clone)]
pub(crate) struct VoiceEnhancer {
    filters: [Biquad; 4],
    compressor: Dynamics,
    limiter: Dynamics,
}

impl VoiceEnhancer {
    pub(crate) fn new(equalizer: EqualizerDecibels) -> Self {
        Self {
            filters: Self::filters_for(equalizer),
            compressor: Dynamics::new(COMPRESSOR),
            limiter: Dynamics::new(LIMITER),
        }
    }

    /// New slider values keep every filter's and the dynamics' state, as the web only sets
    /// `gain.value`: rebuilding would click and burst while a slider moves.
    pub(crate) fn set_equalizer(&mut self, equalizer: EqualizerDecibels) {
        for (filter, retuned) in self.filters.iter_mut().zip(Self::filters_for(equalizer)) {
            filter.take_coefficients_of(retuned);
        }
    }

    fn filters_for(equalizer: EqualizerDecibels) -> [Biquad; 4] {
        [
            Biquad::new(
                FilterShape::HighPass {
                    quality_decibels: RUMBLE_FILTER_QUALITY,
                },
                RUMBLE_CUTOFF_HERTZ,
            ),
            Biquad::new(
                FilterShape::LowShelf {
                    gain_decibels: equalizer.bass,
                },
                BASS_SHELF_HERTZ,
            ),
            Biquad::new(
                FilterShape::Peaking {
                    quality: MIDS_PEAK_QUALITY,
                    gain_decibels: equalizer.mids,
                },
                MIDS_PEAK_HERTZ,
            ),
            Biquad::new(
                FilterShape::HighShelf {
                    gain_decibels: equalizer.treble,
                },
                TREBLE_SHELF_HERTZ,
            ),
        ]
    }

    pub(crate) fn process(&mut self, samples: &mut [f32]) {
        for sample in samples {
            let filtered = self
                .filters
                .iter_mut()
                .fold(*sample, |value, filter| filter.process(value));
            *sample = self.limiter.process(self.compressor.process(filtered));
        }
    }
}

/// Keeps the speakers' mix under full scale without the harsh distortion of clipping: the gain
/// drops at once to fit a peak, then recovers over `release`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PeakLimiter {
    ceiling: f32,
    release_coefficient: f32,
    gain: f32,
}

/// Just under full scale, so the device's integer conversion never wraps.
const OUTPUT_CEILING_DECIBELS: f32 = -1.0;
const OUTPUT_RELEASE_SECONDS: f32 = 0.05;

impl Default for PeakLimiter {
    fn default() -> Self {
        #[allow(clippy::cast_precision_loss)] // 48 000 is exact in f32
        let sample_rate = VOICE_SAMPLE_RATE as f32;
        Self {
            ceiling: decibels_to_linear(OUTPUT_CEILING_DECIBELS),
            release_coefficient: (-1.0 / (OUTPUT_RELEASE_SECONDS * sample_rate)).exp(),
            gain: 1.0,
        }
    }
}

impl PeakLimiter {
    pub(crate) fn process(&mut self, samples: &mut [f32]) {
        for sample in samples {
            let level = sample.abs();
            let fitting_gain = if level > self.ceiling {
                self.ceiling / level
            } else {
                1.0
            };
            self.gain = if fitting_gain < self.gain {
                fitting_gain
            } else {
                fitting_gain + self.release_coefficient * (self.gain - fitting_gain)
            };
            *sample *= self.gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::root_mean_square;

    fn sine(frequency: f32, amplitude: f32, seconds: f32) -> Vec<f32> {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let count = (seconds * VOICE_SAMPLE_RATE as f32) as usize;
        (0..count)
            .map(|index| {
                #[allow(clippy::cast_precision_loss)]
                let time = index as f32 / VOICE_SAMPLE_RATE as f32;
                (std::f32::consts::TAU * frequency * time).sin() * amplitude
            })
            .collect()
    }

    /// The gain, in dB, a filter gives a steady sine (the first half second lets it settle).
    fn filter_gain_decibels(shape: FilterShape, center: f32, probe: f32) -> f32 {
        let mut filter = Biquad::new(shape, center);
        let input = sine(probe, 0.1, 1.0);
        let output: Vec<f32> = input.iter().map(|sample| filter.process(*sample)).collect();
        let half = input.len() / 2;
        linear_to_decibels(root_mean_square(&output[half..]) / root_mean_square(&input[half..]))
    }

    #[test]
    fn filters_follow_web_audio_s_formulas() {
        let peak = FilterShape::Peaking {
            quality: MIDS_PEAK_QUALITY,
            gain_decibels: 6.0,
        };
        assert!((filter_gain_decibels(peak, 350.0, 350.0) - 6.0).abs() < 0.2);
        assert!(filter_gain_decibels(peak, 350.0, 5_000.0).abs() < 0.5);
        let high_pass = FilterShape::HighPass {
            quality_decibels: RUMBLE_FILTER_QUALITY,
        };
        assert!(filter_gain_decibels(high_pass, 90.0, 25.0) < -15.0);
        assert!(filter_gain_decibels(high_pass, 90.0, 1_000.0).abs() < 0.2);
        let treble = FilterShape::HighShelf { gain_decibels: 5.0 };
        assert!((filter_gain_decibels(treble, 5_000.0, 15_000.0) - 5.0).abs() < 0.5);
        let bass = FilterShape::LowShelf {
            gain_decibels: -4.0,
        };
        assert!((filter_gain_decibels(bass, 120.0, 30.0) + 4.0).abs() < 0.5);
    }

    #[test]
    fn the_compressor_s_make_up_gain_is_about_nine_decibels_as_in_chromium() {
        let makeup = linear_to_decibels(Dynamics::new(COMPRESSOR).makeup_gain);
        assert!((7.0..10.0).contains(&makeup), "{makeup}");
    }

    #[test]
    fn the_enhancer_lifts_quiet_speech_and_keeps_loud_speech_under_full_scale() {
        let natural = VoiceEnhancementPreset::Natural.equalizer();
        let mut quiet = sine(440.0, 0.02, 1.0);
        let quiet_before = root_mean_square(&quiet);
        VoiceEnhancer::new(natural).process(&mut quiet);
        assert!(root_mean_square(&quiet[24_000..]) > quiet_before * 2.0);

        let mut loud = sine(440.0, 1.0, 1.0);
        VoiceEnhancer::new(natural).process(&mut loud);
        let loudest = loud[4_800..]
            .iter()
            .fold(0_f32, |peak, sample| peak.max(sample.abs()));
        assert!(loudest < 1.0, "{loudest}");
    }

    #[test]
    fn moving_a_slider_keeps_the_enhancer_s_state() {
        let natural = VoiceEnhancementPreset::Natural.equalizer();
        let speech = sine(440.0, 0.3, 0.2);
        let (first_half, second_half) = speech.split_at(speech.len() / 2);
        let mut untouched = VoiceEnhancer::new(natural);
        let mut retuned = VoiceEnhancer::new(natural);
        for enhancer in [&mut untouched, &mut retuned] {
            enhancer.process(&mut first_half.to_vec());
        }
        retuned.set_equalizer(natural);
        let mut expected = second_half.to_vec();
        let mut actual = second_half.to_vec();
        untouched.process(&mut expected);
        retuned.process(&mut actual);
        assert_eq!(actual, expected);
    }

    #[test]
    fn the_output_limiter_never_lets_a_peak_through_and_recovers() {
        let mut limiter = PeakLimiter::default();
        let mut loud = vec![3.0_f32; 480];
        limiter.process(&mut loud);
        assert!(loud.iter().all(|sample| *sample <= limiter.ceiling + 1e-6));
        let mut quiet = vec![0.1_f32; 48_000];
        limiter.process(&mut quiet);
        assert!((quiet[47_999] - 0.1).abs() < 1e-3, "the gain came back");
    }

    #[test]
    fn presets_and_sliders_stay_in_range() {
        assert_eq!(
            VoiceEnhancementPreset::default().equalizer(),
            EqualizerDecibels::new(3.0, -3.0, 4.0)
        );
        let wild = EqualizerDecibels::new(40.0, f32::NAN, -40.0);
        assert_eq!(wild, EqualizerDecibels::new(12.0, 0.0, -12.0));
    }
}
