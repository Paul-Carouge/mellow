//! A file picked for a new soundboard sound (ADR-0032, `decodeSoundFile` of the web's
//! `soundboard-audio.ts`): any audio file the attachment decoder reads, its type sniffed from its
//! first bytes, mixed to mono and resampled to 48 kHz, for the trimmer to cut and the client core
//! to encode as the instance's canonical WAV. The file comes from the reader's disk but is
//! untrusted all the same: decode it off the interface thread.

use std::sync::Arc;

use crate::audio_format::VOICE_SAMPLE_RATE;
use crate::clip_decoder::{ClipError, open_clip, starts_like};
use crate::resampling::{Interpolation, StreamResampler};

/// The longest file the trimmer opens: ten minutes of mono 48 kHz samples is about 115 MB, and
/// only 5.2 s are kept anyway.
pub const MAXIMUM_SOUND_SOURCE_SECONDS: u32 = 10 * 60;
/// Ten minutes at the file's rate, but never more frames than ten minutes at 48 kHz: a 192 kHz
/// file (2.5 minutes then) must not hold four times as many samples.
fn maximum_source_frames(source_rate: u32) -> usize {
    let frames =
        u64::from(source_rate.min(VOICE_SAMPLE_RATE)) * u64::from(MAXIMUM_SOUND_SOURCE_SECONDS);
    usize::try_from(frames).unwrap_or(usize::MAX)
}
/// Types tried in this order: ADTS AAC before MP3, whose frame sync it also matches.
// shortcut: WebM and AIFF are not sniffed (a browser decodes them on the web); add them when
// someone asks.
const SNIFFED_TYPES: [&str; 6] = [
    "audio/ogg",
    "audio/wav",
    "audio/flac",
    "audio/mp4",
    "audio/aac",
    "audio/mpeg",
];
const RESAMPLER_CHUNK_FRAMES: usize = 1024;

/// Why a picked file cannot become a sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SoundSourceError {
    /// Not audio this app reads, or damaged.
    #[error("this file cannot be read as a sound")]
    Unreadable,
    /// Longer than [`MAXIMUM_SOUND_SOURCE_SECONDS`].
    #[error("this sound file is too long to trim here")]
    TooLong,
}

impl From<ClipError> for SoundSourceError {
    fn from(_: ClipError) -> Self {
        Self::Unreadable
    }
}

/// The whole file as mono 48 kHz samples. Stereo is mixed as speakers would play it on one
/// channel (half of each), as Web Audio's down-mix does.
///
/// # Errors
/// `Unreadable` for anything the decoder refuses, damaged or empty; `TooLong` past ten minutes.
pub fn decode_sound_source(bytes: Arc<[u8]>) -> Result<Vec<f32>, SoundSourceError> {
    // A decoder bug on a hostile file must not take the app down.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode(bytes))).unwrap_or_else(|_| {
        tracing::error!("a sound file's decoder panicked");
        Err(SoundSourceError::Unreadable)
    })
}

fn decode(bytes: Arc<[u8]>) -> Result<Vec<f32>, SoundSourceError> {
    let content_type = SNIFFED_TYPES
        .into_iter()
        .find(|content_type| starts_like(&bytes, content_type))
        .ok_or(SoundSourceError::Unreadable)?;
    let mut decoder = open_clip(bytes, content_type)?;
    let channels = decoder.channels().max(1);
    let source_rate = decoder.sample_rate();
    let maximum_frames = maximum_source_frames(source_rate);
    if decoder
        .duration_frames()
        .is_some_and(|frames| frames > maximum_frames as u64)
    {
        return Err(SoundSourceError::TooLong);
    }
    let mut mono = Vec::new();
    let mut batch = Vec::new();
    loop {
        batch.clear();
        let has_more = decoder.decode_next(&mut batch)?;
        // Channel counts are 1 or 2: exact in `f32`.
        #[allow(clippy::cast_precision_loss)]
        let share = 1.0 / channels as f32;
        mono.extend(
            batch
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() * share),
        );
        if mono.len() > maximum_frames {
            return Err(SoundSourceError::TooLong);
        }
        if !has_more {
            break;
        }
    }
    if mono.is_empty() {
        return Err(SoundSourceError::Unreadable);
    }
    resampled_to_voice_rate(&mono, source_rate)
}

