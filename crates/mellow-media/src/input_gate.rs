//! When the microphone is sent: voice activity with an automatic or manual sensitivity, or
//! push-to-talk, as the web client's voice settings define them (`web/src/lib/voice-settings.ts`).
//! Everything counts 10 ms frames, never the wall clock, so the behaviour is deterministic.

use std::time::Duration;

use crate::audio_format::FRAME_DURATION;

/// The quietest and loudest manual sensitivity, in dB (Discord's scale, as on the web).
pub const MINIMUM_SENSITIVITY_DECIBELS: f32 = -100.0;
pub const MAXIMUM_SENSITIVITY_DECIBELS: f32 = 0.0;
/// The web client's default manual sensitivity.
pub const DEFAULT_SENSITIVITY_DECIBELS: f32 = -50.0;
/// How long a manual gate stays open after the level falls below the threshold.
const SENSITIVITY_GATE_HOLD: Duration = Duration::from_millis(250);
/// The web client's default and longest push-to-talk release delays.
pub const DEFAULT_PUSH_TO_TALK_RELEASE_DELAY: Duration = Duration::from_millis(200);
pub const MAXIMUM_PUSH_TO_TALK_RELEASE_DELAY: Duration = Duration::from_millis(2_000);
/// The microphone RMS from which the user shows as speaking to themselves, and for how long
/// after (`voice-session.svelte.ts`, sampled every 20 ms there).
const SELF_SPEAKING_RMS: f32 = 0.02;
const SELF_SPEAKING_HANGOVER: Duration = Duration::from_millis(300);

/// How the microphone opens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputMode {
    /// Sent whenever the user is not muted; the sensitivity decides what counts as voice.
    VoiceActivity(Sensitivity),
    /// Sent while the push-to-talk key is held, and `release_delay` after it is released.
    PushToTalk { release_delay: Duration },
}

impl Default for InputMode {
    fn default() -> Self {
        Self::VoiceActivity(Sensitivity::Automatic)
    }
}

/// What the voice-activity mode counts as voice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sensitivity {
    /// No gate: noise suppression and Opus DTX keep silence off the wire (the web's default).
    Automatic,
    /// The gate opens when a frame is at least this loud, in dB relative to full scale.
    ThresholdDecibels(f32),
}

/// Number of whole 10 ms frames in `duration`, rounded up.
fn frames_in(duration: Duration) -> u32 {
    let frames = duration.as_micros().div_ceil(FRAME_DURATION.as_micros());
    u32::try_from(frames).unwrap_or(u32::MAX)
}

/// Decides, frame by frame, whether the microphone is transmitted.
#[derive(Debug)]
pub(crate) struct InputGate {
    mode: InputMode,
    is_push_to_talk_pressed: bool,
    /// Frames the gate stays open without voice or key: the hold or the release delay.
    open_frames_left: u32,
}

impl InputGate {
    pub(crate) fn new(mode: InputMode) -> Self {
        Self {
            mode: clamped(mode),
            is_push_to_talk_pressed: false,
            open_frames_left: 0,
        }
    }

    pub(crate) fn set_mode(&mut self, mode: InputMode) {
        self.mode = clamped(mode);
        self.open_frames_left = 0;
    }

    /// The key itself is read by the desktop (a global shortcut); the release delay is applied here.
    pub(crate) fn set_push_to_talk_pressed(&mut self, is_pressed: bool) {
        if self.is_push_to_talk_pressed
            && !is_pressed
            && let InputMode::PushToTalk { release_delay } = self.mode
        {
            self.open_frames_left = frames_in(release_delay);
        }
        self.is_push_to_talk_pressed = is_pressed;
    }

    /// Whether the 10 ms frame at `level_decibels` is transmitted.
    pub(crate) fn is_open_for_frame(&mut self, level_decibels: f32) -> bool {
        match self.mode {
            InputMode::VoiceActivity(Sensitivity::Automatic) => true,
            InputMode::VoiceActivity(Sensitivity::ThresholdDecibels(threshold)) => {
                if level_decibels >= threshold {
                    self.open_frames_left = frames_in(SENSITIVITY_GATE_HOLD);
                    return true;
                }
                self.consume_open_frame()
            }
            InputMode::PushToTalk { .. } => {
                self.is_push_to_talk_pressed || self.consume_open_frame()
            }
        }
    }

    fn consume_open_frame(&mut self) -> bool {
        if self.open_frames_left == 0 {
            return false;
        }
        self.open_frames_left -= 1;
        true
    }
}

