//! Port of `web/src/lib/voice-settings.ts` and `web/src/lib/voice-toggles.ts`: the voice
//! settings, the microphone and headset toggles and each person's volume, as this device remembers
//! them, checked field by field on the way in (anything may have written them), the meter's
//! levels, and recording a push-to-talk key. Pure: storage belongs to the caller, which keeps the
//! web's JSON shape (`mellow.voice-settings`, `mellow.voice-toggles`,
//! `mellow.voice-person-settings`).

use mellow_protocol::Snowflake;
use serde_json::{Map, Value, json};

/// How the microphone opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoiceInputMode {
    #[default]
    VoiceActivity,
    PushToTalk,
}

/// Who removes background noise (ADR-0031): nobody, the platform's filter (WebRTC's on the
/// desktop), or `RNNoise` ("advanced"), which replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoiseSuppression {
    Off,
    Standard,
    #[default]
    Advanced,
}

impl NoiseSuppression {
    pub const ALL: [Self; 3] = [Self::Off, Self::Standard, Self::Advanced];

    /// The web's stored word; "browser" is the platform's own filter.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Standard => "browser",
            Self::Advanced => "advanced",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoiceEnhancementPreset {
    Natural,
    #[default]
    Streamer,
    Warm,
    Clear,
}

impl VoiceEnhancementPreset {
    pub const ALL: [Self; 4] = [Self::Natural, Self::Streamer, Self::Warm, Self::Clear];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Natural => "natural",
            Self::Streamer => "streamer",
            Self::Warm => "warm",
            Self::Clear => "clear",
        }
    }

    /// What the preset fills the sliders with (docs/design/web-voice-enhancement.md).
    #[must_use]
    pub fn equalizer(self) -> EqualizerDecibels {
        let (bass, mids, treble) = match self {
            Self::Natural => (0, 0, 0),
            Self::Streamer => (3, -3, 4),
            Self::Warm => (5, -1, -1),
            Self::Clear => (-2, -2, 5),
        };
        EqualizerDecibels { bass, mids, treble }
    }
}

/// A preset, or the reader's own curve once a slider moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnhancementCurve {
    Preset(VoiceEnhancementPreset),
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqualizerBand {
    Bass,
    Mids,
    Treble,
}

/// Each band's boost or cut, in whole decibels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EqualizerDecibels {
    pub bass: i32,
    pub mids: i32,
    pub treble: i32,
}

pub const MINIMUM_EQUALIZER_DECIBELS: i32 = -12;
pub const MAXIMUM_EQUALIZER_DECIBELS: i32 = 12;

/// A key recorded for push-to-talk: `code` is what is matched (a W3C `KeyboardEvent.code`, such
/// as `KeyV`, `Space` or `ControlRight`, which in-window key handlers and global-shortcut
/// listeners both understand), `label` what was printed on it then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushToTalkKey {
    pub code: String,
    pub label: String,
}

// Independent switches stored as the web stores them: no state machine hides here.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceSettings {
    /// `None`: the system's default device.
    pub input_device_id: Option<String>,
    pub output_device_id: Option<String>,
    pub input_volume_percent: u32,
    pub output_volume_percent: u32,
    pub input_mode: VoiceInputMode,
    /// No default: a letter would stop typing, so the reader records their own.
    pub push_to_talk_key: Option<PushToTalkKey>,
    pub push_to_talk_release_delay_milliseconds: u32,
    pub are_push_to_talk_sounds_on: bool,
    /// Automatic: no gate, the processing and Opus DTX handle silence.
    pub is_sensitivity_automatic: bool,
    pub sensitivity_threshold_decibels: i32,
    pub noise_suppression: NoiseSuppression,
    pub is_voice_enhancement_on: bool,
    pub voice_enhancement_curve: EnhancementCurve,
    pub equalizer_decibels: EqualizerDecibels,
    /// Soundboard and join sounds, on top of the output volume; 0 silences them.
    pub soundboard_volume_percent: u32,
}

