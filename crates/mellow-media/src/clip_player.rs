//! Audio attachments played in a conversation (ADR-0038): one at a time, as the web client does,
//! on their own thread so decoding never touches the interface. The sound keeps its channels
//! (stereo music stays stereo) and is resampled straight to the output's rate, unlike voices,
//! which the engine mixes in mono. The output opens on play and closes once paused for a while.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::clip_decoder::{ClipDecoder, ClipError, open_clip};
use crate::devices::{AudioDevices, PlaybackStream, has_failed};
use crate::resampling::{Interpolation, StreamResampler};

/// How often the thread feeds the open output: well under the queued time.
const WORK_PERIOD: Duration = Duration::from_millis(5);
/// Audio queued ahead in the output: enough to ride over scheduling, little enough that pause
/// and seek answer at once.
const OUTPUT_QUEUE_MILLISECONDS: u64 = 40;
/// Frames handed to each channel's resampler at a time.
const RESAMPLER_CHUNK_FRAMES: usize = 1_024;
/// A paused output stays open this long, so pausing and playing again does not reopen it.
const IDLE_BEFORE_CLOSING: Duration = Duration::from_secs(2);
/// Packets decoded without output (a seek skipping ahead) before the thread looks at its
/// commands again: a long skip never holds up pause or stop for more than a moment.
const MAXIMUM_SILENT_PACKETS_PER_FEED: u64 = 64;

/// Where the loaded sound is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipPhase {
    Playing,
    Paused,
    Ended,
    /// The file is not played here (`Unsupported`) or broke part way (`Corrupt`).
    CannotPlay(ClipError),
    /// No output device could be opened; playing again tries again.
    NoOutput,
}

/// What the interface shows; read with [`ClipPlayer::status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipStatus {
    /// The caller's key of the loaded sound (an attachment id); `None` when nothing is loaded.
    pub clip: Option<u64>,
    pub phase: ClipPhase,
    pub position: Duration,
    pub duration: Option<Duration>,
}

impl Default for ClipStatus {
    fn default() -> Self {
        Self {
            clip: None,
            phase: ClipPhase::Paused,
            position: Duration::ZERO,
            duration: None,
        }
    }
}

enum Command {
    Load {
        clip: u64,
        bytes: Arc<[u8]>,
        content_type: String,
    },
    Play,
    Pause,
    Seek(Duration),
    SetVolume(f32),
    SetOutputDevice(Option<String>),
    Stop,
}

/// Plays one attachment at a time on its own thread. Dropping it stops the thread.
#[derive(Debug)]
pub struct ClipPlayer {
    commands: Option<Sender<Command>>,
    status: Arc<Mutex<ClipStatus>>,
    thread: Option<JoinHandle<()>>,
}

impl ClipPlayer {
    /// Starts the player's thread; nothing is opened until a sound plays.
    ///
    /// # Errors
    /// The thread could not be started.
    pub fn new(devices: Box<dyn AudioDevices>) -> std::io::Result<Self> {
        let (commands, command_receiver) = mpsc::channel();
        let status = Arc::new(Mutex::new(ClipStatus::default()));
        let shared_status = Arc::clone(&status);
        let thread = std::thread::Builder::new()
            .name("mellow-clips".into())
            .spawn(move || run(&*devices, &command_receiver, &shared_status))?;
        Ok(Self {
            commands: Some(commands),
            status,
            thread: Some(thread),
        })
    }

    /// Replaces the loaded sound with `bytes` (a whole file of `content_type`) and plays it from
    /// the start. The type and first bytes are checked before anything is decoded.
    pub fn load_and_play(&self, clip: u64, bytes: Arc<[u8]>, content_type: &str) {
        self.send(Command::Load {
            clip,
            bytes,
            content_type: content_type.to_owned(),
        });
        self.send(Command::Play);
    }

    /// Plays the loaded sound; from the start once it ended.
    pub fn play(&self) {
        self.send(Command::Play);
    }

    pub fn pause(&self) {
        self.send(Command::Pause);
    }

    pub fn seek(&self, position: Duration) {
        self.send(Command::Seek(position));
    }

    /// 0 (silent) to 1 (the file as it is).
    pub fn set_volume(&self, volume: f32) {
        self.send(Command::SetVolume(volume));
    }

    /// `None` is the system's default output.
    pub fn set_output_device(&self, device_id: Option<String>) {
        self.send(Command::SetOutputDevice(device_id));
    }

    /// Stops and frees the loaded sound.
    pub fn stop(&self) {
        self.send(Command::Stop);
    }

