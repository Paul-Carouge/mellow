//! The network thread: drives the `str0m` peer over its own UDP socket, as the server's media
//! loop does on its side. It writes the encoded voice with its RFC 6464 level, and hands the
//! received audio packets to the audio thread.

use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use str0m::change::{SdpAnswer, SdpPendingOffer};
use str0m::format::Codec;
use str0m::media::{Frequency, MediaTime, Mid, Pt};
use str0m::net::{Protocol, Receive};
use str0m::{Event, IceConnectionState, Input, Output, Rtc};

use crate::audio_engine::AudioCommand;
use crate::offer::MediaLines;
use crate::session::VoiceSessionEvent;

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
pub(crate) enum TransportCommand {
    ApplyAnswer(Box<SdpAnswer>),
    Stop,
}

/// Everything the network thread owns.
pub(crate) struct Transport {
    pub(crate) rtc: Rtc,
    pub(crate) pending_offer: Option<SdpPendingOffer>,
    pub(crate) socket: UdpSocket,
    /// The address announced to the server; the socket itself listens on every interface.
    pub(crate) local_candidate: SocketAddr,
    pub(crate) media_lines: MediaLines,
    pub(crate) commands: Receiver<TransportCommand>,
    pub(crate) outgoing: Receiver<OutgoingVoicePacket>,
    pub(crate) incoming: Sender<IncomingAudioPacket>,
    pub(crate) events: Sender<VoiceSessionEvent>,
    /// To stop the microphone when the connection is lost, before saying so.
    pub(crate) audio_commands: Sender<AudioCommand>,
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Transport")
            .field("media_lines", &self.media_lines)
            .finish_non_exhaustive()
    }
}

impl Transport {
    /// Runs until stopped, or until the connection fails (reported as an event).
    pub(crate) fn run(mut self) {
        if let Err(reason) = self.run_until_stopped() {
            tracing::warn!(%reason, "voice connection failed");
            // The session may already be stopping the audio thread itself.
            let _ = self.audio_commands.send(AudioCommand::Stop);
            let _ = self.events.send(VoiceSessionEvent::MediaConnectionLost);
        }
    }

    fn run_until_stopped(&mut self) -> Result<(), String> {
        let mut buffer = vec![0_u8; RECEIVE_BUFFER_BYTES];
        let mut voice_payload_type: Option<Pt> = None;
        loop {
            match self.commands.try_recv() {
                Ok(TransportCommand::ApplyAnswer(answer)) => self.apply_answer(*answer)?,
                Ok(TransportCommand::Stop) | Err(TryRecvError::Disconnected) => return Ok(()),
                Err(TryRecvError::Empty) => {}
            }
            self.send_voice(voice_payload_type)?;
            let timeout = loop {
                match self.rtc.poll_output().map_err(|error| error.to_string())? {
                    Output::Timeout(at) => break at,
                    Output::Transmit(transmit) => {
                        // A lost datagram is a lost packet; UDP has no backpressure to honour.
                        let _ = self
                            .socket
                            .send_to(&transmit.contents, transmit.destination);
                    }
                    Output::Event(event) => {
                        self.handle_event(event, &mut voice_payload_type)?;
                    }
                }
            };
            let wait = timeout
                .saturating_duration_since(Instant::now())
                .clamp(Duration::from_millis(1), MAXIMUM_SOCKET_WAIT);
            self.socket
                .set_read_timeout(Some(wait))
                .map_err(|error| error.to_string())?;
            let input = match self.socket.recv_from(&mut buffer) {
                Ok((length, source)) => {
                    let Ok(contents) = buffer[..length].try_into() else {
                        // Not STUN, DTLS, RTP or RTCP: someone else's datagram.
                        continue;
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
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
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
                .map_err(|error| error.to_string())?;
        }
    }

    fn apply_answer(&mut self, answer: SdpAnswer) -> Result<(), String> {
        let Some(pending) = self.pending_offer.take() else {
            // A second answer to the same offer: the session already filters these out.
            return Ok(());
        };
        self.rtc
            .sdp_api()
            .accept_answer(pending, answer)
            .map_err(|error| error.to_string())
    }

    fn handle_event(
        &mut self,
        event: Event,
        voice_payload_type: &mut Option<Pt>,
    ) -> Result<(), String> {
        match event {
            // `Connected`, not the ICE state: DTLS is done and media can flow.
            Event::Connected => {
                *voice_payload_type =
                    self.rtc
                        .writer(self.media_lines.voice_send)
                        .and_then(|writer| {
                            writer
                                .payload_params()
                                .find(|params| params.spec().codec == Codec::Opus)
                                .map(str0m::format::PayloadParams::pt)
                        });
                let _ = self.events.send(VoiceSessionEvent::Connected);
            }
            Event::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                return Err("the server stopped answering".into());
            }
            Event::MediaData(data)
                if self
                    .media_lines
                    .audio_receive()
                    .any(|(mid, _)| mid == data.mid) =>
            {
                // The audio thread gone means the session is ending.
                let _ = self.incoming.send(IncomingAudioPacket {
                    mid: data.mid,
                    rtp_timestamp: data.time.numer(),
                    arrival: data.network_time,
                    payload: data.data,
                });
            }
            _ => {}
        }
        Ok(())
    }

    /// Writes the voice packets the audio thread encoded; before the connection they are
    /// dropped, as a browser's track sends nothing before then either.
    fn send_voice(&mut self, voice_payload_type: Option<Pt>) -> Result<(), String> {
        while let Ok(packet) = self.outgoing.try_recv() {
            let Some(payload_type) = voice_payload_type else {
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
