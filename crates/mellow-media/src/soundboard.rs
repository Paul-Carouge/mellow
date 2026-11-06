//! Soundboard playback (ADR-0032), as `web/src/lib/soundboard.ts`: every client downloads a
//! played sound once, decodes the instance's canonical WAV and plays it locally at the output
//! volume times the soundboard volume. The rules of who hears what and when the soundboard may
//! play are pure functions here; the cooldown clock lives in the client core's voice state.

use std::collections::VecDeque;

use mellow_client_core::{ClientError, Download, DownloadableFile, RestClient};
use mellow_protocol::{Permissions, Snowflake, SoundboardSound};

use crate::audio_format::VOICE_SAMPLE_RATE;
use crate::mixer::clamped_volume;
use crate::sounds::LocalSound;

/// Discord's limit, which the instance enforces on upload: 5.2 seconds.
const MAXIMUM_SOUND_DURATION_MILLISECONDS: usize = 5_200;
const MAXIMUM_SOUND_SAMPLES: usize =
    VOICE_SAMPLE_RATE as usize / 1_000 * MAXIMUM_SOUND_DURATION_MILLISECONDS;
const CANONICAL_WAV_HEADER_BYTES: usize = 44;
const BYTES_PER_SAMPLE: usize = 2;
/// The most a sound download reads: a canonical WAV of the longest sound.
pub const MAXIMUM_SOUND_FILE_BYTES: usize =
    CANONICAL_WAV_HEADER_BYTES + MAXIMUM_SOUND_SAMPLES * BYTES_PER_SAMPLE;
/// `fmt ` chunk of 16 bytes: PCM, one channel, 48 kHz, 96 000 bytes per second, 2-byte blocks,
/// 16 bits (the server's `CANONICAL_FORMAT_CHUNK`).
const CANONICAL_FORMAT_CHUNK: [u8; 24] = [
    b'f', b'm', b't', b' ', 16, 0, 0, 0, 1, 0, 1, 0, 0x80, 0xBB, 0, 0, 0x00, 0x77, 0x01, 0, 2, 0,
    16, 0,
];
/// The soundboard slider stops at 100 %, as Discord's: a sound is never louder than its file.
pub const MAXIMUM_SOUNDBOARD_VOLUME: f32 = 1.0;
/// The web client's default soundboard volume.
pub const DEFAULT_SOUNDBOARD_VOLUME: f32 = 0.7;
/// Sounds kept decoded; a guild holds at most 48, and the oldest goes first past this.
const MAXIMUM_CACHED_SOUNDS: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum SoundboardError {
    #[error("the sound could not be downloaded: {0}")]
    Download(#[from] ClientError),
    #[error("the instance answered 'not modified' to a download without a cached copy")]
    UnexpectedNotModified,
    #[error("the sound is not the instance's canonical WAV")]
    NotCanonicalWav,
}

/// The samples of a canonical WAV (PCM 16-bit, mono, 48 kHz, 1 sample to 5.2 s, nothing after
/// the samples), checked as strictly as the instance checks uploads.
///
/// # Errors
/// `NotCanonicalWav` for anything else.
pub fn decode_canonical_wav(file: &[u8]) -> Result<LocalSound, SoundboardError> {
    let read_u32 = |offset: usize| {
        file.get(offset..offset + 4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
            .and_then(|value| usize::try_from(value).ok())
    };
    let samples_bytes = file.len().saturating_sub(CANONICAL_WAV_HEADER_BYTES);
    let is_canonical = file.starts_with(b"RIFF")
        && file.get(8..12) == Some(b"WAVE")
        && file.get(12..36) == Some(&CANONICAL_FORMAT_CHUNK)
        && file.get(36..40) == Some(b"data")
        && read_u32(4) == Some(file.len() - 8)
        && read_u32(40) == Some(samples_bytes)
        && samples_bytes > 0
        && samples_bytes.is_multiple_of(BYTES_PER_SAMPLE)
        && file.len() <= MAXIMUM_SOUND_FILE_BYTES;
    let Some(samples) = file
        .get(CANONICAL_WAV_HEADER_BYTES..)
        .filter(|_| is_canonical)
    else {
        return Err(SoundboardError::NotCanonicalWav);
    };
    Ok(samples
        .as_chunks::<BYTES_PER_SAMPLE>()
        .0
        .iter()
        .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32_768.0)
        .collect())
}

/// Downloads and decodes a sound; run it on the async runtime, never the interface thread.
///
/// # Errors
/// The download failed, or the file is not a canonical WAV.
pub async fn download_sound(
    client: &RestClient,
    sound_id: Snowflake,
) -> Result<LocalSound, SoundboardError> {
    match client
        .download(
            DownloadableFile::Sound(sound_id),
            None,
            MAXIMUM_SOUND_FILE_BYTES,
        )
        .await?
    {
        Download::Fetched(file) => decode_canonical_wav(&file.bytes),
        Download::NotModified => Err(SoundboardError::UnexpectedNotModified),
    }
}

/// Decoded sounds by id, so each is downloaded once (sounds never change: an edit keeps the
/// file, a new file is a new sound).
#[derive(Debug, Default)]
pub struct SoundCache {
    sounds: VecDeque<(Snowflake, LocalSound)>,
}

impl SoundCache {
    #[must_use]
    pub fn get(&self, sound_id: Snowflake) -> Option<LocalSound> {
        self.sounds
            .iter()
            .find(|(id, _)| *id == sound_id)
            .map(|(_, sound)| sound.clone())
    }

    pub fn insert(&mut self, sound_id: Snowflake, sound: LocalSound) {
        self.sounds.retain(|(id, _)| *id != sound_id);
        if self.sounds.len() >= MAXIMUM_CACHED_SOUNDS {
            self.sounds.pop_front();
        }
        self.sounds.push_back((sound_id, sound));
    }

    /// A deleted sound is never played again.
    pub fn remove(&mut self, sound_id: Snowflake) {
        self.sounds.retain(|(id, _)| *id != sound_id);
    }
}

/// Why the soundboard cannot play now; the instance would silently ignore the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundboardBlock {
    NotConnected,
    NoPermission,
    CannotSpeak,
    Deafened,
    CoolingDown,
}

