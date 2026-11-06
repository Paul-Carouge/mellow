//! The audio thread: microphone to Opus packets, received packets to the speakers. It is the
//! only owner of the devices' ring buffers on the engine's side; device callbacks only touch the
//! other end of those buffers.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use mellow_protocol::Snowflake;
use str0m::media::Mid;

use crate::audio_format::{Frame, SAMPLES_PER_FRAME, SAMPLES_PER_PACKET, VOICE_SAMPLE_RATE};
use crate::codec::{VoiceDecoder, VoiceEncoder, packet_duration_samples};
use crate::devices::{AudioDevices, CaptureStream, PlaybackStream, has_failed};
use crate::input_gate::{InputGate, InputMode, SelfSpeakingDetector};
use crate::jitter_buffer::{DriftSteering, JitterBuffer, MAXIMUM_STEERING_RATIO};
use crate::level::{
    SILENT_AUDIO_LEVEL, audio_level_attenuation, decibels_full_scale, root_mean_square,
};
use crate::mixer::{limit_to_full_scale, mix_into, playback_gain};
use crate::offer::ReceiveSlotKind;
use crate::processing::{ProcessingSettings, VoiceProcessor};
use crate::resampling::{Interpolation, ResamplingError, StreamResampler};
use crate::session::{AudioDirection, VoiceSessionEvent};
use crate::transport::{IncomingAudioPacket, OutgoingVoicePacket};

/// How often the thread looks at its devices when no command wakes it: well under a frame.
const WORK_PERIOD: Duration = Duration::from_millis(5);
/// How much mixed audio waits in the output ring buffer: enough to ride over the thread's
/// scheduling, little enough to keep the delay low.
const OUTPUT_QUEUE_MILLISECONDS: u32 = 30;
/// The echo canceller's first guess of the speaker-to-microphone path beyond the queued output;
/// it refines the delay itself.
const ASSUMED_DEVICE_LATENCY_MILLISECONDS: u32 = 20;

/// What the session asks of the audio thread.
#[derive(Debug)]
pub(crate) enum AudioCommand {
    AssignSlot { mid: Mid, user_id: Snowflake },
    SetMuted(bool),
    SetDeafened(bool),
    SetPersonVolume { user_id: Snowflake, volume: f32 },
    SetPersonLocallyMuted { user_id: Snowflake, is_muted: bool },
    SetStreamVolume { user_id: Snowflake, volume: f32 },
    SetOutputVolume(f32),
    SetInputMode(InputMode),
    SetPushToTalkPressed(bool),
    SetProcessing(ProcessingSettings),
    SetInputDevice(Option<String>),
    SetOutputDevice(Option<String>),
    Stop,
}

/// The settings the audio thread starts with.
#[derive(Debug, Clone)]
pub(crate) struct AudioSettings {
    pub(crate) is_muted: bool,
    pub(crate) is_deafened: bool,
    pub(crate) input_mode: InputMode,
    pub(crate) processing: ProcessingSettings,
    pub(crate) output_volume: f32,
    pub(crate) input_device_id: Option<String>,
    pub(crate) output_device_id: Option<String>,
}

/// One audio receive m-line: its jitter buffer, decoder and steering.
#[derive(Debug)]
struct ReceiveSlot {
    mid: Mid,
    kind: ReceiveSlotKind,
    user_id: Option<Snowflake>,
    jitter_buffer: JitterBuffer,
    steering: DriftSteering,
    decoder: VoiceDecoder,
    resampler: StreamResampler,
    decoded: VecDeque<f32>,
}

impl ReceiveSlot {
    fn new(mid: Mid, kind: ReceiveSlotKind) -> Result<Self, AudioEngineError> {
        Ok(Self {
            mid,
            kind,
            user_id: None,
            jitter_buffer: JitterBuffer::default(),
            steering: DriftSteering::default(),
            decoder: VoiceDecoder::new()?,
            resampler: StreamResampler::new(
                VOICE_SAMPLE_RATE,
                VOICE_SAMPLE_RATE,
                SAMPLES_PER_PACKET,
                Interpolation::Cubic,
                MAXIMUM_STEERING_RATIO,
            )?,
            decoded: VecDeque::with_capacity(4 * SAMPLES_PER_PACKET),
        })
    }

