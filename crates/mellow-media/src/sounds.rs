//! Sounds the app makes itself: the voice cues (`web/src/components/voice-sounds.ts`) and the
//! message chime (`chime-sound.ts`), soft sine notes rendered here as the web renders them with
//! Web Audio, so there is no sound file. Rendered sounds and soundboard sounds play through
//! [`PlayingSounds`], mixed into the voice output or played by a [`crate::SoundPlayer`].

use std::sync::Arc;

use mellow_protocol::Snowflake;

use crate::audio_format::{Frame, VOICE_SAMPLE_RATE};

/// A sound ready to play: mono, 48 kHz, `f32` in `[-1, 1]`. Cheap to clone.
pub type LocalSound = Arc<[f32]>;

/// The voice panel's cues: joining rises, leaving falls; yours span a fifth, others' a softer,
/// higher third; toggles are one short, quiet note, lower when muting; push-to-talk ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoiceCue {
    SelfJoin,
    SelfLeave,
    PeerJoin,
    PeerLeave,
    Mute,
    Unmute,
    Deafen,
    Undeafen,
    PushToTalkPress,
    PushToTalkRelease,
}

const C5: f32 = 523.25;
const E5: f32 = 659.25;
const G5: f32 = 783.99;
const B5: f32 = 987.77;
const A4: f32 = 440.0;
const D5: f32 = 587.33;
const F4: f32 = 349.23;
const E6: f32 = 1_318.51;
const SECOND_NOTE_OFFSET_SECONDS: f32 = 0.09;
const TOGGLE_LENGTH_SECONDS: f32 = 0.1;
const TOGGLE_GAIN: f32 = 0.035;
const PUSH_TO_TALK_LENGTH_SECONDS: f32 = 0.06;
const PUSH_TO_TALK_GAIN: f32 = 0.02;
const OWN_LENGTH_SECONDS: f32 = 0.24;
const OWN_GAIN: f32 = 0.06;
const PEER_LENGTH_SECONDS: f32 = 0.18;
const PEER_GAIN: f32 = 0.04;

/// The chime's two notes (A5, then E6), a fifth apart.
const CHIME_NOTES: [Note; 2] = [
    Note {
        frequency: 880.0,
        start_seconds: 0.0,
    },
    Note {
        frequency: 1_318.5,
        start_seconds: 0.11,
    },
];
const CHIME_LENGTH_SECONDS: f32 = 0.42;
const CHIME_GAIN: f32 = 0.07;
/// Each note rises this long, so it starts without a click.
const ATTACK_SECONDS: f32 = 0.012;
/// The quietest gain an exponential ramp can start from or reach (it cannot reach zero).
const SILENT_GAIN: f32 = 0.0001;

#[derive(Debug, Clone, Copy)]
struct Note {
    frequency: f32,
    start_seconds: f32,
}

const fn single(frequency: f32) -> [Option<Note>; 2] {
    [
        Some(Note {
            frequency,
            start_seconds: 0.0,
        }),
        None,
    ]
}

const fn two_notes(first: f32, second: f32) -> [Option<Note>; 2] {
    [
        Some(Note {
            frequency: first,
            start_seconds: 0.0,
        }),
        Some(Note {
            frequency: second,
            start_seconds: SECOND_NOTE_OFFSET_SECONDS,
        }),
    ]
}

impl VoiceCue {
    /// The notes, each note's length and its peak gain, as `CUES` in the web client.
    fn shape(self) -> ([Option<Note>; 2], f32, f32) {
        match self {
            Self::SelfJoin => (two_notes(C5, G5), OWN_LENGTH_SECONDS, OWN_GAIN),
            Self::SelfLeave => (two_notes(G5, C5), OWN_LENGTH_SECONDS, OWN_GAIN),
            Self::PeerJoin => (two_notes(E5, B5), PEER_LENGTH_SECONDS, PEER_GAIN),
            Self::PeerLeave => (two_notes(B5, E5), PEER_LENGTH_SECONDS, PEER_GAIN),
            Self::Mute => (single(A4), TOGGLE_LENGTH_SECONDS, TOGGLE_GAIN),
            Self::Unmute => (single(D5), TOGGLE_LENGTH_SECONDS, TOGGLE_GAIN),
            Self::Deafen => (single(F4), TOGGLE_LENGTH_SECONDS, TOGGLE_GAIN),
            Self::Undeafen => (single(C5), TOGGLE_LENGTH_SECONDS, TOGGLE_GAIN),
            Self::PushToTalkPress => (single(E6), PUSH_TO_TALK_LENGTH_SECONDS, PUSH_TO_TALK_GAIN),
            Self::PushToTalkRelease => (single(B5), PUSH_TO_TALK_LENGTH_SECONDS, PUSH_TO_TALK_GAIN),
        }
    }