/// What decides whether the soundboard can play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundboardFacts {
    /// Voice is connected (not joining or reconnecting).
    pub is_connected: bool,
    /// Your permissions in the connected channel; `None` while unknown (the instance decides).
    pub channel_permissions: Option<Permissions>,
    pub is_deafened: bool,
    pub now_unix_milliseconds: u64,
    pub cooldown_ends_at_unix_milliseconds: u64,
}

/// The first reason in the order a person can act on it; self-mute does not block (ADR-0032).
#[must_use]
pub fn soundboard_block(facts: SoundboardFacts) -> Option<SoundboardBlock> {
    if !facts.is_connected {
        return Some(SoundboardBlock::NotConnected);
    }
    if let Some(permissions) = facts.channel_permissions {
        if !permissions.contains(Permissions::USE_SOUNDBOARD) {
            return Some(SoundboardBlock::NoPermission);
        }
        if !permissions.contains(Permissions::SPEAK) {
            return Some(SoundboardBlock::CannotSpeak);
        }
    }
    if facts.is_deafened {
        return Some(SoundboardBlock::Deafened);
    }
    (facts.now_unix_milliseconds < facts.cooldown_ends_at_unix_milliseconds)
        .then_some(SoundboardBlock::CoolingDown)
}

/// A played sound is heard in your channel only, never deafened, never from someone muted for
/// you.
#[must_use]
pub fn should_hear_sound(
    sound_channel_id: Snowflake,
    my_channel_id: Option<Snowflake>,
    is_connected: bool,
    is_deafened: bool,
    is_player_muted_for_me: bool,
) -> bool {
    is_connected
        && my_channel_id == Some(sound_channel_id)
        && !is_deafened
        && !is_player_muted_for_me
}

/// What plays when someone comes into your channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinCue {
    /// Their join sound, still on the instance.
    Sound(Snowflake),
    /// The usual peer-join cue.
    Chime,
}

/// Their join sound when the instance still has it and you would hear it, else the cue
/// (deafened, muted for you, or no sound chosen).
#[must_use]
pub fn join_cue_of(
    join_sound_id: Option<Snowflake>,
    sounds: &[SoundboardSound],
    is_deafened: bool,
    is_muted_for_me: bool,
) -> JoinCue {
    match join_sound_id {
        Some(sound_id)
            if !is_deafened
                && !is_muted_for_me
                && sounds.iter().any(|sound| sound.id == sound_id) =>
        {
            JoinCue::Sound(sound_id)
        }
        _ => JoinCue::Chime,
    }
}

