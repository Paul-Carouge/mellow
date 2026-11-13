//! The shared screen's sound (ADR-0037, ADR-0054): Opus packets of 20 ms, 48 kHz stereo, sent on
//! the screen-sound line as the web client sends them, only while the screen share they belong
//! to is on. Capture that hands over raw samples (`ScreenCaptureKit`, the test source) writes them
//! into a wait-free ring that an encoding thread turns into Opus; capture that encodes itself
//! (Vesper's PipeWire capture) pushes its packets directly.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::video_sender::VideoEncoderControl;

/// The voice line's clock and Opus's frame: 20 ms at 48 kHz.
pub(crate) const SCREEN_SOUND_SAMPLE_RATE: u32 = 48_000;
pub(crate) const SCREEN_SOUND_CHANNELS: usize = 2;
pub(crate) const SCREEN_SOUND_FRAME_SAMPLES_PER_CHANNEL: usize = 960;
/// Music and films rather than a voice; two Opus streams (voice and this) stay far below the
/// server's doubled voice budget of 140 kB a second (ADR-0037).
const SCREEN_SOUND_BITS_PER_SECOND: i32 = 128_000;
/// libopus' largest packet.
const MAXIMUM_OPUS_PACKET_BYTES: usize = 4_000;
/// Half a second of stereo samples between capture and the encoder: room for a late wake-up
/// without ever blocking capture (older samples are dropped when it is full).
const PCM_RING_SAMPLES: usize = SCREEN_SOUND_SAMPLE_RATE as usize * SCREEN_SOUND_CHANNELS / 2;
/// How long the encoder sleeps when less than one frame is waiting.
const PCM_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// One Opus packet of the screen's sound, stamped on the 48 kHz RTP clock.
#[derive(Debug)]
pub(crate) struct OutgoingScreenSoundPacket {
    pub(crate) rtp_timestamp: u64,
    pub(crate) payload: Vec<u8>,
}

/// Where a screen share's sound goes, open while that share's picture is on. Clones share one
/// RTP clock.
#[derive(Debug, Clone)]
pub struct ScreenSoundInput {
    control: Arc<VideoEncoderControl>,
    generation: u64,
    outgoing: Sender<OutgoingScreenSoundPacket>,
    next_rtp_timestamp: Arc<AtomicU64>,
}

impl ScreenSoundInput {
    pub(crate) fn new(
        control: Arc<VideoEncoderControl>,
        generation: u64,
        outgoing: Sender<OutgoingScreenSoundPacket>,
    ) -> Self {
        Self {
            control,
            generation,
            outgoing,
            next_rtp_timestamp: Arc::default(),
        }
    }

    /// Whether the share this sound belongs to is still on.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.control.is_generation_on(self.generation)
    }

    /// Sends one Opus packet of 20 ms (48 kHz, stereo or mono). Packets follow each other
    /// without gaps; a capture that lost time sends silence for it.
    pub fn push_opus(&self, payload: Vec<u8>) {
        let frame = u64::try_from(SCREEN_SOUND_FRAME_SAMPLES_PER_CHANNEL).unwrap_or(u64::MAX);
        let rtp_timestamp = self.next_rtp_timestamp.fetch_add(frame, Ordering::Relaxed);
        if !self.is_open() {
            return;
        }
        // The network thread gone means the session is ending.
        let _ = self.outgoing.send(OutgoingScreenSoundPacket {
            rtp_timestamp,
            payload,
        });
    }
}

