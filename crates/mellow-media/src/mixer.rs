//! Mixing the voices heard into one frame, with the web client's gain rules: output volume times
//! the person's (or the shared screen's) volume, nothing while deafened or locally muted.

use crate::audio_format::Frame;

/// The loudest volume a slider gives, 200 % (`web/src/lib/voice-settings.ts`).
pub const MAXIMUM_VOLUME: f32 = 2.0;

/// A volume brought into `0..=MAXIMUM_VOLUME`; not a number counts as 100 %.
pub(crate) fn clamped_volume(volume: f32) -> f32 {
    if volume.is_nan() {
        1.0
    } else {
        volume.clamp(0.0, MAXIMUM_VOLUME)
    }
}

/// The gain one received source plays at.
pub(crate) fn playback_gain(output_volume: f32, source_volume: f32, is_silenced: bool) -> f32 {
    if is_silenced {
        0.0
    } else {
        clamped_volume(output_volume) * clamped_volume(source_volume)
    }
}

/// Adds `source` times `gain` to `mix`.
pub(crate) fn mix_into(mix: &mut Frame, source: &Frame, gain: f32) {
    if gain == 0.0 {
        return;
    }
    for (mixed, sample) in mix.iter_mut().zip(source) {
        *mixed += sample * gain;
    }
}

/// Keeps the mix within full scale: several loud voices at 200 % would otherwise wrap or clip
/// harshly in the device's integer format.
// shortcut: a hard clip, not the web's limiter; add a look-ahead limiter if clipping is heard.
pub(crate) fn limit_to_full_scale(mix: &mut Frame) {
    for sample in mix {
        *sample = sample.clamp(-1.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_format::SAMPLES_PER_FRAME;

    #[test]
    fn gain_multiplies_volumes_and_silencing_wins() {
        assert!((playback_gain(1.5, 2.0, false) - 3.0).abs() < f32::EPSILON);
        assert!(playback_gain(1.0, 1.0, true).abs() < f32::EPSILON);
        assert!(
            (playback_gain(5.0, -1.0, false)).abs() < f32::EPSILON,
            "clamped"
        );
        assert!((playback_gain(f32::NAN, 1.0, false) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn two_sources_add_up_and_the_mix_stays_within_full_scale() {
        let mut mix = [0.0; SAMPLES_PER_FRAME];
        mix_into(&mut mix, &[0.25; SAMPLES_PER_FRAME], 1.0);
        mix_into(&mut mix, &[0.5; SAMPLES_PER_FRAME], 0.5);
        assert!(mix.iter().all(|sample| (sample - 0.5).abs() < 1e-6));
        mix_into(&mut mix, &[1.0; SAMPLES_PER_FRAME], 2.0);
        limit_to_full_scale(&mut mix);
        assert!(mix.iter().all(|sample| (sample - 1.0).abs() < f32::EPSILON));
    }
}
