//! The media loop (ADR-0030): one OS thread outside the Tokio runtime owns the UDP socket and every
//! participant's WebRTC session (`str0m`, sans-IO). It forwards each speaker's Opus packets, as
//! they are, to the other participants of their channel, and reports speaking edges and dead
//! connections. It never touches SQLite: permissions arrive already decided, as commands.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use mellow_protocol::Snowflake;
use str0m::change::SdpOffer;
use str0m::format::{Codec, FormatParams};
use str0m::media::{Direction, Frequency, MediaData, MediaKind, Mid, Pt};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};
use tokio::sync::mpsc::UnboundedSender;

use crate::domain::voice_rules::{ReceiveSlots, SpeakingDetector};

/// Larger datagrams are not WebRTC audio: dropped unread.
const MAXIMUM_DATAGRAM_BYTES: usize = 1_500;
/// Bigger than `MAXIMUM_DATAGRAM_BYTES`, so an oversized datagram is seen whole and dropped.
const RECEIVE_BUFFER_BYTES: usize = 2_048;
/// Opus sends 50 packets/s; STUN, DTLS and RTCP add a few. More is a flood: dropped.
const MAXIMUM_PACKETS_PER_SECOND: u32 = 200;
const PACKET_RATE_WINDOW: Duration = Duration::from_secs(1);
/// Commands and speaking hangovers are looked at least this often.
const MAXIMUM_IDLE_WAIT: Duration = Duration::from_millis(50);
/// A zero read timeout means "block forever" to the OS.
const MINIMUM_WAIT: Duration = Duration::from_millis(1);
/// ICE and DTLS must complete within this time after the answer.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
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
/// `None` when the offer does not parse or cannot be answered with Opus.
pub(crate) fn answer_offer(
    sdp_offer: &str,
    candidate_address: SocketAddr,
) -> Option<(Box<Rtc>, String)> {
    let offer = SdpOffer::from_sdp_string(sdp_offer).ok()?;
    let mut config = Rtc::builder().set_ice_lite(true).clear_codecs();
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
    /// `is_deafened`: nothing is sent to them.
    SetForwarding {
        user_id: Snowflake,
        is_forwarded: bool,
        is_deafened: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaEvent {
    Speaking {
        user_id: Snowflake,
        connection_id: u64,
        is_speaking: bool,
    },
    /// The connection never came up, stopped, or carried nothing for too long.
    Ended {
        user_id: Snowflake,
        connection_id: u64,
    },
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
    timeout_at: Instant,
    joined_at: Instant,
    last_received_at: Instant,
    is_connected: bool,
    is_forwarded: bool,
    is_deafened: bool,
    speaking: SpeakingDetector,
    packet_window_started_at: Instant,
    packets_in_window: u32,
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
            timeout_at: now,
            joined_at: now,
            last_received_at: now,
            is_connected: false,
            is_forwarded: join.is_forwarded,
            is_deafened: join.is_deafened,
            speaking: SpeakingDetector::default(),
            packet_window_started_at: now,
            packets_in_window: 0,
        }
    }

    /// Counts a packet; `false` once the participant sends more than the rate allows.
    fn is_within_packet_rate(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.packet_window_started_at) >= PACKET_RATE_WINDOW {
            self.packet_window_started_at = now;
            self.packets_in_window = 0;
        }
        self.packets_in_window = self.packets_in_window.saturating_add(1);
        self.packets_in_window <= MAXIMUM_PACKETS_PER_SECOND
    }

    fn is_dead(&self, now: Instant) -> bool {
        let silent_for = now.saturating_duration_since(self.last_received_at);
        !self.rtc.is_alive()
            || (!self.is_connected
                && now.saturating_duration_since(self.joined_at) >= CONNECT_TIMEOUT)
            || (self.is_connected && silent_for >= SILENT_CONNECTION_TIMEOUT)
    }
}

/// Sends what the session wants sent, notes what it learned, collects the voice frames it
/// received and records when it next needs time.
fn drain_output(
    participant: &mut Participant,
    socket: &UdpSocket,
    frames_to_forward: &mut Vec<(Snowflake, MediaData)>,
) {
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
            Ok(Output::Event(Event::MediaData(frame)))
                if Some(frame.mid) == participant.voice_mid =>
            {
                frames_to_forward.push((participant.user_id, frame));
            }
            Ok(Output::Event(Event::IceConnectionStateChange(
                IceConnectionState::Disconnected,
            )))
            | Err(_) => participant.rtc.disconnect(),
            Ok(Output::Event(_)) => {}
        }
    }
}