    /// The cue's samples. A few thousand sines: render it off the interface thread.
    #[must_use]
    pub fn render(self) -> LocalSound {
        let (notes, length, peak_gain) = self.shape();
        render_notes(notes.iter().flatten(), length, peak_gain)
    }
}

/// The notification chime's samples.
#[must_use]
pub fn render_notification_chime() -> LocalSound {
    render_notes(CHIME_NOTES.iter(), CHIME_LENGTH_SECONDS, CHIME_GAIN)
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)] // sample counts of a fraction of a second
fn seconds_to_samples(seconds: f32) -> usize {
    (seconds * VOICE_SAMPLE_RATE as f32).round() as usize
}

/// Web Audio's `exponentialRampToValueAtTime` between two gains over `length` samples.
fn exponential_ramp(from: f32, to: f32, position: usize, length: usize) -> f32 {
    #[allow(clippy::cast_precision_loss)] // sample counts of a fraction of a second
    let progress = position as f32 / length.max(1) as f32;
    from * (to / from).powf(progress)
}

/// Sine notes, each starting at its offset, rising in [`ATTACK_SECONDS`] and fading to silence
/// over `note_length_seconds`, summed into one buffer (`playNotes` in the web client).
fn render_notes<'a>(
    notes: impl Iterator<Item = &'a Note> + Clone,
    note_length_seconds: f32,
    peak_gain: f32,
) -> LocalSound {
    let note_length = seconds_to_samples(note_length_seconds);
    let attack = seconds_to_samples(ATTACK_SECONDS);
    let total = notes
        .clone()
        .map(|note| seconds_to_samples(note.start_seconds) + note_length)
        .max()
        .unwrap_or(0);
    let mut samples = vec![0.0_f32; total];
    for note in notes {
        let start = seconds_to_samples(note.start_seconds);
        #[allow(clippy::cast_precision_loss)] // 48 000 is exact in f32
        let step = std::f32::consts::TAU * note.frequency / VOICE_SAMPLE_RATE as f32;
        for (position, sample) in samples.iter_mut().skip(start).take(note_length).enumerate() {
            let gain = if position < attack {
                exponential_ramp(SILENT_GAIN, peak_gain, position, attack)
            } else {
                exponential_ramp(
                    peak_gain,
                    SILENT_GAIN,
                    position - attack,
                    note_length - attack,
                )
            };
            #[allow(clippy::cast_precision_loss)] // under a second of samples
            let phase = step * position as f32;
            *sample += phase.sin() * gain;
        }
    }
    samples.into()
}

/// The most sounds playing at once; a new one past it replaces the oldest, so a held key or a
/// burst of soundboard plays never grows the list.
const MAXIMUM_SOUNDS_PLAYING: usize = 8;

/// Whom a soundboard sound plays for, so it can be stopped (`stopPlayingSounds` on the web).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundboardPlayer {
    /// A preview, for the reader only.
    Preview,
    /// Played by someone in the call (the reader included).
    Person(Snowflake),
}

/// Which soundboard sounds stop: previews (a dialog's Stop), one person's (muted for the reader),
/// or every one (the reader deafened).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundboardSoundsToStop {
    Previews,
    PlayedBy(Snowflake),
    All,
}

impl SoundboardSoundsToStop {
    fn includes(self, player: SoundboardPlayer) -> bool {
        match self {
            Self::Previews => player == SoundboardPlayer::Preview,
            Self::PlayedBy(user_id) => player == SoundboardPlayer::Person(user_id),
            Self::All => true,
        }
    }
}

/// One sound being played.
#[derive(Debug)]
struct PlayingSound {
    samples: LocalSound,
    position: usize,
    gain: f32,
    /// `None` for the app's own sounds (cues, the chime), which are never stopped.
    soundboard_player: Option<SoundboardPlayer>,
}

/// The sounds playing on an output, mixed 10 ms at a time.
#[derive(Debug, Default)]
pub(crate) struct PlayingSounds {
    sounds: Vec<PlayingSound>,
}

impl PlayingSounds {
    pub(crate) fn play(&mut self, samples: LocalSound, gain: f32) {
        self.start(samples, gain, None);
    }

    pub(crate) fn play_soundboard(
        &mut self,
        samples: LocalSound,
        gain: f32,
        player: SoundboardPlayer,
    ) {
        self.start(samples, gain, Some(player));
    }

    fn start(
        &mut self,
        samples: LocalSound,
        gain: f32,
        soundboard_player: Option<SoundboardPlayer>,
    ) {
        if self.sounds.len() >= MAXIMUM_SOUNDS_PLAYING {
            self.sounds.remove(0);
        }
        self.sounds.push(PlayingSound {
            samples,
            position: 0,
            gain,
            soundboard_player,
        });
    }

