//! The engine's internal audio format: mono, 48 kHz, `f32` in `[-1, 1]`, 10 ms frames, which is
//! what Opus, the echo canceller and `RNNoise` all work in.

use std::time::Duration;

/// Opus's clock and the engine's sample rate.
pub(crate) const VOICE_SAMPLE_RATE: u32 = 48_000;
/// One processing frame: 10 ms at 48 kHz, the frame of the echo canceller and `RNNoise`.
pub(crate) const SAMPLES_PER_FRAME: usize = 480;
pub(crate) const FRAME_DURATION: Duration = Duration::from_millis(10);
/// One Opus packet: 20 ms, as browsers send (10 ms packets would double the packet rate the
/// server allows per participant).
pub(crate) const SAMPLES_PER_PACKET: usize = 960;
/// The longest Opus packet (120 ms): the decoder's output buffer must hold it.
pub(crate) const MAXIMUM_SAMPLES_PER_OPUS_PACKET: usize = 5_760;

/// A 10 ms mono frame at 48 kHz.
pub(crate) type Frame = [f32; SAMPLES_PER_FRAME];