/// Raw samples to Opus, on a thread of its own. Capture writes with
/// [`PcmSoundWriter::write_planar`],
/// which never allocates, locks or waits.
pub(crate) struct PcmSoundEncoder {
    should_stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// The capture's end of a [`PcmSoundEncoder`]'s ring.
pub(crate) struct PcmSoundWriter {
    producer: rtrb::Producer<f32>,
}

impl PcmSoundWriter {
    /// Writes planar stereo samples at 48 kHz (`left` and `right`, or one channel twice for
    /// mono); what does not fit is dropped.
    pub(crate) fn write_planar(&mut self, left: &[f32], right: &[f32]) {
        let frames = left.len().min(right.len());
        let count = (frames * SCREEN_SOUND_CHANNELS).min(self.producer.slots()) & !1;
        if let Ok(chunk) = self.producer.write_chunk_uninit(count) {
            let interleaved = left
                .iter()
                .zip(right)
                .flat_map(|(left, right)| [*left, *right]);
            chunk.fill_from_iter(interleaved);
        }
    }
}

impl PcmSoundEncoder {
    /// Starts the encoding thread for `input`; it ends when the share or the encoder stops.
    pub(crate) fn start(input: ScreenSoundInput) -> std::io::Result<(Self, PcmSoundWriter)> {
        let (producer, mut consumer) = rtrb::RingBuffer::new(PCM_RING_SAMPLES);
        let should_stop = Arc::new(AtomicBool::new(false));
        let mut encoder = opus::Encoder::new(
            SCREEN_SOUND_SAMPLE_RATE,
            opus::Channels::Stereo,
            opus::Application::Audio,
        )
        .map_err(std::io::Error::other)?;
        encoder
            .set_bitrate(opus::Bitrate::Bits(SCREEN_SOUND_BITS_PER_SECOND))
            .map_err(std::io::Error::other)?;
        let thread = {
            let should_stop = Arc::clone(&should_stop);
            std::thread::Builder::new()
                .name("mellow-screen-sound".into())
                .spawn(move || {
                    let mut frame =
                        [0.0_f32; SCREEN_SOUND_FRAME_SAMPLES_PER_CHANNEL * SCREEN_SOUND_CHANNELS];
                    let mut packet = [0_u8; MAXIMUM_OPUS_PACKET_BYTES];
                    while !should_stop.load(Ordering::Relaxed) && input.is_open() {
                        let Ok(chunk) = consumer.read_chunk(frame.len()) else {
                            std::thread::sleep(PCM_POLL_INTERVAL);
                            continue;
                        };
                        let (first, second) = chunk.as_slices();
                        frame[..first.len()].copy_from_slice(first);
                        frame[first.len()..].copy_from_slice(second);
                        chunk.commit_all();
                        match encoder.encode_float(&frame, &mut packet) {
                            Ok(length) => input.push_opus(packet[..length].to_vec()),
                            Err(error) => {
                                tracing::warn!(%error, "the screen's sound could not be encoded");
                            }
                        }
                    }
                })?
        };
        Ok((
            Self {
                should_stop,
                thread: Some(thread),
            },
            PcmSoundWriter { producer },
        ))
    }
}

impl Drop for PcmSoundEncoder {
    fn drop(&mut self) {
        self.should_stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the screen sound's encoding thread panicked");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::simulcast::{ScreenShareQuality, VideoSourceKind};

    #[test]
    fn samples_become_20_ms_opus_packets_only_while_the_share_is_on() {
        let control = Arc::new(VideoEncoderControl::default());
        let video = control.start(VideoSourceKind::Screen(ScreenShareQuality::High));
        let (outgoing, packets) = mpsc::channel();
        let generation = video.generation();
        let input = ScreenSoundInput::new(Arc::clone(&control), generation, outgoing);
        let (encoder, mut writer) = PcmSoundEncoder::start(input.clone()).unwrap();
        // 100 ms of a 440 Hz tone, planar.
        let tone: Vec<f32> = (0..4_800_u16)
            .map(|index| (f32::from(index) * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5)
            .collect();
        writer.write_planar(&tone, &tone);
        let mut stamps = Vec::new();
        while stamps.len() < 5 {
            let packet = packets.recv_timeout(Duration::from_secs(10)).unwrap();
            let decoder = opus::Decoder::new(48_000, opus::Channels::Stereo).unwrap();
            assert_eq!(decoder.get_nb_samples(&packet.payload).unwrap(), 960);
            stamps.push(packet.rtp_timestamp);
        }
        assert_eq!(stamps, [0, 960, 1_920, 2_880, 3_840]);
        drop(video);
        control.stop_source();
        assert!(!input.is_open());
        input.push_opus(vec![1]);
        drop(encoder);
        assert!(packets.try_iter().all(|packet| packet.payload != [1]));
    }
}