    #[must_use]
    pub fn status(&self) -> ClipStatus {
        *self.status.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The position of `clip` while it is the loaded sound (the start otherwise), for a video's
    /// pictures to follow its sound.
    #[must_use]
    pub fn clock(&self, clip: u64) -> ClipClock {
        ClipClock {
            status: Arc::clone(&self.status),
            clip,
        }
    }

    fn send(&self, command: Command) {
        if let Some(commands) = self.commands.as_ref() {
            // A stopped thread only happens on drop.
            let _ = commands.send(command);
        }
    }
}

/// A loaded sound's position, read from another thread: see [`ClipPlayer::clock`].
#[derive(Debug, Clone)]
pub struct ClipClock {
    status: Arc<Mutex<ClipStatus>>,
    clip: u64,
}

impl crate::video_file::VideoClock for ClipClock {
    fn position(&self) -> Duration {
        let status = *self.status.lock().unwrap_or_else(PoisonError::into_inner);
        if status.clip == Some(self.clip) {
            status.position
        } else {
            Duration::ZERO
        }
    }
}

impl Drop for ClipPlayer {
    fn drop(&mut self) {
        // Closing the channel stops the thread.
        self.commands = None;
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the clip player's thread panicked");
        }
    }
}

/// The open output and what waits for it, at the device's rate, one queue per source channel.
struct ClipOutput {
    stream: PlaybackStream,
    resamplers: Vec<StreamResampler>,
    queues: Vec<VecDeque<f32>>,
    channel_samples: Vec<f32>,
}

impl ClipOutput {
    fn open(
        devices: &dyn AudioDevices,
        device_id: Option<&str>,
        source_rate: u32,
        channels: usize,
    ) -> Result<Self, String> {
        let stream = devices
            .open_playback(device_id)
            .map_err(|error| error.to_string())?;
        let mut output = Self {
            resamplers: Vec::new(),
            queues: vec![VecDeque::new(); channels],
            channel_samples: Vec::new(),
            stream,
        };
        output.restart(source_rate)?;
        Ok(output)
    }

    /// Drops what waits (after a seek) and starts the resamplers afresh.
    fn restart(&mut self, source_rate: u32) -> Result<(), String> {
        let device_rate = self.stream.format.sample_rate;
        self.resamplers = (0..self.queues.len())
            .map(|_| {
                StreamResampler::new(
                    source_rate,
                    device_rate,
                    RESAMPLER_CHUNK_FRAMES,
                    Interpolation::Sinc,
                    1.0,
                )
            })
            .collect::<Result<_, _>>()
            .map_err(|error| error.to_string())?;
        self.queues.iter_mut().for_each(VecDeque::clear);
        Ok(())
    }

    fn device_channels(&self) -> usize {
        usize::from(self.stream.format.channels).max(1)
    }

    /// Device frames waiting in the ring buffer and the queues.
    fn queued_device_frames(&self) -> u64 {
        let in_ring = (self.stream.samples.buffer().capacity() - self.stream.samples.slots())
            / self.device_channels();
        let in_queues = self.queues.iter().map(VecDeque::len).min().unwrap_or(0);
        (in_ring + in_queues) as u64
    }

    fn wants_more(&self) -> bool {
        let queued_milliseconds =
            self.queued_device_frames() * 1_000 / u64::from(self.stream.format.sample_rate).max(1);
        queued_milliseconds < OUTPUT_QUEUE_MILLISECONDS
    }