/// Sounds play at your output volume (up to 200 %) times your soundboard volume (up to 100 %).
#[must_use]
pub fn soundboard_gain(output_volume: f32, soundboard_volume: f32) -> f32 {
    clamped_volume(output_volume) * clamped_volume(soundboard_volume).min(MAXIMUM_SOUNDBOARD_VOLUME)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical_wav(samples: &[i16]) -> Vec<u8> {
        let data_bytes = u32::try_from(samples.len() * 2).unwrap();
        let mut file = Vec::new();
        file.extend_from_slice(b"RIFF");
        file.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        file.extend_from_slice(b"WAVE");
        file.extend_from_slice(&CANONICAL_FORMAT_CHUNK);
        file.extend_from_slice(b"data");
        file.extend_from_slice(&data_bytes.to_le_bytes());
        for sample in samples {
            file.extend_from_slice(&sample.to_le_bytes());
        }
        file
    }

    #[test]
    fn a_canonical_wav_decodes_to_its_samples() {
        let sound = decode_canonical_wav(&canonical_wav(&[0, 16_384, -32_768])).unwrap();
        assert_eq!(&sound[..], &[0.0, 0.5, -1.0]);
    }

    #[test]
    fn anything_else_is_refused() {
        let valid = canonical_wav(&[1, 2]);
        let mut trailing = valid.clone();
        trailing.push(0);
        let mut stereo = valid.clone();
        stereo[22] = 2;
        let too_long = canonical_wav(&vec![0; MAXIMUM_SOUND_SAMPLES + 1]);
        for file in [
            &b""[..],
            &valid[..20],
            &trailing,
            &stereo,
            &too_long,
            &canonical_wav(&[]),
        ] {
            assert!(decode_canonical_wav(file).is_err());
        }
        assert!(decode_canonical_wav(&canonical_wav(&vec![0; MAXIMUM_SOUND_SAMPLES])).is_ok());
    }

    #[test]
    fn the_cache_keeps_the_latest_sounds() {
        let mut cache = SoundCache::default();
        for id in 0..=MAXIMUM_CACHED_SOUNDS as u64 {
            cache.insert(Snowflake::from_raw(id), vec![0.0].into());
        }
        assert!(cache.get(Snowflake::from_raw(0)).is_none());
        assert!(cache.get(Snowflake::from_raw(1)).is_some());
        cache.remove(Snowflake::from_raw(1));
        assert!(cache.get(Snowflake::from_raw(1)).is_none());
    }

    fn facts() -> SoundboardFacts {
        SoundboardFacts {
            is_connected: true,
            channel_permissions: Some(Permissions::USE_SOUNDBOARD | Permissions::SPEAK),
            is_deafened: false,
            now_unix_milliseconds: 10_000,
            cooldown_ends_at_unix_milliseconds: 0,
        }
    }

    #[test]
    fn the_soundboard_says_the_first_reason_it_cannot_play() {
        assert_eq!(soundboard_block(facts()), None);
        assert_eq!(
            soundboard_block(SoundboardFacts {
                channel_permissions: None,
                ..facts()
            }),
            None,
            "unknown permissions: the instance decides"
        );
        let cases = [
            (
                SoundboardFacts {
                    is_connected: false,
                    is_deafened: true,
                    ..facts()
                },
                SoundboardBlock::NotConnected,
            ),
            (
                SoundboardFacts {
                    channel_permissions: Some(Permissions::SPEAK),
                    ..facts()
                },
                SoundboardBlock::NoPermission,
            ),
            (
                SoundboardFacts {
                    channel_permissions: Some(Permissions::USE_SOUNDBOARD),
                    ..facts()
                },
                SoundboardBlock::CannotSpeak,
            ),
            (
                SoundboardFacts {
                    is_deafened: true,
                    ..facts()
                },
                SoundboardBlock::Deafened,
            ),
            (
                SoundboardFacts {
                    cooldown_ends_at_unix_milliseconds: 10_001,
                    ..facts()
                },
                SoundboardBlock::CoolingDown,
            ),
        ];
        for (facts, block) in cases {
            assert_eq!(soundboard_block(facts), Some(block));
        }
    }

    #[test]
    fn sounds_are_heard_in_your_channel_only_and_never_deafened_or_from_the_muted() {
        let here = Snowflake::from_raw(1);
        assert!(should_hear_sound(here, Some(here), true, false, false));
        assert!(!should_hear_sound(
            here,
            Some(Snowflake::from_raw(2)),
            true,
            false,
            false
        ));
        assert!(!should_hear_sound(here, Some(here), false, false, false));
        assert!(!should_hear_sound(here, Some(here), true, true, false));
        assert!(!should_hear_sound(here, Some(here), true, false, true));
    }

    #[test]
    fn a_join_sound_replaces_the_cue_only_when_it_exists_and_would_be_heard() {
        let sound = SoundboardSound {
            id: Snowflake::from_raw(7),
            name: "horn".into(),
            emoji: None,
            duration_milliseconds: 1_000,
        };
        let sounds = [sound];
        let id = Some(Snowflake::from_raw(7));
        assert_eq!(
            join_cue_of(id, &sounds, false, false),
            JoinCue::Sound(Snowflake::from_raw(7))
        );
        assert_eq!(join_cue_of(id, &[], false, false), JoinCue::Chime);
        assert_eq!(join_cue_of(id, &sounds, true, false), JoinCue::Chime);
        assert_eq!(join_cue_of(id, &sounds, false, true), JoinCue::Chime);
        assert_eq!(join_cue_of(None, &sounds, false, false), JoinCue::Chime);
    }

    #[test]
    fn the_soundboard_slider_never_makes_a_sound_louder_than_its_file() {
        assert!((soundboard_gain(1.0, 0.7) - 0.7).abs() < f32::EPSILON);
        assert!((soundboard_gain(2.0, 2.0) - 2.0).abs() < f32::EPSILON);
    }
}
