//! The network thread: drives the `str0m` peer of the current offer over its own UDP socket, as
//! the server's media loop does on its side, and runs the connection's state machine
//! ([`crate::connection`]): it sends every gateway frame of the session, makes a new peer for each
//! offer, and reports what happened as session events. It writes the camera's or screen's
//! simulcast layers, hands received video frames to their decoding threads, passes keyframe
//! requests both ways, and keeps the video signalling (`voice_state`'s video, the watch list).

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use mellow_protocol::{ClientFrame, GatewayEvent, Snowflake, WatchedCamera};
use str0m::change::{SdpAnswer, SdpPendingOffer};
use str0m::format::Codec;
use str0m::media::{Frequency, KeyframeRequestKind, MediaTime, Mid, Pt, Rid};
use str0m::net::{Protocol, Receive};
use str0m::{Event, IceConnectionState, Input, Output, Rtc};

use crate::audio_engine::AudioCommand;
use crate::connection::{ConnectionAction, ConnectionInput, ConnectionMachine};
use crate::offer::{MediaLines, OfferError, ReceiveSlotKind, new_peer};
use crate::person_settings::{PersonSoundSource, PersonVoiceSettingsStore};
use crate::session::{VoiceSessionEvent, VoiceSignalling, VoiceToggles};
use crate::simulcast::{LAYERS, VideoSourceKind, rid_of};
use crate::uplink::{UplinkLayerCeiling, loss_percent_of};
use crate::video_receiver::{ReceivedCodec, VideoReceivers};
use crate::video_sender::{OutgoingVideoFrame, VideoEncoderControl};
use crate::watch_list::WatchListPacing;

/// The longest the thread sleeps on its socket, so outgoing voice never waits longer to leave.
const MAXIMUM_SOCKET_WAIT: Duration = Duration::from_millis(5);
/// Larger than any datagram the server sends (it drops anything over 1 500 bytes itself).
const RECEIVE_BUFFER_BYTES: usize = 2_000;
/// The server accepts one keyframe request a second from a watcher; more would be dropped.
const MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS: Duration = Duration::from_secs(1);
/// An empty `VOICE_VIDEO_LAYERS` this soon after a source started is the previous source's
/// ending, not this one's (the web's `VIDEO_RESTART_GRACE_MILLISECONDS`).
const VIDEO_RESTART_GRACE: Duration = Duration::from_secs(2);

/// 20 ms of voice ready to send.
#[derive(Debug)]
pub(crate) struct OutgoingVoicePacket {
    pub(crate) rtp_timestamp: u64,
    pub(crate) payload: Vec<u8>,
    /// RFC 6464 attenuation, 0 (loud) to 127 (silence).
    pub(crate) audio_level: u8,
    pub(crate) is_voice: bool,
}

/// An audio packet received on one of the receive slots.
#[derive(Debug)]
pub(crate) struct IncomingAudioPacket {
    pub(crate) mid: Mid,
    pub(crate) rtp_timestamp: u64,
    pub(crate) arrival: Instant,
    pub(crate) payload: Arc<[u8]>,
}

/// What the session asks of the network thread.
#[derive(Debug)]
pub(crate) enum NetworkCommand {
    /// A voice event of the gateway (the session filters out the others).
    Gateway(Box<GatewayEvent>),
    Connection(ConnectionInput),
    /// New mute and deafen: tell the server.
    SetToggles(VoiceToggles),
    /// The camera or a screen turned on (replacing the other), or off: tell the server.
    SetVideoSource(Option<VideoSourceKind>),
    /// The cameras and screens to receive, and their tile sizes.
    WatchCameras(Vec<WatchedCamera>),
    /// Send `voice_leave` and stop.
    Leave,
    Stop,
}

/// The local address the system routes to `server` from, found without sending anything.
pub(crate) fn local_address_towards(server: Ipv4Addr) -> std::io::Result<IpAddr> {
    let probe = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    // Port 9 (discard): connecting a UDP socket only picks the route.
    probe.connect((server, 9))?;
    Ok(probe.local_addr()?.ip())
}

