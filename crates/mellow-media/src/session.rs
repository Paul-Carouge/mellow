//! A voice session: one channel joined through the gateway, with its network, audio and video
//! threads. The participant's one video source (a camera or a screen, never both) goes in
//! through [`VideoInput`]; the cameras and screens it watches come out through [`VideoFrames`].
//! Signalling goes through [`VoiceSignalling`] (frames out) and
//! [`VoiceSession::handle_gateway_event`] (events in), so the desktop wires it to the client
//! core's gateway and tests to a plain WebSocket. The session reconnects by itself as the web
//! client does: a fresh offer after a lost path, a gateway `READY` or `RESUMED`, or the network
//! coming back, and it says why it stopped.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use mellow_protocol::{
    ClientFrame, GatewayEvent, Snowflake, VideoTileSize, VoiceDisconnectReason, VoiceJoinRefusal,
    VoiceVideoRefusal, WatchedCamera,
};

use crate::audio_engine::{AudioCommand, AudioEngine, AudioEngineChannels, AudioSettings};
use crate::connection::{ConnectionInput, ConnectionMachine, ConnectionProblem, VoicePhase};
use crate::devices::AudioDevices;
use crate::input_gate::InputMode;
use crate::offer::OfferError;
use crate::person_settings::{PersonSoundSource, PersonVoiceSetting, PersonVoiceSettingsStore};
use crate::processing::ProcessingSettings;
use crate::simulcast::{ScreenShareQuality, VideoSourceKind};
use crate::sounds::{LocalSound, SoundboardPlayer, SoundboardSoundsToStop, VoiceCue};
use crate::transport::{NetworkCommand, NetworkThread, PeerConnection, PeerError, VideoLink};
use crate::uplink::UplinkLayerCeiling;
use crate::video_frame::DecodedPixelFormat;
use crate::video_receiver::{VideoFrames, VideoReceivers};
use crate::video_sender::{OutgoingVideoFrame, VideoEncoderControl, VideoInput};
use crate::watch_list::WatchListPacing;

/// Sends the session's gateway frames (`voice_join`, `voice_state`, `voice_leave`). Returns
/// whether the frame went out: an offer the gateway could not carry is sent again on its next
/// `READY` or `RESUMED`.
pub trait VoiceSignalling: Send {
    fn send_frame(&self, frame: ClientFrame) -> bool;
}

impl<F> VoiceSignalling for F
where
    F: Fn(ClientFrame) -> bool + Send,
{
    fn send_frame(&self, frame: ClientFrame) -> bool {
        self(frame)
    }
}

/// A microphone or an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioDirection {
    Input,
    Output,
}

/// What the session tells the interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceSessionEvent {
    /// Media flows for the first time in this channel: play the join cue.
    Connected,
    /// Connecting, connected, or reconnecting in silence.
    PhaseChanged(VoicePhase),
    /// The server refused the offer; the session has stopped.
    JoinRefused(VoiceJoinRefusal),
    /// The server ended the session; it has stopped.
    Disconnected(VoiceDisconnectReason),
    /// The media connection could not be made or brought back; the session has stopped (after
    /// telling the server).
    ConnectionFailed(ConnectionProblem),
    /// The round trip to the instance, every 2 seconds while connected (the voice panel's
    /// latency: [`crate::latency_quality`]).
    RoundTrip(Duration),
    /// The user's own speaking ring (the server never reports a user to themselves).
    SelfSpeaking(bool),
    /// A device could not be opened, or failed and could not be reopened.
    DeviceUnavailable(AudioDirection),
    /// The server refused the camera or screen, or took it away (`STREAM` lost): the source is
    /// off; stop its capture.
    VideoRefused(VoiceVideoRefusal),
    /// The server turned the camera or screen off (nothing came from it for too long): stop
    /// its capture.
    VideoStoppedByServer,
    /// This session's connection is too slow for `user_id`'s video even at its lowest layer
    /// (`is_paused`), or it flows again: show their avatar meanwhile.
    VideoPaused { user_id: Snowflake, is_paused: bool },
}