pub const MINIMUM_VOLUME_PERCENT: u32 = 0;
pub const MAXIMUM_VOLUME_PERCENT: u32 = 200;
pub const DEFAULT_VOLUME_PERCENT: u32 = 100;
pub const MAXIMUM_SOUNDBOARD_VOLUME_PERCENT: u32 = 100;
pub const MAXIMUM_PUSH_TO_TALK_RELEASE_DELAY_MILLISECONDS: u32 = 2_000;
/// Discord's scale: the meter and the threshold run from silence (−100 dB) to full scale (0 dB).
pub const MINIMUM_DECIBELS: i32 = -100;
pub const MAXIMUM_DECIBELS: i32 = 0;
/// The same bounds for levels, which are measured in `f32`.
const MINIMUM_LEVEL_DECIBELS: f32 = -100.0;
const MAXIMUM_LEVEL_DECIBELS: f32 = 0.0;
const MAXIMUM_STORED_TEXT_LENGTH: usize = 256;

impl VoiceSettings {
    pub const DEFAULT: Self = Self {
        input_device_id: None,
        output_device_id: None,
        input_volume_percent: DEFAULT_VOLUME_PERCENT,
        output_volume_percent: DEFAULT_VOLUME_PERCENT,
        input_mode: VoiceInputMode::VoiceActivity,
        push_to_talk_key: None,
        // Long enough not to clip the last syllable when the key comes up with it.
        push_to_talk_release_delay_milliseconds: 200,
        are_push_to_talk_sounds_on: false,
        is_sensitivity_automatic: true,
        // Above a quiet room's hum, below normal speech.
        sensitivity_threshold_decibels: -50,
        noise_suppression: NoiseSuppression::Advanced,
        is_voice_enhancement_on: false,
        voice_enhancement_curve: EnhancementCurve::Preset(VoiceEnhancementPreset::Streamer),
        equalizer_decibels: EqualizerDecibels {
            bass: 3,
            mids: -3,
            treble: 4,
        },
        // Uploads are often mastered louder than speech: a little under full.
        soundboard_volume_percent: 70,
    };

    /// A preset fills the three sliders.
    #[must_use]
    pub fn with_enhancement_preset(&self, preset: VoiceEnhancementPreset) -> Self {
        Self {
            voice_enhancement_curve: EnhancementCurve::Preset(preset),
            equalizer_decibels: preset.equalizer(),
            ..self.clone()
        }
    }

    /// Moving one slider makes the curve the reader's own.
    #[must_use]
    pub fn with_equalizer_band(&self, band: EqualizerBand, decibels: i32) -> Self {
        let clamped = decibels.clamp(MINIMUM_EQUALIZER_DECIBELS, MAXIMUM_EQUALIZER_DECIBELS);
        let mut equalizer = self.equalizer_decibels;
        match band {
            EqualizerBand::Bass => equalizer.bass = clamped,
            EqualizerBand::Mids => equalizer.mids = clamped,
            EqualizerBand::Treble => equalizer.treble = clamped,
        }
        Self {
            voice_enhancement_curve: EnhancementCurve::Custom,
            equalizer_decibels: equalizer,
            ..self.clone()
        }
    }
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

fn clamped_integer(value: Option<&Value>, minimum: i64, maximum: i64, fallback: i64) -> i64 {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        // Rounded and clamped in `f64`, then within `minimum..=maximum`: the cast cannot truncate.
        .map_or(fallback, |number| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
            let rounded = number.round().clamp(minimum as f64, maximum as f64) as i64;
            rounded
        })
}

fn clamped_percent(value: Option<&Value>, maximum: u32, fallback: u32) -> u32 {
    let clamped = clamped_integer(
        value,
        i64::from(MINIMUM_VOLUME_PERCENT),
        i64::from(maximum),
        i64::from(fallback),
    );
    u32::try_from(clamped).unwrap_or(fallback)
}

fn stored_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && text.len() <= MAXIMUM_STORED_TEXT_LENGTH)
        .map(str::to_owned)
}

fn stored_push_to_talk_key(value: Option<&Value>) -> Option<PushToTalkKey> {
    let fields = value?.as_object()?;
    let code = stored_text(fields.get("code"))?;
    let label = stored_text(fields.get("label"))?;
    is_recordable_push_to_talk_code(&code).then_some(PushToTalkKey { code, label })
}

/// Before ADR-0031 noise suppression was a switch: off stays off, on gets the new default.
fn stored_noise_suppression(fields: &Map<String, Value>) -> NoiseSuppression {
    let stored = fields.get("noiseSuppression").and_then(Value::as_str);
    if let Some(mode) = NoiseSuppression::ALL
        .into_iter()
        .find(|mode| Some(mode.as_str()) == stored)
    {
        return mode;
    }
    if fields.get("isNoiseSuppressionOn") == Some(&Value::Bool(false)) {
        NoiseSuppression::Off
    } else {
        VoiceSettings::DEFAULT.noise_suppression
    }
}

