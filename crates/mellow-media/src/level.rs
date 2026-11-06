//! Loudness of a frame: its RMS, its level in dBFS and the RFC 6464 audio level the server reads
//! to tell who speaks (`domain/voice_rules.rs`, louder than -50 dBov).

/// The quietest level RFC 6464 can carry, also what digital silence is sent as.
pub(crate) const SILENT_AUDIO_LEVEL: u8 = 127;

/// The level reported for digital silence, where the logarithm has no value.
pub(crate) const SILENCE_DECIBELS: f32 = -100.0;

/// Root mean square of samples in `[-1, 1]`; 0 for no samples.
pub(crate) fn root_mean_square(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_of_squares: f32 = samples.iter().map(|sample| sample * sample).sum();
    #[allow(clippy::cast_precision_loss)] // a frame holds at most a few thousand samples
    let count = samples.len() as f32;
    (sum_of_squares / count).sqrt()
}

/// The level of an RMS in decibels relative to full scale, at least [`SILENCE_DECIBELS`].
pub(crate) fn decibels_full_scale(rms: f32) -> f32 {
    if rms <= 0.0 {
        return SILENCE_DECIBELS;
    }
    (20.0 * rms.log10()).max(SILENCE_DECIBELS)
}

/// The RFC 6464 audio level: the attenuation below full scale in whole dB, 0 (loudest) to 127
/// (silence), as browsers compute it.
pub(crate) fn audio_level_attenuation(rms: f32) -> u8 {
    if rms <= 0.0 {
        return SILENT_AUDIO_LEVEL;
    }
    let attenuation = (-20.0 * rms.log10()).round();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped to 0..=127 first
    let level = attenuation.clamp(0.0, f32::from(SILENT_AUDIO_LEVEL)) as u8;
    level
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scale_square_wave_is_zero_attenuation_and_silence_is_127() {
        assert_eq!(audio_level_attenuation(1.0), 0);
        assert_eq!(audio_level_attenuation(0.0), SILENT_AUDIO_LEVEL);
        assert_eq!(audio_level_attenuation(1e-9), SILENT_AUDIO_LEVEL);
    }

    #[test]
    fn a_tenth_of_full_scale_is_twenty_decibels_down() {
        assert_eq!(audio_level_attenuation(0.1), 20);
        assert!((decibels_full_scale(0.1) + 20.0).abs() < 1e-4);
    }

    #[test]
    fn root_mean_square_of_a_constant_is_its_magnitude() {
        assert!((root_mean_square(&[-0.5; 480]) - 0.5).abs() < 1e-6);
        assert!(root_mean_square(&[]).abs() < f32::EPSILON);
    }
}
