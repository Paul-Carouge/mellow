//! The receive side of one audio slot: an adaptive jitter buffer that orders Opus packets by RTP
//! timestamp and tells the decoder, frame by frame, to decode, recover a lost packet from the
//! next one's in-band FEC, conceal, or play silence; and the steering that keeps its depth on
//! target despite the sender's clock running slightly faster or slower than the playback device.
//!
//! Timestamps are str0m's extended 64-bit RTP times at 48 kHz. Arrival times are counted in the
//! same unit by the caller, so the buffer itself never reads a clock.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::audio_format::SAMPLES_PER_PACKET;

/// 20 ms, the packet duration assumed for concealment and for packets of unknown length.
const PACKET_SAMPLES: u64 = SAMPLES_PER_PACKET as u64;
/// The shallowest and deepest target delays: 40 ms (browsers measured 22 to 39 ms, ADR-0030) and
/// 200 ms (beyond that a conversation stops feeling live).
const MINIMUM_TARGET_DELAY_SAMPLES: u64 = 2 * PACKET_SAMPLES;
const MAXIMUM_TARGET_DELAY_SAMPLES: u64 = 10 * PACKET_SAMPLES;
/// The target is the jitter times this, plus one packet: RFC 3550's estimate is a mean
/// deviation, and a few deviations cover nearly all late packets.
const JITTER_MULTIPLIER: f64 = 3.0;
/// RFC 3550's smoothing of the interarrival jitter.
const JITTER_SMOOTHING: f64 = 1.0 / 16.0;
/// Packets concealed in a row before the buffer plays silence: longer concealment sounds robotic.
const MAXIMUM_CONCEALED_PACKETS: u32 = 5;
/// A timestamp this far from the playout point means a new stream on the slot (the server gave
/// it to another speaker, ADR-0030), not a late or early packet: the buffer starts over.
const NEW_STREAM_DISTANCE_SAMPLES: u64 = 48_000;
/// Depth beyond the target from which packets are dropped at once rather than steered away.
const MAXIMUM_EXCESS_DELAY_SAMPLES: u64 = 5 * PACKET_SAMPLES;
/// The most packets held: a second of audio, so a flood cannot grow the buffer.
const MAXIMUM_BUFFERED_PACKETS: usize = 50;

/// What the decoder does for the next packet's worth of audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlayoutDecision {
    /// Decode this packet normally.
    Decode(Arc<[u8]>),
    /// The packet due is lost but the next one is here: decode its in-band FEC copy of the lost
    /// one (Opus `decode` with `fec = true`), for one packet's duration.
    RecoverFromNextPacket(Arc<[u8]>),
    /// Nothing to decode: let Opus conceal one packet's duration.
    Conceal,
    /// Play one packet's duration of silence: idle, or concealed too long.
    Silence,
}

#[derive(Debug)]
struct BufferedPacket {
    payload: Arc<[u8]>,
    duration_samples: u64,
}

/// One slot's jitter buffer.
#[derive(Debug, Default)]
pub(crate) struct JitterBuffer {
    packets: BTreeMap<u64, BufferedPacket>,
    /// The RTP time of the next audio to play; `None` while idle (nothing heard yet, or silence
    /// after the last packet).
    playout_timestamp: Option<u64>,
    jitter_estimate_samples: f64,
    previous_transit: Option<i128>,
    packets_concealed_in_a_row: u32,
    /// Whether a packet was decoded since the playout point was anchored: before that, a gap
    /// is the target delay being waited out, not a loss.
    has_decoded_since_anchoring: bool,
}

