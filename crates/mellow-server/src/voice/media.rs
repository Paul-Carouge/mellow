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
use str0m::bwe::{Bitrate, BweKind};
use str0m::change::SdpOffer;
use str0m::format::{Codec, FormatParams};
use str0m::media::{Direction, Frequency, KeyframeRequestKind, MediaData, MediaKind, Mid, Pt, Rid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};
use tokio::sync::mpsc::UnboundedSender;

use crate::domain::video_rules::{
    FrameForwarding, LayerAllocation, LayerByteBudget, LayerCosts, LayerRate, LayerSwitch,
    MAXIMUM_SILENCE_OF_A_CAMERA, MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS,
    MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS_OF_A_WATCHER, Throttle, TileDemand, active_layers,
    arriving_layer_at_most, desired_bits_per_second, fair_share_of_upload, fit_tiles,
    is_keyframe_within_layer_size, is_layer_gone, layer_cost, watcher_video_budget,
};
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
/// A 1440p screen at 30 frames per second sends about 900 packets/s on its three layers, more with
/// keyframes and retransmissions (ADR-0037); a camera about 180.
const MAXIMUM_PACKETS_PER_SECOND_WITH_CAMERA: u32 = 3_000;
/// 12 Mbit/s averaged over a second: a 1440p screen's three layers (8.65 Mbit/s) with keyframes and
/// retransmissions (ADR-0037).
const MAXIMUM_BYTES_PER_SECOND_WITH_CAMERA: usize = 1_500_000;
/// 800 kbit/s: Opus at its highest bit rate (510 kbit/s) with RTCP for 9 watched cameras, so a
/// client with its camera off cannot make the server decrypt full packets for nothing.
const MAXIMUM_BYTES_PER_SECOND_WITHOUT_CAMERA: usize = 100_000;
/// Voice counts apart from video, so a burst of video keyframes never silences its sender
/// (ADR-0037): Opus at its highest bit rate (510 kbit/s) in its smallest packets (10 ms).
const MAXIMUM_VOICE_PACKETS_PER_SECOND: u32 = 120;
const MAXIMUM_VOICE_BYTES_PER_SECOND: usize = 70_000;
/// While a screen is shared with its sound, which has the voice's payload types: twice as much.
const MAXIMUM_VOICE_AND_SCREEN_SOUND_PACKETS_PER_SECOND: u32 = 2 * MAXIMUM_VOICE_PACKETS_PER_SECOND;
const MAXIMUM_VOICE_AND_SCREEN_SOUND_BYTES_PER_SECOND: usize = 2 * MAXIMUM_VOICE_BYTES_PER_SECOND;
const PACKET_RATE_WINDOW: Duration = Duration::from_secs(1);
/// RTP and RTCP datagrams start with a byte in this range (RFC 7983).
const RTP_OR_RTCP_FIRST_BYTES: std::ops::RangeInclusive<u8> = 128..=191;
/// An RTCP packet's second byte, its packet type, falls in this range (RFC 5761).
const RTCP_SECOND_BYTES: std::ops::RangeInclusive<u8> = 192..=223;
const RTP_PAYLOAD_TYPE_MASK: u8 = 0x7F;
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
/// Where a watcher's bandwidth estimate starts (ADR-0035). `str0m` reports it as the first
/// estimate, before any measure: high enough for every usual layout, so tiles do not start low and
/// wait 5 seconds to go up; a slower link shows loss and delay within a second or two and the
/// tiles go down then.
const INITIAL_BANDWIDTH_ESTIMATE: Bitrate = Bitrate::mbps(5);
/// How often every watcher's tiles are fitted again, besides each new estimate: picks up layers
/// whose measured rate changed or that a camera stopped sending, other watchers coming and going,
/// and the wait before going up. Half the silence of a lost layer, so a watcher falls back within
/// a second and a half of the last frame.
const REALLOCATION_INTERVAL: Duration = Duration::from_millis(500);

/// Builds the server side of a participant's session and its SDP answer. Called off the async
/// runtime: building a session generates its DTLS certificate.
///
/// `is_watching_video`: the offer has video receive m-lines, so the server estimates the
/// participant's downlink to fit the cameras they watch into it (ADR-0035).
///
/// # Errors
/// `None` when the offer does not parse or cannot be answered with Opus (and VP8 for video).
pub(crate) fn answer_offer(
    sdp_offer: &str,
    candidate_address: SocketAddr,
    is_watching_video: bool,
) -> Option<(Box<Rtc>, String)> {
    let offer = SdpOffer::from_sdp_string(sdp_offer).ok()?;
    let mut config = Rtc::builder().set_ice_lite(true).clear_codecs();
    if is_watching_video {
        config = config.enable_bwe(Some(INITIAL_BANDWIDTH_ESTIMATE));
    }
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
    /// They hold `SPEAK`: without it, their shared screen's sound is not relayed either.
    pub(crate) may_speak: bool,
    /// The m-lines for the sound of shared screens, when the offer has them (ADR-0037).
    pub(crate) screen_sound: Option<ScreenSoundMids>,
}

/// The audio m-lines of a participant's offer that carry shared screens' sound, not voices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScreenSoundMids {
    pub(crate) sent: Mid,
    pub(crate) received: Vec<Mid>,
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
        /// They hold `SPEAK`: their shared screen's sound is relayed (ADR-0037).
        may_speak: bool,
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
    /// video m-line, or shared screen's sound for a screen sound line.
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
    /// `user_id`'s camera sent nothing for `MAXIMUM_SILENCE_OF_A_CAMERA` and was turned off.
    CameraSilent {
        user_id: Snowflake,
        connection_id: u64,
    },
    /// `camera_id`'s tile is paused for `watcher_id`'s connection, too slow for its lowest layer,
    /// or flows again.
    TilePaused {
        watcher_id: Snowflake,
        connection_id: u64,
        camera_id: Snowflake,
        is_paused: bool,
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
    /// The layer the tile's size asks for.
    asked: SimulcastLayer,
    /// What the watcher's bandwidth allows of it (`None`: paused). Kept asked of the camera
    /// even while gone, so its browser sends it again once its uplink allows.
    allocated: Option<SimulcastLayer>,
    /// Wants the allocated layer, or the highest lower one still arriving.
    switch: LayerSwitch,
    /// The fitting paused the tile (a tile waiting for its first layer is not paused).
    is_paused: bool,
}

