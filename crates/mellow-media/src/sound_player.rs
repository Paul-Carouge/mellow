//! Sounds played outside a voice session: the message chime, and the leave cue once the session
//! is gone. During a session, play them through the session instead, so the echo canceller
//! hears them. The output opens when a sound comes and closes once nothing has played for a
//! while, so no device is held open in between.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::audio_format::{Frame, SAMPLES_PER_FRAME};
use crate::devices::AudioDevices;
use crate::enhancement::PeakLimiter;
use crate::playback::Playback;
use crate::sounds::{
    LocalSound, PlayingSounds, SoundboardPlayer, SoundboardSoundsToStop, VoiceCue,
    render_notification_chime,
};

/// How often the thread feeds the open output: well under a frame.
const WORK_PERIOD: Duration = Duration::from_millis(5);
/// Silent frames written after the last sound before the output closes (one second): a burst of
/// chimes reuses the open device.
const IDLE_FRAMES_BEFORE_CLOSING: u32 = 100;

#[derive(Debug)]
enum SoundPlayerCommand {
    Play { sound: LocalSound, gain: f32 },
    PlayPreview { sound: LocalSound, gain: f32 },
    StopPreviews,
    PlayCue(VoiceCue),
    PlayNotificationChime,
    SetOutputDevice(Option<String>),
}

/// Plays sounds on its own thread. Dropping it stops the thread.
#[derive(Debug)]
pub struct SoundPlayer {
    commands: Option<Sender<SoundPlayerCommand>>,
    thread: Option<JoinHandle<()>>,
}

impl SoundPlayer {
    /// Starts the player's thread; nothing is opened until a sound plays.
    ///
    /// # Errors
    /// The thread could not be started.
    pub fn new(devices: Box<dyn AudioDevices>) -> std::io::Result<Self> {
        let (commands, command_receiver) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("mellow-sounds".into())
            .spawn(move || run(&*devices, &command_receiver))?;
        Ok(Self {
            commands: Some(commands),
            thread: Some(thread),
        })
    }

    pub fn play_cue(&self, cue: VoiceCue) {
        self.send(SoundPlayerCommand::PlayCue(cue));
    }

    pub fn play_notification_chime(&self) {
        self.send(SoundPlayerCommand::PlayNotificationChime);
    }

    /// Any sound at `gain` (soundboard sounds: [`crate::soundboard_gain`]).
    pub fn play(&self, sound: LocalSound, gain: f32) {
        self.send(SoundPlayerCommand::Play { sound, gain });
    }

    /// A soundboard sound previewed outside a call, at `gain` ([`crate::soundboard_gain`]).
    pub fn play_soundboard_preview(&self, sound: LocalSound, gain: f32) {
        self.send(SoundPlayerCommand::PlayPreview { sound, gain });
    }

    pub fn stop_soundboard_previews(&self) {
        self.send(SoundPlayerCommand::StopPreviews);
    }

    /// Sounds play where voices do; `None` is the system's default output.
    pub fn set_output_device(&self, device_id: Option<String>) {
        self.send(SoundPlayerCommand::SetOutputDevice(device_id));
    }

    fn send(&self, command: SoundPlayerCommand) {
        if let Some(commands) = self.commands.as_ref() {
            // A stopped thread only happens on drop.
            let _ = commands.send(command);
        }
    }
}

impl Drop for SoundPlayer {
    fn drop(&mut self) {
        // Closing the channel stops the thread.
        self.commands = None;
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the sound player's thread panicked");
        }
    }
}

