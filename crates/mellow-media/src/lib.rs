//! The desktop client's voice engine (ADR-0051), without UI: a `str0m` WebRTC peer speaking the
//! server's exact offer shape, audio devices behind a trait, microphone processing (echo
//! cancellation, noise suppression, gain), Opus, jitter buffers with loss concealment and drift
//! steering, mixing, voice activity and push-to-talk, the voice enhancement, reconnection as the
//! web client does it, voice cues, the message chime and soundboard playback.
//!
//! Threads: a network thread owns the UDP socket, the peer and the connection's state machine
//! (a new socket and peer per offer); an audio thread owns the codecs,
//! the processing and the engine's end of the devices' ring buffers; device callbacks only move
//! samples through wait-free ring buffers. Video lines are in the offer, as the server requires,
//! but carry nothing yet.

mod audio_engine;
mod audio_format;
mod codec;
mod connection;
mod devices;
mod enhancement;
mod input_gate;
mod jitter_buffer;
mod level;
mod mixer;
mod offer;
mod person_settings;
mod playback;
mod processing;
mod resampling;
mod session;
mod sound_player;
mod soundboard;
mod sounds;
mod transport;

pub use codec::CodecError;
pub use connection::{ConnectionProblem, VoicePhase};
pub use devices::{
    AudioDeviceDescription, AudioDeviceError, AudioDevices, CaptureStream, PlaybackProbe,
    PlaybackStream, RunningStream, StreamFormat, SyntheticAudioDevices, SyntheticSound,
    SystemAudioDevices,
};
pub use enhancement::{
    EqualizerDecibels, MAXIMUM_EQUALIZER_DECIBELS, MINIMUM_EQUALIZER_DECIBELS,
    VoiceEnhancementPreset,
};
pub use input_gate::{
    DEFAULT_PUSH_TO_TALK_RELEASE_DELAY, DEFAULT_SENSITIVITY_DECIBELS, InputMode,
    MAXIMUM_PUSH_TO_TALK_RELEASE_DELAY, MAXIMUM_SENSITIVITY_DECIBELS, MINIMUM_SENSITIVITY_DECIBELS,
    Sensitivity,
};
pub use mixer::MAXIMUM_VOLUME;
pub use offer::{OfferError, ReceiveSlotKind};
pub use person_settings::{
    InMemoryPersonVoiceSettings, PersonSoundSource, PersonVoiceSetting, PersonVoiceSettingsStore,
};
pub use processing::{NoiseSuppressionMode, ProcessingSettings};
pub use resampling::ResamplingError;
pub use session::{
    AudioDirection, GOOD_ROUND_TRIP, LatencyQuality, MEDIUM_ROUND_TRIP, VoiceSession,
    VoiceSessionError, VoiceSessionEvent, VoiceSessionOptions, VoiceSignalling, VoiceToggles,
    latency_quality,
};
pub use sound_player::SoundPlayer;
pub use soundboard::{
    DEFAULT_SOUNDBOARD_VOLUME, MAXIMUM_SOUND_FILE_BYTES, MAXIMUM_SOUNDBOARD_VOLUME, SoundCache,
    SoundboardError, decode_canonical_wav, download_sound, soundboard_gain,
};
pub use sounds::{LocalSound, VoiceCue, render_notification_chime};