/// One tile as the fitting sees it.
struct FittedTile {
    camera_id: Snowflake,
    demand: TileDemand,
    allocated: Option<SimulcastLayer>,
    /// The layers its camera stopped sending.
    gone_layers: Vec<SimulcastLayer>,
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
    /// What each layer of their camera may still send (at most 3 entries).
    layer_budgets: HashMap<SimulcastLayer, LayerByteBudget>,
    /// When their camera was turned on or last sent a frame, to turn off one that sends nothing.
    camera_heard_at: Option<Instant>,
    /// The m-lines for shared screens' sound (ADR-0037); `None` when the offer has none.
    screen_sound: Option<ScreenSoundMids>,
    /// What each layer of their camera sent over the last second, to cost it for watchers.
    layer_rates: HashMap<SimulcastLayer, LayerRate>,
    /// Since when each layer of their camera has been asked of it (`VOICE_VIDEO_LAYERS`).
    layer_requested_since: HashMap<SimulcastLayer, Instant>,
    /// When each layer of their camera last sent a frame, to tell layers its browser dropped.
    layer_heard_at: HashMap<SimulcastLayer, Instant>,
    /// The latest estimate of their downlink in bits per second (TWCC), `None` before the first.
    estimated_bits_per_second: Option<u64>,
    /// An estimate arrived since their tiles were last fitted.
    has_new_estimate: bool,
    /// When their tiles go back up after a slow spell.
    allocation: LayerAllocation,
    /// The cameras their tiles were last fitted with, in order: another set is a new layout.
    fitted_cameras: Vec<Snowflake>,
    /// The bit rate the estimator was last told to probe up to.
    desired_bits_per_second: u64,
    /// Screen sound receive m-lines not given to a watched screen.
    free_screen_sound_slots: Vec<Mid>,
    /// The watched screens this participant hears, by sharer.
    heard_screens: HashMap<Snowflake, Mid>,
}

impl ParticipantVideo {
    fn is_screen_sound_line(&self, mid: Mid) -> bool {
        self.screen_sound
            .as_ref()
            .is_some_and(|lines| lines.sent == mid || lines.received.contains(&mid))
    }

    /// Gives the screen sound slot of `sharer_id` back, once it is no longer watched.
    fn stop_hearing(&mut self, sharer_id: Snowflake) {
        if let Some(mid) = self.heard_screens.remove(&sharer_id) {
            self.free_screen_sound_slots.push(mid);
        }
    }
}

// Independent facts about one media session (connected, forwarded, deafened, may speak).
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug)]
struct Participant {
    user_id: Snowflake,
    connection_id: u64,
    channel_id: Snowflake,
    rtc: Box<Rtc>,
    candidate_address: SocketAddr,
    /// The m-line the participant sends their voice on.
    voice_mid: Option<Mid>,
    /// The payload types of that m-line, read from the clear RTP header to count voice apart.
    voice_payload_types: Vec<u8>,
    slots: ReceiveSlots<Mid, Snowflake>,
    video: ParticipantVideo,
    timeout_at: Instant,
    joined_at: Instant,
    last_received_at: Instant,
    is_connected: bool,
    is_forwarded: bool,
    is_deafened: bool,
    may_speak: bool,
    speaking: SpeakingDetector,
    voice_packets: PacketWindow,
    other_packets: PacketWindow,
}

/// Packets and bytes counted over the current second.
#[derive(Debug)]
struct PacketWindow {
    started_at: Instant,
    packets: u32,
    bytes: usize,
}

impl PacketWindow {
    fn new(now: Instant) -> Self {
        Self {
            started_at: now,
            packets: 0,
            bytes: 0,
        }
    }

    /// Counts a packet of `bytes`; `false` once this second holds more than either maximum.
    fn counts_within(
        &mut self,
        bytes: usize,
        now: Instant,
        maximum_packets: u32,
        maximum_bytes: usize,
    ) -> bool {
        if now.saturating_duration_since(self.started_at) >= PACKET_RATE_WINDOW {
            *self = Self::new(now);
        }
        self.packets = self.packets.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes);
        self.packets <= maximum_packets && self.bytes <= maximum_bytes
    }
}

/// The payload types the remote side agreed to send on the m-line `mid`.
fn payload_types_of(rtc: &Rtc, mid: Mid) -> Vec<u8> {
    rtc.media(mid)
        .map(|media| media.remote_pts().iter().map(|pt| **pt).collect())
        .unwrap_or_default()
}

/// The payload type of an RTP datagram, read from its clear header; `None` for STUN, DTLS and RTCP.
fn rtp_payload_type_of(datagram: &[u8]) -> Option<u8> {
    let [first, second, ..] = datagram else {
        return None;
    };
    (RTP_OR_RTCP_FIRST_BYTES.contains(first) && !RTCP_SECOND_BYTES.contains(second))
        .then_some(second & RTP_PAYLOAD_TYPE_MASK)
}

impl Participant {
    fn new(join: MediaJoin, now: Instant) -> Self {
        let free_screen_sound_slots = join
            .screen_sound
            .as_ref()
            .map(|lines| lines.received.clone())
            .unwrap_or_default();
        Self {
            user_id: join.user_id,
            connection_id: join.connection_id,
            channel_id: join.channel_id,
            rtc: join.rtc,
            candidate_address: join.candidate_address,
            voice_mid: None,
            voice_payload_types: Vec::new(),
            slots: ReceiveSlots::default(),
            video: ParticipantVideo {
                screen_sound: join.screen_sound,
                free_screen_sound_slots,
                ..ParticipantVideo::default()
            },
            timeout_at: now,
            joined_at: now,
            last_received_at: now,
            is_connected: false,
            is_forwarded: join.is_forwarded,
            is_deafened: join.is_deafened,
            may_speak: join.may_speak,
            speaking: SpeakingDetector::default(),
            voice_packets: PacketWindow::new(now),
            other_packets: PacketWindow::new(now),
        }
    }