fn stored_curve(value: Option<&Value>) -> EnhancementCurve {
    let stored = value.and_then(Value::as_str);
    if stored == Some("custom") {
        return EnhancementCurve::Custom;
    }
    VoiceEnhancementPreset::ALL
        .into_iter()
        .find(|preset| Some(preset.as_str()) == stored)
        .map_or(
            VoiceSettings::DEFAULT.voice_enhancement_curve,
            EnhancementCurve::Preset,
        )
}

/// A preset's values come from the preset, so retuning one in a later version reaches everyone.
fn stored_equalizer(curve: EnhancementCurve, value: Option<&Value>) -> EqualizerDecibels {
    if let EnhancementCurve::Preset(preset) = curve {
        return preset.equalizer();
    }
    let bands = value.and_then(Value::as_object);
    let band = |name: &str| {
        let decibels = clamped_integer(
            bands.and_then(|bands| bands.get(name)),
            i64::from(MINIMUM_EQUALIZER_DECIBELS),
            i64::from(MAXIMUM_EQUALIZER_DECIBELS),
            0,
        );
        i32::try_from(decibels).unwrap_or(0)
    };
    EqualizerDecibels {
        bass: band("bass"),
        mids: band("mids"),
        treble: band("treble"),
    }
}

/// The settings from the web's stored JSON; the defaults for anything unreadable.
#[must_use]
pub fn parse_voice_settings(stored: &Value) -> VoiceSettings {
    let Some(fields) = stored.as_object() else {
        return VoiceSettings::DEFAULT;
    };
    let defaults = VoiceSettings::DEFAULT;
    let boolean = |name: &str, fallback: bool| {
        fields
            .get(name)
            .and_then(Value::as_bool)
            .unwrap_or(fallback)
    };
    let curve = stored_curve(fields.get("voiceEnhancementPreset"));
    let release_delay = clamped_integer(
        fields.get("pushToTalkReleaseDelayMilliseconds"),
        0,
        i64::from(MAXIMUM_PUSH_TO_TALK_RELEASE_DELAY_MILLISECONDS),
        i64::from(defaults.push_to_talk_release_delay_milliseconds),
    );
    let threshold = clamped_integer(
        fields.get("sensitivityThresholdDecibels"),
        i64::from(MINIMUM_DECIBELS),
        i64::from(MAXIMUM_DECIBELS),
        i64::from(defaults.sensitivity_threshold_decibels),
    );
    VoiceSettings {
        input_device_id: stored_text(fields.get("inputDeviceId")),
        output_device_id: stored_text(fields.get("outputDeviceId")),
        input_volume_percent: clamped_percent(
            fields.get("inputVolumePercent"),
            MAXIMUM_VOLUME_PERCENT,
            defaults.input_volume_percent,
        ),
        output_volume_percent: clamped_percent(
            fields.get("outputVolumePercent"),
            MAXIMUM_VOLUME_PERCENT,
            defaults.output_volume_percent,
        ),
        input_mode: if fields.get("inputMode").and_then(Value::as_str) == Some("push-to-talk") {
            VoiceInputMode::PushToTalk
        } else {
            VoiceInputMode::VoiceActivity
        },
        push_to_talk_key: stored_push_to_talk_key(fields.get("pushToTalkKey")),
        push_to_talk_release_delay_milliseconds: u32::try_from(release_delay)
            .unwrap_or(defaults.push_to_talk_release_delay_milliseconds),
        are_push_to_talk_sounds_on: boolean(
            "arePushToTalkSoundsOn",
            defaults.are_push_to_talk_sounds_on,
        ),
        is_sensitivity_automatic: boolean(
            "isSensitivityAutomatic",
            defaults.is_sensitivity_automatic,
        ),
        sensitivity_threshold_decibels: i32::try_from(threshold)
            .unwrap_or(defaults.sensitivity_threshold_decibels),
        noise_suppression: stored_noise_suppression(fields),
        is_voice_enhancement_on: boolean("isVoiceEnhancementOn", defaults.is_voice_enhancement_on),
        voice_enhancement_curve: curve,
        equalizer_decibels: stored_equalizer(curve, fields.get("equalizerDecibels")),
        soundboard_volume_percent: clamped_percent(
            fields.get("soundboardVolumePercent"),
            MAXIMUM_SOUNDBOARD_VOLUME_PERCENT,
            defaults.soundboard_volume_percent,
        ),
    }
}

