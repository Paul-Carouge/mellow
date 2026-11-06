//! The network thread: drives the `str0m` peer of the current offer over its own UDP socket, as
//! the server's media loop does on its side, and runs the connection's state machine
//! ([`crate::connection`]): it sends every gateway frame of the session, makes a new peer for each
//! offer, and reports what happened as session events.

use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use mellow_protocol::{ClientFrame, GatewayEvent, Snowflake};
use str0m::change::{SdpAnswer, SdpPendingOffer};
use str0m::format::Codec;
use str0m::media::{Frequency, MediaTime, Mid, Pt};
use str0m::net::{Protocol, Receive};
use str0m::{Event, IceConnectionState, Input, Output, Rtc};

use crate::audio_engine::AudioCommand;
use crate::connection::{ConnectionAction, ConnectionInput, ConnectionMachine};
use crate::offer::{MediaLines, OfferError, ReceiveSlotKind, new_peer};
use crate::person_settings::{PersonSoundSource, PersonVoiceSettingsStore};
use crate::session::{VoiceSessionEvent, VoiceSignalling, VoiceToggles};

/// The longest the thread sleeps on its socket, so outgoing voice never waits longer to leave.
const MAXIMUM_SOCKET_WAIT: Duration = Duration::from_millis(5);
/// Larger than any datagram the server sends (it drops anything over 1 500 bytes itself).
const RECEIVE_BUFFER_BYTES: usize = 2_000;

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

    /// Sends the queued voice, then runs the peer until its next timer or datagram (at most
    /// [`MAXIMUM_SOCKET_WAIT`]). `is_path_dropped` silences the socket both ways (tests).
    fn step(
        &mut self,
        outgoing: &Receiver<OutgoingVoicePacket>,
        incoming: &Sender<IncomingAudioPacket>,
        reports: &mut Vec<PeerReport>,
        is_path_dropped: bool,
        buffer: &mut [u8],
    ) -> Result<(), String> {
        self.send_voice(outgoing)?;
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
                Output::Event(event) => self.handle_event(event, incoming, reports),
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
        reports: &mut Vec<PeerReport>,
    ) {
        match event {
            // `Connected`, not the ICE state: DTLS is done and media can flow.
            Event::Connected => {
                self.voice_payload_type =
                    self.rtc
                        .writer(self.media_lines.voice_send)
                        .and_then(|writer| {
                            writer
                                .payload_params()
                                .find(|params| params.spec().codec == Codec::Opus)
                                .map(str0m::format::PayloadParams::pt)
                        });
                self.is_media_connected = true;
                reports.push(PeerReport::MediaConnected);
            }
            Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                reports.push(PeerReport::IceDisconnected);
            }
            Event::IceConnectionStateChange(
                IceConnectionState::Connected | IceConnectionState::Completed,
            ) if self.is_media_connected => reports.push(PeerReport::MediaConnected),
            Event::PeerStats(stats) => {
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
                // Between peers: voice has nowhere to go.
                while self.outgoing.try_recv().is_ok() {}
                std::thread::sleep(MAXIMUM_SOCKET_WAIT);
                continue;
            };
            let offer_id = peer.offer_id;
            let stepped = peer.step(
                &self.outgoing,
                &self.incoming,
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
                    Ok(()) => ConnectionInput::AnswerApplied { offer_id },
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
            Some(ReceiveSlotKind::Camera) | None => return,
        };
        let setting = self.settings_store.person_setting(source, user_id);
        let _ = self.audio_commands.send(AudioCommand::AssignSlot {
            mid,
            user_id,
            setting,
        });
    }

    fn send_voice_state(&mut self) {
        // Always the full state: the server drops `voice_state` frames over its rate limit.
        let is_sent = self.signalling.send_frame(ClientFrame::VoiceState {
            self_mute: self.toggles.is_muted(),
            self_deaf: self.toggles.is_deafened(),
            self_video: false,
            self_stream: false,
        });
        self.machine.voice_state_sent(is_sent);
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