impl JitterBuffer {
    /// The delay the buffer aims at, from the jitter measured so far.
    pub(crate) fn target_delay_samples(&self) -> u64 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // non-negative, small
        let from_jitter = (self.jitter_estimate_samples * JITTER_MULTIPLIER).ceil() as u64;
        (from_jitter + PACKET_SAMPLES)
            .clamp(MINIMUM_TARGET_DELAY_SAMPLES, MAXIMUM_TARGET_DELAY_SAMPLES)
    }

    /// Audio buffered ahead of the playout point, in samples; 0 while idle.
    pub(crate) fn depth_samples(&self) -> u64 {
        let (Some(playout), Some((timestamp, packet))) =
            (self.playout_timestamp, self.packets.last_key_value())
        else {
            return 0;
        };
        (timestamp + packet.duration_samples).saturating_sub(playout)
    }

    /// Whether nothing is being played: no packet yet, or silence since the last one.
    pub(crate) fn is_idle(&self) -> bool {
        self.playout_timestamp.is_none()
    }

    /// Forgets everything, as when the slot is given to someone else.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Adds a packet with RTP time `timestamp` that arrived at `arrival_samples` (any clock
    /// counted at 48 kHz). Late and duplicate packets are dropped.
    pub(crate) fn push(
        &mut self,
        timestamp: u64,
        arrival_samples: u64,
        payload: Arc<[u8]>,
        duration_samples: Option<u64>,
    ) {
        if let Some(playout) = self.playout_timestamp
            && timestamp.abs_diff(playout) > NEW_STREAM_DISTANCE_SAMPLES
        {
            self.reset();
        }
        self.measure_jitter(timestamp, arrival_samples);
        let target = self.target_delay_samples();
        let playout = *self
            .playout_timestamp
            .get_or_insert(timestamp.saturating_sub(target));
        if timestamp < playout {
            return;
        }
        if self.packets.len() >= MAXIMUM_BUFFERED_PACKETS {
            self.packets.pop_first();
        }
        self.packets
            .entry(timestamp)
            .or_insert_with(|| BufferedPacket {
                payload,
                duration_samples: duration_samples
                    .filter(|duration| *duration > 0)
                    .unwrap_or(PACKET_SAMPLES),
            });
    }

    /// RFC 3550's interarrival jitter, in samples.
    fn measure_jitter(&mut self, timestamp: u64, arrival_samples: u64) {
        let transit = i128::from(arrival_samples) - i128::from(timestamp);
        if let Some(previous) = self.previous_transit {
            #[allow(clippy::cast_precision_loss)] // differences of a few seconds at most
            let difference = (transit - previous).unsigned_abs() as f64;
            // A jump of a second or more is a new stream or a DTX pause, not jitter.
            if difference < 48_000.0 {
                self.jitter_estimate_samples +=
                    (difference - self.jitter_estimate_samples) * JITTER_SMOOTHING;
            }
        }
        self.previous_transit = Some(transit);
    }

    /// The decision for the next packet's worth of audio, advancing the playout point.
    pub(crate) fn next(&mut self) -> PlayoutDecision {
        let Some(playout) = self.playout_timestamp else {
            return PlayoutDecision::Silence;
        };
        let playout = self.drop_excess(playout);
        let Some((&first_timestamp, _)) = self.packets.first_key_value() else {
            self.playout_timestamp = Some(playout + PACKET_SAMPLES);
            return self.conceal_or_go_idle();
        };
        // A packet starting before the next one is due is the current one, even if its
        // timestamp is not aligned on ours (browsers may send 10 ms packets).
        if first_timestamp < playout + PACKET_SAMPLES
            && let Some(packet) = self.packets.remove(&first_timestamp)
        {
            self.playout_timestamp = Some(first_timestamp + packet.duration_samples);
            self.packets_concealed_in_a_row = 0;
            self.has_decoded_since_anchoring = true;
            return PlayoutDecision::Decode(packet.payload);
        }
        self.playout_timestamp = Some(playout + PACKET_SAMPLES);
        if !self.has_decoded_since_anchoring {
            return PlayoutDecision::Silence;
        }
        if first_timestamp < playout + 2 * PACKET_SAMPLES
            && let Some(next_packet) = self.packets.get(&first_timestamp)
        {
            self.packets_concealed_in_a_row = 0;
            return PlayoutDecision::RecoverFromNextPacket(next_packet.payload.clone());
        }
        // A gap with audio after it (losses, or a DTX pause): conceal briefly, then silence
        // until the next packet is due.
        self.packets_concealed_in_a_row += 1;
        if self.packets_concealed_in_a_row > MAXIMUM_CONCEALED_PACKETS {
            PlayoutDecision::Silence
        } else {
            PlayoutDecision::Conceal
        }
    }

    fn conceal_or_go_idle(&mut self) -> PlayoutDecision {
        self.packets_concealed_in_a_row += 1;
        if self.packets_concealed_in_a_row > MAXIMUM_CONCEALED_PACKETS {
            // The next packet re-anchors the playout point with the target delay.
            self.playout_timestamp = None;
            self.packets_concealed_in_a_row = 0;
            self.has_decoded_since_anchoring = false;
            return PlayoutDecision::Silence;
        }
        PlayoutDecision::Conceal
    }

    /// When far deeper than the target (a burst after a stall), skips the oldest audio so the
    /// delay comes back at once; drift steering handles the small differences.
    fn drop_excess(&mut self, playout: u64) -> u64 {
        let target = self.target_delay_samples();
        if self.depth_samples() <= target + MAXIMUM_EXCESS_DELAY_SAMPLES {
            return playout;
        }
        let Some((last_timestamp, last_packet)) = self.packets.last_key_value() else {
            return playout;
        };
        let new_playout = (last_timestamp + last_packet.duration_samples).saturating_sub(target);
        while self
            .packets
            .first_key_value()
            .is_some_and(|(timestamp, _)| *timestamp < new_playout)
        {
            self.packets.pop_first();
        }
        self.playout_timestamp = Some(new_playout);
        new_playout
    }
}

