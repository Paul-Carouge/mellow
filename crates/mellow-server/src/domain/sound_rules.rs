//! Rules of soundboard sounds (ADR-0032) that need no I/O: names, emoji and the one audio format
//! the server stores.

use crate::domain::guild_rules::{GuildRuleViolation, trimmed_within};
use crate::domain::message_rules::reaction_emoji_key;

/// As on Discord at its highest tier.
pub(crate) const MAXIMUM_SOUNDS_PER_GUILD: u32 = 48;
pub(crate) const MINIMUM_SOUND_NAME_LENGTH: usize = 2;
pub(crate) const MAXIMUM_SOUND_NAME_LENGTH: usize = 32;
pub(crate) const SOUND_SAMPLE_RATE_HERTZ: u32 = 48_000;
const SOUND_BYTES_PER_SAMPLE: u32 = 2;
/// Discord's limit: 5.2 seconds.
pub(crate) const MAXIMUM_SOUND_DURATION_MILLISECONDS: u32 = 5_200;
const MAXIMUM_SOUND_SAMPLES: u32 =
    SOUND_SAMPLE_RATE_HERTZ / 1_000 * MAXIMUM_SOUND_DURATION_MILLISECONDS;
const CANONICAL_WAV_HEADER_BYTES: usize = 44;
/// The largest upload read: a canonical WAV of the longest sound.
pub(crate) const MAXIMUM_SOUND_FILE_BYTES: usize =
    CANONICAL_WAV_HEADER_BYTES + (MAXIMUM_SOUND_SAMPLES * SOUND_BYTES_PER_SAMPLE) as usize;

/// `fmt ` chunk of 16 bytes: PCM, one channel, 48 kHz, 96 000 bytes per second, 2-byte blocks,
/// 16 bits.
const CANONICAL_FORMAT_CHUNK: [u8; 24] = [
    b'f', b'm', b't', b' ', 16, 0, 0, 0, 1, 0, 1, 0, 0x80, 0xBB, 0, 0, 0x00, 0x77, 0x01, 0, 2, 0,
    16, 0,
];