    pub(crate) fn stop_soundboard(&mut self, which: SoundboardSoundsToStop) {
        self.sounds.retain(|sound| {
            sound
                .soundboard_player
                .is_none_or(|player| !which.includes(player))
        });
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.sounds.is_empty()
    }

    /// Adds the next 10 ms of every sound to `mix`; finished sounds are dropped.
    pub(crate) fn mix_into(&mut self, mix: &mut Frame) {
        for sound in &mut self.sounds {
            let remaining = sound.samples.get(sound.position..).unwrap_or_default();
            for (mixed, sample) in mix.iter_mut().zip(remaining) {
                *mixed += sample * sound.gain;
            }
            sound.position += mix.len();
        }
        self.sounds
            .retain(|sound| sound.position < sound.samples.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_format::SAMPLES_PER_FRAME;

    fn peak(samples: &[f32]) -> f32 {
        samples
            .iter()
            .fold(0.0, |peak, sample| peak.max(sample.abs()))
    }

    #[test]
    fn cues_have_the_web_s_length_and_loudness() {
        let join = VoiceCue::SelfJoin.render();
        assert_eq!(join.len(), seconds_to_samples(0.09 + 0.24));
        assert!(peak(&join) > 0.05 && peak(&join) < 0.12, "{}", peak(&join));
        let tick = VoiceCue::PushToTalkPress.render();
        assert_eq!(tick.len(), seconds_to_samples(0.06));
        assert!(peak(&tick) <= 0.02 + 1e-4);
        // No click: the first sample starts from silence.
        assert!(join[0].abs() < 1e-3 && tick[0].abs() < 1e-3);
    }

    #[test]
    fn joining_rises_and_leaving_falls() {
        // The dominant note in the first 80 ms is the first note: count zero crossings.
        let crossings = |samples: &[f32]| {
            samples
                .windows(2)
                .filter(|pair| pair[0].signum() != pair[1].signum())
                .count()
        };
        let window = seconds_to_samples(0.08);
        let join = VoiceCue::SelfJoin.render();
        let leave = VoiceCue::SelfLeave.render();
        assert!(crossings(&join[..window]) < crossings(&leave[..window]));
    }

    #[test]
    fn the_chime_is_two_notes_fading_out() {
        let chime = render_notification_chime();
        assert_eq!(chime.len(), seconds_to_samples(0.11 + 0.42));
        assert!(peak(&chime) <= 0.14);
        assert!(peak(&chime[chime.len() - 100..]) < 1e-3, "faded out");
    }

    #[test]
    fn playing_sounds_mix_and_finish() {
        let mut playing = PlayingSounds::default();
        let sound: LocalSound = vec![0.5; SAMPLES_PER_FRAME + 10].into();
        playing.play(sound.clone(), 0.5);
        playing.play(sound, 1.0);
        let mut mix = [0.0; SAMPLES_PER_FRAME];
        playing.mix_into(&mut mix);
        assert!(mix.iter().all(|sample| (sample - 0.75).abs() < 1e-6));
        let mut mix = [0.0; SAMPLES_PER_FRAME];
        playing.mix_into(&mut mix);
        assert!((mix[9] - 0.75).abs() < 1e-6 && mix[10].abs() < f32::EPSILON);
        assert!(playing.is_empty());
    }

    #[test]
    fn too_many_sounds_drop_the_oldest() {
        let mut playing = PlayingSounds::default();
        for _ in 0..MAXIMUM_SOUNDS_PLAYING + 3 {
            playing.play(vec![0.1; 10].into(), 1.0);
        }
        assert_eq!(playing.sounds.len(), MAXIMUM_SOUNDS_PLAYING);
    }

    #[test]
    fn soundboard_sounds_stop_by_whom_they_play_for_and_cues_never() {
        let alice = Snowflake::from_raw(1);
        let bob = Snowflake::from_raw(2);
        let mut playing = PlayingSounds::default();
        let sound: LocalSound = vec![0.1; 10].into();
        playing.play(sound.clone(), 1.0);
        playing.play_soundboard(sound.clone(), 1.0, SoundboardPlayer::Preview);
        playing.play_soundboard(sound.clone(), 1.0, SoundboardPlayer::Person(alice));
        playing.play_soundboard(sound, 1.0, SoundboardPlayer::Person(bob));
        playing.stop_soundboard(SoundboardSoundsToStop::Previews);
        assert_eq!(playing.sounds.len(), 3);
        playing.stop_soundboard(SoundboardSoundsToStop::PlayedBy(alice));
        assert_eq!(playing.sounds.len(), 2);
        playing.stop_soundboard(SoundboardSoundsToStop::All);
        assert_eq!(playing.sounds.len(), 1, "the cue plays on");
    }
}