/// The slowest and fastest playback relative to nominal used to steer the depth: ±0.5 % is
/// inaudible on speech and covers sound card clocks, which differ by tens of ppm.
const MAXIMUM_RATIO_DEVIATION: f64 = 0.005;
/// Depth error (in samples) that asks for the full deviation: one packet.
const FULL_DEVIATION_ERROR_SAMPLES: f64 = 960.0;
/// Smoothing of the depth error per update (once per packet): about a second of memory, since
/// depth jumps by a packet with every arrival.
const ERROR_SMOOTHING: f64 = 0.02;
/// Errors smaller than this are left alone: half a packet of depth is noise.
const DEAD_BAND_SAMPLES: f64 = 480.0;

/// Turns the jitter buffer's depth error into a playback-rate ratio for the slot's resampler:
/// above 1 when too shallow (play slower, let it fill), below 1 when too deep.
#[derive(Debug, Default)]
pub(crate) struct DriftSteering {
    smoothed_error_samples: f64,
}

impl DriftSteering {
    pub(crate) fn update(&mut self, depth_samples: u64, target_samples: u64) -> f64 {
        #[allow(clippy::cast_precision_loss)] // depths are below a second of samples
        let error = depth_samples as f64 - target_samples as f64;
        self.smoothed_error_samples += (error - self.smoothed_error_samples) * ERROR_SMOOTHING;
        if self.smoothed_error_samples.abs() < DEAD_BAND_SAMPLES {
            return 1.0;
        }
        let deviation = (self.smoothed_error_samples / FULL_DEVIATION_ERROR_SAMPLES
            * MAXIMUM_RATIO_DEVIATION)
            .clamp(-MAXIMUM_RATIO_DEVIATION, MAXIMUM_RATIO_DEVIATION);
        1.0 - deviation
    }

    pub(crate) fn reset(&mut self) {
        self.smoothed_error_samples = 0.0;
    }
}

/// The widest relative ratio the steering asks for, for sizing the resampler.
pub(crate) const MAXIMUM_STEERING_RATIO: f64 = 1.0 + MAXIMUM_RATIO_DEVIATION;

#[cfg(test)]
mod tests {
    use super::*;

    /// RTP times start far from zero, as real streams do, so the target delay fits before them.
    const FIRST_TIMESTAMP: u64 = 1_000_000;

