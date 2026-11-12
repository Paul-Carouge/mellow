//! The voice settings' microphone test (`VoiceSettings.svelte`'s "Test the microphone" and "Hear
//! yourself"): the call's audio engine without a call, so the meter and what you hear go through
//! the same processing as your voice in a call. Nothing is sent, and the output opens only while
//! you hear yourself.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Sender};

use crate::audio_engine::{AudioCommand, AudioEngine, AudioEngineChannels, AudioSettings};
use crate::devices::AudioDevices;
use crate::input_gate::InputMode;
use crate::processing::ProcessingSettings;

/// The processed microphone's level, written by the audio thread once per 10 ms frame and read
/// by the interface whenever it redraws its meter.
#[derive(Debug)]
pub(crate) struct MicrophoneLevel {
    /// An `f32` in dBFS; NaN while no microphone is open.
    decibels_bits: AtomicU32,
    is_unavailable: AtomicBool,
}

impl Default for MicrophoneLevel {
    fn default() -> Self {
        Self {
            decibels_bits: AtomicU32::new(f32::NAN.to_bits()),
            is_unavailable: AtomicBool::new(false),
        }
    }
}

impl MicrophoneLevel {
    pub(crate) fn set_decibels(&self, decibels: f32) {
        self.decibels_bits
            .store(decibels.to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn clear(&self) {
        self.set_decibels(f32::NAN);
    }

    pub(crate) fn set_unavailable(&self, is_unavailable: bool) {
        self.is_unavailable.store(is_unavailable, Ordering::Relaxed);
    }

    /// `None` while no microphone is open.
    pub(crate) fn decibels(&self) -> Option<f32> {
        let decibels = f32::from_bits(self.decibels_bits.load(Ordering::Relaxed));
        (!decibels.is_nan()).then_some(decibels)
    }

    pub(crate) fn is_unavailable(&self) -> bool {
        self.is_unavailable.load(Ordering::Relaxed)
    }
}

/// How the test starts; later changes go through its setters.
#[derive(Debug, Clone, Default)]
pub struct MicrophoneTestOptions {
    pub processing: ProcessingSettings,
    pub input_device_id: Option<String>,
    pub output_device_id: Option<String>,
    pub is_monitoring: bool,
}

/// A running microphone test. Dropping it stops its thread, which closes the devices.
#[derive(Debug)]
pub struct MicrophoneTest {
    commands: Sender<AudioCommand>,
    level: Arc<MicrophoneLevel>,
}

impl MicrophoneTest {
    /// Starts the test on its own thread; opening the devices happens there.
    ///
    /// # Errors
    /// The thread could not start.
    pub fn start(
        devices: Box<dyn AudioDevices>,
        options: MicrophoneTestOptions,
    ) -> std::io::Result<Self> {
        let (commands, command_receiver) = mpsc::channel();
        // Nothing reads what a call would send or receive: the test's ends of those are dropped.
        let (_incoming, incoming_receiver) = mpsc::channel();
        let (outgoing, _outgoing_receiver) = mpsc::channel();
        let (events, _event_receiver) = mpsc::channel();
        let level = Arc::new(MicrophoneLevel::default());
        let channels = AudioEngineChannels {
            commands: command_receiver,
            incoming: incoming_receiver,
            outgoing,
            events,
            microphone_level: Arc::clone(&level),
        };
        let settings = AudioSettings {
            is_muted: false,
            is_deafened: false,
            input_mode: InputMode::default(),
            processing: options.processing,
            output_volume: 1.0,
            soundboard_volume: 0.0,
            input_device_id: options.input_device_id,
            output_device_id: options.output_device_id,
            is_monitoring: options.is_monitoring,
            is_microphone_test: true,
        };
        let thread_level = Arc::clone(&level);
        std::thread::Builder::new()
            .name("mellow-microphone-test".into())
            .spawn(move || {
                match AudioEngine::new(devices, channels, settings, std::iter::empty()) {
                    Ok(engine) => engine.run(),
                    Err(error) => {
                        tracing::warn!(%error, "the microphone test could not start");
                        thread_level.set_unavailable(true);
                    }
                }
            })?;
        Ok(Self { commands, level })
    }

    /// The processed microphone's level in dBFS (after the input volume), what the voice
    /// activity threshold compares; `None` until the microphone delivers.
    #[must_use]
    pub fn decibels(&self) -> Option<f32> {
        self.level.decibels()
    }

    /// No microphone could be opened.
    #[must_use]
    pub fn is_microphone_unavailable(&self) -> bool {
        self.level.is_unavailable()
    }

    pub fn set_processing(&self, processing: ProcessingSettings) {
        self.command(AudioCommand::SetProcessing(processing));
    }

    pub fn set_input_device(&self, device_id: Option<String>) {
        self.command(AudioCommand::SetInputDevice(device_id));
    }

    pub fn set_output_device(&self, device_id: Option<String>) {
        self.command(AudioCommand::SetOutputDevice(device_id));
    }

    /// "Hear yourself": the processed microphone on the output.
    pub fn set_monitoring(&self, is_monitoring: bool) {
        self.command(AudioCommand::SetMonitoring(is_monitoring));
    }

    fn command(&self, command: AudioCommand) {
        // A stopped thread already closed its devices: nothing left to change.
        let _ = self.commands.send(command);
    }
}

impl Drop for MicrophoneTest {
    fn drop(&mut self) {
        // The thread is not joined: closing devices can block, and the interface drops the test.
        self.command(AudioCommand::Stop);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::devices::{StreamFormat, SyntheticAudioDevices, SyntheticSound};

    fn wait_until(mut condition: impl FnMut() -> bool) {
        // Waits for the outcome, not a duration: the synthetic devices run in real time.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !condition() {
            assert!(Instant::now() < deadline, "the condition never held");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn the_test_meters_the_microphone_and_plays_it_only_while_hearing_yourself() {
        let format = StreamFormat {
            sample_rate: 48_000,
            channels: 1,
        };
        let devices = SyntheticAudioDevices::new(
            SyntheticSound::Sine {
                frequency: 440.0,
                amplitude: 0.3,
            },
            format,
            format,
        );
        let probe = devices.probe();
        let test = MicrophoneTest::start(
            Box::new(devices),
            MicrophoneTestOptions {
                processing: ProcessingSettings {
                    is_echo_cancellation_on: false,
                    noise_suppression: crate::processing::NoiseSuppressionMode::Off,
                    is_automatic_gain_on: false,
                    ..ProcessingSettings::default()
                },
                ..MicrophoneTestOptions::default()
            },
        )
        .unwrap();
        wait_until(|| test.decibels().is_some_and(|decibels| decibels > -20.0));
        assert!(!test.is_microphone_unavailable());
        assert_eq!(probe.played_samples(), 0, "no output while not monitoring");
        test.set_monitoring(true);
        wait_until(|| probe.audible_samples() > 4_800);
    }
}
