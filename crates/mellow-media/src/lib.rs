//! The desktop client's voice engine (ADR-0051), without UI: a `str0m` WebRTC peer speaking the
//! server's exact offer shape, audio devices behind a trait, microphone processing (echo
//! cancellation, noise suppression, gain), Opus, jitter buffers with loss concealment and drift
//! steering, mixing, voice activity and push-to-talk.
//!
//! Threads: a network thread owns the UDP socket and the peer; an audio thread owns the codecs,
//! the processing and the engine's end of the devices' ring buffers; device callbacks only move
//! samples through wait-free ring buffers. Video lines are in the offer, as the server requires,
//! but carry nothing yet.

mod audio_engine;
mod audio_format;
mod codec;
mod devices;
mod input_gate;
mod jitter_buffer;
mod level;
mod mixer;
mod offer;
mod processing;
mod resampling;
mod session;
mod transport;

pub use codec::CodecError;
pub use devices::{
    AudioDeviceDescription, AudioDeviceError, AudioDevices, CaptureStream, PlaybackProbe,
    PlaybackStream, RunningStream, StreamFormat, SyntheticAudioDevices, SyntheticSound,
    SystemAudioDevices,
};
pub use input_gate::{
    DEFAULT_PUSH_TO_TALK_RELEASE_DELAY, DEFAULT_SENSITIVITY_DECIBELS, InputMode,
    MAXIMUM_PUSH_TO_TALK_RELEASE_DELAY, MAXIMUM_SENSITIVITY_DECIBELS, MINIMUM_SENSITIVITY_DECIBELS,
    Sensitivity,
};
pub use mixer::MAXIMUM_VOLUME;
pub use offer::{OfferError, ReceiveSlotKind};
pub use processing::{NoiseSuppressionMode, ProcessingSettings};
pub use resampling::ResamplingError;
pub use session::{
    AudioDirection, VoiceSession, VoiceSessionError, VoiceSessionEvent, VoiceSessionOptions,
    VoiceSignalling, VoiceToggles,
};