    /// Resamples interleaved source samples into the channel queues.
    fn feed(&mut self, interleaved: &[f32]) -> Result<(), String> {
        let channels = self.queues.len();
        for (channel, (resampler, queue)) in self
            .resamplers
            .iter_mut()
            .zip(self.queues.iter_mut())
            .enumerate()
        {
            self.channel_samples.clear();
            self.channel_samples
                .extend(interleaved.iter().skip(channel).step_by(channels));
            resampler
                .process(&self.channel_samples, queue)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Moves queued frames into the ring buffer at `gain`: a mono sound to every speaker, a
    /// stereo one to the first two (both averaged on a mono device).
    fn push(&mut self, gain: f32) {
        let device_channels = self.device_channels();
        while self.stream.samples.slots() >= device_channels
            && self.queues.iter().all(|queue| !queue.is_empty())
        {
            let left = self.queues[0].pop_front().unwrap_or_default() * gain;
            let right = match self.queues.get_mut(1) {
                Some(queue) => queue.pop_front().unwrap_or_default() * gain,
                None => left,
            };
            for speaker in 0..device_channels {
                let sample = match (device_channels, speaker) {
                    (1, _) => f32::midpoint(left, right),
                    (_, 0) => left,
                    (_, 1) => right,
                    _ if self.queues.len() == 1 => left,
                    _ => 0.0,
                };
                // Room for a whole frame was checked just above.
                let _ = self.stream.samples.push(sample);
            }
        }
    }

    fn is_drained(&self) -> bool {
        self.queued_device_frames() == 0
    }
}

/// The loaded sound and where it stands.
struct LoadedClip {
    decoder: Box<dyn ClipDecoder>,
    /// The frame (source rate) the last seek went to.
    base_frame: u64,
    /// Source frames handed to the output since that seek.
    fed_frames: u64,
    has_decoded_everything: bool,
}

impl LoadedClip {
    fn frames_to_duration(&self, frames: u64) -> Duration {
        #[allow(clippy::cast_precision_loss)] // Frames of a sound stay far below 2^52.
        Duration::from_secs_f64(frames as f64 / f64::from(self.decoder.sample_rate()))
    }
}

struct PlayerState {
    /// The sound the status describes: loaded, or refused (`CannotPlay`).
    key: Option<u64>,
    loaded: Option<LoadedClip>,
    output: Option<ClipOutput>,
    phase: ClipPhase,
    volume: f32,
    output_device_id: Option<String>,
    idle_since_paused: Duration,
    samples: Vec<f32>,
}

impl PlayerState {
    fn new() -> Self {
        Self {
            key: None,
            loaded: None,
            output: None,
            phase: ClipPhase::Paused,
            volume: 1.0,
            output_device_id: None,
            idle_since_paused: Duration::ZERO,
            samples: Vec::new(),
        }
    }
}

fn run(devices: &dyn AudioDevices, commands: &Receiver<Command>, status: &Mutex<ClipStatus>) {
    let mut state = PlayerState::new();
    loop {
        let command = if state.output.is_some() || state.phase == ClipPhase::Playing {
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
            apply(&mut state, command);
        }
        if state.phase == ClipPhase::Playing {
            feed(&mut state, devices);
            state.idle_since_paused = Duration::ZERO;
        } else if state.output.is_some() {
            state.idle_since_paused += WORK_PERIOD;
            if state.idle_since_paused >= IDLE_BEFORE_CLOSING {
                state.output = None;
            }
        }
        publish(&state, status);
    }
}

fn apply(state: &mut PlayerState, command: Command) {
    match command {
        Command::Load {
            clip,
            bytes,
            content_type,
        } => {
            state.output = None;
            state.key = Some(clip);
            match without_panics(|| open_clip(bytes, &content_type)) {
                Ok(decoder) => {
                    state.loaded = Some(LoadedClip {
                        decoder,
                        base_frame: 0,
                        fed_frames: 0,
                        has_decoded_everything: false,
                    });
                    state.phase = ClipPhase::Paused;
                }
                Err(error) => {
                    state.loaded = None;
                    state.phase = ClipPhase::CannotPlay(error);
                }
            }
        }
        Command::Play => {
            if state.loaded.is_none() {
                return;
            }
            if state.phase == ClipPhase::Ended {
                seek_to(state, 0);
            }
            if state.loaded.is_some() {
                state.phase = ClipPhase::Playing;
            }
        }
        Command::Pause => {
            if state.phase == ClipPhase::Playing {
                state.phase = ClipPhase::Paused;
            }
        }
        Command::Seek(position) => {
            let Some(loaded) = state.loaded.as_ref() else {
                return;
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            // A position in seconds times a rate stays far below 2^53, and is never negative.
            let mut frame =
                (position.as_secs_f64() * f64::from(loaded.decoder.sample_rate())) as u64;
            if let Some(duration) = loaded.decoder.duration_frames() {
                frame = frame.min(duration);
            }
            seek_to(state, frame);
            if state.phase == ClipPhase::Ended {
                state.phase = ClipPhase::Paused;
            }
        }
        Command::SetVolume(volume) => state.volume = volume.clamp(0.0, 1.0),
        Command::SetOutputDevice(device_id) => {
            state.output_device_id = device_id;
            state.output = None;
        }
        Command::Stop => {
            state.output = None;
            state.loaded = None;
            state.key = None;
            state.phase = ClipPhase::Paused;
        }
    }
}

fn seek_to(state: &mut PlayerState, frame: u64) {
    let Some(loaded) = state.loaded.as_mut() else {
        return;
    };
    if let Err(error) = without_panics(|| loaded.decoder.seek(frame)) {
        state.loaded = None;
        state.output = None;
        state.phase = ClipPhase::CannotPlay(error);
        return;
    }
    loaded.base_frame = frame;
    loaded.fed_frames = 0;
    loaded.has_decoded_everything = false;
    let rate = loaded.decoder.sample_rate();
    if let Some(output) = state.output.as_mut()
        && output.restart(rate).is_err()
    {
        state.output = None;
    }
}

/// Keeps the output fed while playing; notes the end and failures in the phase.
fn feed(state: &mut PlayerState, devices: &dyn AudioDevices) {
    let Some(loaded) = state.loaded.as_mut() else {
        state.phase = ClipPhase::Paused;
        return;
    };
    if state
        .output
        .as_ref()
        .is_none_or(|output| has_failed(&output.stream.has_failed))
    {
        match ClipOutput::open(
            devices,
            state.output_device_id.as_deref(),
            loaded.decoder.sample_rate(),
            loaded.decoder.channels(),
        ) {
            Ok(output) => state.output = Some(output),
            Err(error) => {
                tracing::warn!(%error, "no audio output for an attachment");
                state.output = None;
                state.phase = ClipPhase::NoOutput;
                return;
            }
        }
    }
    let Some(output) = state.output.as_mut() else {
        return;
    };
    let mut silent_packets = 0;
    loop {
        output.push(state.volume);
        if !output.wants_more() {
            return;
        }
        if loaded.has_decoded_everything {
            if output.is_drained() {
                state.phase = ClipPhase::Ended;
            }
            return;
        }
        if silent_packets >= MAXIMUM_SILENT_PACKETS_PER_FEED {
            return;
        }
        state.samples.clear();
        match without_panics(|| loaded.decoder.decode_next(&mut state.samples)) {
            Ok(has_more) => {
                if state.samples.is_empty() {
                    silent_packets += 1;
                }
                let channels = loaded.decoder.channels();
                loaded.fed_frames += (state.samples.len() / channels) as u64;
                if !has_more {
                    loaded.has_decoded_everything = true;
                    // Silence pushes the resamplers' last partial chunk and delay out.
                    state.samples.resize(
                        state.samples.len() + 2 * RESAMPLER_CHUNK_FRAMES * channels,
                        0.0,
                    );
                }
                if let Err(error) = output.feed(&state.samples) {
                    tracing::warn!(%error, "an attachment could not be resampled");
                    state.output = None;
                    state.phase = ClipPhase::NoOutput;
                    return;
                }
            }
            Err(error) => {
                state.loaded = None;
                state.output = None;
                state.phase = ClipPhase::CannotPlay(error);
                return;
            }
        }
    }
}

/// Runs decoder work so that a panic in a parser fed a hostile file (ogg, libopus, symphonia)
/// reports the file as damaged instead of ending the thread, and with it every later sound.
fn without_panics<T>(work: impl FnOnce() -> Result<T, ClipError>) -> Result<T, ClipError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|_| {
        tracing::error!("an attachment's decoder panicked");
        Err(ClipError::Corrupt)
    })
}

fn publish(state: &PlayerState, status: &Mutex<ClipStatus>) {
    let (position, duration) = state
        .loaded
        .as_ref()
        .map_or((Duration::ZERO, None), |loaded| {
            let duration_frames = loaded.decoder.duration_frames();
            let played = if state.phase == ClipPhase::Ended {
                duration_frames.unwrap_or(loaded.base_frame + loaded.fed_frames)
            } else {
                let queued = state.output.as_ref().map_or(0, |output| {
                    output.queued_device_frames() * u64::from(loaded.decoder.sample_rate())
                        / u64::from(output.stream.format.sample_rate).max(1)
                });
                (loaded.base_frame + loaded.fed_frames).saturating_sub(queued)
            };
            let played = duration_frames.map_or(played, |duration| played.min(duration));
            (
                loaded.frames_to_duration(played),
                duration_frames.map(|frames| loaded.frames_to_duration(frames)),
            )
        });
    *status.lock().unwrap_or_else(PoisonError::into_inner) = ClipStatus {
        clip: state.key,
        phase: state.phase,
        position,
        duration,
    };
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    use super::*;
    use crate::clip_decoder::tests::tone_wav;
    use crate::devices::{StreamFormat, SyntheticAudioDevices, SyntheticSound};

    /// Waits for the player to report `wanted`; the deadline only guards against a hang.
    fn wait_for(player: &ClipPlayer, wanted: impl Fn(&ClipStatus) -> bool) -> ClipStatus {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = player.status();
            if wanted(&status) {
                return status;
            }
            assert!(Instant::now() < deadline, "{status:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn devices() -> SyntheticAudioDevices {
        let format = StreamFormat {
            sample_rate: 48_000,
            channels: 2,
        };
        SyntheticAudioDevices::new(SyntheticSound::Silence, format, format)
    }

    #[test]
    fn a_sound_plays_to_its_end_on_the_output() {
        let devices = devices();
        let probe = devices.probe();
        let player = ClipPlayer::new(Box::new(devices)).unwrap();
        player.load_and_play(7, tone_wav(44_100, 2, 8_820).into(), "audio/wav");

        let ended = wait_for(&player, |status| status.phase == ClipPhase::Ended);
        assert_eq!(ended.clip, Some(7));
        assert_eq!(ended.duration, Some(Duration::from_millis(200)));
        assert_eq!(ended.position, Duration::from_millis(200));
        assert!(probe.audible_samples() > 0);

        // Played again from the start, then stopped and freed.
        player.play();
        wait_for(&player, |status| status.phase == ClipPhase::Playing);
        player.stop();
        assert_eq!(
            wait_for(&player, |status| status.clip.is_none()).phase,
            ClipPhase::Paused
        );
    }

    #[test]
    fn a_seek_while_paused_moves_the_position_without_playing() {
        let player = ClipPlayer::new(Box::new(devices())).unwrap();
        player.load_and_play(1, tone_wav(48_000, 1, 48_000).into(), "audio/wav");
        player.pause();
        player.seek(Duration::from_millis(750));
        let status = wait_for(&player, |status| {
            status.phase == ClipPhase::Paused && status.position == Duration::from_millis(750)
        });
        assert_eq!(status.duration, Some(Duration::from_secs(1)));
    }

    #[test]
    fn a_file_not_played_here_is_reported_with_its_key() {
        let player = ClipPlayer::new(Box::new(devices())).unwrap();
        player.load_and_play(3, b"OggS-not-really".to_vec().into(), "audio/ogg");
        let status = wait_for(&player, |status| status.clip == Some(3));
        assert!(
            matches!(status.phase, ClipPhase::CannotPlay(_)),
            "{status:?}"
        );
    }

    /// Decodes nothing, as a long seek does while it skips ahead, for `limit` packets; or panics,
    /// as a decoder fed a hostile file might.
    struct SkippingDecoder {
        decoded_packets: Arc<AtomicU64>,
        limit: u64,
        panics: bool,
    }

    impl ClipDecoder for SkippingDecoder {
        fn sample_rate(&self) -> u32 {
            48_000
        }
        fn channels(&self) -> usize {
            1
        }
        fn duration_frames(&self) -> Option<u64> {
            None
        }
        fn decode_next(&mut self, _samples: &mut Vec<f32>) -> Result<bool, ClipError> {
            assert!(!self.panics, "a hostile file");
            let decoded = self.decoded_packets.fetch_add(1, Ordering::Relaxed) + 1;
            Ok(decoded < self.limit)
        }
        fn seek(&mut self, _frame: u64) -> Result<(), ClipError> {
            Ok(())
        }
    }

    fn playing(decoder: SkippingDecoder) -> PlayerState {
        let mut state = PlayerState::new();
        state.key = Some(1);
        state.loaded = Some(LoadedClip {
            decoder: Box::new(decoder),
            base_frame: 0,
            fed_frames: 0,
            has_decoded_everything: false,
        });
        state.phase = ClipPhase::Playing;
        state
    }

    #[test]
    fn a_long_skip_ahead_hands_back_to_the_commands_between_batches() {
        let decoded_packets = Arc::new(AtomicU64::new(0));
        let mut state = playing(SkippingDecoder {
            decoded_packets: Arc::clone(&decoded_packets),
            limit: 1_000_000,
            panics: false,
        });
        feed(&mut state, &devices());
        let decoded = decoded_packets.load(Ordering::Relaxed);
        assert!(decoded <= MAXIMUM_SILENT_PACKETS_PER_FEED, "{decoded}");
        assert_eq!(state.phase, ClipPhase::Playing);
    }

    #[test]
    fn a_decoder_that_panics_reports_a_damaged_file_and_the_player_lives_on() {
        let mut state = playing(SkippingDecoder {
            decoded_packets: Arc::new(AtomicU64::new(0)),
            limit: 1,
            panics: true,
        });
        feed(&mut state, &devices());
        assert_eq!(state.phase, ClipPhase::CannotPlay(ClipError::Corrupt));
        assert!(state.loaded.is_none());
    }
}
