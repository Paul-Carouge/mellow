//! A sound attachment's shape for its player (DESIGN.md 5.4, "Sound player", the web's
//! `audio-waveforms.svelte.ts`): the loudest sample of each slice of the sound, scaled so the
//! loudest slice fills the box. Read with the attachment decoder ([`open_clip`]), one packet at a
//! time, and given up as soon as the sound runs past [`MAXIMUM_WAVEFORM_SOURCE_SECONDS`], so a
//! file lying about its length never costs more than that. Untrusted bytes: call it off the
//! interface thread.

use std::sync::Arc;
use std::time::Duration;

use crate::clip_decoder::{ClipError, open_clip};

/// Files larger than this keep the flat line: the shape is only worth a small file's decoding.
pub const MAXIMUM_WAVEFORM_SOURCE_BYTES: u64 = 10 * 1024 * 1024;
/// Nor longer sounds: 10 MiB of silent Opus or FLAC last hours.
pub const MAXIMUM_WAVEFORM_SOURCE_SECONDS: u64 = 15 * 60;
/// Slices read per second before they are folded into bars: fine enough for any bar count a card
/// draws, and at most 90 000 numbers held for the longest sound.
const SLICES_PER_SECOND: u64 = 100;

/// Why a sound keeps its flat line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WaveformError {
    /// Not a sound this app decodes, damaged, or silent from start to end.
    #[error("this sound's shape cannot be read")]
    Unreadable,
    /// Larger than [`MAXIMUM_WAVEFORM_SOURCE_BYTES`] or longer than
    /// [`MAXIMUM_WAVEFORM_SOURCE_SECONDS`].
    #[error("this sound is too long to read its shape")]
    TooLong,
}

impl From<ClipError> for WaveformError {
    fn from(_: ClipError) -> Self {
        Self::Unreadable
    }
}

/// A sound's shape and its decoded length.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundWaveform {
    /// One level per bar, 0 to 1, the loudest at 1.
    pub levels: Vec<f32>,
    pub duration: Duration,
}

/// The shape of a sound attachment of `content_type` in `bar_count` bars.
///
/// # Errors
/// `TooLong` past the size or length limits; `Unreadable` for anything the decoder refuses.
pub fn sound_waveform(
    bytes: Arc<[u8]>,
    content_type: &str,
    bar_count: usize,
) -> Result<SoundWaveform, WaveformError> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAXIMUM_WAVEFORM_SOURCE_BYTES {
        return Err(WaveformError::TooLong);
    }
    // A decoder bug on a hostile file must not take the app down.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        read(
            bytes,
            content_type,
            bar_count,
            MAXIMUM_WAVEFORM_SOURCE_SECONDS,
        )
    }))
    .unwrap_or_else(|_| {
        tracing::error!("a sound's decoder panicked while reading its shape");
        Err(WaveformError::Unreadable)
    })
}

fn read(
    bytes: Arc<[u8]>,
    content_type: &str,
    bar_count: usize,
    maximum_seconds: u64,
) -> Result<SoundWaveform, WaveformError> {
    let mut decoder = open_clip(bytes, content_type)?;
    let channels = decoder.channels().max(1);
    let sample_rate = u64::from(decoder.sample_rate().max(1));
    let maximum_frames = sample_rate * maximum_seconds;
    if decoder
        .duration_frames()
        .is_some_and(|frames| frames > maximum_frames)
    {
        return Err(WaveformError::TooLong);
    }
    let frames_per_slice = usize::try_from((sample_rate / SLICES_PER_SECOND).max(1))
        .map_err(|_| WaveformError::Unreadable)?;
    let mut slices = Vec::new();
    let mut slice_peak = 0.0_f32;
    let mut frames_in_slice = 0_usize;
    let mut frames = 0_u64;
    let mut batch = Vec::new();
    loop {
        batch.clear();
        let has_more = decoder.decode_next(&mut batch)?;
        for frame in batch.chunks_exact(channels) {
            let loudest = frame
                .iter()
                .fold(0.0_f32, |loudest, sample| loudest.max(sample.abs()));
            slice_peak = slice_peak.max(loudest);
            frames_in_slice += 1;
            if frames_in_slice == frames_per_slice {
                slices.push(slice_peak);
                (slice_peak, frames_in_slice) = (0.0, 0);
            }
        }
        frames += (batch.len() / channels) as u64;
        // Checked as it decodes, not after: a sound longer than its header said stops here.
        if frames > maximum_frames {
            return Err(WaveformError::TooLong);
        }
        if !has_more {
            break;
        }
    }
    if frames_in_slice > 0 {
        slices.push(slice_peak);
    }
    let levels = normalised_levels(&bar_peaks(&slices, bar_count));
    if levels.iter().all(|level| *level == 0.0) {
        return Err(WaveformError::Unreadable);
    }
    Ok(SoundWaveform {
        levels,
        duration: Duration::from_secs_f64(
            // Frames stay far below 2^52 under the length limit.
            #[allow(clippy::cast_precision_loss)]
            {
                frames as f64 / sample_rate as f64
            },
        ),
    })
}

