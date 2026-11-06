//! Opus as browsers use it for voice: mono 48 kHz, 20 ms packets, about 32 kbit/s, in-band FEC
//! and DTX (ADR-0030).

use std::sync::Arc;

use crate::audio_format::{MAXIMUM_SAMPLES_PER_OPUS_PACKET, SAMPLES_PER_PACKET, VOICE_SAMPLE_RATE};
use crate::jitter_buffer::PlayoutDecision;

/// What browsers send for mono speech.
const VOICE_BITS_PER_SECOND: i32 = 32_000;
/// The loss the encoder plans its FEC for: enough to cover a lost packet in ten.
const EXPECTED_PACKET_LOSS_PERCENT: i32 = 10;
/// Opus never needs more than this for one 20 ms packet at our bitrate; it is the RTP payload cap.
const MAXIMUM_PACKET_BYTES: usize = 1_275;
/// An encoded packet this short is a DTX frame that need not be sent (libwebrtc does the same).
const LONGEST_DISCONTINUOUS_TRANSMISSION_PACKET: usize = 2;

/// Opus failed: a bad configuration or a corrupt packet.
#[derive(Debug, thiserror::Error)]
#[error("opus failed: {0}")]
pub struct CodecError(#[from] opus::Error);

/// The microphone's encoder.
pub(crate) struct VoiceEncoder {
    encoder: opus::Encoder,
    packet: Vec<u8>,
}

impl std::fmt::Debug for VoiceEncoder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VoiceEncoder")
            .finish_non_exhaustive()
    }
}

impl VoiceEncoder {
    pub(crate) fn new() -> Result<Self, CodecError> {
        let mut encoder = opus::Encoder::new(
            VOICE_SAMPLE_RATE,
            opus::Channels::Mono,
            opus::Application::Voip,
        )?;
        encoder.set_bitrate(opus::Bitrate::Bits(VOICE_BITS_PER_SECOND))?;
        encoder.set_inband_fec(true)?;
        encoder.set_packet_loss_perc(EXPECTED_PACKET_LOSS_PERCENT)?;
        encoder.set_dtx(true)?;
        Ok(Self {
            encoder,
            packet: vec![0; MAXIMUM_PACKET_BYTES],
        })
    }

    /// Encodes 20 ms; `None` when DTX says nothing needs sending.
    pub(crate) fn encode(&mut self, samples: &[f32]) -> Result<Option<&[u8]>, CodecError> {
        let length = self.encoder.encode_float(samples, &mut self.packet)?;
        Ok((length > LONGEST_DISCONTINUOUS_TRANSMISSION_PACKET).then(|| &self.packet[..length]))
    }
}

/// One received voice's decoder, following the jitter buffer's decisions.
pub(crate) struct VoiceDecoder {
    decoder: opus::Decoder,
    output: Vec<f32>,
}

impl std::fmt::Debug for VoiceDecoder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VoiceDecoder")
            .finish_non_exhaustive()
    }
}

impl VoiceDecoder {
    pub(crate) fn new() -> Result<Self, CodecError> {
        Ok(Self {
            decoder: opus::Decoder::new(VOICE_SAMPLE_RATE, opus::Channels::Mono)?,
            output: vec![0.0; MAXIMUM_SAMPLES_PER_OPUS_PACKET],
        })
    }

    /// The samples for one decision. A corrupt packet is concealed instead.
    pub(crate) fn decode(&mut self, decision: &PlayoutDecision) -> &[f32] {
        let length = match decision {
            PlayoutDecision::Decode(packet) => self.decode_packet(packet),
            PlayoutDecision::RecoverFromNextPacket(next_packet) => self
                .decoder
                .decode_float(next_packet, &mut self.output[..SAMPLES_PER_PACKET], true)
                .unwrap_or_else(|_| self.conceal()),
            PlayoutDecision::Conceal => self.conceal(),
            PlayoutDecision::Silence => {
                self.output[..SAMPLES_PER_PACKET].fill(0.0);
                SAMPLES_PER_PACKET
            }
        };
        &self.output[..length]
    }

    fn decode_packet(&mut self, packet: &Arc<[u8]>) -> usize {
        if let Ok(length) = self.decoder.decode_float(packet, &mut self.output, false) {
            length
        } else {
            tracing::debug!("a corrupt voice packet was concealed");
            self.conceal()
        }
    }

    fn conceal(&mut self) -> usize {
        // Opus conceals when given no packet; on failure the frame is silence.
        self.decoder
            .decode_float(&[], &mut self.output[..SAMPLES_PER_PACKET], false)
            .unwrap_or_else(|_| {
                self.output[..SAMPLES_PER_PACKET].fill(0.0);
                SAMPLES_PER_PACKET
            })
    }
}

/// The duration of an Opus packet in 48 kHz samples, if it parses.
pub(crate) fn packet_duration_samples(packet: &[u8]) -> Option<u64> {
    opus::packet::get_nb_samples(packet, VOICE_SAMPLE_RATE)
        .ok()
        .and_then(|samples| u64::try_from(samples).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::root_mean_square;

    fn sine_packet(phase: &mut f32) -> Vec<f32> {
        (0..SAMPLES_PER_PACKET)
            .map(|_| {
                *phase += 2.0 * std::f32::consts::PI * 440.0 / 48_000.0;
                phase.sin() * 0.3
            })
            .collect()
    }

    #[test]
    fn speech_round_trips_and_lost_packets_are_recovered_or_concealed() {
        let mut encoder = VoiceEncoder::new().unwrap();
        let mut decoder = VoiceDecoder::new().unwrap();
        let mut phase = 0.0;
        let packets: Vec<Arc<[u8]>> = (0..20)
            .map(|_| Arc::from(encoder.encode(&sine_packet(&mut phase)).unwrap().unwrap()))
            .collect();
        assert_eq!(packet_duration_samples(&packets[0]), Some(960));
        for packet in &packets[..10] {
            decoder.decode(&PlayoutDecision::Decode(packet.clone()));
        }
        let tone = decoder.decode(&PlayoutDecision::Decode(packets[10].clone()));
        assert!(root_mean_square(tone) > 0.1, "the tone comes back");
        let recovered =
            decoder.decode(&PlayoutDecision::RecoverFromNextPacket(packets[12].clone()));
        assert_eq!(recovered.len(), SAMPLES_PER_PACKET);
        assert!(
            root_mean_square(recovered) > 0.05,
            "FEC rebuilds the lost tone"
        );
        let concealed = decoder.decode(&PlayoutDecision::Conceal);
        assert_eq!(concealed.len(), SAMPLES_PER_PACKET);
        let corrupt = decoder.decode(&PlayoutDecision::Decode(Arc::from(vec![0xff; 3])));
        assert_eq!(corrupt.len(), SAMPLES_PER_PACKET, "concealed instead");
        let silence = decoder.decode(&PlayoutDecision::Silence);
        assert!(root_mean_square(silence) < f32::EPSILON);
    }

    #[test]
    fn silence_is_not_sent_once_discontinuous_transmission_kicks_in() {
        let mut encoder = VoiceEncoder::new().unwrap();
        let silence = [0.0; SAMPLES_PER_PACKET];
        let sent = (0..50)
            .filter(|_| encoder.encode(&silence).unwrap().is_some())
            .count();
        assert!(sent < 15, "{sent} of 50 silent packets sent");
    }
}