/// The settings in the web's JSON shape.
#[must_use]
pub fn voice_settings_to_json(settings: &VoiceSettings) -> Value {
    json!({
        "inputDeviceId": settings.input_device_id,
        "outputDeviceId": settings.output_device_id,
        "inputVolumePercent": settings.input_volume_percent,
        "outputVolumePercent": settings.output_volume_percent,
        "inputMode": match settings.input_mode {
            VoiceInputMode::VoiceActivity => "voice-activity",
            VoiceInputMode::PushToTalk => "push-to-talk",
        },
        "pushToTalkKey": settings.push_to_talk_key.as_ref().map(|key| json!({
            "code": key.code,
            "label": key.label,
        })),
        "pushToTalkReleaseDelayMilliseconds": settings.push_to_talk_release_delay_milliseconds,
        "arePushToTalkSoundsOn": settings.are_push_to_talk_sounds_on,
        "isSensitivityAutomatic": settings.is_sensitivity_automatic,
        "sensitivityThresholdDecibels": settings.sensitivity_threshold_decibels,
        "noiseSuppression": settings.noise_suppression.as_str(),
        "isVoiceEnhancementOn": settings.is_voice_enhancement_on,
        "voiceEnhancementPreset": match settings.voice_enhancement_curve {
            EnhancementCurve::Preset(preset) => preset.as_str(),
            EnhancementCurve::Custom => "custom",
        },
        "equalizerDecibels": {
            "bass": settings.equalizer_decibels.bass,
            "mids": settings.equalizer_decibels.mids,
            "treble": settings.equalizer_decibels.treble,
        },
        "soundboardVolumePercent": settings.soundboard_volume_percent,
    })
}

// ---- Toggles (voice-toggles.ts) ----

/// The microphone and headset toggles as stored; their rules are the voice engine's
/// (`mellow_media::VoiceToggles`).
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StoredVoiceToggles {
    pub is_microphone_muted: bool,
    pub is_deafened: bool,
    pub was_microphone_muted_before_deafening: bool,
}

/// Field by field; deafened always means muted, whatever was stored.
#[must_use]
pub fn parse_voice_toggles(stored: &Value) -> StoredVoiceToggles {
    let Some(fields) = stored.as_object() else {
        return StoredVoiceToggles::default();
    };
    let is_true = |name: &str| fields.get(name) == Some(&Value::Bool(true));
    let is_deafened = is_true("isDeafened");
    StoredVoiceToggles {
        is_microphone_muted: is_deafened || is_true("isMicrophoneMuted"),
        is_deafened,
        was_microphone_muted_before_deafening: is_true("wasMicrophoneMutedBeforeDeafening"),
    }
}

#[must_use]
pub fn voice_toggles_to_json(toggles: StoredVoiceToggles) -> Value {
    json!({
        "isMicrophoneMuted": toggles.is_microphone_muted,
        "isDeafened": toggles.is_deafened,
        "wasMicrophoneMutedBeforeDeafening": toggles.was_microphone_muted_before_deafening,
    })
}

// ---- Levels ----

/// An RMS level (0 to 1) in dBFS, floored at the meter's −100 dB.
#[must_use]
pub fn decibels_of_level(level: f32) -> f32 {
    if level > 0.0 {
        (20.0 * level.log10()).clamp(MINIMUM_LEVEL_DECIBELS, MAXIMUM_LEVEL_DECIBELS)
    } else {
        MINIMUM_LEVEL_DECIBELS
    }
}

/// Where a level sits on the meter, 0 to 1.
#[must_use]
pub fn meter_fraction_of_decibels(decibels: f32) -> f32 {
    (decibels.clamp(MINIMUM_LEVEL_DECIBELS, MAXIMUM_LEVEL_DECIBELS) - MINIMUM_LEVEL_DECIBELS)
        / (MAXIMUM_LEVEL_DECIBELS - MINIMUM_LEVEL_DECIBELS)
}