/// A camera or screen this session shows, and how large its tile is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchedVideo {
    pub user_id: Snowflake,
    /// Decides the layer the server sends (ADR-0035).
    pub tile_size: VideoTileSize,
    /// The tile's size in physical pixels: pictures arrive at most this large.
    pub display_width: u32,
    pub display_height: u32,
}

/// At most this round trip reads as good, then medium; above, poor (`web/src/lib/voice.ts`).
pub const GOOD_ROUND_TRIP: Duration = Duration::from_millis(100);
pub const MEDIUM_ROUND_TRIP: Duration = Duration::from_millis(250);

/// The latency indicator's three levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatencyQuality {
    Good,
    Medium,
    Poor,
}

#[must_use]
pub fn latency_quality(round_trip: Duration) -> LatencyQuality {
    if round_trip <= GOOD_ROUND_TRIP {
        LatencyQuality::Good
    } else if round_trip <= MEDIUM_ROUND_TRIP {
        LatencyQuality::Medium
    } else {
        LatencyQuality::Poor
    }
}
/// The user's mute and deafen, with the web client's rules (`web/src/lib/voice-toggles.ts`):
/// deafening also mutes, undeafening restores the microphone as it was, unmuting undeafens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VoiceToggles {
    is_muted: bool,
    is_deafened: bool,
    was_muted_before_deafening: bool,
}

impl VoiceToggles {
    /// The three fields as the web client stores them (`readVoiceToggles`): deafened always
    /// means muted, whatever was stored.
    #[must_use]
    pub fn new(is_muted: bool, is_deafened: bool, was_muted_before_deafening: bool) -> Self {
        Self {
            is_muted: is_muted || is_deafened,
            is_deafened,
            was_muted_before_deafening,
        }
    }

    #[must_use]
    pub fn is_muted(self) -> bool {
        self.is_muted
    }

    #[must_use]
    pub fn is_deafened(self) -> bool {
        self.is_deafened
    }

    /// The microphone's state when deafening started, restored when it ends; stored with the
    /// other two.
    #[must_use]
    pub fn was_muted_before_deafening(self) -> bool {
        self.was_muted_before_deafening
    }

    pub fn set_muted(&mut self, is_muted: bool) {
        if !is_muted && self.is_deafened {
            self.is_deafened = false;
        }
        self.is_muted = is_muted;
    }

    pub fn set_deafened(&mut self, is_deafened: bool) {
        if is_deafened && !self.is_deafened {
            self.was_muted_before_deafening = self.is_muted;
            self.is_muted = true;
        } else if !is_deafened && self.is_deafened {
            self.is_muted = self.was_muted_before_deafening;
        }
        self.is_deafened = is_deafened;
    }
}

/// How to join.
#[derive(Debug, Clone)]
pub struct VoiceSessionOptions {
    pub channel_id: Snowflake,
    /// The signed-in user: the server dropping their own voice state means offering again.
    pub own_user_id: Snowflake,
    /// The instance's IPv4 address (its media socket listens on IPv4 only): the local interface
    /// facing it becomes the ICE candidate.
    pub server_address: Ipv4Addr,
    pub toggles: VoiceToggles,
    pub input_mode: InputMode,
    pub processing: ProcessingSettings,
    /// 0 to 2 (200 %).
    pub output_volume: f32,
    /// 0 to 1 (100 %), on top of the output volume.
    pub soundboard_volume: f32,
    pub input_device_id: Option<String>,
    pub output_device_id: Option<String>,
    /// How received pictures are handed to the interface.
    pub decoded_pixel_format: DecodedPixelFormat,
}

