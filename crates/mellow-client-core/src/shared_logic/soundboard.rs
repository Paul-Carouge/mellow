//! Port of the soundboard rules of `web/src/lib/soundboard.ts` (ADR-0032): the one WAV format
//! the instance accepts, the part of a file a person keeps, sound names, when the soundboard may
//! play, who hears a played sound and what plays when someone joins. The cooldown clock is in the
//! voice state; decoding files and playing sounds is the media engine's.

use mellow_protocol::{
    ApiErrorCode, MAXIMUM_SOUND_SAMPLES, Permissions, SOUND_SAMPLE_RATE_HERTZ, Snowflake,
    SoundboardSound,
};

use crate::ClientError;

/// A server holds at most this many sounds (`sound_rules.rs`).
pub const MAXIMUM_SOUNDS_PER_GUILD: usize = 48;
pub const MINIMUM_SOUND_NAME_LENGTH: usize = 2;
pub const MAXIMUM_SOUND_NAME_LENGTH: usize = 32;

// ---- The WAV file ----

const WAV_HEADER_BYTES: usize = 44;
const FMT_CHUNK_BYTES: u32 = 16;
const PCM_FORMAT: u16 = 1;
const CHANNEL_COUNT: u16 = 1;
const BYTES_PER_SAMPLE: u16 = 2;
const BITS_PER_SAMPLE: u16 = 16;

/// A sound holds 1 to `MAXIMUM_SOUND_SAMPLES` samples; the trimmer never makes another length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a sound holds 1 to {MAXIMUM_SOUND_SAMPLES} samples, not {0}")]
pub struct SoundLengthError(pub usize);

/// The canonical file the instance accepts, exactly: a 44-byte RIFF/WAVE header (PCM, mono,
/// 48 kHz, 16-bit) and the samples, little-endian, nothing after. Samples outside -1 to 1 are
/// clamped; `NaN` becomes silence.
///
/// # Errors
/// An empty or too long sound.
pub fn encode_wav(samples: &[f32]) -> Result<Vec<u8>, SoundLengthError> {
    let maximum = usize::try_from(MAXIMUM_SOUND_SAMPLES).unwrap_or(usize::MAX);
    if samples.is_empty() || samples.len() > maximum {
        return Err(SoundLengthError(samples.len()));
    }
    let data_bytes = samples.len() * usize::from(BYTES_PER_SAMPLE);
    // Bounded by the check above: at most about 500 kB.
    let to_u32 = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
    let mut bytes = Vec::with_capacity(WAV_HEADER_BYTES + data_bytes);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&to_u32(WAV_HEADER_BYTES + data_bytes - 8).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&FMT_CHUNK_BYTES.to_le_bytes());
    bytes.extend_from_slice(&PCM_FORMAT.to_le_bytes());
    bytes.extend_from_slice(&CHANNEL_COUNT.to_le_bytes());
    bytes.extend_from_slice(&SOUND_SAMPLE_RATE_HERTZ.to_le_bytes());
    let byte_rate =
        SOUND_SAMPLE_RATE_HERTZ * u32::from(CHANNEL_COUNT) * u32::from(BYTES_PER_SAMPLE);
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&(CHANNEL_COUNT * BYTES_PER_SAMPLE).to_le_bytes());
    bytes.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&to_u32(data_bytes).to_le_bytes());
    for sample in samples {
        let clamped = if sample.is_nan() {
            0.0
        } else {
            sample.clamp(-1.0, 1.0)
        };
        let scaled = if clamped < 0.0 {
            clamped * 32_768.0
        } else {
            clamped * 32_767.0
        };
        // Within the `i16` range once clamped.
        #[allow(clippy::cast_possible_truncation)]
        let value = scaled.round() as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Ok(bytes)
}

// ---- Trimming ----

/// The largest file the trimmer opens: decoding holds the whole file in memory, and only 5.2 s
/// are kept anyway.
pub const MAXIMUM_SOUND_SOURCE_BYTES: u64 = 20 * 1024 * 1024;

/// 5.2 s, Discord's longest sound.
pub const MAXIMUM_SOUND_SECONDS: f64 =
    MAXIMUM_SOUND_SAMPLES as f64 / SOUND_SAMPLE_RATE_HERTZ as f64;