fn run(devices: &dyn AudioDevices, commands: &Receiver<SoundPlayerCommand>) {
    let mut playback = Playback::default();
    let mut sounds = PlayingSounds::default();
    let mut limiter = PeakLimiter::default();
    let mut output_device_id: Option<String> = None;
    let mut chime: Option<LocalSound> = None;
    let mut idle_frames = 0_u32;
    let mut frame: Frame = [0.0; SAMPLES_PER_FRAME];
    loop {
        let command = if playback.is_open() {
            match commands.recv_timeout(WORK_PERIOD) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            // Nothing to feed: sleep until a command comes.
            match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        };
        for command in command.into_iter().chain(commands.try_iter()) {
            match command {
                SoundPlayerCommand::Play { sound, gain } => sounds.play(sound, gain),
                SoundPlayerCommand::PlayPreview { sound, gain } => {
                    sounds.play_soundboard(sound, gain, SoundboardPlayer::Preview);
                }
                SoundPlayerCommand::StopPreviews => {
                    sounds.stop_soundboard(SoundboardSoundsToStop::Previews);
                }
                SoundPlayerCommand::PlayCue(cue) => sounds.play(cue.render(), 1.0),
                SoundPlayerCommand::PlayNotificationChime => {
                    sounds.play(
                        chime.get_or_insert_with(render_notification_chime).clone(),
                        1.0,
                    );
                }
                SoundPlayerCommand::SetOutputDevice(device_id) => {
                    output_device_id = device_id;
                    playback.close();
                }
            }
        }
        // A dead output never wants frames, so it would never idle out: close it now and let the
        // next sound reopen one.
        if sounds.is_empty() && playback.has_failed() {
            playback.close();
        }
        if !sounds.is_empty() && (!playback.is_open() || playback.has_failed()) {
            if let Err(error) = playback.open(devices, output_device_id.as_deref()) {
                tracing::warn!(%error, "no audio output for sounds");
                sounds = PlayingSounds::default();
                continue;
            }
            idle_frames = 0;
        }
        while playback.wants_frame() {
            if sounds.is_empty() {
                idle_frames += 1;
            } else {
                idle_frames = 0;
            }
            frame.fill(0.0);
            sounds.mix_into(&mut frame);
            limiter.process(&mut frame);
            match playback.write(&frame) {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    tracing::warn!(%error, "sound playback failed");
                    playback.close();
                }
            }
        }
        if idle_frames >= IDLE_FRAMES_BEFORE_CLOSING {
            playback.close();
            idle_frames = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Instant;

    use super::*;
    use crate::devices::{
        AudioDeviceDescription, AudioDeviceError, CaptureStream, PlaybackStream, RunningStream,
        StreamFormat, SyntheticAudioDevices, SyntheticSound,
    };

    #[test]
    fn a_chime_reaches_the_output() {
        let format = StreamFormat {
            sample_rate: 44_100,
            channels: 2,
        };
        let devices = SyntheticAudioDevices::new(SyntheticSound::Silence, format, format);
        let probe = devices.probe();
        let player = SoundPlayer::new(Box::new(devices)).unwrap();
        player.play(vec![0.5; 4_800].into(), 1.0);
        // Waits for the outcome, not a duration: the synthetic output plays in real time.
        let deadline = Instant::now() + Duration::from_secs(10);
        while probe.audible_samples() < 4_000 {
            assert!(Instant::now() < deadline, "{}", probe.audible_samples());
            std::thread::sleep(Duration::from_millis(10));
        }
        player.play_notification_chime();
        drop(player);
    }

    /// An output that never plays its samples and reports when the player drops it.
    struct DeadOutput {
        has_failed: Arc<AtomicBool>,
        closed_streams: Arc<AtomicUsize>,
    }

    struct CountedStream {
        _samples: rtrb::Consumer<f32>,
        closed_streams: Arc<AtomicUsize>,
    }

    impl RunningStream for CountedStream {}

    impl Drop for CountedStream {
        fn drop(&mut self) {
            self.closed_streams.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl AudioDevices for DeadOutput {
        fn input_devices(&self) -> Result<Vec<AudioDeviceDescription>, AudioDeviceError> {
            Ok(Vec::new())
        }

        fn output_devices(&self) -> Result<Vec<AudioDeviceDescription>, AudioDeviceError> {
            Ok(Vec::new())
        }

        fn open_capture(&self, _: Option<&str>) -> Result<CaptureStream, AudioDeviceError> {
            Err(AudioDeviceError::NoDevice)
        }

        fn open_playback(&self, _: Option<&str>) -> Result<PlaybackStream, AudioDeviceError> {
            let (samples, consumer) = rtrb::RingBuffer::new(48_000);
            Ok(PlaybackStream {
                format: StreamFormat {
                    sample_rate: 48_000,
                    channels: 1,
                },
                samples,
                has_failed: self.has_failed.clone(),
                running: Box::new(CountedStream {
                    _samples: consumer,
                    closed_streams: self.closed_streams.clone(),
                }),
            })
        }
    }

    #[test]
    fn an_output_that_fails_while_idle_is_closed() {
        let has_failed = Arc::new(AtomicBool::new(false));
        let closed_streams = Arc::new(AtomicUsize::new(0));
        let player = SoundPlayer::new(Box::new(DeadOutput {
            has_failed: has_failed.clone(),
            closed_streams: closed_streams.clone(),
        }))
        .unwrap();
        player.play(vec![0.5; 480].into(), 1.0);
        // The unplugged device stops draining its ring buffer, so it never idles out.
        has_failed.store(true, Ordering::Relaxed);
        let deadline = Instant::now() + Duration::from_secs(10);
        while closed_streams.load(Ordering::Relaxed) == 0 {
            assert!(Instant::now() < deadline, "the dead output stayed open");
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(player);
    }
}