impl VoiceSessionOptions {
    /// Default settings, unmuted.
    #[must_use]
    pub fn new(channel_id: Snowflake, own_user_id: Snowflake, server_address: Ipv4Addr) -> Self {
        Self {
            channel_id,
            own_user_id,
            server_address,
            toggles: VoiceToggles::default(),
            input_mode: InputMode::default(),
            processing: ProcessingSettings::default(),
            output_volume: 1.0,
            soundboard_volume: crate::soundboard::DEFAULT_SOUNDBOARD_VOLUME,
            input_device_id: None,
            output_device_id: None,
            decoded_pixel_format: DecodedPixelFormat::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VoiceSessionError {
    #[error("the voice socket could not be opened: {0}")]
    Socket(#[from] std::io::Error),
    #[error(transparent)]
    Offer(#[from] OfferError),
    #[error("the audio engine could not start: {0}")]
    Audio(String),
}

impl From<PeerError> for VoiceSessionError {
    fn from(error: PeerError) -> Self {
        match error {
            PeerError::Socket(error) => Self::Socket(error),
            PeerError::Offer(error) => Self::Offer(error),
        }
    }
}

/// One joined voice channel. Dropping it stops its threads (send `voice_leave` with
/// [`VoiceSession::leave`] to tell the server).
pub struct VoiceSession {
    channel_id: Snowflake,
    own_user_id: Snowflake,
    toggles: VoiceToggles,
    settings_store: Arc<dyn PersonVoiceSettingsStore>,
    audio_commands: Sender<AudioCommand>,
    network_commands: Sender<NetworkCommand>,
    is_path_dropped: Arc<AtomicBool>,
    video_encoder: Arc<VideoEncoderControl>,
    video_frames: VideoFrames,
    /// The last list sent to the network thread: only a change goes again.
    watched_cameras: Vec<WatchedCamera>,
    threads: Vec<JoinHandle<()>>,
}

/// The video encoding thread, idle until a source starts, and where its frames come out.
fn spawn_video_encoder(
    control: &Arc<VideoEncoderControl>,
) -> std::io::Result<(JoinHandle<()>, Receiver<OutgoingVideoFrame>)> {
    let (outgoing, outgoing_receiver) = mpsc::channel();
    let control = Arc::clone(control);
    let thread = std::thread::Builder::new()
        .name("mellow-video-encode".into())
        .spawn(move || control.run(&outgoing))?;
    Ok((thread, outgoing_receiver))
}

impl std::fmt::Debug for VoiceSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VoiceSession")
            .field("channel_id", &self.channel_id)
            .field("toggles", &self.toggles)
            .finish_non_exhaustive()
    }
}

/// Whether a voice session may read `event` (a `VoiceStateDelete` only when it is the session's
/// own user): a client forwards only these to [`VoiceSession::handle_gateway_event`].
#[must_use]
pub fn is_voice_session_event(event: &GatewayEvent) -> bool {
    matches!(
        event,
        GatewayEvent::VoiceSessionDescription { .. }
            | GatewayEvent::VoiceJoinRefused { .. }
            | GatewayEvent::VoiceSlotAssigned { .. }
            | GatewayEvent::VoiceVideoLayers { .. }
            | GatewayEvent::VoiceVideoRefused { .. }
            | GatewayEvent::VoiceVideoPaused { .. }
            | GatewayEvent::VoiceDisconnected { .. }
            | GatewayEvent::VoiceStateDelete { .. }
    )
}

impl VoiceSession {
    /// Opens the devices, makes the offer and sends `voice_join`. Events arrive on the returned
    /// receiver; feed the gateway's events to [`VoiceSession::handle_gateway_event`].
    ///
    /// # Errors
    /// The UDP socket, the offer or the audio engine could not be set up. A missing device is
    /// not an error: it is reported as an event and the session goes on without it.
    pub fn join(
        options: VoiceSessionOptions,
        devices: Box<dyn AudioDevices>,
        signalling: Box<dyn VoiceSignalling>,
        settings_store: Arc<dyn PersonVoiceSettingsStore>,
    ) -> Result<(Self, Receiver<VoiceSessionEvent>), VoiceSessionError> {
        let first_peer = PeerConnection::new(options.server_address, 1)?;
        let (events, event_receiver) = mpsc::channel();
        let (audio_commands, audio_command_receiver) = mpsc::channel();
        let (network_commands, network_command_receiver) = mpsc::channel();
        let (outgoing, outgoing_receiver) = mpsc::channel();
        let (incoming, incoming_receiver) = mpsc::channel();
        let settings = AudioSettings {
            is_muted: options.toggles.is_muted(),
            is_deafened: options.toggles.is_deafened(),
            input_mode: options.input_mode,
            processing: options.processing,
            output_volume: options.output_volume,
            soundboard_volume: options.soundboard_volume,
            input_device_id: options.input_device_id,
            output_device_id: options.output_device_id,
        };
        let (started, has_started) = mpsc::channel();
        let audio_channels = AudioEngineChannels {
            commands: audio_command_receiver,
            incoming: incoming_receiver,
            outgoing,
            events: events.clone(),
        };
        // The engine is built on its own thread: its codecs and device streams never move. Its
        // slots come with the first offer's `NewMediaSession`.
        let audio_thread = std::thread::Builder::new()
            .name("mellow-voice-audio".into())
            .spawn(move || {
                match AudioEngine::new(devices, audio_channels, settings, std::iter::empty()) {
                    Ok(engine) => {
                        let _ = started.send(Ok(()));
                        engine.run();
                    }
                    Err(error) => {
                        let _ = started.send(Err(error.to_string()));
                    }
                }
            })?;
        match has_started.recv() {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => return Err(VoiceSessionError::Audio(reason)),
            Err(_) => return Err(VoiceSessionError::Audio("the audio thread ended".into())),
        }
        let is_path_dropped = Arc::new(AtomicBool::new(false));
        let video_encoder = Arc::new(VideoEncoderControl::default());
        let video_frames = VideoFrames::new(options.decoded_pixel_format);
        let (video_thread, outgoing_video) = spawn_video_encoder(&video_encoder)?;
        let (keyframe_requests, keyframe_request_receiver) = mpsc::channel();
        let video = VideoLink {
            encoder: Arc::clone(&video_encoder),
            outgoing: outgoing_video,
            receivers: VideoReceivers::new(video_frames.clone(), keyframe_requests),
            keyframe_requests: keyframe_request_receiver,
            keyframe_requested_at: HashMap::new(),
            source: None,
            watched_cameras: Vec::new(),
            watch_list_pacing: WatchListPacing::default(),
            uplink: UplinkLayerCeiling::default(),
        };
        let network = NetworkThread {
            machine: ConnectionMachine::new(options.channel_id),
            peer: Some(first_peer),
            server_address: options.server_address,
            own_user_id: options.own_user_id,
            toggles: options.toggles,
            signalling,
            settings_store: settings_store.clone(),
            commands: network_command_receiver,
            outgoing: outgoing_receiver,
            incoming,
            events,
            audio_commands: audio_commands.clone(),
            video,
            is_path_dropped: is_path_dropped.clone(),
        };
        let network_thread = std::thread::Builder::new()
            .name("mellow-voice-network".into())
            .spawn(move || network.run())?;
        let session = Self {
            channel_id: options.channel_id,
            own_user_id: options.own_user_id,
            toggles: options.toggles,
            settings_store,
            audio_commands,
            network_commands,
            is_path_dropped,
            video_encoder,
            video_frames,
            watched_cameras: Vec::new(),
            threads: vec![audio_thread, network_thread, video_thread],
        };
        Ok((session, event_receiver))
    }

    #[must_use]
    pub fn channel_id(&self) -> Snowflake {
        self.channel_id
    }

    #[must_use]
    pub fn toggles(&self) -> VoiceToggles {
        self.toggles
    }

    /// Passes on the gateway's voice events; ignores the others without copying them.
    pub fn handle_gateway_event(&self, event: &GatewayEvent) {
        let is_for_voice = match event {
            GatewayEvent::VoiceStateDelete { user_id, .. } => *user_id == self.own_user_id,
            _ => is_voice_session_event(event),
        };
        if is_for_voice {
            self.network(NetworkCommand::Gateway(Box::new(event.clone())));
        }
    }

    /// The gateway identified again (a new session): it knows nothing of this voice.
    pub fn note_gateway_ready(&self) {
        self.network(NetworkCommand::Connection(ConnectionInput::GatewayReady));
    }

    /// The gateway resumed: voice is offered again unless media still flows.
    pub fn note_gateway_resumed(&self) {
        self.network(NetworkCommand::Connection(ConnectionInput::GatewayResumed));
    }

    /// The system is back online (a network change): voice is offered again unless media
    /// still flows.
    pub fn note_back_online(&self) {
        self.network(NetworkCommand::Connection(ConnectionInput::BackOnline));
    }

    /// Joins another channel with the same microphone: a new offer, announced as a move.
    pub fn move_to(&mut self, channel_id: Snowflake) {
        self.channel_id = channel_id;
        self.network(NetworkCommand::Connection(ConnectionInput::MoveTo {
            channel_id,
        }));
    }

    pub fn set_self_muted(&mut self, is_muted: bool) {
        self.toggles.set_muted(is_muted);
        self.apply_toggles();
    }

    pub fn set_self_deafened(&mut self, is_deafened: bool) {
        self.toggles.set_deafened(is_deafened);
        self.apply_toggles();
    }

    fn apply_toggles(&self) {
        self.command(AudioCommand::SetMuted(self.toggles.is_muted()));
        self.command(AudioCommand::SetDeafened(self.toggles.is_deafened()));
        self.network(NetworkCommand::SetToggles(self.toggles));
    }

    /// Turns the camera or a screen on, replacing the other (never both), and tells the
    /// server; capture pushes its pictures into the returned input. The server answers with
    /// [`VoiceSessionEvent::VideoRefused`] when it does not allow it.
    #[must_use]
    pub fn start_video(&self, kind: VideoSourceKind) -> VideoInput {
        let input = self.video_encoder.start(kind);
        self.network(NetworkCommand::SetVideoSource(Some(kind)));
        input
    }

    /// A new quality for the shared screen: only bit rates change, as on the web.
    pub fn change_screen_share_quality(&self, quality: ScreenShareQuality) {
        if matches!(self.video_source(), Some(VideoSourceKind::Screen(_))) {
            let kind = VideoSourceKind::Screen(quality);
            self.video_encoder.change_kind(kind);
            self.network(NetworkCommand::SetVideoSource(Some(kind)));
        }
    }

    /// Turns the camera or screen off and tells the server; its input stops taking pictures.
    pub fn stop_video(&self) {
        if self.video_source().is_some() {
            self.video_encoder.stop_source();
            self.network(NetworkCommand::SetVideoSource(None));
        }
    }

    /// The source on, if any (the server may have turned it off: see the session's events).
    #[must_use]
    pub fn video_source(&self) -> Option<VideoSourceKind> {
        self.video_encoder.source()
    }

    /// The cameras and screens to show (at most `MAXIMUM_VIDEO_RECEIVE_SLOTS`, the rest is
    /// ignored), replacing the previous list. Only a change of people or tile sizes reaches the
    /// server, at most every 400 ms (the latest change always arrives); drawn sizes apply at
    /// once.
    pub fn watch_videos(&mut self, watched: &[WatchedVideo]) {
        let watched = &watched[..watched
            .len()
            .min(mellow_protocol::MAXIMUM_VIDEO_RECEIVE_SLOTS)];
        self.video_frames.set_display_sizes(
            watched
                .iter()
                .map(|video| {
                    (
                        video.user_id,
                        (video.display_width.max(1), video.display_height.max(1)),
                    )
                })
                .collect::<HashMap<_, _>>(),
        );
        let cameras: Vec<WatchedCamera> = watched
            .iter()
            .map(|video| WatchedCamera {
                user_id: video.user_id,
                size: video.tile_size,
            })
            .collect();
        if cameras != self.watched_cameras {
            self.watched_cameras.clone_from(&cameras);
            self.network(NetworkCommand::WatchCameras(cameras));
        }
    }

    /// The decoded pictures of the watched cameras and screens, for the interface.
    #[must_use]
    pub fn video_frames(&self) -> VideoFrames {
        self.video_frames.clone()
    }

    /// One person's volume and local mute, for their voice or their screen's sound: applied at
    /// once and remembered in the settings store.
    pub fn set_person_setting(
        &self,
        source: PersonSoundSource,
        user_id: Snowflake,
        setting: PersonVoiceSetting,
    ) {
        let setting = setting.clamped();
        self.settings_store
            .write_person_setting(source, user_id, setting);
        self.command(AudioCommand::SetPersonSetting {
            source,
            user_id,
            setting,
        });
    }

    /// 0 to 2 (200 %).
    pub fn set_output_volume(&self, volume: f32) {
        self.command(AudioCommand::SetOutputVolume(volume));
    }

    /// 0 to 1 (100 %), for soundboard and join sounds.
    pub fn set_soundboard_volume(&self, volume: f32) {
        self.command(AudioCommand::SetSoundboardVolume(volume));
    }

    pub fn set_input_mode(&self, mode: InputMode) {
        self.command(AudioCommand::SetInputMode(mode));
    }

    /// The push-to-talk key, read by the desktop (a global shortcut).
    pub fn set_push_to_talk_pressed(&self, is_pressed: bool) {
        self.command(AudioCommand::SetPushToTalkPressed(is_pressed));
    }

    pub fn set_processing(&self, processing: ProcessingSettings) {
        self.command(AudioCommand::SetProcessing(processing));
    }

    /// Switches the microphone; `None` follows the system's default.
    pub fn set_input_device(&self, device_id: Option<String>) {
        self.command(AudioCommand::SetInputDevice(device_id));
    }

    /// Switches the output; `None` follows the system's default.
    pub fn set_output_device(&self, device_id: Option<String>) {
        self.command(AudioCommand::SetOutputDevice(device_id));
    }

    /// Plays a cue on the voice output, where the echo canceller hears it.
    pub fn play_cue(&self, cue: VoiceCue) {
        self.command(AudioCommand::PlayCue(cue));
    }

    /// Plays the message chime on the voice output, where the echo canceller hears it.
    pub fn play_notification_chime(&self) {
        self.command(AudioCommand::PlayNotificationChime);
    }

    /// Plays a soundboard or join sound at the output volume times the soundboard volume; who
    /// should hear it is
    /// [`mellow_client_core::shared_logic::soundboard::should_hear_sound`]'s call.
    pub fn play_soundboard_sound(&self, sound: LocalSound, player: SoundboardPlayer) {
        self.command(AudioCommand::PlaySoundboardSound(sound, player));
    }

    /// Stops soundboard sounds before their end: when the reader deafens, mutes a player for
    /// themself, or stops a preview.
    pub fn stop_soundboard_sounds(&self, which: SoundboardSoundsToStop) {
        self.command(AudioCommand::StopSoundboardSounds(which));
    }

    /// Loses the current network path, as a Wi-Fi drop would: the session must notice and
    /// reconnect on a new one. For tests only.
    #[doc(hidden)]
    pub fn drop_network_path_for_tests(&self) {
        self.is_path_dropped.store(true, Ordering::Relaxed);
    }

    /// Tells the server and stops.
    pub fn leave(self) {
        self.network(NetworkCommand::Leave);
    }

    fn command(&self, command: AudioCommand) {
        // A stopped audio thread means the session already ended and said so.
        let _ = self.audio_commands.send(command);
    }

    fn network(&self, command: NetworkCommand) {
        // A stopped network thread means the session already ended and said so.
        let _ = self.network_commands.send(command);
    }
}

impl Drop for VoiceSession {
    fn drop(&mut self) {
        let _ = self.network_commands.send(NetworkCommand::Stop);
        let _ = self.audio_commands.send(AudioCommand::Stop);
        self.video_encoder.shut_down();
        for thread in self.threads.drain(..) {
            if thread.join().is_err() {
                tracing::error!("a voice thread panicked");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::person_settings::InMemoryPersonVoiceSettings;

    #[test]
    fn deafening_mutes_and_undeafening_restores_the_microphone() {
        let mut toggles = VoiceToggles::new(false, false, false);
        toggles.set_deafened(true);
        assert!(toggles.is_muted() && toggles.is_deafened());
        toggles.set_deafened(false);
        assert!(!toggles.is_muted() && !toggles.is_deafened());

        toggles.set_muted(true);
        toggles.set_deafened(true);
        toggles.set_deafened(false);
        assert!(toggles.is_muted(), "was muted before deafening");
    }

    #[test]
    fn unmuting_while_deafened_undeafens() {
        let mut toggles = VoiceToggles::new(false, true, false);
        assert!(toggles.is_muted());
        toggles.set_muted(false);
        assert!(!toggles.is_muted() && !toggles.is_deafened());
    }

    #[test]
    fn stored_toggles_restore_the_microphone_state_from_before_deafening() {
        let mut toggles = VoiceToggles::new(true, true, false);
        assert!(!toggles.was_muted_before_deafening());
        toggles.set_deafened(false);
        assert!(
            !toggles.is_muted(),
            "unmuted before deafening, unmuted after"
        );
    }

    /// A session without its threads, and the ends of its channels.
    fn session_without_threads() -> (
        VoiceSession,
        Receiver<AudioCommand>,
        Receiver<NetworkCommand>,
        Arc<InMemoryPersonVoiceSettings>,
    ) {
        let (audio_commands, audio_command_receiver) = mpsc::channel();
        let (network_commands, network_command_receiver) = mpsc::channel();
        let store = Arc::new(InMemoryPersonVoiceSettings::default());
        let session = VoiceSession {
            channel_id: Snowflake::from_raw(1),
            own_user_id: Snowflake::from_raw(5),
            toggles: VoiceToggles::default(),
            settings_store: store.clone(),
            audio_commands,
            network_commands,
            is_path_dropped: Arc::new(AtomicBool::new(false)),
            video_encoder: Arc::new(VideoEncoderControl::default()),
            video_frames: VideoFrames::new(DecodedPixelFormat::Bgra),
            watched_cameras: Vec::new(),
            threads: Vec::new(),
        };
        (
            session,
            audio_command_receiver,
            network_command_receiver,
            store,
        )
    }

    #[test]
    fn only_voice_events_and_the_user_s_own_departure_reach_the_network_thread() {
        let (session, _audio, network, _store) = session_without_threads();
        let voice_events = [
            GatewayEvent::VoiceDisconnected {
                channel_id: Snowflake::from_raw(1),
                reason: VoiceDisconnectReason::ChannelDeleted,
            },
            GatewayEvent::VoiceStateDelete {
                guild_id: Snowflake::from_raw(9),
                channel_id: Snowflake::from_raw(1),
                user_id: Snowflake::from_raw(5),
            },
        ];
        let other_events = [
            GatewayEvent::VoiceStateDelete {
                guild_id: Snowflake::from_raw(9),
                channel_id: Snowflake::from_raw(1),
                user_id: Snowflake::from_raw(6),
            },
            GatewayEvent::TypingStart {
                guild_id: None,
                channel_id: Snowflake::from_raw(1),
                user_id: Snowflake::from_raw(6),
            },
        ];
        for event in voice_events.iter().chain(&other_events) {
            session.handle_gateway_event(event);
        }
        let forwarded: Vec<NetworkCommand> = network.try_iter().collect();
        assert_eq!(forwarded.len(), voice_events.len());
        assert!(forwarded.iter().zip(&voice_events).all(
            |(command, event)| matches!(command, NetworkCommand::Gateway(sent) if **sent == *event)
        ));
    }

    #[test]
    fn toggles_reach_the_audio_and_network_threads() {
        let (mut session, audio, network, _store) = session_without_threads();
        session.set_self_deafened(true);
        assert!(matches!(audio.try_recv(), Ok(AudioCommand::SetMuted(true))));
        assert!(matches!(
            audio.try_recv(),
            Ok(AudioCommand::SetDeafened(true))
        ));
        assert!(matches!(
            network.try_recv(),
            Ok(NetworkCommand::SetToggles(toggles)) if toggles.is_deafened()
        ));
    }

    #[test]
    fn a_person_setting_is_applied_and_remembered() {
        let (session, audio, _network, store) = session_without_threads();
        let user_id = Snowflake::from_raw(8);
        let setting = PersonVoiceSetting {
            volume: 3.0,
            is_locally_muted: true,
        };
        session.set_person_setting(PersonSoundSource::Voice, user_id, setting);
        let remembered = store.person_setting(PersonSoundSource::Voice, user_id);
        assert!(remembered.is_locally_muted && (remembered.volume - 2.0).abs() < f32::EPSILON);
        assert!(matches!(
            audio.try_recv(),
            Ok(AudioCommand::SetPersonSetting { setting, .. }) if setting == remembered
        ));
    }

    #[test]
    fn a_screen_replaces_the_camera_and_the_network_thread_hears_of_each() {
        let (session, _audio, network, _store) = session_without_threads();
        let camera = session.start_video(VideoSourceKind::Camera);
        let screen = session.start_video(VideoSourceKind::Screen(ScreenShareQuality::High));
        assert!(!camera.is_open() && screen.is_open(), "never both");
        session.stop_video();
        assert!(!screen.is_open() && session.video_source().is_none());
        let sources: Vec<Option<VideoSourceKind>> = network
            .try_iter()
            .filter_map(|command| match command {
                NetworkCommand::SetVideoSource(kind) => Some(kind),
                _ => None,
            })
            .collect();
        assert_eq!(
            sources,
            [
                Some(VideoSourceKind::Camera),
                Some(VideoSourceKind::Screen(ScreenShareQuality::High)),
                None
            ]
        );
    }

    #[test]
    fn only_a_change_of_people_or_tile_sizes_sends_a_watch_list() {
        let (mut session, _audio, network, _store) = session_without_threads();
        let watched = WatchedVideo {
            user_id: Snowflake::from_raw(7),
            tile_size: VideoTileSize::Large,
            display_width: 1_280,
            display_height: 720,
        };
        session.watch_videos(&[watched]);
        session.watch_videos(&[WatchedVideo {
            display_width: 640,
            ..watched
        }]);
        assert_eq!(network.try_iter().count(), 1, "only the drawn size changed");
        session.watch_videos(&[WatchedVideo {
            tile_size: VideoTileSize::Small,
            ..watched
        }]);
        assert_eq!(network.try_iter().count(), 1);
    }

    #[test]
    fn latency_reads_as_the_web_s_three_levels() {
        assert_eq!(
            latency_quality(Duration::from_millis(100)),
            LatencyQuality::Good
        );
        assert_eq!(
            latency_quality(Duration::from_millis(101)),
            LatencyQuality::Medium
        );
        assert_eq!(
            latency_quality(Duration::from_millis(250)),
            LatencyQuality::Medium
        );
        assert_eq!(
            latency_quality(Duration::from_millis(251)),
            LatencyQuality::Poor
        );
    }
}
