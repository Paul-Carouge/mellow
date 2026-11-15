//! The sounds of private calls (`call-audio.ts`, ADR-0062, DESIGN.md §5.6.3), rendered as samples
//! the voice's sound player plays: the owner's ringtone repeated over the time a call may still
//! ring, the caller's generated waiting tone (425 Hz, 1 s on and 4 s off, the European ringback)
//! over the same time, and a short cue for a call ringing someone already in a call. A sound is
//! as long as the ringing may last, so it ends on time even if no event stops it.

use std::sync::Arc;

use mellow_client_core::ringtones::Ringtone;
use mellow_media::{LocalSound, decode_sound_source};

/// Mono samples per second: the sound player's rate.
pub(crate) const SAMPLE_RATE: u32 = 48_000;
const RINGBACK_FREQUENCY_HERTZ: f32 = 425.0;
const RINGBACK_ON_SECONDS: f32 = 1.0;
const RINGBACK_CYCLE_SECONDS: f32 = 5.0;
/// Soft, under the chime's 0.07, so "I am calling" never covers the call's first words.
const RINGBACK_PEAK_GAIN: f32 = 0.05;
/// Edges without a click.
const RINGBACK_EDGE_SECONDS: f32 = 0.03;
/// The ringtone's level before the output volume: the files are mastered loud.
const RINGTONE_GAIN: f32 = 0.5;
/// Two soft notes a third apart, lower than the chime: "another call" without a ring.
const CALL_WAITING_NOTES: [(f32, f32); 2] = [(659.25, 0.0), (523.25, 0.16)];
const CALL_WAITING_NOTE_SECONDS: f32 = 0.3;
const CALL_WAITING_GAIN: f32 = 0.05;
const ATTACK_SECONDS: f32 = 0.012;
/// The quietest gain an exponential ramp starts from or reaches (it cannot reach zero).
const SILENT_GAIN: f32 = 0.0001;

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // sample counts of at most a minute
fn samples_of(seconds: f32) -> usize {
    (seconds * SAMPLE_RATE as f32).round().max(0.0) as usize
}

/// Samples in `milliseconds` of sound.
#[must_use]
pub(crate) fn samples_in_milliseconds(milliseconds: u64) -> usize {
    let samples = milliseconds.saturating_mul(u64::from(SAMPLE_RATE)) / 1_000;
    usize::try_from(samples).unwrap_or(usize::MAX)
}

/// `sample / SAMPLE_RATE` seconds.
#[allow(clippy::cast_precision_loss)] // sample counts of at most a minute
fn seconds_at(sample: usize) -> f32 {
    sample as f32 / SAMPLE_RATE as f32
}

/// Web Audio's exponential ramp between two gains.
fn exponential_ramp(from: f32, to: f32, progress: f32) -> f32 {
    from * (to / from).powf(progress.clamp(0.0, 1.0))
}

/// The waiting tone for `length` samples: a second of 425 Hz every five, from the start.
#[must_use]
pub(crate) fn render_ringback(length: usize) -> LocalSound {
    let cycle = samples_of(RINGBACK_CYCLE_SECONDS);
    let on = samples_of(RINGBACK_ON_SECONDS);
    let edge = samples_of(RINGBACK_EDGE_SECONDS);
    (0..length)
        .map(|index| {
            let within = index % cycle;
            if within >= on {
                return 0.0;
            }
            let envelope = if within < edge {
                exponential_ramp(
                    SILENT_GAIN,
                    RINGBACK_PEAK_GAIN,
                    seconds_at(within) / RINGBACK_EDGE_SECONDS,
                )
            } else if within >= on - edge {
                exponential_ramp(
                    RINGBACK_PEAK_GAIN,
                    SILENT_GAIN,
                    seconds_at(within - (on - edge)) / RINGBACK_EDGE_SECONDS,
                )
            } else {
                RINGBACK_PEAK_GAIN
            };
            envelope * (std::f32::consts::TAU * RINGBACK_FREQUENCY_HERTZ * seconds_at(index)).sin()
        })
        .collect::<Vec<_>>()
        .into()
}

