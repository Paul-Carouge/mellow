//! The audio thread: microphone to Opus packets, received packets to the speakers. It is the
//! only owner of the devices' ring buffers on the engine's side; device callbacks only touch the
//! other end of those buffers.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use mellow_protocol::Snowflake;
use str0m::media::Mid;

use crate::audio_format::{Frame, SAMPLES_PER_FRAME, SAMPLES_PER_PACKET, VOICE_SAMPLE_RATE};
use crate::codec::{VoiceDecoder, VoiceEncoder, packet_duration_samples};
use crate::devices::{AudioDevices, CaptureStream, has_failed};
use crate::enhancement::PeakLimiter;
use crate::input_gate::{InputGate, InputMode, SelfSpeakingDetector};
use crate::jitter_buffer::{DriftSteering, JitterBuffer, MAXIMUM_STEERING_RATIO};
use crate::level::MicrophoneLevel;
use crate::level::{
    SILENT_AUDIO_LEVEL, audio_level_attenuation, decibels_full_scale, root_mean_square,
};
use crate::mixer::{mix_into, playback_gain};
use crate::offer::ReceiveSlotKind;
use crate::person_settings::{PersonSoundSource, PersonVoiceSetting};
use crate::playback::Playback;
use crate::processing::{ProcessingSettings, VoiceProcessor};
use crate::resampling::{Interpolation, ResamplingError, StreamResampler};
use crate::session::{AudioDirection, VoiceSessionEvent};
use crate::soundboard::soundboard_gain;
use crate::sounds::{
    LocalSound, PlayingSounds, SoundboardPlayer, SoundboardSoundsToStop, VoiceCue,
    render_notification_chime,
};
use crate::transport::{IncomingAudioPacket, OutgoingVoicePacket};

/// How often the thread looks at its devices when no command wakes it: well under a frame.
const WORK_PERIOD: Duration = Duration::from_millis(5);
/// The echo canceller's first guess of the speaker-to-microphone path beyond the queued output;
/// it refines the delay itself.
const ASSUMED_DEVICE_LATENCY_MILLISECONDS: u32 = 20;
/// "Hear yourself" keeps at most this much microphone waiting for the output: the two devices'
/// clocks drift apart, and an old syllable is worse than a dropped one.
const MAXIMUM_MONITORED_SAMPLES: usize = 6 * SAMPLES_PER_FRAME;

/// What the session asks of the audio thread.
#[derive(Debug)]
pub(crate) enum AudioCommand {
    /// A new media session (a new offer): its receive lines, none assigned yet.
    NewMediaSession(Vec<(Mid, ReceiveSlotKind)>),
    AssignSlot {
        mid: Mid,
        user_id: Snowflake,
        setting: PersonVoiceSetting,
    },
    SetMuted(bool),
    SetDeafened(bool),
    SetPersonSetting {
        source: PersonSoundSource,
        user_id: Snowflake,
        setting: PersonVoiceSetting,
    },
    SetOutputVolume(f32),
    SetSoundboardVolume(f32),
    PlayCue(VoiceCue),
    /// The message chime, at full level like the cues.
    PlayNotificationChime,
    /// A soundboard or join sound, at the output volume times the soundboard volume.
    PlaySoundboardSound(LocalSound, SoundboardPlayer),
    StopSoundboardSounds(SoundboardSoundsToStop),
    SetInputMode(InputMode),
    SetPushToTalkPressed(bool),
    SetProcessing(ProcessingSettings),
    SetInputDevice(Option<String>),
    SetOutputDevice(Option<String>),
    /// "Hear yourself": the microphone plays on the output, as others hear it.
    SetMonitoring(bool),
    Stop,
}

/// The settings the audio thread starts with.
#[derive(Debug, Clone)]
// Independent switches the commands flip one by one: no state machine hides here.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct AudioSettings {
    pub(crate) is_muted: bool,
    pub(crate) is_deafened: bool,
    pub(crate) input_mode: InputMode,
    pub(crate) processing: ProcessingSettings,
    pub(crate) output_volume: f32,
    pub(crate) soundboard_volume: f32,
    pub(crate) input_device_id: Option<String>,
    pub(crate) output_device_id: Option<String>,
    pub(crate) is_monitoring: bool,
    /// The settings' microphone test: nothing is sent, the gate stays open for the meter and the
    /// output opens only to hear yourself.
    pub(crate) is_microphone_test: bool,
}