/// Checks that `file` is a canonical PCM WAV, mono, 48 kHz, 16-bit, of 1 sample to 5.2 seconds,
/// with nothing before or after its samples, and returns its duration in milliseconds, rounded
/// up.
///
/// # Errors
/// `SoundFile` for anything else.
pub(crate) fn canonical_wav_duration_milliseconds(file: &[u8]) -> Result<u32, GuildRuleViolation> {
    let read_u32 = |offset: usize| {
        file.get(offset..offset + 4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
    };
    let (Some(riff_size), Some(data_size)) = (read_u32(4), read_u32(40)) else {
        return Err(GuildRuleViolation::SoundFile);
    };
    let samples_bytes = file.len().saturating_sub(CANONICAL_WAV_HEADER_BYTES);
    let is_canonical = file.starts_with(b"RIFF")
        && file.get(8..12) == Some(b"WAVE")
        && file.get(12..36) == Some(&CANONICAL_FORMAT_CHUNK)
        && file.get(36..40) == Some(b"data")
        && usize::try_from(riff_size).is_ok_and(|size| size == file.len() - 8)
        && usize::try_from(data_size).is_ok_and(|size| size == samples_bytes)
        && samples_bytes > 0
        && samples_bytes.is_multiple_of(SOUND_BYTES_PER_SAMPLE as usize)
        && file.len() <= MAXIMUM_SOUND_FILE_BYTES;
    if !is_canonical {
        return Err(GuildRuleViolation::SoundFile);
    }
    let samples = data_size / SOUND_BYTES_PER_SAMPLE;
    Ok((samples * 1_000).div_ceil(SOUND_SAMPLE_RATE_HERTZ))
}

/// A sound's name, trimmed: 2 to 32 characters without hidden ones.
///
/// # Errors
/// `SoundName` otherwise.
pub(crate) fn normalize_sound_name(name: &str) -> Result<String, GuildRuleViolation> {
    trimmed_within(name, MINIMUM_SOUND_NAME_LENGTH, MAXIMUM_SOUND_NAME_LENGTH)
        .map(str::to_owned)
        .ok_or(GuildRuleViolation::SoundName)
}

/// A sound's optional emoji, as a reaction key (ADR-0028); empty means none.
///
/// # Errors
/// `SoundEmoji` for anything that is not one emoji.
pub(crate) fn normalize_sound_emoji(
    emoji: Option<&str>,
) -> Result<Option<String>, GuildRuleViolation> {
    match emoji.map(str::trim) {
        None | Some("") => Ok(None),
        Some(emoji) => reaction_emoji_key(emoji)
            .map(Some)
            .map_err(|_| GuildRuleViolation::SoundEmoji),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A canonical WAV of `samples` samples, as the web client encodes it.
    pub(crate) fn canonical_wav(samples: u32) -> Vec<u8> {
        let data_bytes = samples * SOUND_BYTES_PER_SAMPLE;
        let mut file = Vec::new();
        file.extend_from_slice(b"RIFF");
        file.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        file.extend_from_slice(b"WAVE");
        file.extend_from_slice(&CANONICAL_FORMAT_CHUNK);
        file.extend_from_slice(b"data");
        file.extend_from_slice(&data_bytes.to_le_bytes());
        file.resize(file.len() + data_bytes as usize, 0x7F);
        file
    }

    #[test]
    fn canonical_files_give_their_duration_rounded_up() {
        assert_eq!(
            canonical_wav_duration_milliseconds(&canonical_wav(1)),
            Ok(1)
        );
        assert_eq!(
            canonical_wav_duration_milliseconds(&canonical_wav(48_000)),
            Ok(1_000)
        );
        assert_eq!(
            canonical_wav_duration_milliseconds(&canonical_wav(MAXIMUM_SOUND_SAMPLES)),
            Ok(MAXIMUM_SOUND_DURATION_MILLISECONDS)
        );
        assert_eq!(
            canonical_wav(MAXIMUM_SOUND_SAMPLES).len(),
            MAXIMUM_SOUND_FILE_BYTES
        );
    }

    #[test]
    fn anything_but_a_canonical_file_is_refused() {
        let valid = canonical_wav(480);
        let with_byte_at = |offset: usize, byte: u8| {
            let mut file = valid.clone();
            file[offset] = byte;
            file
        };
        let mut trailing = valid.clone();
        trailing.push(0);
        let mut odd = canonical_wav(480);
        odd.pop();
        let refused = [
            Vec::new(),
            valid[..43].to_vec(),
            canonical_wav(0),
            canonical_wav(MAXIMUM_SOUND_SAMPLES + 1),
            trailing,
            odd,
            with_byte_at(0, b'X'),
            with_byte_at(8, b'X'),
            // Two channels, 44.1 kHz, 8 bits, a float format, a wrong RIFF or data size.
            with_byte_at(22, 2),
            with_byte_at(24, 0x44),
            with_byte_at(34, 8),
            with_byte_at(20, 3),
            with_byte_at(4, 0),
            with_byte_at(40, 0),
        ];
        for file in refused {
            assert_eq!(
                canonical_wav_duration_milliseconds(&file),
                Err(GuildRuleViolation::SoundFile),
                "{:?}",
                file.get(..44)
            );
        }
    }

    #[test]
    fn names_are_trimmed_and_emoji_are_reaction_keys_or_none() {
        assert_eq!(normalize_sound_name("  Air horn "), Ok("Air horn".into()));
        assert_eq!(normalize_sound_name("é!"), Ok("é!".into()));
        for name in ["", " a ", &"x".repeat(33), "a\u{200B}b"] {
            assert_eq!(
                normalize_sound_name(name),
                Err(GuildRuleViolation::SoundName)
            );
        }
        assert_eq!(normalize_sound_emoji(None), Ok(None));
        assert_eq!(normalize_sound_emoji(Some(" ")), Ok(None));
        assert_eq!(normalize_sound_emoji(Some("📯")), Ok(Some("📯".into())));
        assert_eq!(normalize_sound_emoji(Some(":42")), Ok(Some(":42".into())));
        assert_eq!(
            normalize_sound_emoji(Some("horn")),
            Err(GuildRuleViolation::SoundEmoji)
        );
    }
}