/// The meter's two fills, as fractions of its width: the level up to the threshold, and the part
/// past it, which is what others hear (`VoiceLevelMeter.svelte`). Without a threshold (automatic
/// sensitivity) the whole level is "heard".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeterFills {
    pub quiet_width: f32,
    /// Starts at the threshold's mark.
    pub heard_start: f32,
    pub heard_width: f32,
}

/// `decibels` is `None` while nothing listens (an empty meter).
#[must_use]
pub fn meter_fills(decibels: Option<f32>, threshold_decibels: Option<i32>) -> MeterFills {
    let level = decibels.map_or(0.0, meter_fraction_of_decibels);
    // Whole decibels within −100 to 0: exact in `f32`.
    #[allow(clippy::cast_precision_loss)]
    let threshold = threshold_decibels.map_or(0.0, |threshold| {
        meter_fraction_of_decibels(threshold as f32)
    });
    MeterFills {
        quiet_width: level.min(threshold),
        heard_start: threshold,
        heard_width: (level - threshold).max(0.0),
    }
}

// ---- Push-to-talk keys ----

/// Esc cancels recording, and Tab must keep moving focus: neither can be the key.
#[must_use]
pub fn is_recordable_push_to_talk_code(code: &str) -> bool {
    !matches!(code, "Escape" | "Tab" | "" | "Unidentified")
}

/// The key a key press records, or `None` when it cannot be one. `key` is what the press types
/// (or the key's name), `named_key` the catalogue's name for codes whose character says nothing
/// (`voiceSettings.namedKeys`).
#[must_use]
pub fn push_to_talk_key_of(
    code: &str,
    key: &str,
    named_key: impl Fn(&str) -> Option<&'static str>,
) -> Option<PushToTalkKey> {
    if !is_recordable_push_to_talk_code(code) {
        return None;
    }
    let mut characters = key.chars();
    let printed = match (characters.next(), characters.next()) {
        (Some(character), None) if !character.is_whitespace() => Some(key.to_uppercase()),
        _ => None,
    };
    let label = named_key(code)
        .map(str::to_owned)
        .or(printed)
        .unwrap_or_else(|| code.to_owned());
    Some(PushToTalkKey {
        code: code.to_owned(),
        label,
    })
}

/// A key that types a character: held for push-to-talk, it must not also type into the composer.
#[must_use]
pub fn is_character_key(key: &str, has_command_modifier: bool) -> bool {
    key.chars().count() == 1 && !has_command_modifier
}

// ---- Devices ----

/// Whether a remembered device is plugged in, among the ids listed now. Unknown (`true`) while
/// no list could be read.
#[must_use]
pub fn is_remembered_device_present(
    listed_device_ids: Option<&[String]>,
    device_id: Option<&str>,
) -> bool {
    match (device_id, listed_device_ids) {
        (None, _) | (_, None) => true,
        (Some(device_id), Some(listed)) => listed.iter().any(|listed| listed == device_id),
    }
}

/// A device as the system lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedAudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// One device a person can pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceChoice {
    pub device_id: String,
    pub label: String,
}

/// The choices of one kind (`deviceChoices`), after the "Default" entry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceChoices {
    /// The system's default device's name, when the system tells it.
    pub default_device_name: Option<String>,
    pub devices: Vec<DeviceChoice>,
}

/// The choices among `listed`; unnamed devices get `fallback_name(n)`, counted from 1.
#[must_use]
pub fn device_choices(
    listed: &[ListedAudioDevice],
    fallback_name: impl Fn(usize) -> String,
) -> DeviceChoices {
    let name_of = |position: usize, device: &ListedAudioDevice| {
        let name = device.name.trim();
        if name.is_empty() {
            fallback_name(position + 1)
        } else {
            name.to_owned()
        }
    };
    DeviceChoices {
        default_device_name: listed
            .iter()
            .enumerate()
            .find(|(_, device)| device.is_default)
            .map(|(position, device)| name_of(position, device)),
        devices: listed
            .iter()
            .enumerate()
            .map(|(position, device)| DeviceChoice {
                device_id: device.id.clone(),
                label: name_of(position, device),
            })
            .collect(),
    }
}

/// A chosen device that is not listed now: unplugged.
#[must_use]
pub fn is_chosen_device_unplugged(choices: &DeviceChoices, chosen_id: Option<&str>) -> bool {
    chosen_id.is_some_and(|chosen| {
        !choices
            .devices
            .iter()
            .any(|device| device.device_id == chosen)
    })
}

