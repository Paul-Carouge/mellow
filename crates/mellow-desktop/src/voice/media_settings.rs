//! The voice settings as the voice engine takes them (`voice-processing.ts`'s
//! `microphoneChoiceOf` and `microphoneProcessingOf`): echo cancellation always, automatic gain
//! unless the voice enhancement sets the loudness, and percentages as gains.

use std::time::Duration;

use mellow_client_core::shared_logic::voice_settings::{
    NoiseSuppression, PersonVoiceSetting, VoiceInputMode, VoiceSettings,
};
use mellow_media::{
    EqualizerDecibels, InputMode, NoiseSuppressionMode, ProcessingSettings, Sensitivity,
};

/// A percentage (at most 200) as a gain: exact in `f32`.
#[must_use]
pub fn gain_of_percent(percent: u32) -> f32 {
    // At most 200: exact in `f32`.
    #[allow(clippy::cast_precision_loss)]
    let gain = percent as f32 / 100.0;
    gain
}

#[must_use]
pub fn input_mode_of(settings: &VoiceSettings) -> InputMode {
    match settings.input_mode {
        VoiceInputMode::PushToTalk => InputMode::PushToTalk {
            release_delay: Duration::from_millis(u64::from(
                settings.push_to_talk_release_delay_milliseconds,
            )),
        },
        VoiceInputMode::VoiceActivity if settings.is_sensitivity_automatic => {
            InputMode::VoiceActivity(Sensitivity::Automatic)
        }
        // Within −100 to 0: exact in `f32`.
        #[allow(clippy::cast_precision_loss)]
        VoiceInputMode::VoiceActivity => InputMode::VoiceActivity(Sensitivity::ThresholdDecibels(
            settings.sensitivity_threshold_decibels as f32,
        )),
    }
}

#[must_use]
pub fn processing_of(settings: &VoiceSettings) -> ProcessingSettings {
    let equalizer = settings.equalizer_decibels;
    // Within −12 to 12: exact in `f32`.
    #[allow(clippy::cast_precision_loss)]
    let voice_enhancement = settings.is_voice_enhancement_on.then(|| {
        EqualizerDecibels::new(
            equalizer.bass as f32,
            equalizer.mids as f32,
            equalizer.treble as f32,
        )
    });
    ProcessingSettings {
        is_echo_cancellation_on: true,
        noise_suppression: match settings.noise_suppression {
            NoiseSuppression::Off => NoiseSuppressionMode::Off,
            NoiseSuppression::Standard => NoiseSuppressionMode::Standard,
            NoiseSuppression::Advanced => NoiseSuppressionMode::Advanced,
        },
        is_automatic_gain_on: !settings.is_voice_enhancement_on,
        voice_enhancement,
        input_volume: gain_of_percent(settings.input_volume_percent),
    }
}

#[must_use]
pub fn person_setting_of(setting: PersonVoiceSetting) -> mellow_media::PersonVoiceSetting {
    mellow_media::PersonVoiceSetting {
        volume: gain_of_percent(setting.volume_percent),
        is_locally_muted: setting.is_locally_muted,
    }
}

#[cfg(test)]
mod tests {
    use mellow_client_core::shared_logic::voice_settings::EqualizerBand;

    use super::*;

    #[test]
    fn the_defaults_cancel_echo_run_rnnoise_and_keep_automatic_gain() {
        let processing = processing_of(&VoiceSettings::DEFAULT);
        assert!(processing.is_echo_cancellation_on);
        assert!(processing.is_automatic_gain_on);
        assert_eq!(processing.noise_suppression, NoiseSuppressionMode::Advanced);
        assert_eq!(processing.voice_enhancement, None);
        assert_eq!(
            input_mode_of(&VoiceSettings::DEFAULT),
            InputMode::VoiceActivity(Sensitivity::Automatic)
        );
    }

    #[test]
    fn the_enhancement_replaces_automatic_gain_with_its_curve() {
        let settings = VoiceSettings {
            is_voice_enhancement_on: true,
            input_volume_percent: 150,
            ..VoiceSettings::DEFAULT.with_equalizer_band(EqualizerBand::Bass, 6)
        };
        let processing = processing_of(&settings);
        assert!(!processing.is_automatic_gain_on);
        assert_eq!(
            processing.voice_enhancement,
            Some(EqualizerDecibels::new(6.0, -3.0, 4.0))
        );
        assert!((processing.input_volume - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn push_to_talk_keeps_its_release_delay_and_a_manual_sensitivity_its_threshold() {
        let push_to_talk = VoiceSettings {
            input_mode: VoiceInputMode::PushToTalk,
            push_to_talk_release_delay_milliseconds: 350,
            ..VoiceSettings::DEFAULT
        };
        assert_eq!(
            input_mode_of(&push_to_talk),
            InputMode::PushToTalk {
                release_delay: Duration::from_millis(350)
            }
        );
        let manual = VoiceSettings {
            is_sensitivity_automatic: false,
            sensitivity_threshold_decibels: -42,
            ..VoiceSettings::DEFAULT
        };
        assert_eq!(
            input_mode_of(&manual),
            InputMode::VoiceActivity(Sensitivity::ThresholdDecibels(-42.0))
        );
    }
}