/// One audio receive m-line: its jitter buffer, decoder and steering.
#[derive(Debug)]
struct ReceiveSlot {
    mid: Mid,
    kind: ReceiveSlotKind,
    user_id: Option<Snowflake>,
    /// The assigned person's volume and local mute for this slot's source.
    setting: PersonVoiceSetting,
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
            setting: PersonVoiceSetting::default(),
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
    /// Processed microphone waiting to be heard ("hear yourself").
    monitored: VecDeque<f32>,
}

/// The audio thread's state.
pub(crate) struct AudioEngine {
    devices: Box<dyn AudioDevices>,
    commands: Receiver<AudioCommand>,
    incoming: Receiver<IncomingAudioPacket>,
    outgoing: Sender<OutgoingVoicePacket>,
    events: Sender<VoiceSessionEvent>,
    microphone_level: Arc<MicrophoneLevel>,
    settings: AudioSettings,
    slots: Vec<ReceiveSlot>,
    capture: Capture,
    playback: Playback,
    sounds: PlayingSounds,
    limiter: PeakLimiter,
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
    /// Where the processed microphone's level is published for the settings' meter.
    pub(crate) microphone_level: Arc<MicrophoneLevel>,
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
            microphone_level: channels.microphone_level,
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
                monitored: VecDeque::with_capacity(MAXIMUM_MONITORED_SAMPLES + SAMPLES_PER_FRAME),
            },
            playback: Playback::default(),
            sounds: PlayingSounds::default(),
            limiter: PeakLimiter::default(),
            settings,
            slots,
            clock_origin: Instant::now(),
            mix: [0.0; SAMPLES_PER_FRAME],
            slot_frame: [0.0; SAMPLES_PER_FRAME],
        })
    }

    /// Runs until `Stop` or until the session is gone.
    pub(crate) fn run(mut self) {
        self.open_capture();
        if self.is_output_wanted() {
            self.open_playback();
        }
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
            AudioCommand::NewMediaSession(lines) => {
                match lines
                    .into_iter()
                    .map(|(mid, kind)| ReceiveSlot::new(mid, kind))
                    .collect()
                {
                    Ok(slots) => self.slots = slots,
                    Err(error) => {
                        tracing::warn!(%error, "the new media session's decoders could not start");
                        self.slots.clear();
                    }
                }
            }
            AudioCommand::AssignSlot {
                mid,
                user_id,
                setting,
            } => {
                if let Some(slot) = self.slots.iter_mut().find(|slot| slot.mid == mid) {
                    // The server reuses the slot's stream with another speaker's timestamps.
                    slot.reset();
                    slot.user_id = Some(user_id);
                    slot.setting = setting;
                }
            }
            AudioCommand::SetMuted(is_muted) => self.settings.is_muted = is_muted,
            AudioCommand::SetDeafened(is_deafened) => self.settings.is_deafened = is_deafened,
            AudioCommand::SetPersonSetting {
                source,
                user_id,
                setting,
            } => {
                let kind = match source {
                    PersonSoundSource::Voice => ReceiveSlotKind::Voice,
                    PersonSoundSource::ScreenSound => ReceiveSlotKind::ScreenSound,
                };
                for slot in &mut self.slots {
                    if slot.kind == kind && slot.user_id == Some(user_id) {
                        slot.setting = setting;
                    }
                }
            }
            AudioCommand::SetOutputVolume(volume) => self.settings.output_volume = volume,
            AudioCommand::SetSoundboardVolume(volume) => self.settings.soundboard_volume = volume,
            AudioCommand::PlayCue(cue) => self.sounds.play(cue.render(), 1.0),
            AudioCommand::PlayNotificationChime => {
                self.sounds.play(render_notification_chime(), 1.0);
            }
            AudioCommand::PlaySoundboardSound(sound, player) => self.sounds.play_soundboard(
                sound,
                soundboard_gain(self.settings.output_volume, self.settings.soundboard_volume),
                player,
            ),
            AudioCommand::StopSoundboardSounds(which) => self.sounds.stop_soundboard(which),
            AudioCommand::SetInputMode(mode) => {
                self.settings.input_mode = mode;
                self.capture.gate.set_mode(mode);
            }
            AudioCommand::SetPushToTalkPressed(is_pressed) => {
                self.capture.gate.set_push_to_talk_pressed(is_pressed);
            }
            AudioCommand::SetProcessing(processing) => {
                self.settings.processing = processing;
                self.capture.processor.apply(processing);
            }
            AudioCommand::SetInputDevice(device_id) => {
                self.settings.input_device_id = device_id;
                self.open_capture();
            }
            AudioCommand::SetOutputDevice(device_id) => {
                self.settings.output_device_id = device_id;
                if self.is_output_wanted() {
                    self.open_playback();
                }
            }
            AudioCommand::SetMonitoring(is_monitoring) => {
                self.settings.is_monitoring = is_monitoring;
                self.capture.monitored.clear();
                if !self.is_output_wanted() {
                    self.playback.close();
                } else if !self.playback.is_open() {
                    self.open_playback();
                }
            }
            AudioCommand::Stop => {}
        }
    }

    /// A call always plays; the microphone test only while you hear yourself.
    fn is_output_wanted(&self) -> bool {
        !self.settings.is_microphone_test || self.settings.is_monitoring
    }

    fn open_capture(&mut self) {
        self.capture.stream = None;
        self.capture.resampler = None;
        self.capture.at_voice_rate.clear();
        self.microphone_level.clear();
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
                self.microphone_level.set_unavailable(false);
            }
            Err(error) => {
                tracing::warn!(%error, "no microphone");
                self.microphone_level.set_unavailable(true);
                let _ = self
                    .events
                    .send(VoiceSessionEvent::DeviceUnavailable(AudioDirection::Input));
            }
        }
    }

    fn open_playback(&mut self) {
        if let Err(error) = self
            .playback
            .open(&*self.devices, self.settings.output_device_id.as_deref())
        {
            tracing::warn!(%error, "no audio output");
            let _ = self
                .events
                .send(VoiceSessionEvent::DeviceUnavailable(AudioDirection::Output));
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
        if self.playback.has_failed() {
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
        let decibels = decibels_full_scale(rms);
        self.microphone_level.set_decibels(decibels);
        if self.settings.is_microphone_test {
            if self.settings.is_monitoring {
                keep_for_monitoring(&mut capture.monitored, frame);
            }
            return Ok(());
        }
        let is_transmitting = !self.settings.is_muted
            && !self.settings.is_deafened
            && capture.gate.is_open_for_frame(decibels);
        if !is_transmitting {
            frame.fill(0.0);
        }
        // What the others hear: gated, muted and pushed to talk as the packet is.
        if self.settings.is_monitoring {
            keep_for_monitoring(&mut capture.monitored, frame);
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
        while self.playback.wants_frame() {
            self.mix_one_frame();
            self.capture.processor.analyze_playback(&self.mix);
            // After the echo canceller's far end: it would otherwise cancel your own voice.
            self.add_monitored_microphone();
            if !self.playback.write(&self.mix)? {
                break;
            }
        }
        Ok(())
    }

    fn mix_one_frame(&mut self) {
        self.mix.fill(0.0);
        for slot in &mut self.slots {
            match slot.render(&mut self.slot_frame) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    // One received voice failing must not reopen everyone's output: that voice
                    // starts over with its next packet.
                    tracing::warn!(%error, "a received voice could not be resampled; resetting it");
                    slot.reset();
                    continue;
                }
            }
            // Audio may come before its slot assignment: it plays at the default setting.
            let gain = match slot.kind {
                ReceiveSlotKind::Voice | ReceiveSlotKind::ScreenSound => playback_gain(
                    self.settings.output_volume,
                    slot.setting.volume,
                    self.settings.is_deafened || slot.setting.is_locally_muted,
                ),
                ReceiveSlotKind::Camera => 0.0,
            };
            mix_into(&mut self.mix, &self.slot_frame, gain);
        }
        // Mixed before the echo canceller reads its far end: a soundboard sound coming back
        // through the microphone is cancelled like a voice.
        self.sounds.mix_into(&mut self.mix);
        self.limiter.process(&mut self.mix);
    }

    fn add_monitored_microphone(&mut self) {
        let count = self.capture.monitored.len().min(SAMPLES_PER_FRAME);
        for (output, heard) in self
            .mix
            .iter_mut()
            .zip(self.capture.monitored.drain(..count))
        {
            *output = (*output + heard).clamp(-1.0, 1.0);
        }
    }
}