// ---- Per-person volume ----

/// One person's volume and local mute, as the reader set them for themselves only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersonVoiceSetting {
    pub volume_percent: u32,
    pub is_locally_muted: bool,
}

impl Default for PersonVoiceSetting {
    fn default() -> Self {
        Self {
            volume_percent: DEFAULT_VOLUME_PERCENT,
            is_locally_muted: false,
        }
    }
}

impl PersonVoiceSetting {
    #[must_use]
    pub fn is_default(self) -> bool {
        self == Self::default()
    }
}

/// Only valid ids and values; a stored map of people to settings.
#[must_use]
pub fn parse_person_voice_settings(stored: &Value) -> Vec<(Snowflake, PersonVoiceSetting)> {
    let Some(people) = stored.as_object() else {
        return Vec::new();
    };
    people
        .iter()
        .filter_map(|(user_id, value)| {
            let is_decimal = !user_id.is_empty()
                && user_id.len() <= 20
                && user_id.bytes().all(|byte| byte.is_ascii_digit());
            let user_id = is_decimal
                .then(|| user_id.parse::<Snowflake>().ok())
                .flatten()?;
            let fields = value.as_object()?;
            Some((
                user_id,
                PersonVoiceSetting {
                    volume_percent: clamped_percent(
                        fields.get("volumePercent"),
                        MAXIMUM_VOLUME_PERCENT,
                        DEFAULT_VOLUME_PERCENT,
                    ),
                    is_locally_muted: fields.get("isLocallyMuted") == Some(&Value::Bool(true)),
                },
            ))
        })
        .collect()
}