    fn payload(marker: u8) -> Arc<[u8]> {
        Arc::from(vec![marker; 4])
    }

    fn timestamp_of(index: u64) -> u64 {
        FIRST_TIMESTAMP + index * PACKET_SAMPLES
    }

    /// A buffer fed `count` packets from timestamp 0, each arriving exactly on time.
    fn steady_buffer(count: u64) -> JitterBuffer {
        let mut buffer = JitterBuffer::default();
        for index in 0..count {
            let timestamp = timestamp_of(index);
            buffer.push(
                timestamp,
                timestamp,
                payload(u8::try_from(index).unwrap()),
                None,
            );
        }
        buffer
    }

    /// Drains the leading silence the target delay inserts before the first packet.
    fn skip_leading_silence(buffer: &mut JitterBuffer) -> PlayoutDecision {
        loop {
            match buffer.next() {
                PlayoutDecision::Silence | PlayoutDecision::Conceal => {}
                decision => return decision,
            }
        }
    }

    #[test]
    fn packets_come_out_in_timestamp_order_whatever_their_arrival_order() {
        let mut buffer = JitterBuffer::default();
        for index in [0_u8, 2, 1, 3] {
            buffer.push(
                timestamp_of(u64::from(index)),
                u64::from(index),
                payload(index),
                None,
            );
        }
        assert_eq!(
            skip_leading_silence(&mut buffer),
            PlayoutDecision::Decode(payload(0))
        );
        for index in 1..=3 {
            assert_eq!(buffer.next(), PlayoutDecision::Decode(payload(index)));
        }
    }

    #[test]
    fn playback_starts_after_the_target_delay() {
        let mut buffer = steady_buffer(1);
        let silent_packets = std::iter::from_fn(|| Some(buffer.next()))
            .take_while(|decision| *decision == PlayoutDecision::Silence)
            .count();
        assert_eq!(
            u64::try_from(silent_packets).unwrap() * PACKET_SAMPLES,
            MINIMUM_TARGET_DELAY_SAMPLES
        );
    }

    #[test]
    fn a_single_loss_is_recovered_from_the_next_packets_forward_error_correction() {
        let mut buffer = JitterBuffer::default();
        for index in [0_u8, 2, 3] {
            let timestamp = timestamp_of(u64::from(index));
            buffer.push(timestamp, timestamp, payload(index), None);
        }
        assert_eq!(
            skip_leading_silence(&mut buffer),
            PlayoutDecision::Decode(payload(0))
        );
        assert_eq!(
            buffer.next(),
            PlayoutDecision::RecoverFromNextPacket(payload(2))
        );
        assert_eq!(buffer.next(), PlayoutDecision::Decode(payload(2)));
    }

    #[test]
    fn lost_packets_are_concealed_then_recovered_from_the_first_one_back() {
        let mut buffer = steady_buffer(1);
        assert_eq!(
            skip_leading_silence(&mut buffer),
            PlayoutDecision::Decode(payload(0))
        );
        for _ in 0..3 {
            assert_eq!(buffer.next(), PlayoutDecision::Conceal);
        }
        // Packets 1 to 4 were lost; 5 arrives on time, while 4 is due.
        buffer.push(timestamp_of(5), timestamp_of(5), payload(5), None);
        assert_eq!(
            buffer.next(),
            PlayoutDecision::RecoverFromNextPacket(payload(5)),
            "the packet just before it is rebuilt from its FEC"
        );
        assert_eq!(buffer.next(), PlayoutDecision::Decode(payload(5)));
    }