/// Queues a processed frame for "hear yourself", dropping the oldest beyond the bound.
fn keep_for_monitoring(monitored: &mut VecDeque<f32>, frame: &Frame) {
    monitored.extend(frame.iter().copied());
    let excess = monitored.len().saturating_sub(MAXIMUM_MONITORED_SAMPLES);
    monitored.drain(..excess);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc;

    use super::*;
    use crate::devices::{StreamFormat, SyntheticAudioDevices, SyntheticSound};
    use crate::processing::NoiseSuppressionMode;

    const TONE_AMPLITUDE: f32 = 0.3;

    fn speaker() -> Snowflake {
        Snowflake::from_raw(3)
    }

    /// The engine, never run: tests drive its steps by hand, so no device is opened.
    struct DrivenEngine {
        engine: AudioEngine,
        incoming: Sender<IncomingAudioPacket>,
        outgoing: Receiver<OutgoingVoicePacket>,
        voice_mid: Mid,
    }

    fn driven_engine() -> DrivenEngine {
        let format = StreamFormat {
            sample_rate: VOICE_SAMPLE_RATE,
            channels: 1,
        };
        let devices = SyntheticAudioDevices::new(SyntheticSound::Silence, format, format);
        let (_commands, command_receiver) = mpsc::channel();
        let (incoming, incoming_receiver) = mpsc::channel();
        let (outgoing, outgoing_receiver) = mpsc::channel();
        let (events, _event_receiver) = mpsc::channel();
        let voice_mid = Mid::from("v1");
        let settings = AudioSettings {
            is_muted: false,
            is_deafened: false,
            input_mode: InputMode::default(),
            // A steady tone is what these would remove or reshape.
            processing: ProcessingSettings {
                is_echo_cancellation_on: false,
                noise_suppression: NoiseSuppressionMode::Off,
                is_automatic_gain_on: false,
                ..ProcessingSettings::default()
            },
            output_volume: 1.0,
            soundboard_volume: 0.5,
            input_device_id: None,
            output_device_id: None,
            is_monitoring: false,
            is_microphone_test: false,
        };
        let engine = AudioEngine::new(
            Box::new(devices),
            AudioEngineChannels {
                commands: command_receiver,
                incoming: incoming_receiver,
                outgoing,
                events,
                microphone_level: Arc::default(),
            },
            settings,
            [(voice_mid, ReceiveSlotKind::Voice)].into_iter(),
        )
        .unwrap();
        DrivenEngine {
            engine,
            incoming,
            outgoing: outgoing_receiver,
            voice_mid,
        }
    }

    fn tone_frame(phase: &mut f32) -> Frame {
        let mut frame = [0.0; SAMPLES_PER_FRAME];
        for sample in &mut frame {
            *phase += std::f32::consts::TAU * 440.0 / 48_000.0;
            *sample = phase.sin() * TONE_AMPLITUDE;
        }
        frame
    }

    /// Voice packets sent for a second of microphone tone.
    fn voice_packets_sent(driven: &mut DrivenEngine) -> usize {
        let mut phase = 0.0;
        for _ in 0..100 {
            let mut frame = tone_frame(&mut phase);
            driven
                .engine
                .process_microphone_frame(&mut frame, 0)
                .unwrap();
        }
        driven
            .outgoing
            .try_iter()
            .filter(|packet| packet.is_voice)
            .count()
    }

    /// The loudest 10 ms played while a second of the speaker's tone arrives in order.
    fn loudest_played_frame(driven: &mut DrivenEngine) -> f32 {
        let mut encoder = VoiceEncoder::new().unwrap();
        let mut phase = 0.0;
        let mut loudest: f32 = 0.0;
        for index in 0..50_u64 {
            let mut packet = Vec::with_capacity(SAMPLES_PER_PACKET);
            packet.extend_from_slice(&tone_frame(&mut phase));
            packet.extend_from_slice(&tone_frame(&mut phase));
            if let Some(payload) = encoder.encode(&packet).unwrap() {
                driven
                    .incoming
                    .send(IncomingAudioPacket {
                        mid: driven.voice_mid,
                        rtp_timestamp: index * SAMPLES_PER_PACKET as u64,
                        arrival: driven.engine.clock_origin + Duration::from_millis(index * 20),
                        payload: Arc::from(payload),
                    })
                    .unwrap();
            }
            driven.engine.receive_packets();
            for _ in 0..2 {
                driven.engine.mix_one_frame();
                loudest = loudest.max(root_mean_square(&driven.engine.mix));
            }
        }
        loudest
    }

    #[test]
    fn the_microphone_is_sent_unless_muted_or_deafened() {
        assert!(voice_packets_sent(&mut driven_engine()) > 0);
        for command in [
            AudioCommand::SetMuted(true),
            AudioCommand::SetDeafened(true),
        ] {
            let mut driven = driven_engine();
            driven.engine.handle_command(command);
            assert_eq!(voice_packets_sent(&mut driven), 0);
        }
    }

    #[test]
    fn received_voices_are_heard_unless_deafened_or_muted_for_this_user() {
        let assign = |driven: &mut DrivenEngine| {
            let mid = driven.voice_mid;
            driven.engine.handle_command(AudioCommand::AssignSlot {
                mid,
                user_id: speaker(),
                setting: PersonVoiceSetting::default(),
            });
        };
        let mut driven = driven_engine();
        assign(&mut driven);
        assert!(loudest_played_frame(&mut driven) > TONE_AMPLITUDE / 2.0);
        let silencers = [
            AudioCommand::SetDeafened(true),
            AudioCommand::SetPersonSetting {
                source: PersonSoundSource::Voice,
                user_id: speaker(),
                setting: PersonVoiceSetting {
                    volume: 1.0,
                    is_locally_muted: true,
                },
            },
        ];
        for silencer in silencers {
            let mut driven = driven_engine();
            assign(&mut driven);
            driven.engine.handle_command(silencer);
            assert!(loudest_played_frame(&mut driven) < f32::EPSILON);
        }
    }

    #[test]
    fn a_person_s_volume_scales_their_voice_and_a_screen_setting_does_not() {
        let mut driven = driven_engine();
        let mid = driven.voice_mid;
        driven.engine.handle_command(AudioCommand::AssignSlot {
            mid,
            user_id: speaker(),
            setting: PersonVoiceSetting::default(),
        });
        let full = loudest_played_frame(&mut driven);
        let mut driven = driven_engine();
        driven.engine.handle_command(AudioCommand::AssignSlot {
            mid,
            user_id: speaker(),
            setting: PersonVoiceSetting::default(),
        });
        for source in [PersonSoundSource::ScreenSound, PersonSoundSource::Voice] {
            driven
                .engine
                .handle_command(AudioCommand::SetPersonSetting {
                    source,
                    user_id: speaker(),
                    setting: PersonVoiceSetting {
                        volume: if source == PersonSoundSource::Voice {
                            0.5
                        } else {
                            0.0
                        },
                        is_locally_muted: false,
                    },
                });
        }
        let halved = loudest_played_frame(&mut driven);
        assert!(
            (halved / full - 0.5).abs() < 0.05,
            "{halved} against {full}"
        );
    }

    #[test]
    fn cues_and_soundboard_sounds_are_mixed_into_the_output() {
        let mut driven = driven_engine();
        driven
            .engine
            .handle_command(AudioCommand::PlaySoundboardSound(
                vec![0.4; SAMPLES_PER_FRAME].into(),
                SoundboardPlayer::Preview,
            ));
        driven.engine.mix_one_frame();
        // Output volume 1 times soundboard volume 0.5.
        assert!((driven.engine.mix[100] - 0.2).abs() < 1e-3);
        driven
            .engine
            .handle_command(AudioCommand::PlayCue(VoiceCue::Mute));
        driven.engine.mix_one_frame();
        driven.engine.mix_one_frame();
        assert!(
            root_mean_square(&driven.engine.mix) > 0.001,
            "the cue plays"
        );
        for _ in 0..30 {
            driven.engine.mix_one_frame();
        }
        driven
            .engine
            .handle_command(AudioCommand::PlayNotificationChime);
        driven.engine.mix_one_frame();
        driven.engine.mix_one_frame();
        assert!(
            root_mean_square(&driven.engine.mix) > 0.001,
            "the chime plays"
        );
    }

    #[test]
    fn a_new_media_session_forgets_who_each_slot_carried() {
        let mut driven = driven_engine();
        let mid = driven.voice_mid;
        driven.engine.handle_command(AudioCommand::AssignSlot {
            mid,
            user_id: speaker(),
            setting: PersonVoiceSetting::default(),
        });
        let new_mid = Mid::from("v2");
        driven
            .engine
            .handle_command(AudioCommand::NewMediaSession(vec![(
                new_mid,
                ReceiveSlotKind::Voice,
            )]));
        assert_eq!(driven.engine.slots.len(), 1);
        assert_eq!(driven.engine.slots[0].mid, new_mid);
        assert_eq!(driven.engine.slots[0].user_id, None);
    }

    #[test]
    fn the_meter_reads_the_processed_microphone() {
        let mut driven = driven_engine();
        assert_eq!(driven.engine.microphone_level.decibels(), None);
        let mut phase = 0.0;
        let mut frame = tone_frame(&mut phase);
        driven
            .engine
            .process_microphone_frame(&mut frame, 0)
            .unwrap();
        let decibels = driven.engine.microphone_level.decibels().unwrap();
        // A sine's RMS is its amplitude over √2; the processing's filters take a little.
        let expected = 20.0 * (TONE_AMPLITUDE / std::f32::consts::SQRT_2).log10();
        assert!((decibels - expected).abs() < 1.5, "{decibels}");
    }

    #[test]
    fn hearing_yourself_plays_what_is_sent_and_nothing_once_muted() {
        let mut driven = driven_engine();
        driven
            .engine
            .handle_command(AudioCommand::SetMonitoring(true));
        let heard = |driven: &mut DrivenEngine| {
            let mut phase = 0.0;
            let mut frame = tone_frame(&mut phase);
            driven
                .engine
                .process_microphone_frame(&mut frame, 0)
                .unwrap();
            driven.engine.mix_one_frame();
            driven.engine.add_monitored_microphone();
            root_mean_square(&driven.engine.mix)
        };
        assert!(heard(&mut driven) > TONE_AMPLITUDE / 2.0);
        driven.engine.handle_command(AudioCommand::SetMuted(true));
        assert!(heard(&mut driven) < f32::EPSILON);
        driven
            .engine
            .handle_command(AudioCommand::SetMonitoring(false));
        driven.engine.handle_command(AudioCommand::SetMuted(false));
        assert!(heard(&mut driven) < f32::EPSILON);
    }

    #[test]
    fn the_microphone_test_sends_nothing_and_ignores_the_gate() {
        let mut driven = driven_engine();
        driven.engine.settings.is_microphone_test = true;
        driven.engine.handle_command(AudioCommand::SetMuted(true));
        driven
            .engine
            .handle_command(AudioCommand::SetMonitoring(true));
        assert_eq!(voice_packets_sent(&mut driven), 0);
        assert!(driven.engine.microphone_level.decibels().is_some());
        driven.engine.mix_one_frame();
        driven.engine.add_monitored_microphone();
        assert!(root_mean_square(&driven.engine.mix) > TONE_AMPLITUDE / 2.0);
    }

    #[test]
    fn hearing_yourself_waits_at_most_a_few_frames() {
        let mut monitored = VecDeque::new();
        for _ in 0..20 {
            keep_for_monitoring(&mut monitored, &[0.1; SAMPLES_PER_FRAME]);
        }
        assert_eq!(monitored.len(), MAXIMUM_MONITORED_SAMPLES);
    }
}
