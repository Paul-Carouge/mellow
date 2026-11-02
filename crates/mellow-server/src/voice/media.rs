//! The media loop (ADR-0030): one OS thread outside the Tokio runtime owns the UDP socket and every
//! participant's WebRTC session (`str0m`, sans-IO). It forwards each speaker's Opus packets, as
//! they are, to the other participants of their channel, and each camera's VP8 frames, one
//! simulcast layer per watcher (ADR-0035), and reports speaking edges and dead connections. It
//! never touches SQLite: permissions arrive already decided, as commands.

use std::collections::{HashMap, VecDeque};
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use mellow_protocol::{SimulcastLayer, Snowflake};
use str0m::change::SdpOffer;
use str0m::format::{Codec, FormatParams};
use str0m::media::{Direction, Frequency, KeyframeRequestKind, MediaData, MediaKind, Mid, Pt, Rid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};
use tokio::sync::mpsc::UnboundedSender;

use crate::domain::video_rules::{FrameForwarding, KeyframeThrottle, LayerSwitch, active_layers};
use crate::domain::voice_rules::{ReceiveSlots, SpeakingDetector};

/// Addresses remembered per participant: a client changing networks adds one now and then.
const MAXIMUM_SOURCE_ADDRESSES_PER_PARTICIPANT: usize = 4;
/// Larger datagrams are not WebRTC media: dropped unread.
const MAXIMUM_DATAGRAM_BYTES: usize = 1_500;
/// Bigger than `MAXIMUM_DATAGRAM_BYTES`, so an oversized datagram is seen whole and dropped.
const RECEIVE_BUFFER_BYTES: usize = 2_048;
/// Opus sends 50 packets/s; STUN, DTLS and RTCP add a few. More is a flood: dropped.
const MAXIMUM_PACKETS_PER_SECOND: u32 = 200;
/// A participant able to watch cameras also sends their RTCP (receiver reports, retransmission
/// and keyframe requests, congestion feedback) for up to 9 video streams.
const MAXIMUM_PACKETS_PER_SECOND_WITH_VIDEO: u32 = 400;
/// A camera sends three layers of about 180 packets/s together, keyframes three times larger and
/// retransmissions (ADR-0035).
const MAXIMUM_PACKETS_PER_SECOND_WITH_CAMERA: u32 = 1_500;
/// 4 Mbit/s averaged over a second: a camera's three layers (2.2 Mbit/s) with keyframes and
/// retransmissions.
const MAXIMUM_BYTES_PER_SECOND_WITH_CAMERA: usize = 500_000;
/// 800 kbit/s: Opus at its highest bit rate (510 kbit/s) with RTCP for 9 watched cameras, so a
/// client with its camera off cannot make the server decrypt full packets for nothing.
const MAXIMUM_BYTES_PER_SECOND_WITHOUT_CAMERA: usize = 100_000;
const PACKET_RATE_WINDOW: Duration = Duration::from_secs(1);
/// Commands and speaking hangovers are looked at least this often.
const MAXIMUM_IDLE_WAIT: Duration = Duration::from_millis(50);
/// A zero read timeout means "block forever" to the OS.
const MINIMUM_WAIT: Duration = Duration::from_millis(1);
/// ICE and DTLS must complete within this time after the answer. Short, since a participant who
/// never connects still holds a place of the instance's `max_participants`.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Browsers send STUN consent checks every few seconds, even in silence: nothing at all for this
/// long means the client is gone.
const SILENT_CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);
/// The dynamic payload type the answer gives Opus when the offer does not impose one.
const OPUS_PAYLOAD_TYPE: u8 = 111;
const OPUS_CHANNELS: u8 = 2;
const OPUS_MINIMUM_PACKET_MILLISECONDS: u8 = 10;

/// Builds the server side of a participant's session and its SDP answer. Called off the async
/// runtime: building a session generates its DTLS certificate.
///
/// # Errors
/// `None` when the offer does not parse or cannot be answered with Opus (and VP8 for video).
pub(crate) fn answer_offer(
    sdp_offer: &str,
    candidate_address: SocketAddr,
) -> Option<(Box<Rtc>, String)> {
    let offer = SdpOffer::from_sdp_string(sdp_offer).ok()?;
    let mut config = Rtc::builder().set_ice_lite(true).clear_codecs();
    // VP8 with its retransmission stream is the only video codec (ADR-0035): every watcher can
    // decode every camera, and the server never has to choose.
    config.codec_config().enable_vp8(true);
    // In-band FEC and DTX in the answer ask the browser to send them (ADR-0030): FEC hides lost
    // packets, DTX sends almost nothing in silence.
    config.codec_config().add_config(
        Pt::new_with_value(OPUS_PAYLOAD_TYPE),
        None,
        Codec::Opus,
        Frequency::FORTY_EIGHT_KHZ,
        Some(OPUS_CHANNELS),
        FormatParams {
            min_p_time: Some(OPUS_MINIMUM_PACKET_MILLISECONDS),
            use_inband_fec: Some(true),
            use_dtx: Some(true),
            ..FormatParams::default()
        },
    );
    let mut rtc = config.build(Instant::now());
    rtc.add_local_candidate(Candidate::host(candidate_address, "udp").ok()?)?;
    let answer = rtc.sdp_api().accept_offer(offer).ok()?;
    Some((Box::new(rtc), answer.to_sdp_string()))
}