/// Shorter selections are hard to hear and to grab; a shorter file is kept whole.
pub const MINIMUM_SELECTION_SECONDS: f64 = 0.1;

/// The part of the file that becomes the sound, in seconds from its start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundSelection {
    pub start_seconds: f64,
    pub end_seconds: f64,
}

impl SoundSelection {
    /// The kept length, to the millisecond.
    #[must_use]
    pub fn kept_milliseconds(self) -> i64 {
        // A few seconds at most.
        #[allow(clippy::cast_possible_truncation)]
        let milliseconds = ((self.end_seconds - self.start_seconds) * 1000.0).round() as i64;
        milliseconds
    }
}

/// `f64::clamp` panics when `minimum > maximum`; the web's `Math.min(max, Math.max(min, v))`
/// gives `maximum` then.
fn clamp(value: f64, minimum: f64, maximum: f64) -> f64 {
    value.max(minimum).min(maximum)
}

fn shortest_selection(duration_seconds: f64) -> f64 {
    MINIMUM_SELECTION_SECONDS.min(duration_seconds)
}

/// The start of the file, as long as a sound can be.
#[must_use]
pub fn initial_selection(duration_seconds: f64) -> SoundSelection {
    SoundSelection {
        start_seconds: 0.0,
        end_seconds: duration_seconds.min(MAXIMUM_SOUND_SECONDS),
    }
}

/// Moving the start keeps the end where it was, unless the length leaves the allowed range.
#[must_use]
pub fn with_selection_start(
    selection: SoundSelection,
    start_seconds: f64,
    duration_seconds: f64,
) -> SoundSelection {
    let shortest = shortest_selection(duration_seconds);
    let start = clamp(start_seconds, 0.0, duration_seconds - shortest);
    let end = clamp(
        selection.end_seconds,
        start + shortest,
        (start + MAXIMUM_SOUND_SECONDS).min(duration_seconds),
    );
    SoundSelection {
        start_seconds: start,
        end_seconds: end,
    }
}

/// Moving the end keeps the start where it was, unless the length leaves the allowed range.
#[must_use]
pub fn with_selection_end(
    selection: SoundSelection,
    end_seconds: f64,
    duration_seconds: f64,
) -> SoundSelection {
    let shortest = shortest_selection(duration_seconds);
    let end = clamp(end_seconds, shortest, duration_seconds);
    let start = clamp(
        selection.start_seconds,
        (end - MAXIMUM_SOUND_SECONDS).max(0.0),
        end - shortest,
    );
    SoundSelection {
        start_seconds: start,
        end_seconds: end,
    }
}

/// A click or drag on the waveform: the selection keeps its length and centres there.
#[must_use]
pub fn with_selection_centred_at(
    selection: SoundSelection,
    centre_seconds: f64,
    duration_seconds: f64,
) -> SoundSelection {
    let length = selection.end_seconds - selection.start_seconds;
    let start = clamp(
        centre_seconds - length / 2.0,
        0.0,
        duration_seconds - length,
    );
    SoundSelection {
        start_seconds: start,
        end_seconds: start + length,
    }
}

/// How many samples the rendered sound holds: never empty, never past the instance's limit.
#[must_use]
pub fn selection_sample_count(selection: SoundSelection) -> usize {
    let samples = ((selection.end_seconds - selection.start_seconds)
        * f64::from(SOUND_SAMPLE_RATE_HERTZ))
    .round();
    // Clamped to 1 to 249 600 first.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = clamp(samples, 1.0, f64::from(MAXIMUM_SOUND_SAMPLES)) as usize;
    count
}

/// The selection as the instance will store it, from the file decoded to mono at 48 kHz: its
/// samples at `volume_percent` (`encode_wav` clamps what the gain pushes past full scale). Past
/// the file's end is silence.
#[must_use]
pub fn render_sound(source: &[f32], selection: SoundSelection, volume_percent: u32) -> Vec<f32> {
    let count = selection_sample_count(selection);
    // Non-negative, and a position within a file of at most a few minutes.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let first =
        (selection.start_seconds.max(0.0) * f64::from(SOUND_SAMPLE_RATE_HERTZ)).round() as usize;
    // Whole percents up to 100: exact in `f32`.
    #[allow(clippy::cast_precision_loss)]
    let gain = volume_percent as f32 / 100.0;
    (first..first + count)
        .map(|index| source.get(index).copied().unwrap_or(0.0) * gain)
        .collect()
}