    #[test]
    fn a_pause_is_concealed_briefly_then_silent_and_the_next_talk_spurt_is_delayed_again() {
        let mut buffer = steady_buffer(1);
        skip_leading_silence(&mut buffer);
        let decisions: Vec<_> = (0..8).map(|_| buffer.next()).collect();
        let concealed = MAXIMUM_CONCEALED_PACKETS as usize;
        assert!(
            decisions[..concealed]
                .iter()
                .all(|decision| *decision == PlayoutDecision::Conceal)
        );
        assert!(
            decisions[concealed..]
                .iter()
                .all(|decision| *decision == PlayoutDecision::Silence)
        );
        assert!(buffer.is_idle());
        let resumed_at = timestamp_of(100);
        buffer.push(resumed_at, resumed_at, payload(100), None);
        assert_eq!(
            buffer.next(),
            PlayoutDecision::Silence,
            "the target delay again"
        );
        assert_eq!(
            skip_leading_silence(&mut buffer),
            PlayoutDecision::Decode(payload(100))
        );
    }

    #[test]
    fn late_packets_are_dropped() {
        let mut buffer = steady_buffer(4);
        skip_leading_silence(&mut buffer);
        buffer.next();
        buffer.push(timestamp_of(0), 10_000, payload(99), None);
        assert_eq!(buffer.next(), PlayoutDecision::Decode(payload(2)));
    }

    #[test]
    fn a_timestamp_jump_starts_a_new_stream() {
        let mut buffer = steady_buffer(3);
        skip_leading_silence(&mut buffer);
        let new_stream = FIRST_TIMESTAMP + 10 * 48_000;
        buffer.push(new_stream, 100, payload(7), None);
        assert_eq!(
            skip_leading_silence(&mut buffer),
            PlayoutDecision::Decode(payload(7))
        );
        assert_eq!(buffer.depth_samples(), 0);
    }

    #[test]
    fn jittery_arrivals_raise_the_target_delay() {
        let mut steady = steady_buffer(40);
        let mut jittery = JitterBuffer::default();
        // 40 packets: under a second ahead of the playout point, which never moves here.
        for index in 0..40_u64 {
            let timestamp = timestamp_of(index);
            let late_by = if index % 2 == 0 { 0 } else { 1_920 };
            jittery.push(timestamp, timestamp + late_by, payload(0), None);
        }
        assert_eq!(steady.target_delay_samples(), MINIMUM_TARGET_DELAY_SAMPLES);
        assert!(jittery.target_delay_samples() > 4 * PACKET_SAMPLES);
        assert!(jittery.target_delay_samples() <= MAXIMUM_TARGET_DELAY_SAMPLES);
        steady.reset();
        assert_eq!(steady.depth_samples(), 0);
    }

    #[test]
    fn a_burst_after_a_stall_is_cut_back_to_the_target() {
        let mut buffer = steady_buffer(1);
        skip_leading_silence(&mut buffer);
        for index in 1..30 {
            buffer.push(timestamp_of(index), PACKET_SAMPLES, payload(0), None);
        }
        buffer.next();
        assert!(buffer.depth_samples() <= buffer.target_delay_samples());
    }

    #[test]
    fn steering_holds_the_depth_against_a_sender_clock_two_per_mille_fast() {
        // Each step is 10 ms of playback; the sender produces 0.2 % more than that.
        let target = 1_920.0;
        let produced_per_step = 480.0 * 1.002;
        let simulate = |is_steered: bool| {
            let mut steering = DriftSteering::default();
            let mut depth: f64 = target;
            for _ in 0..60_000 {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let ratio = if is_steered {
                    steering.update(depth.max(0.0) as u64, 1_920)
                } else {
                    1.0
                };
                // A ratio below 1 plays the same input in fewer output samples: it consumes more.
                depth += produced_per_step - 480.0 / ratio;
            }
            depth
        };
        assert!(
            simulate(false) > target + 50_000.0,
            "ten minutes drift away"
        );
        let steered = simulate(true);
        assert!(
            (steered - target).abs() < 1_500.0,
            "steered depth {steered} stays within a packet and a half of the target"
        );
    }

    #[test]
    fn steering_leaves_small_errors_alone() {
        let mut steering = DriftSteering::default();
        for _ in 0..1_000 {
            assert!((steering.update(2_100, 1_920) - 1.0).abs() < f64::EPSILON);
        }
        steering.reset();
    }
}
