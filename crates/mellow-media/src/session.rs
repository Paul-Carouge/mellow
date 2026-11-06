//! A voice session: one channel joined through the gateway, with its network, audio and video
//! threads. The participant's one video source (a camera or a screen, never both) goes in through
//! [`VideoInput`]; the cameras and screens it watches come out through [`VideoFrames`].
//! Signalling goes through [`VoiceSignalling`] (frames out) and
//! [`VoiceSession::handle_gateway_event`] (events in), so the desktop wires it to the client
//! core's gateway and tests to a plain WebSocket.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mellow_protocol::{
    ClientFrame, GatewayEvent, Snowflake, VideoTileSize, VoiceDisconnectReason, VoiceJoinRefusal,
    VoiceVideoRefusal, WatchedCamera,
};
use str0m::change::SdpAnswer;
use str0m::media::Mid;

use crate::audio_engine::{AudioCommand, AudioEngine, AudioEngineChannels, AudioSettings};
use crate::devices::AudioDevices;
use crate::input_gate::InputMode;
use crate::offer::{MediaLines, OfferError, ReceiveSlotKind, new_peer};
use crate::processing::ProcessingSettings;
use crate::simulcast::{ScreenShareQuality, VideoSourceKind};
use crate::transport::{Transport, TransportCommand};
use crate::uplink::UplinkLayerCeiling;
use crate::video_frame::DecodedPixelFormat;
use crate::video_receiver::{VideoFrames, VideoReceivers};
use crate::video_sender::{OutgoingVideoFrame, VideoEncoderControl, VideoInput};

/// An empty `VOICE_VIDEO_LAYERS` this soon after a source started is the previous source's
/// ending, not this one's (the web's `VIDEO_RESTART_GRACE_MILLISECONDS`).
const VIDEO_RESTART_GRACE: Duration = Duration::from_secs(2);

/// Sends the session's gateway frames (`voice_join`, `voice_state`, `voice_leave`).
pub trait VoiceSignalling: Send {
    fn send_frame(&self, frame: ClientFrame);
}