/// The call-waiting cue: E5 then C5, soft (`playCallWaitingCue`).
#[must_use]
pub(crate) fn render_call_waiting_cue() -> LocalSound {
    let last_start = CALL_WAITING_NOTES
        .iter()
        .map(|(_, start)| *start)
        .fold(0.0, f32::max);
    let length = samples_of(last_start + CALL_WAITING_NOTE_SECONDS);
    let mut samples = vec![0.0_f32; length];
    for (frequency, start) in CALL_WAITING_NOTES {
        let first = samples_of(start);
        let note_length = samples_of(CALL_WAITING_NOTE_SECONDS);
        for (offset, sample) in samples.iter_mut().skip(first).take(note_length).enumerate() {
            let at = seconds_at(offset);
            let envelope = if at < ATTACK_SECONDS {
                exponential_ramp(SILENT_GAIN, CALL_WAITING_GAIN, at / ATTACK_SECONDS)
            } else {
                exponential_ramp(
                    CALL_WAITING_GAIN,
                    SILENT_GAIN,
                    (at - ATTACK_SECONDS) / (CALL_WAITING_NOTE_SECONDS - ATTACK_SECONDS),
                )
            };
            *sample += envelope * (std::f32::consts::TAU * frequency * at).sin();
        }
    }
    samples.into()
}

/// The ringtone's file as mono 48 kHz samples. Decoding takes a moment: off the interface
/// thread. `None` when the file cannot be read (it is embedded, so only a broken build).
#[must_use]
pub(crate) fn decode_ringtone(ringtone: Ringtone) -> Option<LocalSound> {
    match decode_sound_source(Arc::from(ringtone.ogg_opus_bytes())) {
        Ok(samples) if !samples.is_empty() => Some(samples.into()),
        Ok(_) => None,
        Err(error) => {
            tracing::warn!(%error, file = ringtone.file_name(), "cannot decode the ringtone");
            None
        }
    }
}

/// The ringtone looped over `length` samples, at its playing level.
#[must_use]
pub(crate) fn looped_ringtone(ringtone: &[f32], length: usize) -> LocalSound {
    if ringtone.is_empty() {
        return Arc::from([]);
    }
    ringtone
        .iter()
        .cycle()
        .take(length)
        .map(|sample| sample * RINGTONE_GAIN)
        .collect::<Vec<_>>()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ringback_sounds_one_second_in_five() {
        let five_seconds = samples_of(5.0);
        let tone = render_ringback(2 * five_seconds);
        assert_eq!(tone.len(), 2 * five_seconds);
        let loudness = |range: std::ops::Range<usize>| {
            tone[range]
                .iter()
                .map(|sample| sample.abs())
                .fold(0.0, f32::max)
        };
        assert!(loudness(samples_of(0.4)..samples_of(0.6)) > 0.04);
        assert!(loudness(samples_of(1.01)..samples_of(4.99)) <= f32::EPSILON);
        assert!(loudness(samples_of(5.4)..samples_of(5.6)) > 0.04);
        assert!(tone.iter().all(|sample| sample.abs() <= RINGBACK_PEAK_GAIN));
    }

    #[test]
    fn the_call_waiting_cue_is_short_and_soft() {
        let cue = render_call_waiting_cue();
        assert_eq!(cue.len(), samples_of(0.46));
        assert!(
            cue.iter()
                .all(|sample| sample.abs() <= 2.0 * CALL_WAITING_GAIN)
        );
    }

    #[test]
    fn the_ringtone_loops_over_the_time_left_and_decodes_from_its_file() {
        let looped = looped_ringtone(&[0.2, 0.4], 5);
        assert_eq!(&*looped, &[0.1, 0.2, 0.1, 0.2, 0.1]);
        assert!(looped_ringtone(&[], 5).is_empty());
        let decoded = decode_ringtone(Ringtone::Standard).unwrap();
        // About 10 s at 48 kHz.
        assert!(decoded.len() > samples_of(8.0), "{}", decoded.len());
    }

    #[test]
    fn milliseconds_become_samples() {
        assert_eq!(samples_in_milliseconds(1_000), 48_000);
        assert_eq!(samples_in_milliseconds(30_000), 1_440_000);
    }
}
