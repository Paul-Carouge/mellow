//! Each person's volume and local mute, set by this user for themselves only and remembered across
//! sessions (`mellow.voice-person-settings` and `mellow.stream-sound-settings` in the web client).
//! The desktop implements the store; the session reads it whenever a person gets a receive slot.

use std::collections::HashMap;
use std::sync::Mutex;

use mellow_protocol::Snowflake;

use crate::mixer::clamped_volume;

/// What a setting applies to: a person's voice, or the sound of the screen they share (kept
/// apart, as on Discord).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PersonSoundSource {
    Voice,
    ScreenSound,
}

/// One person's volume and local mute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PersonVoiceSetting {
    /// 0 to 2 (200 %).
    pub volume: f32,
    pub is_locally_muted: bool,
}

impl Default for PersonVoiceSetting {
    fn default() -> Self {
        Self {
            volume: 1.0,
            is_locally_muted: false,
        }
    }
}

impl PersonVoiceSetting {
    /// The volume clamped to the slider's range, as the web reads stored values.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            volume: clamped_volume(self.volume),
            ..self
        }
    }

    /// Only settings away from the default need storing.
    #[must_use]
    pub fn is_default(self) -> bool {
        self == Self::default()
    }
}

/// Where the settings live. Called from the session's threads: reads must not block (keep the
/// settings in memory), and a write may persist later, off the caller's thread.
pub trait PersonVoiceSettingsStore: Send + Sync {
    fn person_setting(&self, source: PersonSoundSource, user_id: Snowflake) -> PersonVoiceSetting;
    fn write_person_setting(
        &self,
        source: PersonSoundSource,
        user_id: Snowflake,
        setting: PersonVoiceSetting,
    );
}

/// A store that forgets at exit: for tests, and before the desktop wires its own.
#[derive(Debug, Default)]
pub struct InMemoryPersonVoiceSettings {
    settings: Mutex<HashMap<(PersonSoundSource, Snowflake), PersonVoiceSetting>>,
}

impl PersonVoiceSettingsStore for InMemoryPersonVoiceSettings {
    fn person_setting(&self, source: PersonSoundSource, user_id: Snowflake) -> PersonVoiceSetting {
        self.settings
            .lock()
            .map(|settings| {
                settings
                    .get(&(source, user_id))
                    .copied()
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }

    fn write_person_setting(
        &self,
        source: PersonSoundSource,
        user_id: Snowflake,
        setting: PersonVoiceSetting,
    ) {
        if let Ok(mut settings) = self.settings.lock() {
            if setting.is_default() {
                settings.remove(&(source, user_id));
            } else {
                settings.insert((source, user_id), setting.clamped());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_in_memory_store_keeps_voices_and_screen_sounds_apart() {
        let store = InMemoryPersonVoiceSettings::default();
        let user_id = Snowflake::from_raw(4);
        let quiet = PersonVoiceSetting {
            volume: 0.3,
            is_locally_muted: false,
        };
        store.write_person_setting(PersonSoundSource::Voice, user_id, quiet);
        assert_eq!(
            store.person_setting(PersonSoundSource::Voice, user_id),
            quiet
        );
        assert!(
            store
                .person_setting(PersonSoundSource::ScreenSound, user_id)
                .is_default()
        );
        store.write_person_setting(
            PersonSoundSource::Voice,
            user_id,
            PersonVoiceSetting {
                volume: 9.0,
                is_locally_muted: true,
            },
        );
        assert!(
            (store
                .person_setting(PersonSoundSource::Voice, user_id)
                .volume
                - 2.0)
                .abs()
                < f32::EPSILON
        );
    }
}