/// The loudest slice of each of `bar_count` equal shares of `slices` (`waveformPeaks`).
fn bar_peaks(slices: &[f32], bar_count: usize) -> Vec<f32> {
    let mut peaks = vec![0.0_f32; bar_count];
    if slices.is_empty() || bar_count == 0 {
        return peaks;
    }
    if slices.len() < bar_count {
        // Each bar takes the slice under it: a short sound keeps a whole line of bars.
        for (bar, peak) in peaks.iter_mut().enumerate() {
            *peak = slices[bar * slices.len() / bar_count].min(1.0);
        }
        return peaks;
    }
    for (index, peak) in slices.iter().enumerate() {
        let bar = (index * bar_count / slices.len()).min(bar_count - 1);
        peaks[bar] = peaks[bar].max(peak.min(1.0));
    }
    peaks
}

/// Scaled so the loudest bar fills the box: a quiet voice note still shows its words.
fn normalised_levels(peaks: &[f32]) -> Vec<f32> {
    let loudest = peaks.iter().copied().fold(0.0_f32, f32::max);
    if loudest <= 0.0 {
        return vec![0.0; peaks.len()];
    }
    peaks
        .iter()
        .map(|peak| (peak / loudest).clamp(0.0, 1.0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip_decoder::tests::{tone_ogg_opus, tone_wav};

    #[test]
    fn a_tone_fills_every_bar_and_says_its_length() {
        let waveform = sound_waveform(tone_wav(8_000, 2, 8_000).into(), "audio/wav", 48).unwrap();
        assert_eq!(waveform.levels.len(), 48);
        assert!(
            waveform.levels.iter().all(|level| *level > 0.9),
            "{:?}",
            waveform.levels
        );
        assert_eq!(waveform.duration, Duration::from_secs(1));
    }

    #[test]
    fn an_ogg_opus_sound_is_read_like_the_instance_stores_it() {
        let waveform =
            sound_waveform(tone_ogg_opus(50, 312, 48_000).into(), "audio/ogg", 48).unwrap();
        assert_eq!(waveform.levels.len(), 48);
        assert!(waveform.levels.contains(&1.0));
    }

    #[test]
    fn the_loudest_bar_fills_the_box_and_the_others_keep_their_share() {
        assert_eq!(bar_peaks(&[0.1, 0.4, 0.2, 0.0], 2), [0.4, 0.2]);
        assert_eq!(normalised_levels(&[0.4, 0.2]), [1.0, 0.5]);
        assert_eq!(normalised_levels(&[0.0, 0.0]), [0.0, 0.0]);
        // Fewer slices than bars (a sound under half a second): each bar takes the slice under
        // it, never a comb of silent bars.
        assert_eq!(bar_peaks(&[0.5], 3), [0.5, 0.5, 0.5]);
        assert_eq!(bar_peaks(&[0.5, 0.25], 4), [0.5, 0.5, 0.25, 0.25]);
        assert_eq!(bar_peaks(&[], 2), [0.0, 0.0]);
        assert_eq!(bar_peaks(&[1.0], 0), Vec::<f32>::new());
    }

    #[test]
    fn an_empty_sound_and_anything_else_keep_the_flat_line() {
        assert_eq!(
            sound_waveform(tone_wav(8_000, 1, 0).into(), "audio/wav", 48),
            Err(WaveformError::Unreadable)
        );
        assert_eq!(
            sound_waveform(b"not a sound".as_slice().into(), "audio/wav", 48),
            Err(WaveformError::Unreadable)
        );
    }

    #[test]
    fn a_sound_past_its_length_limit_or_ten_mebibytes_is_not_read() {
        // Two seconds against a one-second limit, as fifteen minutes and one second would be.
        assert_eq!(
            read(tone_wav(8_000, 1, 16_000).into(), "audio/wav", 48, 1),
            Err(WaveformError::TooLong)
        );
        let large = vec![0_u8; usize::try_from(MAXIMUM_WAVEFORM_SOURCE_BYTES).unwrap() + 1];
        assert_eq!(
            sound_waveform(large.into(), "audio/wav", 48),
            Err(WaveformError::TooLong)
        );
    }
}