/// The loudest sample of each of `bucket_count` equal slices of the file, 0 to 1, across its
/// channels: the bars of the trimmer's waveform.
#[must_use]
pub fn waveform_peaks(channels: &[&[f32]], bucket_count: usize) -> Vec<f32> {
    let length = channels
        .iter()
        .map(|channel| channel.len())
        .max()
        .unwrap_or(0);
    let mut peaks = vec![0.0_f32; bucket_count];
    if length == 0 || bucket_count == 0 {
        return peaks;
    }
    for channel in channels {
        for (index, sample) in channel.iter().enumerate() {
            let bucket = (bucket_count - 1).min(index * bucket_count / length);
            let magnitude = sample.abs();
            if let Some(peak) = peaks.get_mut(bucket)
                && magnitude > *peak
            {
                *peak = magnitude.min(1.0);
            }
        }
    }
    peaks
}

/// "0:03.2": the trimmer's times, to the tenth.
#[must_use]
pub fn format_sound_seconds(seconds: f64, decimal_separator: &str) -> String {
    // Seconds of a file of at most 20 MB: far within `u64`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let tenths = (seconds.max(0.0) * 10.0).round() as u64;
    let minutes = tenths / 600;
    let remaining_tenths = tenths - minutes * 600;
    let whole_seconds = remaining_tenths / 10;
    format!(
        "{minutes}:{whole_seconds:02}{decimal_separator}{}",
        remaining_tenths % 10
    )
}

// ---- Names ----

/// Why the instance would refuse a sound's name. Names need not be unique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundNameProblem {
    Length,
    ControlCharacter,
}

/// Counted in code points, as the instance counts.
#[must_use]
pub fn sound_name_problem(name: &str) -> Option<SoundNameProblem> {
    let trimmed = name.trim();
    if trimmed.chars().any(char::is_control) {
        return Some(SoundNameProblem::ControlCharacter);
    }
    let length = trimmed.chars().count();
    (!(MINIMUM_SOUND_NAME_LENGTH..=MAXIMUM_SOUND_NAME_LENGTH).contains(&length))
        .then_some(SoundNameProblem::Length)
}