struct MediaLoop<'a> {
    socket: UdpSocket,
    commands: &'a Receiver<MediaCommand>,
    events: &'a UnboundedSender<MediaEvent>,
    participants: HashMap<Snowflake, Participant>,
    /// Who sends from which address: `Rtc::accepts` on everyone would cost O(n) per packet.
    participant_by_source: HashMap<SocketAddr, Snowflake>,
    frames_to_forward: Vec<(Snowflake, MediaData)>,
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
        participant_by_source: HashMap::new(),
        frames_to_forward: Vec::new(),
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
                    media.receive(contents, source);
                }
            }
            // Timeouts, and ICMP errors some systems report on UDP sockets (an unreachable
            // client): neither stops the loop.
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => {}
        }
        let now = Instant::now();
        media.handle_timeouts(now);
        media.forward_frames(now);
        media.tick_speaking(now);
        media.end_dead_connections(now);
    }
}

impl MediaLoop<'_> {
    /// `false` once the voice service is gone.
    fn apply_commands(&mut self) -> bool {
        loop {
            match self.commands.try_recv() {
                Ok(command) => self.apply(command),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn apply(&mut self, command: MediaCommand) {
        match command {
            MediaCommand::Join(join) => {
                let now = Instant::now();
                self.remove(join.user_id);
                let mut participant = Participant::new(*join, now);
                drain_output(&mut participant, &self.socket, &mut self.frames_to_forward);
                self.participants.insert(participant.user_id, participant);
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
        }
    }

    fn emit(&self, event: MediaEvent) {
        // The receiver is gone only while the server shuts down; nothing is left to tell.
        let _ = self.events.send(event);
    }

    fn remove(&mut self, user_id: Snowflake) {
        if self.participants.remove(&user_id).is_none() {
            return;
        }
        self.participant_by_source
            .retain(|_, participant_id| *participant_id != user_id);
        for listener in self.participants.values_mut() {
            listener.slots.release(user_id);
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
        let known = self
            .participant_by_source
            .get(&source)
            .copied()
            .filter(|user_id| {
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
            self.participant_by_source.insert(source, user_id);
            user_id
        };
        let Some(participant) = self.participants.get_mut(&user_id) else {
            return;
        };
        if !participant.is_within_packet_rate(now) {
            return;
        }
        participant.last_received_at = now;
        let Some(input) = Self::input_for(contents, source, participant.candidate_address, now)
        else {
            return;
        };
        if participant.rtc.handle_input(input).is_err() {
            participant.rtc.disconnect();
        }
        drain_output(participant, &self.socket, &mut self.frames_to_forward);
    }

    fn handle_timeouts(&mut self, now: Instant) {
        for participant in self
            .participants
            .values_mut()
            .filter(|participant| participant.timeout_at <= now)
        {
            if participant.rtc.handle_input(Input::Timeout(now)).is_err() {
                participant.rtc.disconnect();
            }
            drain_output(participant, &self.socket, &mut self.frames_to_forward);
        }
    }

    fn forward_frames(&mut self, now: Instant) {
        for (speaker_id, frame) in std::mem::take(&mut self.frames_to_forward) {
            self.forward(speaker_id, &frame, now);
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
        let mut frames_from_listeners = Vec::new();
        for listener in self.participants.values_mut().filter(|listener| {
            listener.user_id != speaker_id
                && listener.channel_id == channel_id
                && listener.is_connected
                && !listener.is_deafened
        }) {
            let Some(slot) = listener.slots.slot_for(speaker_id, now) else {
                continue;
            };
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

    #[test]
    fn offers_are_answered_with_opus_dtx_and_fec_even_without_a_track_yet() {
        str0m::crypto::from_feature_flags().install_process_default();
        let mut client = Rtc::builder()
            .clear_codecs()
            .enable_opus(true, false)
            .build(Instant::now());
        let mut change = client.sdp_api();
        change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
        change.add_media(MediaKind::Audio, Direction::RecvOnly, None, None, None);
        let (offer, _pending) = change.apply().unwrap();
        // A browser transceiver with no microphone track yet announces no stream.
        let without_track: String = offer
            .to_sdp_string()
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