/// The same sound at 48 kHz, as long as the original.
fn resampled_to_voice_rate(mono: &[f32], source_rate: u32) -> Result<Vec<f32>, SoundSourceError> {
    if source_rate == VOICE_SAMPLE_RATE {
        return Ok(mono.to_vec());
    }
    let expected = usize::try_from(
        u64::try_from(mono.len()).unwrap_or(u64::MAX) * u64::from(VOICE_SAMPLE_RATE)
            / u64::from(source_rate.max(1)),
    )
    .unwrap_or(usize::MAX);
    let mut resampler = StreamResampler::new(
        source_rate,
        VOICE_SAMPLE_RATE,
        RESAMPLER_CHUNK_FRAMES,
        Interpolation::Sinc,
        1.0,
    )
    .map_err(|_| SoundSourceError::Unreadable)?;
    let mut output = std::collections::VecDeque::with_capacity(expected + RESAMPLER_CHUNK_FRAMES);
    resampler
        .process(mono, &mut output)
        .map_err(|_| SoundSourceError::Unreadable)?;
    // Silence pushes the last partial chunk (and the filter's delay) through.
    resampler
        .process(&[0.0; 2 * RESAMPLER_CHUNK_FRAMES], &mut output)
        .map_err(|_| SoundSourceError::Unreadable)?;
    let mut samples = Vec::from(output);
    samples.truncate(expected);
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip_decoder::tests::tone_wav;

    #[test]
    fn a_stereo_44_1_khz_file_becomes_mono_48_khz_of_the_same_length() {
        let samples = decode_sound_source(tone_wav(44_100, 2, 44_100).into()).unwrap();
        assert_eq!(samples.len(), 48_000);
        let loudest = samples
            .iter()
            .fold(0.0_f32, |loudest, sample| loudest.max(sample.abs()));
        // 8 000 / 32 768 on both channels, mixed at half each.
        assert!((loudest - 0.244).abs() < 0.02, "{loudest}");
    }

    #[test]
    fn an_ogg_vorbis_file_is_read_like_the_others() {
        // Half a second of 440 Hz at 1/8 of full scale, stereo at 44.1 kHz (ffmpeg's encoder).
        let bytes = include_bytes!("../tests/fixtures/tone-vorbis.ogg");
        let samples = decode_sound_source(bytes.as_slice().into()).unwrap();
        assert!(
            (23_000..=25_000).contains(&samples.len()),
            "{}",
            samples.len()
        );
        assert!(
            samples.iter().any(|sample| sample.abs() > 0.08),
            "the tone is there"
        );
    }

    #[test]
    fn a_48_khz_mono_file_is_kept_as_it_is() {
        let samples = decode_sound_source(tone_wav(48_000, 1, 480).into()).unwrap();
        assert_eq!(samples.len(), 480);
    }

    #[test]
    fn anything_that_is_not_audio_is_unreadable() {
        for bytes in [&b""[..], b"hello, not a sound at all", b"RIFF\0\0\0\0WAVE"] {
            assert_eq!(
                decode_sound_source(bytes.into()),
                Err(SoundSourceError::Unreadable)
            );
        }
    }

    #[test]
    fn a_high_rate_file_holds_no_more_frames_than_ten_minutes_at_48_khz() {
        assert_eq!(maximum_source_frames(8_000), 8_000 * 600);
        assert_eq!(maximum_source_frames(48_000), 48_000 * 600);
        assert_eq!(maximum_source_frames(192_000), 48_000 * 600);
    }

    #[test]
    fn a_file_longer_than_ten_minutes_is_refused() {
        let frames = 8_000 * MAXIMUM_SOUND_SOURCE_SECONDS + 1;
        assert_eq!(
            decode_sound_source(tone_wav(8_000, 1, frames).into()),
            Err(SoundSourceError::TooLong)
        );
    }
}