impl<F> VoiceSignalling for F
where
    F: Fn(ClientFrame) + Send,
{
    fn send_frame(&self, frame: ClientFrame) {
        self(frame);
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
    /// Media flows: play the join cue.
    Connected,
    JoinRefused(VoiceJoinRefusal),
    /// The server ended the session; it has stopped.
    Disconnected(VoiceDisconnectReason),
    /// The media connection failed or stopped answering; its threads have stopped (the microphone
    /// is closed). Drop the session or call [`VoiceSession::leave`]; rejoining makes a new
    /// session.
    MediaConnectionLost,
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
    VideoPaused {
        user_id: Snowflake,
        is_paused: bool,
    },
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
    /// The instance's IPv4 address (its media socket listens on IPv4 only): the local interface
    /// facing it becomes the ICE candidate.
    pub server_address: Ipv4Addr,
    pub toggles: VoiceToggles,
    pub input_mode: InputMode,
    pub processing: ProcessingSettings,
    /// 0 to 2 (200 %).
    pub output_volume: f32,
    pub input_device_id: Option<String>,
    pub output_device_id: Option<String>,
    /// How received pictures are handed to the interface.
    pub decoded_pixel_format: DecodedPixelFormat,
}

impl VoiceSessionOptions {
    /// Default settings, unmuted.
    #[must_use]
    pub fn new(channel_id: Snowflake, server_address: Ipv4Addr) -> Self {
        Self {
            channel_id,
            server_address,
            toggles: VoiceToggles::default(),
            input_mode: InputMode::default(),
            processing: ProcessingSettings::default(),
            output_volume: 1.0,
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

/// One joined voice channel. Dropping it stops its threads (send `voice_leave` with
/// [`VoiceSession::leave`] to tell the server).
pub struct VoiceSession {
    channel_id: Snowflake,
    offer_id: u32,
    toggles: VoiceToggles,
    signalling: Box<dyn VoiceSignalling>,
    media_lines: MediaLines,
    events: Sender<VoiceSessionEvent>,
    audio_commands: Sender<AudioCommand>,
    transport_commands: Sender<TransportCommand>,
    video_encoder: Arc<VideoEncoderControl>,
    video_frames: VideoFrames,
    /// The source on, and when it started.
    video_source: Option<(VideoSourceKind, Instant)>,
    /// The last `voice_video_watch` list, sent again after each answer (a join forgets it).
    watched_cameras: Vec<WatchedCamera>,
    threads: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for VoiceSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VoiceSession")
            .field("channel_id", &self.channel_id)
            .field("offer_id", &self.offer_id)
            .field("toggles", &self.toggles)
            .finish_non_exhaustive()
    }
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

/// The local address the system routes to `server` from, found without sending anything.
fn local_address_towards(server: Ipv4Addr) -> std::io::Result<IpAddr> {
    let probe = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    // Port 9 (discard): connecting a UDP socket only picks the route.
    probe.connect((server, 9))?;
    Ok(probe.local_addr()?.ip())
}

impl VoiceSession {
    /// Opens the devices, makes the offer and sends `voice_join` (offer 1). Events arrive on the
    /// returned receiver; feed the gateway's events to [`VoiceSession::handle_gateway_event`].
    ///
    /// # Errors
    /// The UDP socket, the offer or the audio engine could not be set up. A missing device is
    /// not an error: it is reported as an event and the session goes on without it.
    pub fn join(
        options: VoiceSessionOptions,
        devices: Box<dyn AudioDevices>,
        signalling: Box<dyn VoiceSignalling>,
    ) -> Result<(Self, Receiver<VoiceSessionEvent>), VoiceSessionError> {
        // Bound to every interface so the system picks the source per destination (the server
        // may answer from another of its addresses); only the candidate names one interface.
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        let local_candidate = SocketAddr::new(
            local_address_towards(options.server_address)?,
            socket.local_addr()?.port(),
        );
        let peer = new_peer(local_candidate, Instant::now())?;
        let (events, event_receiver) = mpsc::channel();
        let (audio_commands, audio_command_receiver) = mpsc::channel();
        let (transport_commands, transport_command_receiver) = mpsc::channel();
        let (outgoing, outgoing_receiver) = mpsc::channel();
        let (incoming, incoming_receiver) = mpsc::channel();
        let settings = AudioSettings {
            is_muted: options.toggles.is_muted(),
            is_deafened: options.toggles.is_deafened(),
            input_mode: options.input_mode,
            processing: options.processing,
            output_volume: options.output_volume,
            input_device_id: options.input_device_id,
            output_device_id: options.output_device_id,
        };
        let receive_slots: Vec<(Mid, ReceiveSlotKind)> = peer.media_lines.audio_receive().collect();
        let (started, has_started) = mpsc::channel();
        let audio_channels = AudioEngineChannels {
            commands: audio_command_receiver,
            incoming: incoming_receiver,
            outgoing,
            events: events.clone(),
        };
        // The engine is built on its own thread: its codecs and device streams never move.
        let audio_thread = std::thread::Builder::new()
            .name("mellow-voice-audio".into())
            .spawn(move || {
                match AudioEngine::new(devices, audio_channels, settings, receive_slots.into_iter())
                {
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
        let media_lines = peer.media_lines.clone();
        let sdp_offer = peer.sdp_offer;
        let video_encoder = Arc::new(VideoEncoderControl::default());
        let video_frames = VideoFrames::new(options.decoded_pixel_format);
        let (video_thread, outgoing_video_receiver) = spawn_video_encoder(&video_encoder)?;
        let (keyframe_requests, keyframe_request_receiver) = mpsc::channel();
        let transport = Transport {
            rtc: peer.rtc,
            pending_offer: Some(peer.pending),
            socket,
            local_candidate,
            media_lines: peer.media_lines,
            commands: transport_command_receiver,
            outgoing: outgoing_receiver,
            incoming,
            events: events.clone(),
            audio_commands: audio_commands.clone(),
            outgoing_video: outgoing_video_receiver,
            video_encoder: Arc::clone(&video_encoder),
            video_receivers: VideoReceivers::new(video_frames.clone(), keyframe_requests),
            keyframe_requests: keyframe_request_receiver,
            uplink: UplinkLayerCeiling::default(),
        };
        let transport_thread = std::thread::Builder::new()
            .name("mellow-voice-network".into())
            .spawn(move || transport.run())?;
        let session = Self {
            channel_id: options.channel_id,
            offer_id: 1,
            toggles: options.toggles,
            signalling,
            media_lines,
            events,
            audio_commands,
            transport_commands,
            video_encoder,
            video_frames,
            video_source: None,
            watched_cameras: Vec::new(),
            threads: vec![audio_thread, transport_thread, video_thread],
        };
        session.signalling.send_frame(ClientFrame::VoiceJoin {
            channel_id: session.channel_id,
            offer_id: session.offer_id,
            sdp_offer,
            self_mute: session.toggles.is_muted(),
            self_deaf: session.toggles.is_deafened(),
        });
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

    /// Applies the gateway events meant for this session; ignores the others.
    pub fn handle_gateway_event(&mut self, event: &GatewayEvent) {
        match event {
            GatewayEvent::VoiceSessionDescription {
                channel_id,
                offer_id,
                sdp_answer,
            } if *channel_id == self.channel_id && *offer_id == self.offer_id => {
                match SdpAnswer::from_sdp_string(sdp_answer) {
                    Ok(answer) => {
                        let _ = self
                            .transport_commands
                            .send(TransportCommand::ApplyAnswer(Box::new(answer)));
                        // The join forgot any earlier list.
                        if !self.watched_cameras.is_empty() {
                            self.send_watched_cameras();
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the server's voice answer does not parse");
                        self.stop_with(VoiceSessionEvent::MediaConnectionLost);
                    }
                }
            }
            GatewayEvent::VoiceJoinRefused {
                channel_id,
                offer_id,
                reason,
            } if *channel_id == self.channel_id && *offer_id == self.offer_id => {
                self.stop_with(VoiceSessionEvent::JoinRefused(*reason));
            }
            GatewayEvent::VoiceSlotAssigned {
                channel_id,
                mid,
                user_id,
            } if *channel_id == self.channel_id => {
                let mid = Mid::from(mid.as_str());
                match self.media_lines.receive_slot_kind(mid) {
                    Some(ReceiveSlotKind::Voice | ReceiveSlotKind::ScreenSound) => {
                        self.command(AudioCommand::AssignSlot {
                            mid,
                            user_id: *user_id,
                        });
                    }
                    Some(ReceiveSlotKind::Camera) => self.video_frames.assign_slot(mid, *user_id),
                    None => {}
                }
            }
            GatewayEvent::VoiceVideoLayers { channel_id, active }
                if *channel_id == self.channel_id =>
            {
                let Some((_, started_at)) = self.video_source else {
                    return;
                };
                if !active.is_empty() {
                    self.video_encoder.set_active_layers(active.clone());
                } else if started_at.elapsed() >= VIDEO_RESTART_GRACE {
                    self.end_video();
                    let _ = self.events.send(VoiceSessionEvent::VideoStoppedByServer);
                }
            }
            GatewayEvent::VoiceVideoRefused { channel_id, reason }
                if *channel_id == self.channel_id =>
            {
                self.end_video();
                let _ = self.events.send(VoiceSessionEvent::VideoRefused(*reason));
            }
            GatewayEvent::VoiceVideoPaused {
                channel_id,
                user_id,
                is_paused,
            } if *channel_id == self.channel_id => {
                let _ = self.events.send(VoiceSessionEvent::VideoPaused {
                    user_id: *user_id,
                    is_paused: *is_paused,
                });
            }
            GatewayEvent::VoiceDisconnected { channel_id, reason }
                if *channel_id == self.channel_id =>
            {
                self.stop_with(VoiceSessionEvent::Disconnected(*reason));
            }
            _ => {}
        }
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
        self.send_voice_state();
    }

    /// Always the full state: the server drops `voice_state` frames over its rate limit.
    fn send_voice_state(&self) {
        let source = self.video_source.map(|(kind, _)| kind);
        self.signalling.send_frame(ClientFrame::VoiceState {
            self_mute: self.toggles.is_muted(),
            self_deaf: self.toggles.is_deafened(),
            self_video: source.is_some(),
            self_stream: matches!(source, Some(VideoSourceKind::Screen(_))),
        });
    }

    /// Turns the camera or a screen on, replacing the other (never both), and tells the
    /// server; capture pushes its pictures into the returned input. Call it once
    /// [`VoiceSessionEvent::Connected`] came: a join starts with video off. The server answers
    /// with [`VoiceSessionEvent::VideoRefused`] when it does not allow it.
    pub fn start_video(&mut self, kind: VideoSourceKind) -> VideoInput {
        let input = self.video_encoder.start(kind);
        self.video_source = Some((kind, Instant::now()));
        self.send_voice_state();
        input
    }

    /// A new quality for the shared screen: only bit rates change, as on the web.
    pub fn change_screen_share_quality(&mut self, quality: ScreenShareQuality) {
        if let Some((VideoSourceKind::Screen(_), started_at)) = self.video_source {
            let kind = VideoSourceKind::Screen(quality);
            self.video_source = Some((kind, started_at));
            self.video_encoder.change_kind(kind);
        }
    }

    /// Turns the camera or screen off and tells the server; its input stops taking pictures.
    pub fn stop_video(&mut self) {
        if self.video_source.is_some() {
            self.end_video();
            self.send_voice_state();
        }
    }

    /// The source on, if any.
    #[must_use]
    pub fn video_source(&self) -> Option<VideoSourceKind> {
        self.video_source.map(|(kind, _)| kind)
    }

    fn end_video(&mut self) {
        self.video_source = None;
        self.video_encoder.stop_source();
    }

    /// The cameras and screens to show (at most `MAXIMUM_VIDEO_RECEIVE_SLOTS`, the rest is
    /// ignored), replacing the previous list. Only a change of people or tile sizes reaches the
    /// server; drawn sizes apply at once.
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
            self.watched_cameras = cameras;
            self.send_watched_cameras();
        }
    }

    fn send_watched_cameras(&self) {
        self.signalling.send_frame(ClientFrame::VoiceVideoWatch {
            cameras: self.watched_cameras.clone(),
        });
    }

    /// The decoded pictures of the watched cameras and screens, for the interface.
    #[must_use]
    pub fn video_frames(&self) -> VideoFrames {
        self.video_frames.clone()
    }

    /// 0 to 2 (200 %) for one person's voice.
    pub fn set_person_volume(&self, user_id: Snowflake, volume: f32) {
        self.command(AudioCommand::SetPersonVolume { user_id, volume });
    }

    /// Silences one person for this user only.
    pub fn set_person_locally_muted(&self, user_id: Snowflake, is_muted: bool) {
        self.command(AudioCommand::SetPersonLocallyMuted { user_id, is_muted });
    }

    /// 0 to 2 (200 %) for the sound of one person's shared screen.
    pub fn set_stream_volume(&self, user_id: Snowflake, volume: f32) {
        self.command(AudioCommand::SetStreamVolume { user_id, volume });
    }

    pub fn set_output_volume(&self, volume: f32) {
        self.command(AudioCommand::SetOutputVolume(volume));
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

    /// Tells the server and stops.
    pub fn leave(self) {
        self.signalling.send_frame(ClientFrame::VoiceLeave);
    }

    fn command(&self, command: AudioCommand) {
        // A stopped audio thread means the session already ended and said so.
        let _ = self.audio_commands.send(command);
    }

    fn stop_with(&mut self, event: VoiceSessionEvent) {
        self.stop_threads();
        let _ = self.events.send(event);
    }

    fn stop_threads(&mut self) {
        let _ = self.audio_commands.send(AudioCommand::Stop);
        let _ = self.transport_commands.send(TransportCommand::Stop);
        self.video_encoder.shut_down();
        for thread in self.threads.drain(..) {
            if thread.join().is_err() {
                tracing::error!("a voice thread panicked");
            }
        }
    }
}

impl Drop for VoiceSession {
    fn drop(&mut self) {
        self.stop_threads();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        Receiver<TransportCommand>,
        Receiver<VoiceSessionEvent>,
    ) {
        let peer = new_peer(
            SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 9),
            Instant::now(),
        )
        .unwrap();
        let (events, event_receiver) = mpsc::channel();
        let (audio_commands, audio_command_receiver) = mpsc::channel();
        let (transport_commands, transport_command_receiver) = mpsc::channel();
        let session = VoiceSession {
            channel_id: Snowflake::from_raw(1),
            offer_id: 1,
            toggles: VoiceToggles::default(),
            signalling: Box::new(|_| {}),
            media_lines: peer.media_lines,
            events,
            audio_commands,
            transport_commands,
            video_encoder: Arc::new(VideoEncoderControl::default()),
            video_frames: VideoFrames::new(DecodedPixelFormat::Bgra),
            video_source: None,
            watched_cameras: Vec::new(),
            threads: Vec::new(),
        };
        (
            session,
            audio_command_receiver,
            transport_command_receiver,
            event_receiver,
        )
    }

    #[test]
    fn events_for_another_channel_or_offer_are_ignored() {
        let (mut session, audio, transport, events) = session_without_threads();
        let other_channel = Snowflake::from_raw(2);
        session.handle_gateway_event(&GatewayEvent::VoiceSessionDescription {
            channel_id: session.channel_id,
            offer_id: 2,
            sdp_answer: "not even sdp".into(),
        });
        session.handle_gateway_event(&GatewayEvent::VoiceJoinRefused {
            channel_id: other_channel,
            offer_id: 1,
            reason: VoiceJoinRefusal::ChannelFull,
        });
        session.handle_gateway_event(&GatewayEvent::VoiceDisconnected {
            channel_id: other_channel,
            reason: VoiceDisconnectReason::ChannelDeleted,
        });
        let voice_mid = session.media_lines.voice_receive[0];
        session.handle_gateway_event(&GatewayEvent::VoiceSlotAssigned {
            channel_id: other_channel,
            mid: voice_mid.to_string(),
            user_id: Snowflake::from_raw(3),
        });
        assert!(audio.try_recv().is_err());
        assert!(transport.try_recv().is_err());
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn only_audio_slots_are_assigned_to_the_audio_thread() {
        let (mut session, audio, _transport, _events) = session_without_threads();
        let camera_mid = session.media_lines.camera_receive[0];
        let voice_mid = session.media_lines.voice_receive[0];
        let user_id = Snowflake::from_raw(3);
        for mid in [camera_mid, voice_mid] {
            session.handle_gateway_event(&GatewayEvent::VoiceSlotAssigned {
                channel_id: session.channel_id,
                mid: mid.to_string(),
                user_id,
            });
        }
        assert!(matches!(
            audio.try_recv(),
            Ok(AudioCommand::AssignSlot { mid, user_id: assigned })
                if mid == voice_mid && assigned == user_id
        ));
        assert!(audio.try_recv().is_err(), "the camera slot is not audio");
    }

    #[test]
    fn a_refusal_of_this_offer_stops_the_session() {
        let (mut session, audio, transport, events) = session_without_threads();
        session.handle_gateway_event(&GatewayEvent::VoiceJoinRefused {
            channel_id: session.channel_id,
            offer_id: 1,
            reason: VoiceJoinRefusal::ChannelFull,
        });
        assert!(matches!(audio.try_recv(), Ok(AudioCommand::Stop)));
        assert!(matches!(transport.try_recv(), Ok(TransportCommand::Stop)));
        assert_eq!(
            events.try_recv(),
            Ok(VoiceSessionEvent::JoinRefused(
                VoiceJoinRefusal::ChannelFull
            ))
        );
    }

    #[test]
    fn the_loopback_server_is_reached_from_the_loopback_interface() {
        let local = local_address_towards(Ipv4Addr::LOCALHOST).unwrap();
        assert!(local.is_loopback());
    }

    /// A session without threads whose gateway frames are kept.
    fn session_sending_frames() -> (
        VoiceSession,
        Receiver<ClientFrame>,
        Receiver<VoiceSessionEvent>,
    ) {
        let (mut session, _audio, _transport, events) = session_without_threads();
        let (frames, sent) = mpsc::channel();
        session.signalling = Box::new(move |frame| {
            let _ = frames.send(frame);
        });
        (session, sent, events)
    }

    /// A well-formed answer, as a peer accepting this client's offer writes it.
    fn answer_to_an_offer() -> String {
        let offer = new_peer(
            SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 9),
            Instant::now(),
        )
        .unwrap()
        .sdp_offer;
        let mut answering_peer = str0m::Rtc::builder()
            .enable_vp8(true)
            .enable_opus(true, false)
            .build(Instant::now());
        answering_peer
            .sdp_api()
            .accept_offer(str0m::change::SdpOffer::from_sdp_string(&offer).unwrap())
            .unwrap()
            .to_sdp_string()
    }

    fn video_state(sent: &Receiver<ClientFrame>) -> Option<(bool, bool)> {
        sent.try_iter().last().and_then(|frame| match frame {
            ClientFrame::VoiceState {
                self_video,
                self_stream,
                ..
            } => Some((self_video, self_stream)),
            _ => None,
        })
    }

    #[test]
    fn a_screen_replaces_the_camera_and_the_server_hears_of_each() {
        let (mut session, sent, _events) = session_sending_frames();
        let camera = session.start_video(VideoSourceKind::Camera);
        assert_eq!(video_state(&sent), Some((true, false)));
        let screen = session.start_video(VideoSourceKind::Screen(ScreenShareQuality::High));
        assert_eq!(video_state(&sent), Some((true, true)));
        assert!(!camera.is_open() && screen.is_open(), "never both");
        session.set_self_muted(true);
        assert_eq!(
            video_state(&sent),
            Some((true, true)),
            "muting keeps the share"
        );
        session.stop_video();
        assert_eq!(video_state(&sent), Some((false, false)));
        assert!(!screen.is_open());
    }

    #[test]
    fn a_refusal_turns_the_video_off_and_says_why() {
        let (mut session, _sent, events) = session_sending_frames();
        let camera = session.start_video(VideoSourceKind::Camera);
        session.handle_gateway_event(&GatewayEvent::VoiceVideoRefused {
            channel_id: session.channel_id,
            reason: VoiceVideoRefusal::TooManyCameras,
        });
        assert!(!camera.is_open() && session.video_source().is_none());
        assert_eq!(
            events.try_recv(),
            Ok(VoiceSessionEvent::VideoRefused(
                VoiceVideoRefusal::TooManyCameras
            ))
        );
    }

    #[test]
    fn no_layer_watched_right_after_a_start_belongs_to_the_previous_source() {
        let (mut session, _sent, events) = session_sending_frames();
        let camera = session.start_video(VideoSourceKind::Camera);
        session.handle_gateway_event(&GatewayEvent::VoiceVideoLayers {
            channel_id: session.channel_id,
            active: Vec::new(),
        });
        assert!(camera.is_open());
        // As if the grace period had passed.
        session.video_source = Some((
            VideoSourceKind::Camera,
            Instant::now().checked_sub(VIDEO_RESTART_GRACE).unwrap(),
        ));
        session.handle_gateway_event(&GatewayEvent::VoiceVideoLayers {
            channel_id: session.channel_id,
            active: Vec::new(),
        });
        assert!(!camera.is_open());
        assert_eq!(
            events.try_recv(),
            Ok(VoiceSessionEvent::VideoStoppedByServer)
        );
    }

    #[test]
    fn the_watch_list_is_sent_on_change_and_again_after_each_answer() {
        let (mut session, sent, _events) = session_sending_frames();
        let watched = WatchedVideo {
            user_id: Snowflake::from_raw(5),
            tile_size: VideoTileSize::Large,
            display_width: 1_280,
            display_height: 720,
        };
        session.watch_videos(&[watched]);
        assert_eq!(sent.try_iter().count(), 1);
        session.watch_videos(&[WatchedVideo {
            display_width: 640,
            ..watched
        }]);
        assert_eq!(sent.try_iter().count(), 0, "only the drawn size changed");
        session.handle_gateway_event(&GatewayEvent::VoiceSessionDescription {
            channel_id: session.channel_id,
            offer_id: session.offer_id,
            sdp_answer: answer_to_an_offer(),
        });
        let resent: Vec<ClientFrame> = sent.try_iter().collect();
        assert!(
            matches!(
                resent.as_slice(),
                [ClientFrame::VoiceVideoWatch { cameras }] if cameras.len() == 1
            ),
            "{resent:?}"
        );
    }
}