    /// Counts a datagram; `false` once the participant sends more than their budget allows:
    /// voice has its own, the rest a larger one with video, largest with a camera on (ADR-0035).
    /// A client marking video as voice only reaches the voice budget sooner.
    fn is_within_packet_budget(&mut self, datagram: &[u8], now: Instant) -> bool {
        let is_voice = rtp_payload_type_of(datagram)
            .is_some_and(|payload_type| self.voice_payload_types.contains(&payload_type));
        if is_voice {
            let (maximum_packets, maximum_bytes) =
                if self.video.is_camera_forwarded && self.video.screen_sound.is_some() {
                    (
                        MAXIMUM_VOICE_AND_SCREEN_SOUND_PACKETS_PER_SECOND,
                        MAXIMUM_VOICE_AND_SCREEN_SOUND_BYTES_PER_SECOND,
                    )
                } else {
                    (
                        MAXIMUM_VOICE_PACKETS_PER_SECOND,
                        MAXIMUM_VOICE_BYTES_PER_SECOND,
                    )
                };
            return self.voice_packets.counts_within(
                datagram.len(),
                now,
                maximum_packets,
                maximum_bytes,
            );
        }
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
        self.other_packets
            .counts_within(datagram.len(), now, maximum_packets, maximum_bytes)
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
            // Screen sound lines are audio too: their place in the offer, not their kind, tells.
            // What the sharer sends on theirs counts in the voice budget like their voice.
            Ok(Output::Event(Event::MediaAdded(added)))
                if participant.video.is_screen_sound_line(added.mid) =>
            {
                if participant
                    .video
                    .screen_sound
                    .as_ref()
                    .is_some_and(|lines| lines.sent == added.mid)
                {
                    let payload_types = payload_types_of(&participant.rtc, added.mid);
                    participant.voice_payload_types.extend(payload_types);
                }
            }
            Ok(Output::Event(Event::MediaAdded(added))) if added.kind == MediaKind::Audio => {
                match added.direction {
                    Direction::RecvOnly | Direction::SendRecv
                        if participant.voice_mid.is_none() =>
                    {
                        participant.voice_mid = Some(added.mid);
                        participant.voice_payload_types =
                            payload_types_of(&participant.rtc, added.mid);
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
                        && (Some(frame.mid) == participant.video.camera_mid
                            || (participant.may_speak
                                && Some(frame.mid)
                                    == participant
                                        .video
                                        .screen_sound
                                        .as_ref()
                                        .map(|lines| lines.sent)))) =>
            {
                collected.frames.push((participant.user_id, frame));
            }
            Ok(Output::Event(Event::EgressBitrateEstimate(BweKind::Twcc { estimate, .. }))) => {
                participant.video.estimated_bits_per_second = Some(estimate.as_u64());
                participant.video.has_new_estimate = true;
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
    /// `[voice] video_upload_mbps` in bits per second, shared among watchers (ADR-0035).
    video_upload_bits_per_second: u64,
    /// When every watcher's tiles are next fitted to their bandwidth.
    next_reallocation_at: Instant,
    /// Keyframe requests to a camera, by (camera, layer).
    keyframe_throttle: Throttle<(Snowflake, SimulcastLayer)>,
    /// Keyframe requests a watcher passes on, by (watcher, camera).
    watcher_keyframe_throttle: Throttle<(Snowflake, Snowflake)>,
    /// The participant whose session is being driven, so a panic inside `str0m` drops only them.
    current_participant: Option<Snowflake>,
}

/// Runs until the voice service drops its command sender.
/// `video_upload_bits_per_second`: from `[voice] video_upload_mbps` (ADR-0035).
pub(crate) fn run_media_loop(
    socket: UdpSocket,
    commands: &Receiver<MediaCommand>,
    events: &UnboundedSender<MediaEvent>,
    video_upload_bits_per_second: u64,
) {
    let mut media = MediaLoop {
        video_upload_bits_per_second,
        next_reallocation_at: Instant::now(),
        socket,
        commands,
        events,
        participants: HashMap::new(),
        sources: SourceAddresses::default(),
        collected: Collected::default(),
        keyframe_throttle: Throttle::new(MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS),
        watcher_keyframe_throttle: Throttle::new(
            MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS_OF_A_WATCHER,
        ),
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
        media.guarded(|media| media.follow_bandwidth(now));
        media.guarded(|media| media.tick_speaking(now));
        media.guarded(|media| media.turn_off_silent_cameras(now));
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
                may_speak,
            } => {
                let Some(participant) = self.participants.get_mut(&user_id) else {
                    return;
                };
                participant.is_forwarded = is_forwarded;
                participant.is_deafened = is_deafened;
                participant.may_speak = may_speak;
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
        let was_forwarded =
            std::mem::replace(&mut camera.video.is_camera_forwarded, is_camera_forwarded);
        if is_camera_forwarded && !was_forwarded {
            camera.video.camera_heard_at = Some(Instant::now());
        }
        if !camera.video.is_camera_forwarded {
            // The next stream (maybe a screen instead of a camera) starts over: watchers wait for
            // its first keyframe, and its layers are measured again.
            camera.video.layer_rates.clear();
            camera.video.layer_heard_at.clear();
            self.restart_watchers_of(user_id);
        }
        let mut changed_cameras = vec![user_id];
        if is_camera_forwarded != was_forwarded {
            // Its watchers' tiles change: fitted again now, not at the next periodic pass.
            let now = Instant::now();
            let watchers: Vec<Snowflake> = self
                .participants
                .values()
                .filter(|watcher| watcher.video.watched_cameras.contains_key(&user_id))
                .map(|watcher| watcher.user_id)
                .collect();
            for watcher_id in watchers {
                self.current_participant = Some(watcher_id);
                changed_cameras.extend(self.fit_tiles_of(watcher_id, now, false));
            }
        }
        for camera_id in changed_cameras {
            self.report_camera_layers(camera_id);
        }
    }

    /// The watchers forwarded `layer` of `camera_id` wait for its next keyframe.
    fn restart_watchers_on_layer(&mut self, camera_id: Snowflake, layer: SimulcastLayer) {
        for watched in self
            .participants
            .values_mut()
            .filter_map(|watcher| watcher.video.watched_cameras.get_mut(&camera_id))
            .filter(|watched| watched.switch.forwarded() == Some(layer))
        {
            watched.switch = LayerSwitch::new(watched.switch.wanted());
        }
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
    /// its tile fits within the watcher's bandwidth, frees the slots of cameras no longer shown,
    /// and reports the cameras whose watched layers changed.
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
        // Only cameras no longer shown are in the list yet: their sound stops with them.
        for camera_id in &changed_cameras {
            watcher.video.stop_hearing(*camera_id);
        }
        let mut is_new_layout = !changed_cameras.is_empty();
        // A participant's own camera is shown from its local preview, never sent back.
        for (camera_id, layer) in cameras
            .iter()
            .filter(|(camera_id, _)| *camera_id != user_id)
        {
            if let Some(watched) = watcher.video.watched_cameras.get_mut(camera_id) {
                is_new_layout |= watched.asked != *layer;
                watched.asked = *layer;
            } else if let Some(mid) = watcher.video.free_video_slots.pop() {
                // The layer comes from the fitting below; until then the tile waits.
                watcher.video.watched_cameras.insert(
                    *camera_id,
                    WatchedCamera {
                        mid,
                        asked: *layer,
                        allocated: None,
                        switch: LayerSwitch::new(None),
                        is_paused: false,
                    },
                );
                is_new_layout = true;
                slot_assignments.push(MediaEvent::SlotAssigned {
                    listener_id: user_id,
                    connection_id: watcher.connection_id,
                    mid,
                    speaker_id: *camera_id,
                });
            }
        }
        // Before any pause of the same tiles: a tile starts as not paused, at its slot assignment.
        for event in slot_assignments {
            self.emit(event);
        }
        // The same list again keeps the wait before going up: resending it does not skip it.
        changed_cameras.extend(self.fit_tiles_of(user_id, Instant::now(), is_new_layout));
        for camera_id in changed_cameras {
            self.report_camera_layers(camera_id);
        }
    }

    /// What each layer of `camera_id` costs a watcher: what it sent over the last second, and at
    /// least its target.
    fn layer_costs_of(&self, camera_id: Snowflake, now: Instant) -> LayerCosts {
        let cost = |layer| {
            let measured = self
                .participants
                .get(&camera_id)
                .and_then(|camera| camera.video.layer_rates.get(&layer))
                .map_or(0, |rate| rate.bits_per_second(now));
            layer_cost(layer, measured)
        };
        LayerCosts {
            quarter: cost(SimulcastLayer::Quarter),
            low: cost(SimulcastLayer::Low),
            high: cost(SimulcastLayer::High),
        }
    }

    /// Fits `watcher_id`'s tiles into their bandwidth (ADR-0035) and returns the cameras whose
    /// wanted layer changed. `is_new_layout`: the tiles changed, so they get what fits at once;
    /// otherwise they go back up only after a steady spell (`LayerAllocation`).
    fn fit_tiles_of(
        &mut self,
        watcher_id: Snowflake,
        now: Instant,
        is_new_layout: bool,
    ) -> Vec<Snowflake> {
        let watchers = self
            .participants
            .values()
            .filter(|participant| {
                participant
                    .video
                    .watched_cameras
                    .keys()
                    .any(|camera_id| self.is_camera_on_in(*camera_id, participant.channel_id))
            })
            .count();
        let Some(watcher) = self.participants.get(&watcher_id) else {
            return Vec::new();
        };
        // In the order the watcher lists them: among equal tiles, the last listed gives way first.
        // Only cameras on in the watcher's channel count: any other id in a watch list receives
        // nothing, and must not reveal another channel's camera through its cost.
        let tiles: Vec<FittedTile> = watcher
            .video
            .shown_cameras
            .iter()
            .filter(|(camera_id, _)| self.is_camera_on_in(*camera_id, watcher.channel_id))
            .filter_map(|(camera_id, _)| {
                let tile = watcher.video.watched_cameras.get(camera_id)?;
                Some(FittedTile {
                    camera_id: *camera_id,
                    demand: TileDemand {
                        asked: tile.asked,
                        costs: self.layer_costs_of(*camera_id, now),
                    },
                    allocated: tile.allocated,
                    gone_layers: self.gone_layers_of(*camera_id, now),
                })
            })
            .collect();
        let fair_share = fair_share_of_upload(self.video_upload_bits_per_second, watchers);
        let budget = watcher_video_budget(watcher.video.estimated_bits_per_second, fair_share);
        let demands: Vec<TileDemand> = tiles.iter().map(|tile| tile.demand).collect();
        let Some(watcher) = self.participants.get_mut(&watcher_id) else {
            return Vec::new();
        };
        // A camera turned on, off or moved in or out of the channel changes the tiles too.
        let cameras: Vec<Snowflake> = tiles.iter().map(|tile| tile.camera_id).collect();
        let is_new_layout = is_new_layout || cameras != watcher.video.fitted_cameras;
        watcher.video.fitted_cameras = cameras;
        let layers = if is_new_layout {
            watcher.video.allocation = LayerAllocation::default();
            fit_tiles(&demands, budget)
        } else {
            let current: Vec<Option<SimulcastLayer>> =
                tiles.iter().map(|tile| tile.allocated).collect();
            watcher
                .video
                .allocation
                .next_layers(&demands, &current, budget, now)
        };
        let mut changed_cameras = Vec::new();
        let mut pauses = Vec::new();
        for (tile, layer) in tiles.iter().zip(layers) {
            let Some(shown) = watcher.video.watched_cameras.get_mut(&tile.camera_id) else {
                continue;
            };
            let arriving = layer.map(|layer| {
                arriving_layer_at_most(layer, |candidate| tile.gone_layers.contains(&candidate))
            });
            if shown.allocated != layer || shown.switch.wanted() != arriving {
                shown.allocated = layer;
                shown.switch.want(arriving);
                changed_cameras.push(tile.camera_id);
            }
            if shown.is_paused != layer.is_none() {
                shown.is_paused = layer.is_none();
                pauses.push(MediaEvent::TilePaused {
                    watcher_id,
                    connection_id: watcher.connection_id,
                    camera_id: tile.camera_id,
                    is_paused: shown.is_paused,
                });
            }
        }
        let desired_bits_per_second = desired_bits_per_second(&demands, fair_share);
        if desired_bits_per_second != watcher.video.desired_bits_per_second {
            watcher.video.desired_bits_per_second = desired_bits_per_second;
            self.current_participant = Some(watcher_id);
            watcher
                .rtc
                .bwe()
                .set_desired_bitrate(Bitrate::bps(desired_bits_per_second));
        }
        for event in pauses {
            self.emit(event);
        }
        changed_cameras
    }

    /// Fits again the tiles of watchers with a new estimate, and every watcher's once per
    /// `REALLOCATION_INTERVAL`.
    fn follow_bandwidth(&mut self, now: Instant) {
        let is_periodic = now >= self.next_reallocation_at;
        if is_periodic {
            self.next_reallocation_at = now + REALLOCATION_INTERVAL;
        }
        let due: Vec<Snowflake> = self
            .participants
            .values_mut()
            .filter_map(|participant| {
                let has_new_estimate = std::mem::take(&mut participant.video.has_new_estimate);
                ((has_new_estimate || is_periodic) && !participant.video.watched_cameras.is_empty())
                    .then_some(participant.user_id)
            })
            .collect();
        for watcher_id in due {
            self.current_participant = Some(watcher_id);
            for camera_id in self.fit_tiles_of(watcher_id, now, false) {
                self.report_camera_layers(camera_id);
            }
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
            let watched: Vec<&WatchedCamera> = self
                .participants
                .values()
                .filter(|watcher| {
                    watcher.user_id != camera_id && watcher.channel_id == camera.channel_id
                })
                .filter_map(|watcher| watcher.video.watched_cameras.get(&camera_id))
                .collect();
            let mut active = active_layers(watched.iter().map(|watched| &watched.switch));
            // A layer gone stays asked for: the browser sends it again once its uplink allows.
            active.extend(watched.iter().filter_map(|watched| watched.allocated));
            active.sort_unstable();
            active.dedup();
            active
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
            let now = Instant::now();
            let requested_since = &mut camera.video.layer_requested_since;
            requested_since.retain(|layer, _| active.contains(layer));
            for layer in &active {
                requested_since.entry(*layer).or_insert(now);
            }
            camera.video.reported_camera_layers = active;
        }
        self.emit(event);
    }

    /// The layers `camera_id` was asked for but stopped sending (ADR-0035).
    fn gone_layers_of(&self, camera_id: Snowflake, now: Instant) -> Vec<SimulcastLayer> {
        let Some(camera) = self.participants.get(&camera_id) else {
            return Vec::new();
        };
        [SimulcastLayer::Low, SimulcastLayer::High]
            .into_iter()
            .filter(|layer| {
                is_layer_gone(
                    camera.video.layer_requested_since.get(layer).copied(),
                    camera.video.layer_heard_at.get(layer).copied(),
                    now,
                )
            })
            .collect()
    }

    /// Whether `camera_id` is a camera turned on in `channel_id`, so watchers there receive it.
    fn is_camera_on_in(&self, camera_id: Snowflake, channel_id: Snowflake) -> bool {
        self.participants.get(&camera_id).is_some_and(|camera| {
            camera.channel_id == channel_id && camera.video.is_camera_forwarded
        })
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
        self.keyframe_throttle
            .forget(|(camera_id, _)| *camera_id == user_id);
        self.watcher_keyframe_throttle
            .forget(|(watcher_id, camera_id)| *watcher_id == user_id || *camera_id == user_id);
        for listener in self.participants.values_mut() {
            listener.slots.release(user_id);
            listener.video.stop_hearing(user_id);
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
        if !participant.is_within_packet_budget(contents, now) {
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
            let Some(sender) = self.participants.get(&sender_id) else {
                continue;
            };
            if Some(frame.mid) == sender.video.camera_mid {
                self.forward_camera_frame(sender_id, &frame, now);
            } else if sender.video.is_screen_sound_line(frame.mid) {
                self.forward_screen_sound(sender_id, &frame);
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
            .get_mut(&camera_id)
            .filter(|camera| camera.video.is_camera_forwarded)
        else {
            return;
        };
        // shortcut: a camera without the web client's RIDs (no simulcast) is not forwarded; a
        // native client sending one layer would need a fourth, single-layer path.
        let Some(layer) = frame.rid.and_then(layer_of_rid) else {
            return;
        };
        let is_keyframe = frame.is_keyframe();
        // A keyframe declaring a picture too large for its layer reaches no one (ADR-0035).
        if is_keyframe && !is_keyframe_within_layer_size(layer, &frame.data) {
            return;
        }
        // Over its layer's budget, the frame reaches no one (ADR-0035): a client cannot send a
        // large picture under the name of a small layer.
        if !camera
            .video
            .layer_budgets
            .entry(layer)
            .or_insert_with(|| LayerByteBudget::new(layer, now))
            .has_room_for(frame.data.len(), now)
        {
            // Watchers on this layer miss a frame its next ones refer to: they wait for a
            // keyframe instead of decoding a broken picture.
            self.restart_watchers_on_layer(camera_id, layer);
            self.request_keyframe(camera_id, layer, now);
            return;
        }
        camera.video.camera_heard_at = Some(now);
        camera.video.layer_heard_at.insert(layer, now);
        let channel_id = camera.channel_id;
        let mut has_switched = false;
        let mut is_keyframe_wanted = false;
        let mut has_reached_a_watcher = false;
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
            has_reached_a_watcher = true;
            // A watcher only receives media here; what it sends arrives through its own input.
            drain_output(watcher, &self.socket, &mut ignored);
        }
        // Only frames someone received count, in the budget and in the layer's measured cost: a
        // layer sent to no one cannot make itself look costly to push other tiles down.
        if has_reached_a_watcher && let Some(camera) = self.participants.get_mut(&camera_id) {
            if let Some(budget) = camera.video.layer_budgets.get_mut(&layer) {
                budget.spend(frame.data.len());
            }
            camera
                .video
                .layer_rates
                .entry(layer)
                .or_insert_with(|| LayerRate::new(now))
                .record(frame.data.len(), now);
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
        if !self.keyframe_throttle.may_pass((camera_id, layer), now) {
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
            if self
                .watcher_keyframe_throttle
                .may_pass((watcher_id, camera_id), now)
            {
                self.request_keyframe(camera_id, layer, now);
            }
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

    /// Writes a shared screen's sound to the participants of its channel watching that screen
    /// (ADR-0037), on a screen sound slot given at its first frame. It is not a voice: no speaking
    /// ring, and muting the microphone does not stop it; `STREAM`, `SPEAK` and deafening do.
    fn forward_screen_sound(&mut self, sharer_id: Snowflake, frame: &MediaData) {
        // Collected before a `SetCamera` false applied since: it stops here too.
        let Some(sharer) = self
            .participants
            .get(&sharer_id)
            .filter(|sharer| sharer.video.is_camera_forwarded && sharer.may_speak)
        else {
            return;
        };
        let channel_id = sharer.channel_id;
        let mut ignored = Collected::default();
        let mut slot_assignments = Vec::new();
        for watcher in self.participants.values_mut().filter(|watcher| {
            watcher.user_id != sharer_id
                && watcher.channel_id == channel_id
                && watcher.is_connected
                && !watcher.is_deafened
                && watcher.video.watched_cameras.contains_key(&sharer_id)
        }) {
            self.current_participant = Some(watcher.user_id);
            let slot = if let Some(slot) = watcher.video.heard_screens.get(&sharer_id) {
                *slot
            } else {
                let Some(slot) = watcher.video.free_screen_sound_slots.pop() else {
                    continue;
                };
                watcher.video.heard_screens.insert(sharer_id, slot);
                slot_assignments.push(MediaEvent::SlotAssigned {
                    listener_id: watcher.user_id,
                    connection_id: watcher.connection_id,
                    mid: slot,
                    speaker_id: sharer_id,
                });
                slot
            };
            let Some(writer) = watcher.rtc.writer(slot) else {
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
            drain_output(watcher, &self.socket, &mut ignored);
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

    /// Turns off the cameras that sent nothing for `MAXIMUM_SILENCE_OF_A_CAMERA`, and tells the
    /// voice service, which turns `self_video` off.
    fn turn_off_silent_cameras(&mut self, now: Instant) {
        let silent: Vec<(Snowflake, u64)> = self
            .participants
            .values()
            .filter(|camera| {
                camera.video.is_camera_forwarded
                    && camera.video.camera_heard_at.is_some_and(|heard_at| {
                        now.saturating_duration_since(heard_at) >= MAXIMUM_SILENCE_OF_A_CAMERA
                    })
            })
            .map(|camera| (camera.user_id, camera.connection_id))
            .collect();
        for (user_id, connection_id) in silent {
            self.set_camera(user_id, false);
            self.emit(MediaEvent::CameraSilent {
                user_id,
                connection_id,
            });
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

    fn media_loop<'a>(
        commands: &'a Receiver<MediaCommand>,
        events: &'a UnboundedSender<MediaEvent>,
    ) -> MediaLoop<'a> {
        str0m::crypto::from_feature_flags().install_process_default();
        MediaLoop {
            video_upload_bits_per_second: u64::MAX,
            next_reallocation_at: Instant::now(),
            socket: UdpSocket::bind("127.0.0.1:0").unwrap(),
            commands,
            events,
            participants: HashMap::new(),
            sources: SourceAddresses::default(),
            collected: Collected::default(),
            keyframe_throttle: Throttle::new(MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS),
            watcher_keyframe_throttle: Throttle::new(
                MINIMUM_INTERVAL_BETWEEN_KEYFRAME_REQUESTS_OF_A_WATCHER,
            ),
            current_participant: None,
        }
    }

    #[test]
    fn rtp_payload_types_are_read_only_from_rtp_datagrams() {
        let opus_with_marker = [0x80, 0x80 | OPUS_PAYLOAD_TYPE, 0, 1];
        assert_eq!(
            rtp_payload_type_of(&opus_with_marker),
            Some(OPUS_PAYLOAD_TYPE)
        );
        assert_eq!(
            rtp_payload_type_of(&[0x80, 200, 0, 1]),
            None,
            "RTCP receiver report"
        );
        assert_eq!(
            rtp_payload_type_of(&[0x00, 0x01, 0, 0]),
            None,
            "STUN binding request"
        );
        assert_eq!(
            rtp_payload_type_of(&[22, 0xFE, 0xFD]),
            None,
            "DTLS handshake"
        );
        assert_eq!(rtp_payload_type_of(&[0x80]), None);
    }

    #[test]
    fn a_burst_of_video_does_not_silence_its_sender() {
        let (_commands, command_receiver) = std::sync::mpsc::channel();
        let (event_sender, _events) = tokio::sync::mpsc::unbounded_channel();
        let mut media = media_loop(&command_receiver, &event_sender);
        let candidate: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        let (rtc, answer) = answer_offer(&client_offer(), candidate, false).unwrap();
        // The client's send line is the offer's first m-line; `str0m` announces it once connected.
        let voice_mid = answer
            .lines()
            .find_map(|line| line.strip_prefix("a=mid:"))
            .map(Mid::from)
            .unwrap();
        let sharer_id = Snowflake::from_raw(1);
        media.apply(MediaCommand::Join(Box::new(MediaJoin {
            user_id: sharer_id,
            connection_id: 10,
            channel_id: Snowflake::from_raw(5),
            rtc,
            candidate_address: candidate,
            is_forwarded: true,
            is_deafened: false,
            may_speak: true,
            screen_sound: None,
        })));
        let sharer = media.participants.get_mut(&sharer_id).unwrap();
        sharer.voice_payload_types = payload_types_of(&sharer.rtc, voice_mid);
        assert_eq!(sharer.voice_payload_types, vec![OPUS_PAYLOAD_TYPE]);
        sharer.video.is_camera_forwarded = true;
        let now = Instant::now();
        let mut video = vec![0_u8; MAXIMUM_DATAGRAM_BYTES];
        video[..2].copy_from_slice(&[0x80, 96]);
        let voice = [0x80, OPUS_PAYLOAD_TYPE, 0, 1];

        let video_packets_within_budget =
            std::iter::repeat_with(|| sharer.is_within_packet_budget(&video, now))
                .take(2_000)
                .filter(|is_within| *is_within)
                .count();

        assert_eq!(
            video_packets_within_budget,
            MAXIMUM_BYTES_PER_SECOND_WITH_CAMERA / MAXIMUM_DATAGRAM_BYTES
        );
        assert!(
            sharer.is_within_packet_budget(&voice, now),
            "voice still goes through"
        );
        let voice_packets_within_budget =
            std::iter::repeat_with(|| sharer.is_within_packet_budget(&voice, now))
                .take(1_000)
                .filter(|is_within| *is_within)
                .count();
        assert_eq!(
            voice_packets_within_budget,
            usize::try_from(MAXIMUM_VOICE_PACKETS_PER_SECOND).unwrap() - 1,
            "voice has a budget of its own, not an exemption"
        );
        assert!(sharer.is_within_packet_budget(&voice, now + PACKET_RATE_WINDOW));
    }

    #[test]
    fn a_camera_that_sends_nothing_is_turned_off_and_reported() {
        let (_commands, command_receiver) = std::sync::mpsc::channel();
        let (event_sender, mut events) = tokio::sync::mpsc::unbounded_channel();
        let mut media = media_loop(&command_receiver, &event_sender);
        let candidate: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        let (rtc, _) = answer_offer(&client_offer(), candidate, false).unwrap();
        let camera_id = Snowflake::from_raw(1);
        media.apply(MediaCommand::Join(Box::new(MediaJoin {
            user_id: camera_id,
            connection_id: 10,
            channel_id: Snowflake::from_raw(5),
            rtc,
            candidate_address: candidate,
            is_forwarded: true,
            is_deafened: false,
            may_speak: true,
            screen_sound: None,
        })));
        media.apply(MediaCommand::SetCamera {
            user_id: camera_id,
            is_camera_forwarded: true,
        });
        let turned_on_at = media.participants[&camera_id]
            .video
            .camera_heard_at
            .unwrap();
        assert_eq!(
            events.try_recv(),
            Ok(MediaEvent::CameraLayers {
                user_id: camera_id,
                connection_id: 10,
                active: vec![SimulcastLayer::Quarter],
            })
        );

        media.turn_off_silent_cameras(turned_on_at + MAXIMUM_SILENCE_OF_A_CAMERA / 2);
        assert!(events.try_recv().is_err(), "still starting");
        media.turn_off_silent_cameras(turned_on_at + MAXIMUM_SILENCE_OF_A_CAMERA);

        assert!(!media.participants[&camera_id].video.is_camera_forwarded);
        assert_eq!(
            events.try_recv(),
            Ok(MediaEvent::CameraLayers {
                user_id: camera_id,
                connection_id: 10,
                active: Vec::new(),
            })
        );
        assert_eq!(
            events.try_recv(),
            Ok(MediaEvent::CameraSilent {
                user_id: camera_id,
                connection_id: 10,
            })
        );
        media.turn_off_silent_cameras(turned_on_at + MAXIMUM_SILENCE_OF_A_CAMERA * 2);
        assert!(events.try_recv().is_err(), "reported once");
    }

    /// A camera in channel 5 and a watcher with one video slot in `watcher_channel_id`.
    fn join_camera_and_watcher(
        media: &mut MediaLoop<'_>,
        watcher_channel_id: Snowflake,
    ) -> (Snowflake, Snowflake) {
        let candidate: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        let (camera_id, watcher_id) = (Snowflake::from_raw(1), Snowflake::from_raw(2));
        for (user_id, connection_id, channel_id) in [
            (camera_id, 10, Snowflake::from_raw(5)),
            (watcher_id, 20, watcher_channel_id),
        ] {
            let (rtc, _) = answer_offer(&client_offer(), candidate, false).unwrap();
            media.apply(MediaCommand::Join(Box::new(MediaJoin {
                user_id,
                connection_id,
                channel_id,
                rtc,
                candidate_address: candidate,
                is_forwarded: true,
                is_deafened: false,
                may_speak: true,
                screen_sound: None,
            })));
        }
        // `str0m` announces video slots once connected; this session never connects.
        media
            .participants
            .get_mut(&watcher_id)
            .unwrap()
            .video
            .free_video_slots
            .push(Mid::from("v1"));
        (camera_id, watcher_id)
    }

    #[test]
    fn a_camera_of_another_channel_in_a_watch_list_is_neither_fitted_nor_counted() {
        let (_commands, command_receiver) = std::sync::mpsc::channel();
        let (event_sender, _events) = tokio::sync::mpsc::unbounded_channel();
        let mut media = media_loop(&command_receiver, &event_sender);
        let (camera_id, watcher_id) = join_camera_and_watcher(&mut media, Snowflake::from_raw(6));
        media.set_camera(camera_id, true);
        media.watch_cameras(watcher_id, &[(camera_id, SimulcastLayer::High)]);
        let watcher = &media.participants[&watcher_id].video;
        assert_eq!(watcher.watched_cameras[&camera_id].switch.wanted(), None);
        assert_eq!(watcher.desired_bits_per_second, 0, "its cost is never read");
        assert_eq!(watcher.fitted_cameras, []);
    }

    #[test]
    fn a_tile_pauses_when_its_watcher_slows_down_and_flows_again_five_seconds_after() {
        let (_commands, command_receiver) = std::sync::mpsc::channel();
        let (event_sender, mut events) = tokio::sync::mpsc::unbounded_channel();
        let mut media = media_loop(&command_receiver, &event_sender);
        let (camera_id, watcher_id) = join_camera_and_watcher(&mut media, Snowflake::from_raw(5));
        media.set_camera(camera_id, true);
        let large_tile = [(camera_id, SimulcastLayer::High)];
        media.watch_cameras(watcher_id, &large_tile);
        let pauses = |events: &mut tokio::sync::mpsc::UnboundedReceiver<MediaEvent>| {
            std::iter::from_fn(|| events.try_recv().ok())
                .filter_map(|event| match event {
                    MediaEvent::TilePaused { is_paused, .. } => Some(is_paused),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let wanted = |media: &MediaLoop<'_>| {
            media.participants[&watcher_id].video.watched_cameras[&camera_id]
                .switch
                .wanted()
        };
        assert_eq!(wanted(&media), Some(SimulcastLayer::High));
        assert_eq!(pauses(&mut events), [false; 0]);

        let start = Instant::now();
        let estimate = |media: &mut MediaLoop<'_>, bits_per_second, seconds| {
            let watcher = media.participants.get_mut(&watcher_id).unwrap();
            watcher.video.estimated_bits_per_second = Some(bits_per_second);
            watcher.video.has_new_estimate = true;
            media.follow_bandwidth(start + Duration::from_secs(seconds));
        };
        estimate(&mut media, 1_000_000, 0);
        assert_eq!(wanted(&media), Some(SimulcastLayer::Low));
        estimate(&mut media, 100_000, 1);
        assert_eq!(wanted(&media), None);
        assert_eq!(pauses(&mut events), [true]);

        estimate(&mut media, 10_000_000, 2);
        media.watch_cameras(watcher_id, &large_tile);
        estimate(&mut media, 10_000_000, 6);
        assert_eq!(
            wanted(&media),
            None,
            "the same tiles again do not skip the wait"
        );
        estimate(&mut media, 10_000_000, 7);
        assert_eq!(wanted(&media), Some(SimulcastLayer::High));
        assert_eq!(pauses(&mut events), [false]);
    }

    #[test]
    fn a_watcher_falls_back_from_a_layer_its_camera_stopped_and_returns_when_it_comes_back() {
        let (_commands, command_receiver) = std::sync::mpsc::channel();
        let (event_sender, mut events) = tokio::sync::mpsc::unbounded_channel();
        let mut media = media_loop(&command_receiver, &event_sender);
        let (camera_id, watcher_id) = join_camera_and_watcher(&mut media, Snowflake::from_raw(5));
        media.set_camera(camera_id, true);
        media.watch_cameras(watcher_id, &[(camera_id, SimulcastLayer::High)]);
        let start = Instant::now();
        let at = |milliseconds| start + Duration::from_millis(milliseconds);
        let wanted = |media: &MediaLoop<'_>| {
            media.participants[&watcher_id].video.watched_cameras[&camera_id]
                .switch
                .wanted()
        };
        let heard = |media: &mut MediaLoop<'_>, layer, at| {
            let camera = media.participants.get_mut(&camera_id).unwrap();
            camera.video.layer_heard_at.insert(layer, at);
        };
        heard(&mut media, SimulcastLayer::Low, at(1_400));
        media.follow_bandwidth(at(900));
        assert_eq!(wanted(&media), Some(SimulcastLayer::High), "just asked");

        media.next_reallocation_at = at(0);
        media.follow_bandwidth(at(1_500));
        assert_eq!(wanted(&media), Some(SimulcastLayer::Low));
        let reported = std::iter::from_fn(|| events.try_recv().ok())
            .filter_map(|event| match event {
                MediaEvent::CameraLayers { active, .. } => Some(active),
                _ => None,
            })
            .last();
        assert_eq!(
            reported,
            Some(vec![
                SimulcastLayer::Quarter,
                SimulcastLayer::Low,
                SimulcastLayer::High
            ]),
            "the layer gone stays asked for, next to the fallback"
        );

        heard(&mut media, SimulcastLayer::High, at(1_600));
        media.next_reallocation_at = at(0);
        media.follow_bandwidth(at(1_700));
        assert_eq!(wanted(&media), Some(SimulcastLayer::High));
    }

    #[test]
    fn a_panic_in_a_session_drops_only_that_participant() {
        let (_commands, command_receiver) = std::sync::mpsc::channel();
        let (event_sender, mut events) = tokio::sync::mpsc::unbounded_channel();
        let mut media = media_loop(&command_receiver, &event_sender);
        let candidate: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        for (user_id, connection_id) in [(1, 10), (2, 20)] {
            let (rtc, _) = answer_offer(&client_offer(), candidate, false).unwrap();
            media.apply(MediaCommand::Join(Box::new(MediaJoin {
                user_id: Snowflake::from_raw(user_id),
                connection_id,
                channel_id: Snowflake::from_raw(5),
                rtc,
                candidate_address: candidate,
                is_forwarded: true,
                is_deafened: false,
                may_speak: true,
                screen_sound: None,
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

        let (_, answer) = answer_offer(&without_track, candidate, false).unwrap();

        assert!(answer.contains("a=ice-lite"), "{answer}");
        assert!(answer.contains("opus/48000/2"), "{answer}");
        assert!(answer.contains("useinbandfec=1"), "{answer}");
        assert!(answer.contains("usedtx=1"), "{answer}");
        assert!(answer.contains("a=candidate:") && answer.contains("127.0.0.1 50000"));
        assert!(answer_offer("not sdp", candidate, false).is_none());
    }
}