    fn reset(&mut self) {
        self.jitter_buffer.reset();
        self.steering.reset();
        self.decoded.clear();
    }

    /// The next 10 ms of this slot; `false` when it has nothing to play.
    fn render(&mut self, output: &mut Frame) -> Result<bool, ResamplingError> {
        if self.jitter_buffer.is_idle() && self.decoded.len() < SAMPLES_PER_FRAME {
            self.decoded.clear();
            return Ok(false);
        }
        while self.decoded.len() < SAMPLES_PER_FRAME {
            let decision = self.jitter_buffer.next();
            let samples = self.decoder.decode(&decision);
            let ratio = self.steering.update(
                self.jitter_buffer.depth_samples(),
                self.jitter_buffer.target_delay_samples(),
            );
            self.resampler.set_relative_ratio(ratio)?;
            self.resampler.process(samples, &mut self.decoded)?;
        }
        for (sample, decoded) in output
            .iter_mut()
            .zip(self.decoded.drain(..SAMPLES_PER_FRAME))
        {
            *sample = decoded;
        }
        Ok(true)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AudioEngineError {
    #[error(transparent)]
    Codec(#[from] crate::codec::CodecError),
    #[error(transparent)]
    Resampling(#[from] ResamplingError),
}

/// The microphone side.
#[derive(Debug)]
struct Capture {
    stream: Option<CaptureStream>,
    /// Device rate to 48 kHz, for the open stream.
    resampler: Option<StreamResampler>,
    downmixed: Vec<f32>,
    at_voice_rate: VecDeque<f32>,
    processor: VoiceProcessor,
    gate: InputGate,
    speaking: SelfSpeakingDetector,
    is_speaking: bool,
    encoder: VoiceEncoder,
    packet: Vec<f32>,
    rtp_timestamp: u64,
}

/// The speakers side.
#[derive(Debug)]
struct Playback {
    stream: Option<PlaybackStream>,
    /// 48 kHz to the device rate, for the open stream.
    resampler: Option<StreamResampler>,
    at_device_rate: VecDeque<f32>,
}

impl Playback {
    /// Milliseconds of audio queued for the device.
    fn queued_milliseconds(&self) -> u32 {
        let Some(stream) = self.stream.as_ref() else {
            return 0;
        };
        let queued = stream.samples.buffer().capacity() - stream.samples.slots();
        let per_millisecond =
            u64::from(stream.format.sample_rate) * u64::from(stream.format.channels) / 1_000;
        u32::try_from(u64::try_from(queued).unwrap_or(u64::MAX) / per_millisecond.max(1))
            .unwrap_or(u32::MAX)
    }
}

/// The audio thread's state.
pub(crate) struct AudioEngine {
    devices: Box<dyn AudioDevices>,
    commands: Receiver<AudioCommand>,
    incoming: Receiver<IncomingAudioPacket>,
    outgoing: Sender<OutgoingVoicePacket>,
    events: Sender<VoiceSessionEvent>,
    settings: AudioSettings,
    person_volumes: HashMap<Snowflake, f32>,
    locally_muted: HashSet<Snowflake>,
    stream_volumes: HashMap<Snowflake, f32>,
    slots: Vec<ReceiveSlot>,
    capture: Capture,
    playback: Playback,
    clock_origin: Instant,
    mix: Frame,
    slot_frame: Frame,
}

impl std::fmt::Debug for AudioEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AudioEngine")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

/// The channels of the audio thread.
pub(crate) struct AudioEngineChannels {
    pub(crate) commands: Receiver<AudioCommand>,
    pub(crate) incoming: Receiver<IncomingAudioPacket>,
    pub(crate) outgoing: Sender<OutgoingVoicePacket>,
    pub(crate) events: Sender<VoiceSessionEvent>,
}

impl AudioEngine {
    pub(crate) fn new(
        devices: Box<dyn AudioDevices>,
        channels: AudioEngineChannels,
        settings: AudioSettings,
        receive_slots: impl Iterator<Item = (Mid, ReceiveSlotKind)>,
    ) -> Result<Self, AudioEngineError> {
        let slots = receive_slots
            .map(|(mid, kind)| ReceiveSlot::new(mid, kind))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            devices,
            commands: channels.commands,
            incoming: channels.incoming,
            outgoing: channels.outgoing,
            events: channels.events,
            capture: Capture {
                stream: None,
                resampler: None,
                downmixed: Vec::new(),
                at_voice_rate: VecDeque::with_capacity(4 * SAMPLES_PER_PACKET),
                processor: VoiceProcessor::new(settings.processing),
                gate: InputGate::new(settings.input_mode),
                speaking: SelfSpeakingDetector::default(),
                is_speaking: false,
                encoder: VoiceEncoder::new()?,
                packet: Vec::with_capacity(SAMPLES_PER_PACKET),
                rtp_timestamp: 0,
            },
            playback: Playback {
                stream: None,
                resampler: None,
                at_device_rate: VecDeque::new(),
            },
            settings,
            person_volumes: HashMap::new(),
            locally_muted: HashSet::new(),
            stream_volumes: HashMap::new(),
            slots,
            clock_origin: Instant::now(),
            mix: [0.0; SAMPLES_PER_FRAME],
            slot_frame: [0.0; SAMPLES_PER_FRAME],
        })
    }

    /// Runs until `Stop` or until the session is gone.
    pub(crate) fn run(mut self) {
        self.open_capture();
        self.open_playback();
        loop {
            match self.commands.recv_timeout(WORK_PERIOD) {
                Ok(AudioCommand::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(command) => self.handle_command(command),
                Err(RecvTimeoutError::Timeout) => {}
            }
            while let Ok(command) = self.commands.try_recv() {
                if matches!(command, AudioCommand::Stop) {
                    return;
                }
                self.handle_command(command);
            }
            self.receive_packets();
            self.reopen_failed_devices();
            if let Err(error) = self.capture_microphone() {
                tracing::warn!(%error, "microphone processing failed; reopening it");
                self.open_capture();
            }
            if let Err(error) = self.play_received_audio() {
                tracing::warn!(%error, "playback processing failed; reopening it");
                self.open_playback();
            }
        }
    }

    fn handle_command(&mut self, command: AudioCommand) {
        match command {
            AudioCommand::AssignSlot { mid, user_id } => {
                if let Some(slot) = self.slots.iter_mut().find(|slot| slot.mid == mid) {
                    // The server reuses the slot's stream with another speaker's timestamps.
                    slot.reset();
                    slot.user_id = Some(user_id);
                }
            }
            AudioCommand::SetMuted(is_muted) => self.settings.is_muted = is_muted,
            AudioCommand::SetDeafened(is_deafened) => self.settings.is_deafened = is_deafened,
            AudioCommand::SetPersonVolume { user_id, volume } => {
                self.person_volumes.insert(user_id, volume);
            }
            AudioCommand::SetPersonLocallyMuted { user_id, is_muted } => {
                if is_muted {
                    self.locally_muted.insert(user_id);
                } else {
                    self.locally_muted.remove(&user_id);
                }
            }
            AudioCommand::SetStreamVolume { user_id, volume } => {
                self.stream_volumes.insert(user_id, volume);
            }
            AudioCommand::SetOutputVolume(volume) => self.settings.output_volume = volume,
            AudioCommand::SetInputMode(mode) => {
                self.settings.input_mode = mode;
                self.capture.gate.set_mode(mode);
            }
            AudioCommand::SetPushToTalkPressed(is_pressed) => {
                self.capture.gate.set_push_to_talk_pressed(is_pressed);
            }
            AudioCommand::SetProcessing(processing) => {
                if processing != self.capture.processor.settings() {
                    self.settings.processing = processing;
                    self.capture.processor = VoiceProcessor::new(processing);
                }
            }
            AudioCommand::SetInputDevice(device_id) => {
                self.settings.input_device_id = device_id;
                self.open_capture();
            }
            AudioCommand::SetOutputDevice(device_id) => {
                self.settings.output_device_id = device_id;
                self.open_playback();
            }
            AudioCommand::Stop => {}
        }
    }

    fn open_capture(&mut self) {
        self.capture.stream = None;
        self.capture.resampler = None;
        self.capture.at_voice_rate.clear();
        let opened = self
            .devices
            .open_capture(self.settings.input_device_id.as_deref())
            .map_err(|error| error.to_string())
            .and_then(|stream| {
                let device_rate = stream.format.sample_rate;
                let chunk = usize::try_from(device_rate / 100).unwrap_or(0).max(1);
                StreamResampler::new(
                    device_rate,
                    VOICE_SAMPLE_RATE,
                    chunk,
                    Interpolation::Sinc,
                    1.0,
                )
                .map(|resampler| (stream, resampler))
                .map_err(|error| error.to_string())
            });
        match opened {
            Ok((stream, resampler)) => {
                self.capture.stream = Some(stream);
                self.capture.resampler = Some(resampler);
            }
            Err(error) => {
                tracing::warn!(%error, "no microphone");
                let _ = self
                    .events
                    .send(VoiceSessionEvent::DeviceUnavailable(AudioDirection::Input));
            }
        }
    }

    fn open_playback(&mut self) {
        self.playback.stream = None;
        self.playback.resampler = None;
        self.playback.at_device_rate.clear();
        let opened = self
            .devices
            .open_playback(self.settings.output_device_id.as_deref())
            .map_err(|error| error.to_string())
            .and_then(|stream| {
                StreamResampler::new(
                    VOICE_SAMPLE_RATE,
                    stream.format.sample_rate,
                    SAMPLES_PER_FRAME,
                    Interpolation::Sinc,
                    1.0,
                )
                .map(|resampler| (stream, resampler))
                .map_err(|error| error.to_string())
            });
        match opened {
            Ok((stream, resampler)) => {
                self.playback.stream = Some(stream);
                self.playback.resampler = Some(resampler);
            }
            Err(error) => {
                tracing::warn!(%error, "no audio output");
                let _ = self
                    .events
                    .send(VoiceSessionEvent::DeviceUnavailable(AudioDirection::Output));
            }
        }
    }

    /// A device unplugged mid-call: open the chosen one again, which falls back to the default.
    fn reopen_failed_devices(&mut self) {
        if self
            .capture
            .stream
            .as_ref()
            .is_some_and(|stream| has_failed(&stream.has_failed))
        {
            self.open_capture();
        }
        if self
            .playback
            .stream
            .as_ref()
            .is_some_and(|stream| has_failed(&stream.has_failed))
        {
            self.open_playback();
        }
    }

    fn receive_packets(&mut self) {
        while let Ok(packet) = self.incoming.try_recv() {
            let Some(slot) = self.slots.iter_mut().find(|slot| slot.mid == packet.mid) else {
                continue;
            };
            let since_origin = packet.arrival.saturating_duration_since(self.clock_origin);
            let arrival_samples =
                u64::try_from(since_origin.as_micros() * u128::from(VOICE_SAMPLE_RATE) / 1_000_000)
                    .unwrap_or(u64::MAX);
            let duration = packet_duration_samples(&packet.payload);
            slot.jitter_buffer.push(
                packet.rtp_timestamp,
                arrival_samples,
                packet.payload,
                duration,
            );
        }
    }

    fn capture_microphone(&mut self) -> Result<(), AudioEngineError> {
        let (Some(stream), Some(resampler)) = (
            self.capture.stream.as_mut(),
            self.capture.resampler.as_mut(),
        ) else {
            return Ok(());
        };
        let channels = usize::from(stream.format.channels).max(1);
        let frames = stream.samples.slots() / channels;
        if frames == 0 {
            return Ok(());
        }
        if let Ok(chunk) = stream.samples.read_chunk(frames * channels) {
            let (first, second) = chunk.as_slices();
            // A device frame can straddle the ring's wrap: walk both halves as one sequence.
            let mut samples = first.iter().chain(second);
            self.capture.downmixed.clear();
            #[allow(clippy::cast_precision_loss)] // a handful of channels
            let channel_count = channels as f32;
            for _ in 0..frames {
                let sum: f32 = samples.by_ref().take(channels).sum();
                self.capture.downmixed.push(sum / channel_count);
            }
            chunk.commit_all();
        }
        resampler.process(&self.capture.downmixed, &mut self.capture.at_voice_rate)?;
        let playback_delay =
            self.playback.queued_milliseconds() + ASSUMED_DEVICE_LATENCY_MILLISECONDS;
        while self.capture.at_voice_rate.len() >= SAMPLES_PER_FRAME {
            let mut frame = [0.0; SAMPLES_PER_FRAME];
            for (sample, captured) in frame
                .iter_mut()
                .zip(self.capture.at_voice_rate.drain(..SAMPLES_PER_FRAME))
            {
                *sample = captured;
            }
            self.process_microphone_frame(&mut frame, playback_delay)?;
        }
        Ok(())
    }

    fn process_microphone_frame(
        &mut self,
        frame: &mut Frame,
        playback_delay_milliseconds: u32,
    ) -> Result<(), AudioEngineError> {
        let capture = &mut self.capture;
        capture.processor.process_capture(
            frame,
            i32::try_from(playback_delay_milliseconds).unwrap_or(i32::MAX),
        );
        let rms = root_mean_square(frame);
        let is_transmitting = !self.settings.is_muted
            && !self.settings.is_deafened
            && capture.gate.is_open_for_frame(decibels_full_scale(rms));
        if !is_transmitting {
            frame.fill(0.0);
        }
        let is_speaking = capture.speaking.update(rms, is_transmitting);
        if is_speaking != capture.is_speaking {
            capture.is_speaking = is_speaking;
            let _ = self
                .events
                .send(VoiceSessionEvent::SelfSpeaking(is_speaking));
        }
        capture.packet.extend_from_slice(frame);
        if capture.packet.len() < SAMPLES_PER_PACKET {
            return Ok(());
        }
        let audio_level = audio_level_attenuation(root_mean_square(&capture.packet));
        if let Some(payload) = capture.encoder.encode(&capture.packet)? {
            // The network thread gone means the session is ending.
            let _ = self.outgoing.send(OutgoingVoicePacket {
                rtp_timestamp: capture.rtp_timestamp,
                payload: payload.to_vec(),
                audio_level,
                is_voice: is_transmitting && audio_level < SILENT_AUDIO_LEVEL,
            });
        }
        // Timestamps advance through DTX pauses too, as RFC 3550 asks.
        capture.rtp_timestamp = capture
            .rtp_timestamp
            .wrapping_add(SAMPLES_PER_PACKET as u64);
        capture.packet.clear();
        Ok(())
    }

    fn play_received_audio(&mut self) -> Result<(), AudioEngineError> {
        while self.playback.stream.is_some()
            && self.playback.queued_milliseconds() < OUTPUT_QUEUE_MILLISECONDS
        {
            self.mix_one_frame()?;
            self.capture.processor.analyze_playback(&self.mix);
            let (Some(stream), Some(resampler)) = (
                self.playback.stream.as_mut(),
                self.playback.resampler.as_mut(),
            ) else {
                return Ok(());
            };
            resampler.process(&self.mix, &mut self.playback.at_device_rate)?;
            let channels = usize::from(stream.format.channels).max(1);
            while stream.samples.slots() >= channels {
                let Some(sample) = self.playback.at_device_rate.pop_front() else {
                    break;
                };
                for _ in 0..channels {
                    // Room for `channels` samples was checked just above.
                    let _ = stream.samples.push(sample);
                }
            }
            if stream.samples.slots() < channels {
                break;
            }
        }
        Ok(())
    }

    fn mix_one_frame(&mut self) -> Result<(), AudioEngineError> {
        self.mix.fill(0.0);
        for slot in &mut self.slots {
            if !slot.render(&mut self.slot_frame)? {
                continue;
            }
            // Audio may come before its slot assignment: play it at the default volume.
            let user_id = slot.user_id;
            let gain = match slot.kind {
                ReceiveSlotKind::Voice => playback_gain(
                    self.settings.output_volume,
                    user_id
                        .and_then(|user_id| self.person_volumes.get(&user_id).copied())
                        .unwrap_or(1.0),
                    self.settings.is_deafened
                        || user_id.is_some_and(|user_id| self.locally_muted.contains(&user_id)),
                ),
                ReceiveSlotKind::ScreenSound => playback_gain(
                    self.settings.output_volume,
                    user_id
                        .and_then(|user_id| self.stream_volumes.get(&user_id).copied())
                        .unwrap_or(1.0),
                    self.settings.is_deafened,
                ),
                ReceiveSlotKind::Camera => 0.0,
            };
            mix_into(&mut self.mix, &self.slot_frame, gain);
        }
        limit_to_full_scale(&mut self.mix);
        Ok(())
    }
}