/// Only the people changed from the default are stored.
#[must_use]
pub fn person_voice_settings_to_json(settings: &[(Snowflake, PersonVoiceSetting)]) -> Value {
    Value::Object(
        settings
            .iter()
            .filter(|(_, setting)| !setting.is_default())
            .map(|(user_id, setting)| {
                (
                    user_id.to_string(),
                    json!({
                        "volumePercent": setting.volume_percent,
                        "isLocallyMuted": setting.is_locally_muted,
                    }),
                )
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(code: &str) -> Option<&'static str> {
        match code {
            "ControlRight" => Some("Ctrl droit"),
            "Space" => Some("Espace"),
            _ => None,
        }
    }

    fn listed(id: &str, name: &str, is_default: bool) -> ListedAudioDevice {
        ListedAudioDevice {
            id: id.into(),
            name: name.into(),
            is_default,
        }
    }

    #[test]
    fn lists_the_devices_names_the_default_and_the_unnamed() {
        let choices = device_choices(
            &[
                listed("a1", "MacBook Pro Microphone", true),
                listed("a2", "  ", false),
            ],
            |position| format!("Micro {position}"),
        );
        assert_eq!(
            choices,
            DeviceChoices {
                default_device_name: Some("MacBook Pro Microphone".into()),
                devices: vec![
                    DeviceChoice {
                        device_id: "a1".into(),
                        label: "MacBook Pro Microphone".into(),
                    },
                    DeviceChoice {
                        device_id: "a2".into(),
                        label: "Micro 2".into(),
                    },
                ],
            }
        );
        assert!(!is_chosen_device_unplugged(&choices, None));
        assert!(!is_chosen_device_unplugged(&choices, Some("a2")));
        assert!(is_chosen_device_unplugged(&choices, Some("gone")));
        // PipeWire's default is a node of its own: no name to show.
        let unnamed_default = device_choices(&[listed("a1", "Yeti", false)], |_| String::new());
        assert_eq!(unnamed_default.default_device_name, None);
    }

    #[test]
    fn the_meter_lights_only_the_part_past_the_threshold_as_heard() {
        let fills = meter_fills(Some(-30.0), Some(-50));
        assert!((fills.quiet_width - 0.5).abs() < 1e-6);
        assert!((fills.heard_start - 0.5).abs() < 1e-6);
        assert!((fills.heard_width - 0.2).abs() < 1e-6);
        let below = meter_fills(Some(-70.0), Some(-50));
        assert!((below.quiet_width - 0.3).abs() < 1e-6);
        assert!(below.heard_width.abs() < f32::EPSILON);
        let automatic = meter_fills(Some(-30.0), None);
        assert!(automatic.quiet_width.abs() < f32::EPSILON);
        assert!((automatic.heard_width - 0.7).abs() < 1e-6);
        let silent = meter_fills(None, Some(-50));
        assert!(silent.quiet_width.abs() < f32::EPSILON && silent.heard_width.abs() < f32::EPSILON);
    }

    #[test]
    fn reads_defaults_from_nothing_garbage_or_another_shape() {
        assert_eq!(parse_voice_settings(&Value::Null), VoiceSettings::DEFAULT);
        assert_eq!(parse_voice_settings(&json!([1, 2])), VoiceSettings::DEFAULT);
        assert_eq!(parse_voice_settings(&json!("x")), VoiceSettings::DEFAULT);
    }

    #[test]
    fn validates_each_field_clamps_numbers_drops_unknown_modes_and_unusable_keys() {
        let stored = json!({
            "inputDeviceId": 42,
            "outputDeviceId": "headset",
            "inputVolumePercent": 999,
            "outputVolumePercent": -5,
            "inputMode": "always",
            "pushToTalkKey": { "code": "Escape", "label": "Échap" },
            "pushToTalkReleaseDelayMilliseconds": 2500.4,
            "sensitivityThresholdDecibels": -42.6,
            "isNoiseSuppressionOn": "no",
        });
        assert_eq!(
            parse_voice_settings(&stored),
            VoiceSettings {
                output_device_id: Some("headset".into()),
                input_volume_percent: 200,
                output_volume_percent: 0,
                push_to_talk_release_delay_milliseconds: 2000,
                sensitivity_threshold_decibels: -43,
                ..VoiceSettings::DEFAULT
            }
        );
    }

    #[test]
    fn reads_the_soundboard_volume_from_0_to_100_percent() {
        let read = |fields: Value| parse_voice_settings(&fields).soundboard_volume_percent;
        assert_eq!(read(json!({})), 70);
        assert_eq!(read(json!({ "soundboardVolumePercent": 150 })), 100);
        assert_eq!(read(json!({ "soundboardVolumePercent": -1 })), 0);
        assert_eq!(read(json!({ "soundboardVolumePercent": "loud" })), 70);
    }

    #[test]
    fn keeps_an_older_noise_suppression_switch() {
        let read = |fields: Value| parse_voice_settings(&fields).noise_suppression;
        assert_eq!(
            read(json!({ "isNoiseSuppressionOn": false })),
            NoiseSuppression::Off
        );
        assert_eq!(
            read(json!({ "isNoiseSuppressionOn": true })),
            NoiseSuppression::Advanced
        );
        assert_eq!(
            read(json!({ "noiseSuppression": "browser" })),
            NoiseSuppression::Standard
        );
    }

    #[test]
    fn fills_a_presets_sliders_from_the_preset_and_clamps_a_custom_curve() {
        let preset = parse_voice_settings(&json!({
            "voiceEnhancementPreset": "warm",
            "equalizerDecibels": { "bass": 9, "mids": 9, "treble": 9 },
        }));
        assert_eq!(
            preset.equalizer_decibels,
            VoiceEnhancementPreset::Warm.equalizer()
        );
        let custom = parse_voice_settings(&json!({
            "voiceEnhancementPreset": "custom",
            "equalizerDecibels": { "bass": 40, "mids": -2.4, "treble": "x" },
        }));
        assert_eq!(
            custom.equalizer_decibels,
            EqualizerDecibels {
                bass: 12,
                mids: -2,
                treble: 0
            }
        );
    }

    #[test]
    fn switches_to_custom_when_a_slider_moves_and_back_with_a_preset() {
        let moved = VoiceSettings::DEFAULT.with_equalizer_band(EqualizerBand::Treble, 30);
        assert_eq!(moved.voice_enhancement_curve, EnhancementCurve::Custom);
        assert_eq!(moved.equalizer_decibels.treble, 12);
        let back = moved.with_enhancement_preset(VoiceEnhancementPreset::Clear);
        assert_eq!(
            back.voice_enhancement_curve,
            EnhancementCurve::Preset(VoiceEnhancementPreset::Clear)
        );
        assert_eq!(
            back.equalizer_decibels,
            VoiceEnhancementPreset::Clear.equalizer()
        );
    }

    #[test]
    fn reads_back_what_it_wrote() {
        let settings = VoiceSettings {
            input_device_id: Some("pipewire:alsa_input.usb-Mic".into()),
            input_mode: VoiceInputMode::PushToTalk,
            push_to_talk_key: Some(PushToTalkKey {
                code: "KeyV".into(),
                label: "V".into(),
            }),
            noise_suppression: NoiseSuppression::Standard,
            ..VoiceSettings::DEFAULT.with_equalizer_band(EqualizerBand::Bass, -4)
        };
        assert_eq!(
            parse_voice_settings(&voice_settings_to_json(&settings)),
            settings
        );
    }

    #[test]
    fn converts_an_rms_level_to_dbfs_on_the_meters_scale() {
        assert!((decibels_of_level(1.0) - 0.0).abs() < 1e-4);
        assert!((decibels_of_level(0.1) + 20.0).abs() < 1e-4);
        assert!((decibels_of_level(0.0) + 100.0).abs() < 1e-4);
        assert!((decibels_of_level(1e-9) + 100.0).abs() < 1e-4);
        assert!((meter_fraction_of_decibels(-50.0) - 0.5).abs() < 1e-6);
        assert!((meter_fraction_of_decibels(10.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn records_the_printed_letter_or_the_keys_name() {
        assert_eq!(
            push_to_talk_key_of("KeyQ", "a", named),
            Some(PushToTalkKey {
                code: "KeyQ".into(),
                label: "A".into()
            })
        );
        assert_eq!(
            push_to_talk_key_of("ControlRight", "Control", named).map(|key| key.label),
            Some("Ctrl droit".into())
        );
        assert_eq!(
            push_to_talk_key_of("Space", " ", named).map(|key| key.label),
            Some("Espace".into())
        );
        assert_eq!(
            push_to_talk_key_of("F13", "F13", named).map(|key| key.label),
            Some("F13".into())
        );
    }

    #[test]
    fn refuses_esc_which_cancels_and_tab_which_moves_focus() {
        assert_eq!(push_to_talk_key_of("Escape", "Escape", named), None);
        assert_eq!(push_to_talk_key_of("Tab", "Tab", named), None);
    }

    #[test]
    fn tells_a_key_that_types_a_character() {
        assert!(is_character_key("v", false));
        assert!(is_character_key(" ", false));
        assert!(!is_character_key("Control", false));
        assert!(!is_character_key("v", true));
    }

    #[test]
    fn knows_whether_a_remembered_device_is_plugged_in_or_cannot_tell() {
        let listed = vec!["a".to_owned(), "b".to_owned()];
        assert!(is_remembered_device_present(Some(&listed), None));
        assert!(is_remembered_device_present(Some(&listed), Some("b")));
        assert!(!is_remembered_device_present(Some(&listed), Some("c")));
        assert!(is_remembered_device_present(None, Some("c")));
    }

    #[test]
    fn round_trips_the_toggles_and_deafened_always_means_muted() {
        let toggles = StoredVoiceToggles {
            is_microphone_muted: true,
            is_deafened: true,
            was_microphone_muted_before_deafening: false,
        };
        assert_eq!(
            parse_voice_toggles(&voice_toggles_to_json(toggles)),
            toggles
        );
        assert!(parse_voice_toggles(&json!({ "isDeafened": true })).is_microphone_muted);
        assert_eq!(
            parse_voice_toggles(&json!(null)),
            StoredVoiceToggles::default()
        );
    }

    #[test]
    fn reads_only_valid_people_and_values() {
        let stored = json!({
            "7": { "volumePercent": 250, "isLocallyMuted": true },
            "not-an-id": { "volumePercent": 50 },
            "8": "loud",
        });
        assert_eq!(
            parse_person_voice_settings(&stored),
            vec![(
                Snowflake::from_raw(7),
                PersonVoiceSetting {
                    volume_percent: 200,
                    is_locally_muted: true
                }
            )]
        );
    }

    #[test]
    fn stores_only_the_people_changed_from_the_default() {
        let stored = person_voice_settings_to_json(&[
            (Snowflake::from_raw(7), PersonVoiceSetting::default()),
            (
                Snowflake::from_raw(8),
                PersonVoiceSetting {
                    volume_percent: 60,
                    is_locally_muted: false,
                },
            ),
        ]);
        assert_eq!(
            stored,
            json!({ "8": { "volumePercent": 60, "isLocallyMuted": false } })
        );
    }
}