/// Values outside the settings' ranges are brought back into them, as the web client does.
fn clamped(mode: InputMode) -> InputMode {
    match mode {
        InputMode::VoiceActivity(Sensitivity::ThresholdDecibels(threshold)) => {
            InputMode::VoiceActivity(Sensitivity::ThresholdDecibels(if threshold.is_nan() {
                DEFAULT_SENSITIVITY_DECIBELS
            } else {
                threshold.clamp(MINIMUM_SENSITIVITY_DECIBELS, MAXIMUM_SENSITIVITY_DECIBELS)
            }))
        }
        InputMode::PushToTalk { release_delay } => InputMode::PushToTalk {
            release_delay: release_delay.min(MAXIMUM_PUSH_TO_TALK_RELEASE_DELAY),
        },
        InputMode::VoiceActivity(Sensitivity::Automatic) => mode,
    }
}

/// Whether the user speaks, for their own speaking ring: the server never reports a user to
/// themselves.
#[derive(Debug, Default)]
pub(crate) struct SelfSpeakingDetector {
    frames_since_voice: Option<u32>,
}

impl SelfSpeakingDetector {
    /// Feeds one 10 ms frame; returns whether the user counts as speaking after it.
    pub(crate) fn update(&mut self, rms: f32, is_transmitting: bool) -> bool {
        if is_transmitting && rms >= SELF_SPEAKING_RMS {
            self.frames_since_voice = Some(0);
        } else if let Some(frames) = self.frames_since_voice.as_mut() {
            *frames = frames.saturating_add(1);
        }
        is_transmitting
            && self
                .frames_since_voice
                .is_some_and(|frames| frames < frames_in(SELF_SPEAKING_HANGOVER))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOUD: f32 = -20.0;
    const QUIET: f32 = -70.0;

    fn open_frames(gate: &mut InputGate, level: f32, frames: u32) -> u32 {
        (0..frames)
            .map(|_| u32::from(gate.is_open_for_frame(level)))
            .sum()
    }

    #[test]
    fn automatic_sensitivity_never_gates() {
        let mut gate = InputGate::new(InputMode::VoiceActivity(Sensitivity::Automatic));
        assert_eq!(open_frames(&mut gate, -100.0, 10), 10);
    }

    #[test]
    fn manual_sensitivity_opens_on_voice_and_holds_250_milliseconds() {
        let mut gate = InputGate::new(InputMode::VoiceActivity(Sensitivity::ThresholdDecibels(
            DEFAULT_SENSITIVITY_DECIBELS,
        )));
        assert!(!gate.is_open_for_frame(QUIET));
        assert!(gate.is_open_for_frame(LOUD));
        assert_eq!(open_frames(&mut gate, QUIET, 40), 25, "25 frames of hold");
        assert!(!gate.is_open_for_frame(QUIET));
    }

    #[test]
    fn push_to_talk_stays_open_for_the_release_delay_only() {
        let mut gate = InputGate::new(InputMode::PushToTalk {
            release_delay: DEFAULT_PUSH_TO_TALK_RELEASE_DELAY,
        });
        assert!(
            !gate.is_open_for_frame(LOUD),
            "closed until the key is held"
        );
        gate.set_push_to_talk_pressed(true);
        assert_eq!(open_frames(&mut gate, QUIET, 5), 5);
        gate.set_push_to_talk_pressed(false);
        assert_eq!(
            open_frames(&mut gate, LOUD, 30),
            20,
            "200 ms of release delay"
        );
    }

    #[test]
    fn out_of_range_settings_are_clamped() {
        let mut gate = InputGate::new(InputMode::PushToTalk {
            release_delay: Duration::from_secs(60),
        });
        gate.set_push_to_talk_pressed(true);
        gate.set_push_to_talk_pressed(false);
        assert_eq!(open_frames(&mut gate, QUIET, 1_000), 200, "at most 2 s");
        let mut gate = InputGate::new(InputMode::VoiceActivity(Sensitivity::ThresholdDecibels(
            25.0,
        )));
        assert!(gate.is_open_for_frame(0.0), "the threshold is at most 0 dB");
    }

    #[test]
    fn self_speaking_needs_transmission_and_ends_300_milliseconds_after_voice() {
        let mut detector = SelfSpeakingDetector::default();
        assert!(
            !detector.update(0.5, false),
            "a closed gate is not speaking"
        );
        assert!(detector.update(0.5, true));
        let still_speaking = (0..40).filter(|_| detector.update(0.0, true)).count();
        assert_eq!(
            still_speaking, 29,
            "300 ms hangover counting the voiced frame"
        );
    }
}