/// A peer made for one offer, with its own socket: a reconnection takes a new network path.
pub(crate) struct PeerConnection {
    pub(crate) offer_id: u32,
    pub(crate) sdp_offer: String,
    rtc: Rtc,
    pending_offer: Option<SdpPendingOffer>,
    socket: UdpSocket,
    /// The address announced to the server; the socket itself listens on every interface.
    local_candidate: SocketAddr,
    pub(crate) media_lines: MediaLines,
    voice_payload_type: Option<Pt>,
    video_payload_type: Option<Pt>,
    /// DTLS is done: ICE coming back to `connected` means media flows again.
    is_media_connected: bool,
}

impl std::fmt::Debug for PeerConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeerConnection")
            .field("offer_id", &self.offer_id)
            .field("media_lines", &self.media_lines)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PeerError {
    #[error("the voice socket could not be opened: {0}")]
    Socket(#[from] std::io::Error),
    #[error(transparent)]
    Offer(#[from] OfferError),
}

impl PeerConnection {
    /// A new socket and peer facing `server_address`, with its offer made.
    pub(crate) fn new(server_address: Ipv4Addr, offer_id: u32) -> Result<Self, PeerError> {
        // Bound to every interface so the system picks the source per destination (the server
        // may answer from another of its addresses); only the candidate names one interface.
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        let local_candidate = SocketAddr::new(
            local_address_towards(server_address)?,
            socket.local_addr()?.port(),
        );
        let peer = new_peer(local_candidate, Instant::now())?;
        Ok(Self {
            offer_id,
            sdp_offer: peer.sdp_offer,
            rtc: peer.rtc,
            pending_offer: Some(peer.pending),
            socket,
            local_candidate,
            media_lines: peer.media_lines,
            voice_payload_type: None,
            video_payload_type: None,
            is_media_connected: false,
        })
    }

    fn apply_answer(&mut self, answer: SdpAnswer) -> Result<(), String> {
        let Some(pending) = self.pending_offer.take() else {
            // A second answer to the same offer: offer ids filter these out already.
            return Ok(());
        };
        self.rtc
            .sdp_api()
            .accept_answer(pending, answer)
            .map_err(|error| error.to_string())
    }

    /// Sends the queued voice and video, then runs the peer until its next timer or datagram
    /// (at most [`MAXIMUM_SOCKET_WAIT`]). `is_path_dropped` silences the socket both ways
    /// (tests).
    fn step(
        &mut self,
        outgoing: &Receiver<OutgoingVoicePacket>,
        incoming: &Sender<IncomingAudioPacket>,
        video: &mut VideoLink,
        reports: &mut Vec<PeerReport>,
        is_path_dropped: bool,
        buffer: &mut [u8],
    ) -> Result<(), String> {
        self.send_voice(outgoing)?;
        self.send_video(video);
        self.request_keyframes(video);
        let timeout = loop {
            match self.rtc.poll_output().map_err(|error| error.to_string())? {
                Output::Timeout(at) => break at,
                Output::Transmit(transmit) => {
                    if !is_path_dropped {
                        // A lost datagram is a lost packet; UDP has no backpressure to honour.
                        let _ = self
                            .socket
                            .send_to(&transmit.contents, transmit.destination);
                    }
                }
                Output::Event(event) => self.handle_event(event, incoming, video, reports),
            }
        };
        let wait = timeout
            .saturating_duration_since(Instant::now())
            .clamp(Duration::from_millis(1), MAXIMUM_SOCKET_WAIT);
        self.socket
            .set_read_timeout(Some(wait))
            .map_err(|error| error.to_string())?;
        let input = match self.socket.recv_from(buffer) {
            Ok(_) if is_path_dropped => Input::Timeout(Instant::now()),
            Ok((length, source)) => {
                let Ok(contents) = buffer[..length].try_into() else {
                    // Not STUN, DTLS, RTP or RTCP: someone else's datagram.
                    return Ok(());
                };
                Input::Receive(
                    Instant::now(),
                    Receive {
                        proto: Protocol::Udp,
                        source,
                        destination: self.local_candidate,
                        contents,
                    },
                )
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                Input::Timeout(Instant::now())
            }
            // An ICMP "port unreachable" from an earlier send: ICE handles reachability.
            Err(error) if error.kind() == ErrorKind::ConnectionReset => {
                Input::Timeout(Instant::now())
            }
            Err(error) => return Err(error.to_string()),
        };
        self.rtc
            .handle_input(input)
            .map_err(|error| error.to_string())
    }

    fn handle_event(
        &mut self,
        event: Event,
        incoming: &Sender<IncomingAudioPacket>,
        video: &mut VideoLink,
        reports: &mut Vec<PeerReport>,
    ) {
        match event {
            // `Connected`, not the ICE state: DTLS is done and media can flow.
            Event::Connected => {
                self.voice_payload_type =
                    self.payload_type(self.media_lines.voice_send, Codec::Opus);
                self.video_payload_type =
                    self.payload_type(self.media_lines.camera_send, Codec::Vp8);
                self.is_media_connected = true;
                // Pictures encoded before now were dropped: start every layer afresh.
                video.encoder.request_keyframe(None);
                reports.push(PeerReport::MediaConnected);
            }
            Event::KeyframeRequest(request) if request.mid == self.media_lines.camera_send => {
                let layer = request
                    .rid
                    .and_then(|rid| LAYERS.into_iter().find(|layer| *rid == *rid_of(*layer)));
                // A request naming an unknown RID asks for nothing this client sends.
                if request.rid.is_none() || layer.is_some() {
                    video.encoder.request_keyframe(layer);
                }
            }
            Event::MediaData(data) if self.media_lines.camera_receive.contains(&data.mid) => {
                let codec = match data.params.spec().codec {
                    Codec::Vp8 => Some(ReceivedCodec::Vp8),
                    Codec::H264 => Some(ReceivedCodec::H264),
                    _ => None,
                };
                // The offer names no other video codec: anything else is not decoded.
                if let Some(codec) = codec {
                    video
                        .receivers
                        .deliver(data.mid, data.data, codec, data.contiguous);
                }
            }
            Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                reports.push(PeerReport::IceDisconnected);
            }
            Event::IceConnectionStateChange(
                IceConnectionState::Connected | IceConnectionState::Completed,
            ) if self.is_media_connected => reports.push(PeerReport::MediaConnected),
            Event::PeerStats(stats) => {
                let previous = video.uplink.ceiling();
                let loss_percent = stats.egress_loss_fraction.map(loss_percent_of);
                let ceiling = video.uplink.after_report(loss_percent, Instant::now());
                if ceiling != previous {
                    tracing::debug!(?ceiling, "the uplink's video layers changed");
                    video.encoder.set_layer_ceiling(ceiling);
                }
                if let Some(round_trip) = stats
                    .selected_candidate_pair
                    .and_then(|pair| pair.current_round_trip_time)
                {
                    reports.push(PeerReport::RoundTrip(round_trip));
                }
            }
            Event::MediaData(data)
                if self
                    .media_lines
                    .audio_receive()
                    .any(|(mid, _)| mid == data.mid) =>
            {
                // The audio thread gone means the session is ending.
                let _ = incoming.send(IncomingAudioPacket {
                    mid: data.mid,
                    rtp_timestamp: data.time.numer(),
                    arrival: data.network_time,
                    payload: data.data,
                });
            }
            _ => {}
        }
    }

    fn payload_type(&mut self, mid: Mid, codec: Codec) -> Option<Pt> {
        self.rtc.writer(mid).and_then(|writer| {
            writer
                .payload_params()
                .find(|params| params.spec().codec == codec)
                .map(str0m::format::PayloadParams::pt)
        })
    }

    /// Writes the video layers the encoding thread produced; before the connection they are
    /// dropped (a keyframe of each follows the connection). A frame that cannot be written is
    /// lost, not the call: the server asks for a keyframe when its watchers need one.
    fn send_video(&mut self, video: &VideoLink) {
        while let Ok(frame) = video.outgoing.try_recv() {
            let Some(payload_type) = self.video_payload_type else {
                continue;
            };
            let Some(writer) = self.rtc.writer(self.media_lines.camera_send) else {
                continue;
            };
            let written = writer.rid(Rid::from(rid_of(frame.layer))).write(
                payload_type,
                Instant::now(),
                MediaTime::new(frame.rtp_timestamp, Frequency::NINETY_KHZ),
                frame.bytes,
            );
            if let Err(error) = written {
                tracing::warn!(%error, "a video frame could not be sent");
            }
        }
    }

    /// Asks the server for a keyframe on the receive slots whose decoder lost track, at most
    /// once a second each.
    fn request_keyframes(&mut self, video: &mut VideoLink) {
        let now = Instant::now();
        while let Ok(mid) = video.keyframe_requests.try_recv() {
            if video.keyframe_requested_at.get(&mid).is_some_and(|at| {
                now.saturating_duration_since(*at) < MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS
            }) {
                continue;
            }
            // Before the connection the request fails: the decoder asks again later.
            if let Some(mut writer) = self.rtc.writer(mid)
                && writer
                    .request_keyframe(None, KeyframeRequestKind::Pli)
                    .is_ok()
            {
                video.keyframe_requested_at.insert(mid, now);
            }
        }
    }

    /// Writes the voice packets the audio thread encoded; before the connection they are
    /// dropped, as a browser's track sends nothing before then either.
    fn send_voice(&mut self, outgoing: &Receiver<OutgoingVoicePacket>) -> Result<(), String> {
        while let Ok(packet) = outgoing.try_recv() {
            let Some(payload_type) = self.voice_payload_type else {
                continue;
            };
            let Some(writer) = self.rtc.writer(self.media_lines.voice_send) else {
                continue;
            };
            // str0m takes the level as negative dBov.
            let level = -i8::try_from(packet.audio_level).unwrap_or(i8::MAX);
            writer
                .audio_level(level, packet.is_voice)
                .write(
                    payload_type,
                    Instant::now(),
                    MediaTime::new(packet.rtp_timestamp, Frequency::FORTY_EIGHT_KHZ),
                    packet.payload,
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

/// What a peer noticed during a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeerReport {
    MediaConnected,
    IceDisconnected,
    RoundTrip(Duration),
}

/// The network thread's video, kept across peers.
#[derive(Debug)]
pub(crate) struct VideoLink {
    /// Told about the server's keyframe requests and watched layers.
    pub(crate) encoder: Arc<VideoEncoderControl>,
    pub(crate) outgoing: Receiver<OutgoingVideoFrame>,
    pub(crate) receivers: VideoReceivers,
    /// Receive slots whose decoder needs a keyframe.
    pub(crate) keyframe_requests: Receiver<Mid>,
    pub(crate) keyframe_requested_at: HashMap<Mid, Instant>,
    /// The source on, and when it started.
    pub(crate) source: Option<(VideoSourceKind, Instant)>,
    /// The last `voice_video_watch` list, sent again after each answer (a join forgets it).
    pub(crate) watched_cameras: Vec<WatchedCamera>,
    pub(crate) watch_list_pacing: WatchListPacing,
    /// The layers the uplink carries, from the server's loss feedback.
    pub(crate) uplink: UplinkLayerCeiling,
}

/// Everything the network thread owns.
pub(crate) struct NetworkThread {
    pub(crate) machine: ConnectionMachine,
    pub(crate) peer: Option<PeerConnection>,
    pub(crate) server_address: Ipv4Addr,
    pub(crate) own_user_id: Snowflake,
    pub(crate) toggles: VoiceToggles,
    pub(crate) signalling: Box<dyn VoiceSignalling>,
    pub(crate) settings_store: Arc<dyn PersonVoiceSettingsStore>,
    pub(crate) commands: Receiver<NetworkCommand>,
    pub(crate) outgoing: Receiver<OutgoingVoicePacket>,
    pub(crate) incoming: Sender<IncomingAudioPacket>,
    pub(crate) events: Sender<VoiceSessionEvent>,
    pub(crate) audio_commands: Sender<AudioCommand>,
    pub(crate) video: VideoLink,
    /// Set by tests to lose the current network path; a new peer takes a new one.
    pub(crate) is_path_dropped: Arc<AtomicBool>,
}

impl std::fmt::Debug for NetworkThread {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NetworkThread")
            .field("machine", &self.machine)
            .field("peer", &self.peer)
            .finish_non_exhaustive()
    }
}

impl NetworkThread {
    /// Sends the first offer (already made by the caller, so its errors reach `join`) and runs
    /// until the session ends.
    pub(crate) fn run(mut self) {
        let mut buffer = vec![0_u8; RECEIVE_BUFFER_BYTES];
        let mut reports = Vec::new();
        let actions = self.machine.start();
        self.perform(actions);
        while !self.machine.is_stopped() {
            loop {
                match self.commands.try_recv() {
                    Ok(NetworkCommand::Stop) | Err(TryRecvError::Disconnected) => {
                        self.stop();
                        return;
                    }
                    Ok(command) => self.handle_command(command),
                    Err(TryRecvError::Empty) => break,
                }
            }
            let now = Instant::now();
            if let Some(cameras) = self.video.watch_list_pacing.take_due(now)
                && !self.machine.is_stopped()
            {
                self.send_watch_list(cameras);
            }
            if self
                .machine
                .next_deadline()
                .is_some_and(|deadline| deadline <= now)
            {
                let actions = self.machine.on_deadlines(now);
                self.perform(actions);
            }
            let is_path_dropped = self.is_path_dropped.load(Ordering::Relaxed);
            let Some(peer) = self.peer.as_mut() else {
                // Between peers: voice and video have nowhere to go.
                while self.outgoing.try_recv().is_ok() {}
                while self.video.outgoing.try_recv().is_ok() {}
                std::thread::sleep(MAXIMUM_SOCKET_WAIT);
                continue;
            };
            let offer_id = peer.offer_id;
            let stepped = peer.step(
                &self.outgoing,
                &self.incoming,
                &mut self.video,
                &mut reports,
                is_path_dropped,
                &mut buffer,
            );
            for report in reports.drain(..) {
                let input = match report {
                    PeerReport::MediaConnected => ConnectionInput::MediaConnected { offer_id },
                    PeerReport::IceDisconnected => ConnectionInput::IceDisconnected { offer_id },
                    PeerReport::RoundTrip(round_trip) => {
                        let _ = self.events.send(VoiceSessionEvent::RoundTrip(round_trip));
                        continue;
                    }
                };
                let actions = self.machine.handle(input, Instant::now());
                self.perform(actions);
            }
            if let Err(reason) = stepped {
                tracing::warn!(%reason, "voice connection failed");
                self.peer = None;
                let actions = self
                    .machine
                    .handle(ConnectionInput::MediaFailed { offer_id }, Instant::now());
                self.perform(actions);
            }
        }
        self.stop();
    }

    fn stop(&mut self) {
        self.peer = None;
        // The session may already be stopping the audio thread itself.
        let _ = self.audio_commands.send(AudioCommand::Stop);
    }

    fn handle_command(&mut self, command: NetworkCommand) {
        let now = Instant::now();
        match command {
            NetworkCommand::Gateway(event) => self.handle_gateway_event(*event),
            NetworkCommand::Connection(input) => {
                let actions = self.machine.handle(input, now);
                self.perform(actions);
            }
            NetworkCommand::SetToggles(toggles) => {
                self.toggles = toggles;
                self.send_voice_state();
            }
            NetworkCommand::SetVideoSource(kind) => self.set_video_source(kind),
            NetworkCommand::WatchCameras(cameras) => {
                self.video.watched_cameras.clone_from(&cameras);
                if let Some(cameras) = self.video.watch_list_pacing.offer(cameras, now) {
                    self.send_watch_list(cameras);
                }
            }
            NetworkCommand::Leave => {
                self.signalling.send_frame(ClientFrame::VoiceLeave);
                self.machine.leave();
            }
            NetworkCommand::Stop => {}
        }
    }

    fn handle_gateway_event(&mut self, event: GatewayEvent) {
        let now = Instant::now();
        let input = match event {
            GatewayEvent::VoiceSessionDescription {
                channel_id,
                offer_id,
                sdp_answer,
            } => {
                let Some(peer) = self.peer.as_mut().filter(|peer| peer.offer_id == offer_id) else {
                    return;
                };
                if !self.machine.is_current_offer(channel_id, offer_id) {
                    return;
                }
                let applied = SdpAnswer::from_sdp_string(&sdp_answer)
                    .map_err(|error| error.to_string())
                    .and_then(|answer| peer.apply_answer(answer));
                match applied {
                    Ok(()) => {
                        // The join forgot any earlier list.
                        if !self.video.watched_cameras.is_empty() {
                            self.video.watch_list_pacing.sent_now(now);
                            self.send_watch_list(self.video.watched_cameras.clone());
                        }
                        ConnectionInput::AnswerApplied { offer_id }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the server's voice answer was refused");
                        ConnectionInput::MediaFailed { offer_id }
                    }
                }
            }
            GatewayEvent::VoiceJoinRefused {
                channel_id,
                offer_id,
                reason,
            } => ConnectionInput::Refused {
                channel_id,
                offer_id,
                reason,
            },
            GatewayEvent::VoiceDisconnected { channel_id, reason } => {
                ConnectionInput::ServerDisconnected { channel_id, reason }
            }
            GatewayEvent::VoiceStateDelete {
                channel_id,
                user_id,
                ..
            } if user_id == self.own_user_id => ConnectionInput::OwnStateDropped { channel_id },
            GatewayEvent::VoiceSlotAssigned {
                channel_id,
                mid,
                user_id,
            } => {
                self.assign_slot(channel_id, &mid, user_id);
                return;
            }
            GatewayEvent::VoiceVideoLayers { channel_id, active }
                if channel_id == self.machine.channel_id() =>
            {
                let Some((_, started_at)) = self.video.source else {
                    return;
                };
                if !active.is_empty() {
                    self.video.encoder.set_active_layers(active);
                } else if now.saturating_duration_since(started_at) >= VIDEO_RESTART_GRACE {
                    self.end_video(VoiceSessionEvent::VideoStoppedByServer);
                }
                return;
            }
            GatewayEvent::VoiceVideoRefused { channel_id, reason }
                if channel_id == self.machine.channel_id() =>
            {
                self.end_video(VoiceSessionEvent::VideoRefused(reason));
                return;
            }
            GatewayEvent::VoiceVideoPaused {
                channel_id,
                user_id,
                is_paused,
            } if channel_id == self.machine.channel_id() => {
                let _ = self
                    .events
                    .send(VoiceSessionEvent::VideoPaused { user_id, is_paused });
                return;
            }
            _ => return,
        };
        let actions = self.machine.handle(input, now);
        self.perform(actions);
    }

    /// Hands the slot to the audio thread with the person's setting for what it carries.
    fn assign_slot(&self, channel_id: Snowflake, mid: &str, user_id: Snowflake) {
        let Some(peer) = self.peer.as_ref() else {
            return;
        };
        if channel_id != self.machine.channel_id() {
            return;
        }
        let mid = Mid::from(mid);
        let source = match peer.media_lines.receive_slot_kind(mid) {
            Some(ReceiveSlotKind::Voice) => PersonSoundSource::Voice,
            Some(ReceiveSlotKind::ScreenSound) => PersonSoundSource::ScreenSound,
            Some(ReceiveSlotKind::Camera) => {
                self.video.receivers.assign_slot(mid, user_id);
                return;
            }
            None => return,
        };
        let setting = self.settings_store.person_setting(source, user_id);
        let _ = self.audio_commands.send(AudioCommand::AssignSlot {
            mid,
            user_id,
            setting,
        });
    }

    fn send_voice_state(&mut self) {
        let source = self.video.source.map(|(kind, _)| kind);
        // Always the full state: the server drops `voice_state` frames over its rate limit.
        let is_sent = self.signalling.send_frame(ClientFrame::VoiceState {
            self_mute: self.toggles.is_muted(),
            self_deaf: self.toggles.is_deafened(),
            self_video: source.is_some(),
            self_stream: matches!(source, Some(VideoSourceKind::Screen(_))),
        });
        self.machine.voice_state_sent(is_sent);
    }

    fn set_video_source(&mut self, kind: Option<VideoSourceKind>) {
        // A new screen quality changes nothing the server knows of: keep the start.
        if let (
            Some((VideoSourceKind::Screen(_), started_at)),
            Some(kind @ VideoSourceKind::Screen(_)),
        ) = (self.video.source, kind)
        {
            self.video.source = Some((kind, started_at));
        } else {
            self.video.source = kind.map(|kind| (kind, Instant::now()));
            self.send_voice_state();
        }
    }

    /// The server refused or turned off the camera or screen: it already knows.
    fn end_video(&mut self, event: VoiceSessionEvent) {
        self.video.source = None;
        self.video.encoder.stop_source();
        let _ = self.events.send(event);
    }

    fn send_watch_list(&self, cameras: Vec<WatchedCamera>) {
        self.signalling
            .send_frame(ClientFrame::VoiceVideoWatch { cameras });
    }

    fn perform(&mut self, actions: Vec<ConnectionAction>) {
        for action in actions {
            match action {
                ConnectionAction::Offer {
                    channel_id,
                    offer_id,
                } => self.offer(channel_id, offer_id),
                ConnectionAction::SendVoiceState => self.send_voice_state(),
                ConnectionAction::Stop { should_send_leave } => {
                    if should_send_leave {
                        self.signalling.send_frame(ClientFrame::VoiceLeave);
                    }
                    self.stop();
                }
                ConnectionAction::Emit(event) => {
                    // Nobody listening means the session is being dropped.
                    let _ = self.events.send(event);
                }
            }
        }
    }

    /// A new peer on a new path, and its `voice_join`; the server swaps the media session.
    fn offer(&mut self, channel_id: Snowflake, offer_id: u32) {
        let peer = match self.peer.take() {
            // The first offer's peer, made by `join`.
            Some(peer) if peer.offer_id == offer_id => Ok(peer),
            _ => PeerConnection::new(self.server_address, offer_id),
        };
        let peer = match peer {
            Ok(peer) => peer,
            Err(PeerError::Socket(error)) => {
                // No route (Wi-Fi dropped): like an offer the gateway could not carry, wait for
                // `BackOnline`, `READY` or `RESUMED` instead of using up the silent attempts.
                tracing::debug!(%error, "no network for the voice offer, waiting for it");
                self.machine.offer_sent(false, Instant::now());
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "the voice offer could not be made");
                let actions = self
                    .machine
                    .handle(ConnectionInput::MediaFailed { offer_id }, Instant::now());
                self.perform(actions);
                return;
            }
        };
        self.is_path_dropped.store(false, Ordering::Relaxed);
        self.video.receivers.new_media_session();
        self.video.keyframe_requested_at.clear();
        let _ = self.audio_commands.send(AudioCommand::NewMediaSession(
            peer.media_lines.audio_receive().collect(),
        ));
        let is_sent = self.signalling.send_frame(ClientFrame::VoiceJoin {
            channel_id,
            offer_id,
            sdp_offer: peer.sdp_offer.clone(),
            self_mute: self.toggles.is_muted(),
            self_deaf: self.toggles.is_deafened(),
        });
        self.peer = Some(peer);
        self.machine.offer_sent(is_sent, Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use mellow_protocol::{SimulcastLayer, VideoTileSize, VoiceVideoRefusal};

    use super::*;
    use crate::person_settings::InMemoryPersonVoiceSettings;
    use crate::simulcast::ScreenShareQuality;
    use crate::video_frame::DecodedPixelFormat;
    use crate::video_receiver::VideoFrames;

    const CHANNEL: Snowflake = Snowflake::from_raw(1);

    /// A network thread without a peer, its gateway frames and events kept.
    fn network_without_peer() -> (
        NetworkThread,
        Receiver<ClientFrame>,
        Receiver<VoiceSessionEvent>,
    ) {
        let (frames, sent) = mpsc::channel();
        let (events, event_receiver) = mpsc::channel();
        let (_commands, command_receiver) = mpsc::channel();
        let (_outgoing, outgoing_receiver) = mpsc::channel();
        let (incoming, _incoming_receiver) = mpsc::channel();
        let (audio_commands, _audio_receiver) = mpsc::channel();
        let (_video_out, video_out_receiver) = mpsc::channel();
        let (keyframe_requests, keyframe_request_receiver) = mpsc::channel();
        let network = NetworkThread {
            machine: ConnectionMachine::new(CHANNEL),
            peer: None,
            server_address: Ipv4Addr::LOCALHOST,
            own_user_id: Snowflake::from_raw(5),
            toggles: VoiceToggles::default(),
            signalling: Box::new(move |frame| frames.send(frame).is_ok()),
            settings_store: Arc::new(InMemoryPersonVoiceSettings::default()),
            commands: command_receiver,
            outgoing: outgoing_receiver,
            incoming,
            events,
            audio_commands,
            video: VideoLink {
                encoder: Arc::new(VideoEncoderControl::default()),
                outgoing: video_out_receiver,
                receivers: VideoReceivers::new(
                    VideoFrames::new(DecodedPixelFormat::Bgra),
                    keyframe_requests,
                ),
                keyframe_requests: keyframe_request_receiver,
                keyframe_requested_at: HashMap::new(),
                source: None,
                watched_cameras: Vec::new(),
                watch_list_pacing: WatchListPacing::default(),
                uplink: UplinkLayerCeiling::default(),
            },
            is_path_dropped: Arc::new(AtomicBool::new(false)),
        };
        (network, sent, event_receiver)
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
    fn the_server_hears_of_each_source_but_not_of_a_screen_quality_change() {
        let (mut network, sent, _events) = network_without_peer();
        network.handle_command(NetworkCommand::SetVideoSource(Some(
            VideoSourceKind::Camera,
        )));
        assert_eq!(video_state(&sent), Some((true, false)));
        let screen = VideoSourceKind::Screen(ScreenShareQuality::High);
        network.handle_command(NetworkCommand::SetVideoSource(Some(screen)));
        assert_eq!(video_state(&sent), Some((true, true)));
        let highest = VideoSourceKind::Screen(ScreenShareQuality::Highest);
        network.handle_command(NetworkCommand::SetVideoSource(Some(highest)));
        assert_eq!(sent.try_iter().count(), 0);
        network.handle_command(NetworkCommand::SetToggles(VoiceToggles::new(
            true, false, false,
        )));
        assert_eq!(
            video_state(&sent),
            Some((true, true)),
            "muting keeps the share"
        );
        network.handle_command(NetworkCommand::SetVideoSource(None));
        assert_eq!(video_state(&sent), Some((false, false)));
    }

    #[test]
    fn a_refusal_turns_the_video_off_and_says_why() {
        let (mut network, _sent, events) = network_without_peer();
        let camera = network.video.encoder.start(VideoSourceKind::Camera);
        network.handle_command(NetworkCommand::SetVideoSource(Some(
            VideoSourceKind::Camera,
        )));
        network.handle_gateway_event(GatewayEvent::VoiceVideoRefused {
            channel_id: CHANNEL,
            reason: VoiceVideoRefusal::TooManyCameras,
        });
        assert!(!camera.is_open() && network.video.source.is_none());
        assert_eq!(
            events.try_recv(),
            Ok(VoiceSessionEvent::VideoRefused(
                VoiceVideoRefusal::TooManyCameras
            ))
        );
    }

    #[test]
    fn no_layer_watched_right_after_a_start_belongs_to_the_previous_source() {
        let (mut network, _sent, events) = network_without_peer();
        let camera = network.video.encoder.start(VideoSourceKind::Camera);
        network.handle_command(NetworkCommand::SetVideoSource(Some(
            VideoSourceKind::Camera,
        )));
        let no_layer = GatewayEvent::VoiceVideoLayers {
            channel_id: CHANNEL,
            active: Vec::new(),
        };
        network.handle_gateway_event(no_layer.clone());
        assert!(camera.is_open());
        // As if the grace period had passed.
        network.video.source = Some((
            VideoSourceKind::Camera,
            Instant::now().checked_sub(VIDEO_RESTART_GRACE).unwrap(),
        ));
        network.handle_gateway_event(GatewayEvent::VoiceVideoLayers {
            channel_id: CHANNEL,
            active: vec![SimulcastLayer::Quarter],
        });
        assert!(camera.is_open(), "a list with layers keeps it");
        network.handle_gateway_event(no_layer);
        assert!(!camera.is_open());
        assert_eq!(
            events.try_recv(),
            Ok(VoiceSessionEvent::VideoStoppedByServer)
        );
    }

    #[test]
    fn a_watch_list_goes_at_once_and_the_next_one_waits() {
        let (mut network, sent, _events) = network_without_peer();
        let cameras = vec![WatchedCamera {
            user_id: Snowflake::from_raw(7),
            size: VideoTileSize::Large,
        }];
        network.handle_command(NetworkCommand::WatchCameras(cameras.clone()));
        // Right after: it waits for the network thread's next turns.
        network.handle_command(NetworkCommand::WatchCameras(Vec::new()));
        let sent: Vec<ClientFrame> = sent.try_iter().collect();
        assert_eq!(sent, [ClientFrame::VoiceVideoWatch { cameras }]);
    }
}