#[derive(Debug)]
pub(crate) struct MediaJoin {
    pub(crate) user_id: Snowflake,
    /// Tells this connection from an earlier one of the same user in the events.
    pub(crate) connection_id: u64,
    pub(crate) channel_id: Snowflake,
    pub(crate) rtc: Box<Rtc>,
    /// The host candidate announced in the answer: packets of this participant are received as
    /// sent to it, since the socket itself listens on the wildcard address.
    pub(crate) candidate_address: SocketAddr,
    pub(crate) is_forwarded: bool,
    pub(crate) is_deafened: bool,
}

#[derive(Debug)]
pub(crate) enum MediaCommand {
    /// Adds the participant, or replaces their session (a reconnection or a move).
    Join(Box<MediaJoin>),
    Leave {
        user_id: Snowflake,
    },
    /// `is_forwarded`: their audio reaches others (they hold `SPEAK` and are not muted).
    /// `is_deafened`: no voice is sent to them.
    SetForwarding {
        user_id: Snowflake,
        is_forwarded: bool,
        is_deafened: bool,
    },
    /// `is_camera_forwarded`: their camera reaches those who watch it (they hold `STREAM` and
    /// turned it on). Every join starts with it off.
    SetCamera {
        user_id: Snowflake,
        is_camera_forwarded: bool,
    },
    /// The cameras the participant shows and the layer each tile fits, at most their number of
    /// video receive slots; replaces the previous list.
    WatchCameras {
        user_id: Snowflake,
        cameras: Vec<(Snowflake, SimulcastLayer)>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MediaEvent {
    Speaking {
        user_id: Snowflake,
        connection_id: u64,
        is_speaking: bool,
    },
    /// `listener_id`'s receive m-line `mid` now carries `speaker_id`'s voice, or camera for a
    /// video m-line.
    SlotAssigned {
        listener_id: Snowflake,
        connection_id: u64,
        mid: Mid,
        speaker_id: Snowflake,
    },
    /// The connection never came up, stopped, or carried nothing for too long.
    Ended {
        user_id: Snowflake,
        connection_id: u64,
    },
    /// The layers of `user_id`'s camera that someone watches changed.
    CameraLayers {
        user_id: Snowflake,
        connection_id: u64,
        active: Vec<SimulcastLayer>,
    },
}

/// One camera as one watcher receives it.
#[derive(Debug)]
struct WatchedCamera {
    /// The watcher's video receive m-line carrying it.
    mid: Mid,
    switch: LayerSwitch,
}

/// The simulcast layer a camera's frame belongs to, from the RIDs the web client sends (ADR-0035).
fn layer_of_rid(rid: Rid) -> Option<SimulcastLayer> {
    match &*rid {
        "h" => Some(SimulcastLayer::High),
        "l" => Some(SimulcastLayer::Low),
        "q" => Some(SimulcastLayer::Quarter),
        _ => None,
    }
}

fn rid_of_layer(layer: SimulcastLayer) -> Rid {
    Rid::from(match layer {
        SimulcastLayer::High => "h",
        SimulcastLayer::Low => "l",
        SimulcastLayer::Quarter => "q",
    })
}

/// What draining sessions found, handled once the session that produced it is no longer borrowed.
#[derive(Debug, Default)]
struct Collected {
    /// Voice and camera frames, by the participant who sent them.
    frames: Vec<(Snowflake, MediaData)>,
    /// A watcher lost packets of the camera on its video receive m-line, or its decoder failed.
    keyframe_requests: Vec<(Snowflake, Mid)>,
}

/// A participant's cameras: the one they send and those they watch (ADR-0035).
#[derive(Debug, Default)]
struct ParticipantVideo {
    /// The m-line the participant sends their camera on; `None` when their offer has no video.
    camera_mid: Option<Mid>,
    /// Video receive m-lines not given to a camera.
    free_video_slots: Vec<Mid>,
    /// The cameras this participant shows, with the layer each tile fits, as last asked.
    shown_cameras: Vec<(Snowflake, SimulcastLayer)>,
    /// The cameras this participant watches, by camera: those shown that got a slot.
    watched_cameras: HashMap<Snowflake, WatchedCamera>,
    /// Video receive slots appeared since the shown cameras were last given slots: `str0m`
    /// announces m-lines once the session is up, often after the first list.
    has_new_video_slots: bool,
    is_camera_forwarded: bool,
    /// The layers of this participant's camera last reported as watched.
    reported_camera_layers: Vec<SimulcastLayer>,
}

#[derive(Debug)]
struct Participant {
    user_id: Snowflake,
    connection_id: u64,
    channel_id: Snowflake,
    rtc: Box<Rtc>,
    candidate_address: SocketAddr,
    /// The m-line the participant sends their voice on.
    voice_mid: Option<Mid>,
    slots: ReceiveSlots<Mid, Snowflake>,
    video: ParticipantVideo,
    timeout_at: Instant,
    joined_at: Instant,
    last_received_at: Instant,
    is_connected: bool,
    is_forwarded: bool,
    is_deafened: bool,
    speaking: SpeakingDetector,
    packet_window_started_at: Instant,
    packets_in_window: u32,
    bytes_in_window: usize,
}

impl Participant {
    fn new(join: MediaJoin, now: Instant) -> Self {
        Self {
            user_id: join.user_id,
            connection_id: join.connection_id,
            channel_id: join.channel_id,
            rtc: join.rtc,
            candidate_address: join.candidate_address,
            voice_mid: None,
            slots: ReceiveSlots::default(),
            video: ParticipantVideo::default(),
            timeout_at: now,
            joined_at: now,
            last_received_at: now,
            is_connected: false,
            is_forwarded: join.is_forwarded,
            is_deafened: join.is_deafened,
            speaking: SpeakingDetector::default(),
            packet_window_started_at: now,
            packets_in_window: 0,
            bytes_in_window: 0,
        }
    }

    /// Counts a packet of `bytes`; `false` once the participant sends more than their budget
    /// allows: larger with video, largest with a camera on (ADR-0035).
    fn is_within_packet_budget(&mut self, bytes: usize, now: Instant) -> bool {
        if now.saturating_duration_since(self.packet_window_started_at) >= PACKET_RATE_WINDOW {
            self.packet_window_started_at = now;
            self.packets_in_window = 0;
            self.bytes_in_window = 0;
        }
        self.packets_in_window = self.packets_in_window.saturating_add(1);
        self.bytes_in_window = self.bytes_in_window.saturating_add(bytes);
        let (maximum_packets, maximum_bytes) = if self.video.is_camera_forwarded {
            (
                MAXIMUM_PACKETS_PER_SECOND_WITH_CAMERA,
                MAXIMUM_BYTES_PER_SECOND_WITH_CAMERA,
            )
        } else if self.video.camera_mid.is_some() {
            (
                MAXIMUM_PACKETS_PER_SECOND_WITH_VIDEO,
                MAXIMUM_BYTES_PER_SECOND_WITHOUT_CAMERA,
            )
        } else {
            (
                MAXIMUM_PACKETS_PER_SECOND,
                MAXIMUM_BYTES_PER_SECOND_WITHOUT_CAMERA,
            )
        };
        self.packets_in_window <= maximum_packets && self.bytes_in_window <= maximum_bytes
    }

    /// The watched camera on the video receive m-line `mid`.
    fn camera_on_slot(&self, mid: Mid) -> Option<(Snowflake, &WatchedCamera)> {
        self.video
            .watched_cameras
            .iter()
            .find(|(_, watched)| watched.mid == mid)
            .map(|(camera_id, watched)| (*camera_id, watched))
    }

    fn is_dead(&self, now: Instant) -> bool {
        let silent_for = now.saturating_duration_since(self.last_received_at);
        !self.rtc.is_alive()
            || (!self.is_connected
                && now.saturating_duration_since(self.joined_at) >= CONNECT_TIMEOUT)
            || (self.is_connected && silent_for >= SILENT_CONNECTION_TIMEOUT)
    }
}

/// Sends what the session wants sent, notes what it learned, collects the frames and keyframe
/// requests it received and records when it next needs time.
fn drain_output(participant: &mut Participant, socket: &UdpSocket, collected: &mut Collected) {
    loop {
        if !participant.rtc.is_alive() {
            return;
        }
        match participant.rtc.poll_output() {
            Ok(Output::Timeout(at)) => {
                participant.timeout_at = at;
                return;
            }
            Ok(Output::Transmit(transmit)) => {
                // UDP is best effort: a failed send is a lost packet, which WebRTC tolerates.
                let _ = socket.send_to(&transmit.contents, transmit.destination);
            }
            Ok(Output::Event(Event::Connected)) => participant.is_connected = true,
            Ok(Output::Event(Event::MediaAdded(added))) if added.kind == MediaKind::Audio => {
                match added.direction {
                    Direction::RecvOnly | Direction::SendRecv
                        if participant.voice_mid.is_none() =>
                    {
                        participant.voice_mid = Some(added.mid);
                    }
                    Direction::SendOnly => participant.slots.add_free_slot(added.mid),
                    _ => {}
                }
            }
            Ok(Output::Event(Event::MediaAdded(added))) if added.kind == MediaKind::Video => {
                match added.direction {
                    Direction::RecvOnly | Direction::SendRecv
                        if participant.video.camera_mid.is_none() =>
                    {
                        participant.video.camera_mid = Some(added.mid);
                    }
                    Direction::SendOnly => {
                        participant.video.free_video_slots.push(added.mid);
                        participant.video.has_new_video_slots = true;
                    }
                    _ => {}
                }
            }
            Ok(Output::Event(Event::MediaData(frame)))
                if Some(frame.mid) == participant.voice_mid
                    || (participant.video.is_camera_forwarded
                        && Some(frame.mid) == participant.video.camera_mid) =>
            {
                collected.frames.push((participant.user_id, frame));
            }
            Ok(Output::Event(Event::KeyframeRequest(request))) => {
                collected
                    .keyframe_requests
                    .push((participant.user_id, request.mid));
            }
            Ok(Output::Event(Event::IceConnectionStateChange(
                IceConnectionState::Disconnected,
            )))
            | Err(_) => participant.rtc.disconnect(),
            Ok(Output::Event(_)) => {}
        }
    }
}

/// The source addresses each participant sent from, at most
/// `MAXIMUM_SOURCE_ADDRESSES_PER_PARTICIPANT` each (the oldest is forgotten first): a client
/// changing networks adds one, and spoofed sources cannot grow the map without bound.
#[derive(Debug, Default)]
struct SourceAddresses {
    participant_by_source: HashMap<SocketAddr, Snowflake>,
    sources_by_participant: HashMap<Snowflake, VecDeque<SocketAddr>>,
}

impl SourceAddresses {
    fn participant_at(&self, source: SocketAddr) -> Option<Snowflake> {
        self.participant_by_source.get(&source).copied()
    }

    fn remember(&mut self, source: SocketAddr, user_id: Snowflake) {
        if let Some(previous_owner) = self.participant_by_source.insert(source, user_id)
            && let Some(sources) = self.sources_by_participant.get_mut(&previous_owner)
        {
            sources.retain(|known| *known != source);
        }
        let sources = self.sources_by_participant.entry(user_id).or_default();
        sources.push_back(source);
        while sources.len() > MAXIMUM_SOURCE_ADDRESSES_PER_PARTICIPANT {
            if let Some(oldest) = sources.pop_front() {
                self.participant_by_source.remove(&oldest);
            }
        }
    }

    fn forget(&mut self, user_id: Snowflake) {
        for source in self
            .sources_by_participant
            .remove(&user_id)
            .unwrap_or_default()
        {
            self.participant_by_source.remove(&source);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.participant_by_source.len()
    }
}

struct MediaLoop<'a> {
    socket: UdpSocket,
    commands: &'a Receiver<MediaCommand>,
    events: &'a UnboundedSender<MediaEvent>,
    participants: HashMap<Snowflake, Participant>,
    /// Who sends from which address: `Rtc::accepts` on everyone would cost O(n) per packet.
    sources: SourceAddresses,
    collected: Collected,
    keyframe_throttle: KeyframeThrottle<Snowflake>,
    /// The participant whose session is being driven, so a panic inside `str0m` drops only them.
    current_participant: Option<Snowflake>,
}

/// Runs until the voice service drops its command sender.
pub(crate) fn run_media_loop(
    socket: UdpSocket,
    commands: &Receiver<MediaCommand>,
    events: &UnboundedSender<MediaEvent>,
) {
    let mut media = MediaLoop {
        socket,
        commands,
        events,
        participants: HashMap::new(),
        sources: SourceAddresses::default(),
        collected: Collected::default(),
        keyframe_throttle: KeyframeThrottle::default(),
        current_participant: None,
    };
    let mut buffer = vec![0_u8; RECEIVE_BUFFER_BYTES];
    loop {
        if !media.apply_commands() {
            return;
        }
        let now = Instant::now();
        let wait = media
            .next_wake_up(now)
            .saturating_duration_since(now)
            .max(MINIMUM_WAIT);
        if let Err(error) = media.socket.set_read_timeout(Some(wait)) {
            tracing::error!(%error, "voice: cannot wait on the UDP socket; voice stops");
            return;
        }
        match media.socket.recv_from(&mut buffer) {
            Ok((length, source)) => {
                if let Some(contents) = buffer.get(..length) {
                    media.guarded(|media| media.receive(contents, source));
                }
            }
            // Timeouts, and ICMP errors some systems report on UDP sockets (an unreachable
            // client): neither stops the loop.
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => {}
        }
        let now = Instant::now();
        media.guarded(|media| media.handle_timeouts(now));
        media.guarded(|media| media.forward_frames(now));
        media.guarded(|media| media.pass_keyframe_requests(now));
        media.guarded(MediaLoop::give_new_video_slots);
        media.guarded(|media| media.tick_speaking(now));
        media.guarded(|media| media.end_dead_connections(now));
    }
}

impl MediaLoop<'_> {
    /// Runs one step of the loop. A panic in a participant's WebRTC session (a bug in `str0m`
    /// met by some packet) drops that participant alone, as a media timeout, instead of ending
    /// voice for the whole instance.
    fn guarded(&mut self, step: impl FnOnce(&mut Self)) {
        self.current_participant = None;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| step(self)));
        let culprit = self.current_participant.take();
        if outcome.is_ok() {
            return;
        }
        self.collected = Collected::default();
        tracing::error!("voice: a media session failed; its participant was disconnected");
        let Some(user_id) = culprit else {
            return;
        };
        let connection_id = self
            .participants
            .get(&user_id)
            .map(|participant| participant.connection_id);
        self.remove(user_id);
        if let Some(connection_id) = connection_id {
            self.emit(MediaEvent::Ended {
                user_id,
                connection_id,
            });
        }
    }

    /// `false` once the voice service is gone.
    fn apply_commands(&mut self) -> bool {
        loop {
            match self.commands.try_recv() {
                Ok(command) => self.guarded(|media| media.apply(command)),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn apply(&mut self, command: MediaCommand) {
        match command {
            MediaCommand::Join(join) => {
                let now = Instant::now();
                // A reconnection mid-sentence: the new session starts silent, so the old one's
                // speech ends now, or the viewers' speaking ring would stay on.
                let was_speaking = self
                    .participants
                    .get_mut(&join.user_id)
                    .and_then(|replaced| replaced.speaking.stop())
                    .is_some();
                self.remove(join.user_id);
                let mut participant = Participant::new(*join, now);
                self.current_participant = Some(participant.user_id);
                drain_output(&mut participant, &self.socket, &mut self.collected);
                let (user_id, connection_id) = (participant.user_id, participant.connection_id);
                self.participants.insert(user_id, participant);
                if was_speaking {
                    self.emit(MediaEvent::Speaking {
                        user_id,
                        connection_id,
                        is_speaking: false,
                    });
                }
            }
            MediaCommand::Leave { user_id } => self.remove(user_id),
            MediaCommand::SetForwarding {
                user_id,
                is_forwarded,
                is_deafened,
            } => {
                let Some(participant) = self.participants.get_mut(&user_id) else {
                    return;
                };
                participant.is_forwarded = is_forwarded;
                participant.is_deafened = is_deafened;
                if !is_forwarded && let Some(is_speaking) = participant.speaking.stop() {
                    let event = MediaEvent::Speaking {
                        user_id,
                        connection_id: participant.connection_id,
                        is_speaking,
                    };
                    self.emit(event);
                }
            }
            MediaCommand::SetCamera {
                user_id,
                is_camera_forwarded,
            } => self.set_camera(user_id, is_camera_forwarded),
            MediaCommand::WatchCameras { user_id, cameras } => {
                self.watch_cameras(user_id, &cameras);
            }
        }
    }

    fn set_camera(&mut self, user_id: Snowflake, is_camera_forwarded: bool) {
        let Some(camera) = self.participants.get_mut(&user_id) else {
            return;
        };
        camera.video.is_camera_forwarded = is_camera_forwarded;
        if !camera.video.is_camera_forwarded {
            // The next stream starts over: watchers wait for its first keyframe.
            self.restart_watchers_of(user_id);
        }
        self.report_camera_layers(user_id);
    }

    /// Every watcher of `camera_id` waits for a keyframe again, keeping its slot and layer.
    fn restart_watchers_of(&mut self, camera_id: Snowflake) {
        for watched in self
            .participants
            .values_mut()
            .filter_map(|watcher| watcher.video.watched_cameras.get_mut(&camera_id))
        {
            watched.switch = LayerSwitch::new(watched.switch.wanted());
        }
    }

    fn watch_cameras(&mut self, user_id: Snowflake, cameras: &[(Snowflake, SimulcastLayer)]) {
        let Some(watcher) = self.participants.get_mut(&user_id) else {
            return;
        };
        watcher.video.shown_cameras = cameras.to_vec();
        self.give_slots_to_shown_cameras(user_id);
    }

    /// Gives the shown cameras of participants whose video slots just appeared their slots.
    fn give_new_video_slots(&mut self) {
        let waiting: Vec<Snowflake> = self
            .participants
            .values_mut()
            .filter_map(|participant| {
                std::mem::take(&mut participant.video.has_new_video_slots)
                    .then_some(participant.user_id)
            })
            .collect();
        for user_id in waiting {
            self.current_participant = Some(user_id);
            self.give_slots_to_shown_cameras(user_id);
        }
    }

    /// Gives each shown camera a video receive slot (keeping those already given) and the layer
    /// its tile fits, frees the slots of cameras no longer shown, and reports the cameras whose
    /// watched layers changed.
    fn give_slots_to_shown_cameras(&mut self, user_id: Snowflake) {
        let Some(watcher) = self.participants.get_mut(&user_id) else {
            return;
        };
        let cameras = watcher.video.shown_cameras.clone();
        let mut changed_cameras = Vec::new();
        let mut slot_assignments = Vec::new();
        let free_video_slots = &mut watcher.video.free_video_slots;
        watcher.video.watched_cameras.retain(|camera_id, watched| {
            let is_still_shown = cameras.iter().any(|(shown, _)| shown == camera_id);
            if !is_still_shown {
                free_video_slots.push(watched.mid);
                changed_cameras.push(*camera_id);
            }
            is_still_shown
        });
        // A participant's own camera is shown from its local preview, never sent back.
        for (camera_id, layer) in cameras
            .iter()
            .filter(|(camera_id, _)| *camera_id != user_id)
        {
            if let Some(watched) = watcher.video.watched_cameras.get_mut(camera_id) {
                if watched.switch.wanted() != *layer {
                    watched.switch.want(*layer);
                    changed_cameras.push(*camera_id);
                }
            } else if let Some(mid) = watcher.video.free_video_slots.pop() {
                watcher.video.watched_cameras.insert(
                    *camera_id,
                    WatchedCamera {
                        mid,
                        switch: LayerSwitch::new(*layer),
                    },
                );
                changed_cameras.push(*camera_id);
                slot_assignments.push(MediaEvent::SlotAssigned {
                    listener_id: user_id,
                    connection_id: watcher.connection_id,
                    mid,
                    speaker_id: *camera_id,
                });
            }
        }
        for event in slot_assignments {
            self.emit(event);
        }
        for camera_id in changed_cameras {
            self.report_camera_layers(camera_id);
        }
    }

    /// Tells the camera which of its layers its channel watches, when that changed, so it pauses
    /// the others (ADR-0035): always the lowest while it is on, and an empty list once it is
    /// turned off.
    fn report_camera_layers(&mut self, camera_id: Snowflake) {
        let Some(camera) = self.participants.get(&camera_id) else {
            return;
        };
        let active = if camera.video.is_camera_forwarded {
            active_layers(
                self.participants
                    .values()
                    .filter(|watcher| {
                        watcher.user_id != camera_id && watcher.channel_id == camera.channel_id
                    })
                    .filter_map(|watcher| watcher.video.watched_cameras.get(&camera_id))
                    .map(|watched| &watched.switch),
            )
        } else {
            Vec::new()
        };
        if active == camera.video.reported_camera_layers {
            return;
        }
        let event = MediaEvent::CameraLayers {
            user_id: camera_id,
            connection_id: camera.connection_id,
            active: active.clone(),
        };
        if let Some(camera) = self.participants.get_mut(&camera_id) {
            camera.video.reported_camera_layers = active;
        }
        self.emit(event);
    }

    fn emit(&self, event: MediaEvent) {
        // The receiver is gone only while the server shuts down; nothing is left to tell.
        let _ = self.events.send(event);
    }

    fn remove(&mut self, user_id: Snowflake) {
        let Some(removed) = self.participants.remove(&user_id) else {
            return;
        };
        self.sources.forget(user_id);
        self.keyframe_throttle.forget(user_id);
        for listener in self.participants.values_mut() {
            listener.slots.release(user_id);
        }
        // Watchers keep their slot for this camera: a reconnection resumes it at a keyframe.
        self.restart_watchers_of(user_id);
        for camera_id in removed.video.watched_cameras.into_keys() {
            self.report_camera_layers(camera_id);
        }
    }

    fn next_wake_up(&self, now: Instant) -> Instant {
        self.participants
            .values()
            .map(|participant| participant.timeout_at)
            .fold(now + MAXIMUM_IDLE_WAIT, Instant::min)
    }

    fn input_for(
        contents: &[u8],
        source: SocketAddr,
        destination: SocketAddr,
        now: Instant,
    ) -> Option<Input<'_>> {
        Some(Input::Receive(
            now,
            Receive {
                proto: Protocol::Udp,
                source,
                destination,
                contents: contents.try_into().ok()?,
            },
        ))
    }

    fn accepts(
        participant: &Participant,
        contents: &[u8],
        source: SocketAddr,
        now: Instant,
    ) -> bool {
        Self::input_for(contents, source, participant.candidate_address, now)
            .is_some_and(|input| participant.rtc.accepts(&input))
    }

    fn receive(&mut self, contents: &[u8], source: SocketAddr) {
        if contents.len() > MAXIMUM_DATAGRAM_BYTES {
            return;
        }
        let now = Instant::now();
        let known = self.sources.participant_at(source).filter(|user_id| {
            self.participants
                .get(user_id)
                .is_some_and(|participant| Self::accepts(participant, contents, source, now))
        });
        let user_id = if let Some(user_id) = known {
            user_id
        } else {
            let Some(user_id) = self
                .participants
                .values()
                .find(|participant| Self::accepts(participant, contents, source, now))
                .map(|participant| participant.user_id)
            else {
                return;
            };
            user_id
        };
        let Some(participant) = self.participants.get_mut(&user_id) else {
            return;
        };
        if !participant.is_within_packet_budget(contents.len(), now) {
            return;
        }
        self.current_participant = Some(user_id);
        // Remembered only once the packet counts, so a flood cannot grow the map.
        if known.is_none() {
            self.sources.remember(source, user_id);
        }
        participant.last_received_at = now;
        let Some(input) = Self::input_for(contents, source, participant.candidate_address, now)
        else {
            return;
        };
        if participant.rtc.handle_input(input).is_err() {
            participant.rtc.disconnect();
        }
        drain_output(participant, &self.socket, &mut self.collected);
    }

    fn handle_timeouts(&mut self, now: Instant) {
        for participant in self
            .participants
            .values_mut()
            .filter(|participant| participant.timeout_at <= now)
        {
            self.current_participant = Some(participant.user_id);
            if participant.rtc.handle_input(Input::Timeout(now)).is_err() {
                participant.rtc.disconnect();
            }
            drain_output(participant, &self.socket, &mut self.collected);
        }
    }

    fn forward_frames(&mut self, now: Instant) {
        for (sender_id, frame) in std::mem::take(&mut self.collected.frames) {
            let is_camera_frame = self
                .participants
                .get(&sender_id)
                .is_some_and(|sender| Some(frame.mid) == sender.video.camera_mid);
            if is_camera_frame {
                self.forward_camera_frame(sender_id, &frame, now);
            } else {
                self.forward(sender_id, &frame, now);
            }
        }
    }

    /// Writes a camera's frame on the slot of every watcher of its channel that receives this
    /// layer, starting or switching watchers at a keyframe of the layer they want (ADR-0035).
    fn forward_camera_frame(&mut self, camera_id: Snowflake, frame: &MediaData, now: Instant) {
        let Some(camera) = self
            .participants
            .get(&camera_id)
            .filter(|camera| camera.video.is_camera_forwarded)
        else {
            return;
        };
        // shortcut: a camera without the web client's RIDs (no simulcast) is not forwarded; a
        // native client sending one layer would need a fourth, single-layer path.
        let Some(layer) = frame.rid.and_then(layer_of_rid) else {
            return;
        };
        let channel_id = camera.channel_id;
        let is_keyframe = frame.is_keyframe();
        let mut has_switched = false;
        let mut is_keyframe_wanted = false;
        let mut ignored = Collected::default();
        for watcher in self.participants.values_mut().filter(|watcher| {
            watcher.user_id != camera_id && watcher.channel_id == channel_id && watcher.is_connected
        }) {
            let Some(watched) = watcher.video.watched_cameras.get_mut(&camera_id) else {
                continue;
            };
            match watched.switch.on_frame(layer, is_keyframe) {
                FrameForwarding::Drop => continue,
                FrameForwarding::DropAndRequestKeyframe => {
                    is_keyframe_wanted = true;
                    continue;
                }
                FrameForwarding::Forward { is_switch } => has_switched |= is_switch,
            }
            let mid = watched.mid;
            self.current_participant = Some(watcher.user_id);
            let Some(writer) = watcher.rtc.writer(mid) else {
                continue;
            };
            let Some(payload_type) = writer.match_params(frame.params) else {
                continue;
            };
            if writer
                .write(
                    payload_type,
                    frame.network_time,
                    frame.time,
                    frame.data.clone(),
                )
                .is_err()
            {
                watcher.rtc.disconnect();
                continue;
            }
            // A watcher only receives media here; what it sends arrives through its own input.
            drain_output(watcher, &self.socket, &mut ignored);
        }
        if is_keyframe_wanted {
            self.request_keyframe(camera_id, layer, now);
        }
        if has_switched {
            self.report_camera_layers(camera_id);
        }
    }

    /// Asks the camera for a keyframe of `layer`, at most once per layer every 300 ms.
    fn request_keyframe(&mut self, camera_id: Snowflake, layer: SimulcastLayer, now: Instant) {
        if !self.keyframe_throttle.may_request(camera_id, layer, now) {
            return;
        }
        let Some(camera) = self.participants.get_mut(&camera_id) else {
            return;
        };
        let Some(mid) = camera.video.camera_mid else {
            return;
        };
        self.current_participant = Some(camera_id);
        // A layer the camera has not sent yet has no stream to ask: its first frame is a keyframe.
        if let Some(mut writer) = camera.rtc.writer(mid)
            && writer
                .request_keyframe(Some(rid_of_layer(layer)), KeyframeRequestKind::Pli)
                .is_ok()
        {
            drain_output(camera, &self.socket, &mut self.collected);
        }
    }

    /// A watcher's decoder needs a keyframe (after a loss): passed to the camera on that slot,
    /// for the layer the watcher receives.
    fn pass_keyframe_requests(&mut self, now: Instant) {
        for (watcher_id, mid) in std::mem::take(&mut self.collected.keyframe_requests) {
            let Some((camera_id, layer)) = self.participants.get(&watcher_id).and_then(|watcher| {
                let (camera_id, watched) = watcher.camera_on_slot(mid)?;
                Some((camera_id, watched.switch.forwarded()?))
            }) else {
                continue;
            };
            self.request_keyframe(camera_id, layer, now);
        }
    }

    /// Writes the frame on the speaker's slot of every other participant of the channel.
    fn forward(&mut self, speaker_id: Snowflake, frame: &MediaData, now: Instant) {
        let Some(speaker) = self.participants.get_mut(&speaker_id) else {
            return;
        };
        // Muted or without SPEAK: the server, not the client, keeps them silent (ADR-0030).
        if !speaker.is_forwarded {
            return;
        }
        self.current_participant = Some(speaker_id);
        let channel_id = speaker.channel_id;
        if let Some(is_speaking) = speaker.speaking.on_packet(frame.ext_vals.audio_level, now) {
            let event = MediaEvent::Speaking {
                user_id: speaker_id,
                connection_id: speaker.connection_id,
                is_speaking,
            };
            self.emit(event);
        }
        // A listener only receives media here; frames it sends arrive through its own input.
        let mut frames_from_listeners = Collected::default();
        let mut slot_assignments = Vec::new();
        for listener in self.participants.values_mut().filter(|listener| {
            listener.user_id != speaker_id
                && listener.channel_id == channel_id
                && listener.is_connected
                && !listener.is_deafened
        }) {
            self.current_participant = Some(listener.user_id);
            let is_new_assignment = !listener.slots.has_slot(speaker_id);
            let Some(slot) = listener.slots.slot_for(speaker_id, now) else {
                continue;
            };
            if is_new_assignment {
                slot_assignments.push(MediaEvent::SlotAssigned {
                    listener_id: listener.user_id,
                    connection_id: listener.connection_id,
                    mid: slot,
                    speaker_id,
                });
            }
            let Some(writer) = listener.rtc.writer(slot) else {
                continue;
            };
            let Some(payload_type) = writer.match_params(frame.params) else {
                continue;
            };
            // The level travels on, so browsers can show who is loud.
            let writer = match frame.ext_vals.audio_level {
                Some(level) => {
                    writer.audio_level(level, frame.ext_vals.voice_activity.unwrap_or(false))
                }
                None => writer,
            };
            if writer
                .write(
                    payload_type,
                    frame.network_time,
                    frame.time,
                    frame.data.clone(),
                )
                .is_err()
            {
                listener.rtc.disconnect();
                continue;
            }
            drain_output(listener, &self.socket, &mut frames_from_listeners);
        }
        for event in slot_assignments {
            self.emit(event);
        }
    }

    fn tick_speaking(&mut self, now: Instant) {
        let stopped: Vec<MediaEvent> = self
            .participants
            .values_mut()
            .filter_map(|participant| {
                let is_speaking = participant.speaking.on_tick(now)?;
                Some(MediaEvent::Speaking {
                    user_id: participant.user_id,
                    connection_id: participant.connection_id,
                    is_speaking,
                })
            })
            .collect();
        for event in stopped {
            self.emit(event);
        }
    }

    fn end_dead_connections(&mut self, now: Instant) {
        let dead: Vec<(Snowflake, u64)> = self
            .participants
            .values()
            .filter(|participant| participant.is_dead(now))
            .map(|participant| (participant.user_id, participant.connection_id))
            .collect();
        for (user_id, connection_id) in dead {
            self.remove(user_id);
            self.emit(MediaEvent::Ended {
                user_id,
                connection_id,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client offer with one send and one receive line.
    fn client_offer() -> String {
        let mut client = Rtc::builder()
            .clear_codecs()
            .enable_opus(true, false)
            .build(Instant::now());
        let mut change = client.sdp_api();
        change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
        change.add_media(MediaKind::Audio, Direction::RecvOnly, None, None, None);
        change.apply().unwrap().0.to_sdp_string()
    }

    #[test]
    fn a_panic_in_a_session_drops_only_that_participant() {
        str0m::crypto::from_feature_flags().install_process_default();
        let (_commands, command_receiver) = std::sync::mpsc::channel();
        let (event_sender, mut events) = tokio::sync::mpsc::unbounded_channel();
        let mut media = MediaLoop {
            socket: UdpSocket::bind("127.0.0.1:0").unwrap(),
            commands: &command_receiver,
            events: &event_sender,
            participants: HashMap::new(),
            sources: SourceAddresses::default(),
            collected: Collected::default(),
            keyframe_throttle: KeyframeThrottle::default(),
            current_participant: None,
        };
        let candidate: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        for (user_id, connection_id) in [(1, 10), (2, 20)] {
            let (rtc, _) = answer_offer(&client_offer(), candidate).unwrap();
            media.apply(MediaCommand::Join(Box::new(MediaJoin {
                user_id: Snowflake::from_raw(user_id),
                connection_id,
                channel_id: Snowflake::from_raw(5),
                rtc,
                candidate_address: candidate,
                is_forwarded: true,
                is_deafened: false,
            })));
        }

        media.guarded(|media| {
            media.current_participant = Some(Snowflake::from_raw(1));
            panic!("a bug met while driving participant 1's session");
        });
        media.guarded(|_| panic!("a bug outside any participant's session"));

        assert!(!media.participants.contains_key(&Snowflake::from_raw(1)));
        assert!(media.participants.contains_key(&Snowflake::from_raw(2)));
        assert_eq!(
            events.try_recv(),
            Ok(MediaEvent::Ended {
                user_id: Snowflake::from_raw(1),
                connection_id: 10,
            })
        );
        assert!(events.try_recv().is_err(), "nothing else ended");
    }

    #[test]
    fn each_participant_keeps_at_most_four_source_addresses() {
        let mut sources = SourceAddresses::default();
        let (alice, bob) = (Snowflake::from_raw(1), Snowflake::from_raw(2));
        let address = |port: u16| SocketAddr::from(([192, 0, 2, 1], port));
        for port in 1..=100 {
            sources.remember(address(port), alice);
        }
        sources.remember(address(1_000), bob);
        assert_eq!(sources.len(), 5);
        assert_eq!(sources.participant_at(address(100)), Some(alice));
        assert_eq!(
            sources.participant_at(address(96)),
            None,
            "oldest forgotten"
        );
        // An address that moves to another participant leaves the first one's list.
        sources.remember(address(100), bob);
        assert_eq!(sources.participant_at(address(100)), Some(bob));
        sources.forget(alice);
        assert_eq!(sources.len(), 2);
        assert_eq!(sources.participant_at(address(1_000)), Some(bob));
    }

    #[test]
    fn offers_are_answered_with_opus_dtx_and_fec_even_without_a_track_yet() {
        str0m::crypto::from_feature_flags().install_process_default();
        // A browser transceiver with no microphone track yet announces no stream.
        let without_track: String = client_offer()
            .split_inclusive('\n')
            .filter(|line| !line.starts_with("a=msid") && !line.starts_with("a=ssrc"))
            .collect();
        let candidate: SocketAddr = "127.0.0.1:50000".parse().unwrap();

        let (_, answer) = answer_offer(&without_track, candidate).unwrap();

        assert!(answer.contains("a=ice-lite"), "{answer}");
        assert!(answer.contains("opus/48000/2"), "{answer}");
        assert!(answer.contains("useinbandfec=1"), "{answer}");
        assert!(answer.contains("usedtx=1"), "{answer}");
        assert!(answer.contains("a=candidate:") && answer.contains("127.0.0.1 50000"));
        assert!(answer_offer("not sdp", candidate).is_none());
    }
}