/// "`airhorn_long-v2.mp3`" → "airhorn long v2": a starting point the person can edit.
#[must_use]
pub fn sound_name_from_file_name(file_name: &str) -> String {
    let without_extension = match file_name.rsplit_once('.') {
        Some((stem, extension)) if (1..=5).contains(&extension.chars().count()) => stem,
        _ => file_name,
    };
    let spaced: String = without_extension
        .chars()
        .filter(|character| !character.is_control())
        .map(|character| match character {
            '_' | '-' => ' ',
            other => other,
        })
        .collect();
    let joined = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    joined
        .chars()
        .take(MAXIMUM_SOUND_NAME_LENGTH)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Why the instance refused a new sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundUploadRefusal {
    /// The server holds its 48 sounds.
    LimitReached,
    TooMany,
    /// Not the canonical WAV, or a name it refused.
    Refused,
    Other,
}

#[must_use]
pub fn sound_upload_refusal(error: &ClientError) -> SoundUploadRefusal {
    match error {
        ClientError::Api { code, .. } => match code {
            ApiErrorCode::LimitReached => SoundUploadRefusal::LimitReached,
            ApiErrorCode::RateLimited => SoundUploadRefusal::TooMany,
            ApiErrorCode::ValidationFailed => SoundUploadRefusal::Refused,
            _ => SoundUploadRefusal::Other,
        },
        _ => SoundUploadRefusal::Other,
    }
}

// ---- Playing ----

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

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from `soundboard.test.ts`.

    fn read_u16(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([bytes[at], bytes[at + 1]])
    }

    fn read_u32(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    fn selection(start_seconds: f64, end_seconds: f64) -> SoundSelection {
        SoundSelection {
            start_seconds,
            end_seconds,
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    #[test]
    fn the_wav_has_the_canonical_44_byte_header_the_instance_checks() {
        let bytes = encode_wav(&[0.0, 0.5, -0.5]).unwrap();
        assert_eq!(bytes.len(), 44 + 6);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(read_u32(&bytes, 4), 42);
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(read_u32(&bytes, 16), 16);
        assert_eq!(read_u16(&bytes, 20), 1);
        assert_eq!(read_u16(&bytes, 22), 1);
        assert_eq!(read_u32(&bytes, 24), 48_000);
        assert_eq!(read_u32(&bytes, 28), 96_000);
        assert_eq!(read_u16(&bytes, 32), 2);
        assert_eq!(read_u16(&bytes, 34), 16);
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(read_u32(&bytes, 40), 6);
        assert_eq!(bytes[12..36], mellow_protocol::CANONICAL_SOUND_FORMAT_CHUNK);
    }

    #[test]
    fn samples_are_little_endian_16_bit_clamped_and_nan_is_silence() {
        let bytes = encode_wav(&[1.0, -1.0, 2.0, -3.0, f32::NAN, 0.5]).unwrap();
        let sample =
            |index: usize| i16::from_le_bytes([bytes[44 + index * 2], bytes[45 + index * 2]]);
        assert_eq!(
            (0..6).map(sample).collect::<Vec<_>>(),
            [32_767, -32_768, 32_767, -32_768, 0, 16_384]
        );
    }

    #[test]
    fn a_wav_holds_1_to_249_600_samples() {
        let maximum = MAXIMUM_SOUND_SAMPLES as usize;
        assert_eq!(encode_wav(&[0.0]).unwrap().len(), 46);
        assert_eq!(encode_wav(&vec![0.0; maximum]).unwrap().len(), 44 + 499_200);
        assert_eq!(encode_wav(&[]), Err(SoundLengthError(0)));
        assert!(encode_wav(&vec![0.0; maximum + 1]).is_err());
    }

    #[test]
    fn the_trimmer_starts_with_the_beginning_of_the_file_at_most_5_2_s() {
        assert_close(MAXIMUM_SOUND_SECONDS, 5.2);
        assert_eq!(initial_selection(3.0), selection(0.0, 3.0));
        assert_close(initial_selection(60.0).end_seconds, 5.2);
    }

    #[test]
    fn moving_the_start_never_passes_the_end_nor_stretches_past_5_2_s() {
        let current = selection(2.0, 6.0);
        assert_eq!(
            with_selection_start(current, 1.0, 60.0),
            selection(1.0, 6.0)
        );
        assert_close(with_selection_start(current, 0.0, 60.0).end_seconds, 5.2);
        assert_eq!(
            with_selection_start(current, 7.0, 60.0),
            selection(7.0, 7.0 + MINIMUM_SELECTION_SECONDS)
        );
        assert_close(
            with_selection_start(current, 100.0, 10.0).start_seconds,
            10.0 - MINIMUM_SELECTION_SECONDS,
        );
        assert_close(with_selection_start(current, -3.0, 60.0).start_seconds, 0.0);
    }

    #[test]
    fn moving_the_end_never_passes_the_start_nor_stretches_past_5_2_s() {
        let current = selection(2.0, 4.0);
        assert_eq!(with_selection_end(current, 5.0, 60.0), selection(2.0, 5.0));
        assert_close(with_selection_end(current, 10.0, 60.0).start_seconds, 4.8);
        assert_eq!(
            with_selection_end(current, 1.0, 60.0),
            selection(1.0 - MINIMUM_SELECTION_SECONDS, 1.0)
        );
        let clamped = with_selection_end(current, 99.0, 8.0);
        assert_close(clamped.start_seconds, 2.8);
        assert_close(clamped.end_seconds, 8.0);
    }

    #[test]
    fn a_file_shorter_than_the_shortest_selection_is_kept_whole() {
        assert_eq!(
            with_selection_start(selection(0.0, 0.05), 0.04, 0.05),
            selection(0.0, 0.05)
        );
        assert_eq!(
            with_selection_end(selection(0.0, 0.05), 0.0, 0.05),
            selection(0.0, 0.05)
        );
    }

    #[test]
    fn a_click_on_the_waveform_centres_the_selection_inside_the_file() {
        let current = selection(0.0, 2.0);
        assert_eq!(
            with_selection_centred_at(current, 10.0, 60.0),
            selection(9.0, 11.0)
        );
        assert_eq!(
            with_selection_centred_at(current, 0.5, 60.0),
            selection(0.0, 2.0)
        );
        assert_eq!(
            with_selection_centred_at(current, 59.5, 60.0),
            selection(58.0, 60.0)
        );
    }

    #[test]
    fn samples_are_counted_at_48_khz_never_zero_never_past_the_limit() {
        assert_eq!(selection_sample_count(selection(1.0, 2.0)), 48_000);
        assert_eq!(selection_sample_count(selection(1.0, 1.0)), 1);
        assert_eq!(
            selection_sample_count(selection(0.0, 5.200_000_1)),
            MAXIMUM_SOUND_SAMPLES as usize
        );
    }

    #[test]
    fn the_rendered_sound_is_the_selection_at_its_volume_then_silence() {
        let source: Vec<f32> = (0..48_010)
            .map(|index| if index < 48_000 { 0.0 } else { 0.5 })
            .collect();
        let rendered = render_sound(&source, selection(1.0, 1.0 + 20.0 / 48_000.0), 50);
        assert_eq!(rendered.len(), 20);
        assert!(
            rendered[..10]
                .iter()
                .all(|sample| (sample - 0.25).abs() < 1e-6)
        );
        assert!(rendered[10..].iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn the_waveform_is_the_loudest_sample_per_slice_across_channels() {
        let left = [0.1, -0.8, 0.0, 0.2];
        let right = [0.3, 0.0, -0.4, 0.0];
        let peaks = waveform_peaks(&[&left, &right], 2);
        assert!((peaks[0] - 0.8).abs() < 1e-6 && (peaks[1] - 0.4).abs() < 1e-6);
        assert_eq!(waveform_peaks(&[], 3), [0.0, 0.0, 0.0]);
        assert_eq!(waveform_peaks(&[&[2.0]], 1), [1.0]);
    }

    #[test]
    fn times_print_to_the_tenth() {
        assert_eq!(format_sound_seconds(3.24, ","), "0:03,2");
        assert_eq!(format_sound_seconds(65.06, "."), "1:05.1");
        assert_eq!(format_sound_seconds(-1.0, "."), "0:00.0");
        assert_eq!(selection(1.0, 2.5).kept_milliseconds(), 1_500);
    }

    #[test]
    fn names_follow_the_instance_rule() {
        assert_eq!(sound_name_problem("  ok  "), None);
        assert_eq!(sound_name_problem("a"), Some(SoundNameProblem::Length));
        assert_eq!(sound_name_problem(" a "), Some(SoundNameProblem::Length));
        assert_eq!(sound_name_problem(&"x".repeat(32)), None);
        assert_eq!(
            sound_name_problem(&"x".repeat(33)),
            Some(SoundNameProblem::Length)
        );
        assert_eq!(sound_name_problem(&"🎺".repeat(32)), None);
        assert_eq!(
            sound_name_problem("air\u{7}horn"),
            Some(SoundNameProblem::ControlCharacter)
        );
    }

    #[test]
    fn a_name_is_suggested_from_the_file_name() {
        assert_eq!(
            sound_name_from_file_name("airhorn_long-v2.mp3"),
            "airhorn long v2"
        );
        assert_eq!(
            sound_name_from_file_name("Sad Trombone.wav"),
            "Sad Trombone"
        );
        assert_eq!(
            sound_name_from_file_name(&format!("{}.ogg", "a".repeat(40))),
            "a".repeat(32)
        );
        assert_eq!(sound_name_from_file_name(".m4a"), "");
    }

    #[test]
    fn the_instance_refusals_are_told_apart() {
        let api = |code| ClientError::Api {
            status: 400,
            code,
            message: String::new(),
        };
        assert_eq!(
            sound_upload_refusal(&api(ApiErrorCode::LimitReached)),
            SoundUploadRefusal::LimitReached
        );
        assert_eq!(
            sound_upload_refusal(&api(ApiErrorCode::RateLimited)),
            SoundUploadRefusal::TooMany
        );
        assert_eq!(
            sound_upload_refusal(&api(ApiErrorCode::ValidationFailed)),
            SoundUploadRefusal::Refused
        );
        assert_eq!(
            sound_upload_refusal(&ClientError::NotSignedIn),
            SoundUploadRefusal::Other
        );
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
}
