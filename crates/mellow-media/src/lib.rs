//! The desktop client's voice engine (ADR-0051), without UI: a `str0m` WebRTC peer speaking the
//! server's exact offer shape, audio devices behind a trait, microphone processing (echo
//! cancellation, noise suppression, gain), Opus, jitter buffers with loss concealment and drift
//! steering, mixing, voice activity and push-to-talk, the voice enhancement, reconnection as the
//! web client does it, voice cues, the message chime, soundboard playback and audio attachments.
//!
//! Threads: a network thread owns the UDP socket, the peer and the connection's state machine
//! (a new socket and peer per offer); an audio thread owns the codecs,
//! the processing and the engine's end of the devices' ring buffers; device callbacks only move
//! samples through wait-free ring buffers. Video: an encoding thread turns the camera's or
//! screen's pictures into three VP8 simulcast layers (libvpx, built from source by `build.rs`),
//! and one decoding thread per watched slot turns received frames (VP8, or H.264 where the
//! system decodes it: `VideoToolbox`, FFmpeg on Linux) into pictures for the interface.

mod audio_engine;
mod audio_format;
mod camera;
#[cfg(any(target_os = "linux", test))]
mod camera_conversion;
#[cfg(target_os = "linux")]
mod camera_linux;
// AVFoundation is reached only through `unsafe` Objective-C calls; each states its invariant.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod camera_macos;
mod clip_decoder;
mod clip_player;
mod codec;
mod connection;
mod devices;
mod enhancement;
mod h264;
// FFmpeg's C API, reached through `unsafe` calls; each states its invariant.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod h264_ffmpeg;
// VideoToolbox and CoreMedia, reached through `unsafe` calls; each states its invariant.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod h264_videotoolbox;
mod input_gate;
mod jitter_buffer;
mod level;
mod microphone_test;
mod mixer;
mod offer;
mod person_settings;
mod playback;
mod processing;
mod resampling;
mod session;
mod simulcast;
mod sound_player;
mod sound_source;
mod soundboard;
mod sounds;
mod transport;
mod uplink;
mod video_frame;
mod video_receiver;
mod video_sender;
mod vp8;
mod watch_list;

pub use camera::{
    CameraDescription, CameraError, CameraFeed, Cameras, RunningCapture, SyntheticCameras,
    SystemCameras,
};
pub use clip_decoder::{ClipError, PLAYABLE_ATTACHMENT_AUDIO_TYPES};
pub use clip_player::{ClipPhase, ClipPlayer, ClipStatus};
pub use codec::CodecError;
pub use connection::{ConnectionProblem, VoicePhase};
pub use devices::{
    AudioDeviceDescription, AudioDeviceError, AudioDevices, CaptureStream, DeviceListing,
    PlaybackProbe, PlaybackStream, RunningStream, StreamFormat, SyntheticAudioDevices,
    SyntheticSound, SystemAudioDevices,
};
pub use enhancement::{
    EqualizerDecibels, MAXIMUM_EQUALIZER_DECIBELS, MINIMUM_EQUALIZER_DECIBELS,
    VoiceEnhancementPreset,
};
pub use h264::decodes_video_codec;
pub use input_gate::{
    DEFAULT_PUSH_TO_TALK_RELEASE_DELAY, DEFAULT_SENSITIVITY_DECIBELS, InputMode,
    MAXIMUM_PUSH_TO_TALK_RELEASE_DELAY, MAXIMUM_SENSITIVITY_DECIBELS, MINIMUM_SENSITIVITY_DECIBELS,
    Sensitivity,
};
pub use microphone_test::{MicrophoneTest, MicrophoneTestOptions};
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
    WatchedVideo, is_voice_session_event, latency_quality,
};
pub use simulcast::{ScreenShareQuality, VideoSourceKind};
pub use sound_player::SoundPlayer;
pub use sound_source::{MAXIMUM_SOUND_SOURCE_SECONDS, SoundSourceError, decode_sound_source};
pub use soundboard::{
    DEFAULT_SOUNDBOARD_VOLUME, MAXIMUM_SOUND_FILE_BYTES, MAXIMUM_SOUNDBOARD_VOLUME, SoundCache,
    SoundboardError, decode_canonical_wav, download_sound, soundboard_gain,
};
pub use sounds::{
    LocalSound, SoundboardPlayer, SoundboardSoundsToStop, VoiceCue, render_notification_chime,
};
pub use video_frame::{CapturedFrame, CapturedPixelFormat, DecodedPixelFormat, DecodedVideoFrame};
pub use video_receiver::VideoFrames;
pub use video_sender::VideoInput;
