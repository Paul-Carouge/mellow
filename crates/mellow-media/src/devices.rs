//! Audio devices behind a trait: the system's through `cpal`, or synthetic ones for tests and
//! headless use. A device stream moves samples through a wait-free ring buffer only: its
//! real-time callback never allocates, locks or blocks.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// One device as the settings list it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDeviceDescription {
    /// Stable across runs where the system allows it: store this one in the settings.
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// The format a device stream runs at; samples are interleaved `f32` in `[-1, 1]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, thiserror::Error)]
pub enum AudioDeviceError {
    #[error("no audio device is available")]
    NoDevice,
    #[error("the audio device has a sample format the engine does not handle")]
    UnsupportedFormat,
    #[error("the audio system refused: {0}")]
    System(String),
}

/// Keeps a device stream running; dropping it stops the stream.
pub trait RunningStream {}

/// An open microphone: its samples arrive in `samples`.
pub struct CaptureStream {
    pub format: StreamFormat,
    pub samples: rtrb::Consumer<f32>,
    /// Set by the stream when the device fails or disappears.
    pub has_failed: Arc<AtomicBool>,
    pub running: Box<dyn RunningStream>,
}

/// An open output: samples pushed into `samples` are played.
pub struct PlaybackStream {
    pub format: StreamFormat,
    pub samples: rtrb::Producer<f32>,
    pub has_failed: Arc<AtomicBool>,
    /// Interleaved samples the device took in its latest callback: its period, which a
    /// PipeWire graph sets to its quantum (2048 frames, 43 ms, by default at most; 8192 at
    /// the limit). Written by the device.
    pub period_samples: Arc<AtomicUsize>,
    pub running: Box<dyn RunningStream>,
}

/// Queued beyond one device period, so the engine's next 10 ms frame always lands in time.
const QUEUE_MARGIN_BEYOND_PERIOD_MILLISECONDS: u64 = 10;

impl PlaybackStream {
    /// How much audio to keep queued for the device: `minimum_milliseconds`, or more when the
    /// device's period is longer. A queue shorter than one period leaves the rest of every
    /// callback silent: 13 ms of each 43 ms with a 30 ms queue and a 2048-frame quantum.
    /// Never more than the ring buffer holds: a longer period (a driver's wrong value) would
    /// otherwise keep the engine writing into a full ring.
    #[must_use]
    pub fn queue_target_milliseconds(&self, minimum_milliseconds: u64) -> u64 {
        let samples_per_millisecond =
            u64::from(self.format.sample_rate) * u64::from(self.format.channels) / 1_000;
        let period = u64::try_from(self.period_samples.load(Ordering::Relaxed)).unwrap_or(0);
        let period_milliseconds = period.div_ceil(samples_per_millisecond.max(1));
        minimum_milliseconds
            .max(period_milliseconds + QUEUE_MARGIN_BEYOND_PERIOD_MILLISECONDS)
            .min(u64::from(RING_BUFFER_MILLISECONDS) - QUEUE_MARGIN_BEYOND_PERIOD_MILLISECONDS)
    }
}

impl std::fmt::Debug for CaptureStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CaptureStream")
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PlaybackStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlaybackStream")
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

/// One direction's devices, or why they could not be listed.
pub type DeviceListing = Result<Vec<AudioDeviceDescription>, AudioDeviceError>;

/// Lists and opens audio devices. Opened on the engine's audio thread, which keeps the streams.
pub trait AudioDevices: Send {
    /// # Errors
    /// The audio system cannot list devices.
    fn input_devices(&self) -> Result<Vec<AudioDeviceDescription>, AudioDeviceError>;
    /// # Errors
    /// The audio system cannot list devices.
    fn output_devices(&self) -> Result<Vec<AudioDeviceDescription>, AudioDeviceError>;
    /// Both lists at once, `(inputs, outputs)`; one connection to the audio system where listing
    /// opens one (`PipeWire`). Either list fails on its own.
    fn input_and_output_devices(&self) -> (DeviceListing, DeviceListing) {
        (self.input_devices(), self.output_devices())
    }
    /// Opens the microphone `device_id`, or the default one when `None` or no longer there.
    ///
    /// # Errors
    /// No device, an unsupported format, or the system refused (permission...).
    fn open_capture(&self, device_id: Option<&str>) -> Result<CaptureStream, AudioDeviceError>;
    /// Opens the output `device_id`, or the default one when `None` or no longer there.
    ///
    /// # Errors
    /// No device, an unsupported format, or the system refused.
    fn open_playback(&self, device_id: Option<&str>) -> Result<PlaybackStream, AudioDeviceError>;
}

/// Ring buffers hold this much audio: room for scheduling hiccups of the engine's thread.
const RING_BUFFER_MILLISECONDS: u32 = 250;

pub(crate) fn ring_buffer_capacity(format: StreamFormat) -> usize {
    let samples = u64::from(format.sample_rate)
        * u64::from(format.channels)
        * u64::from(RING_BUFFER_MILLISECONDS)
        / 1_000;
    usize::try_from(samples).unwrap_or(usize::MAX)
}

/// Which of the host's devices the settings list, by cpal host name. Linux hosts list more than
/// the devices the system's sound settings show:
/// - cpal's `PipeWire` host adds its three "follow the default" nodes (the settings' own "Default"
///   entry stands for them), every sink again as a capture source (it would record what the sink
///   plays; sinks are its only duplex nodes) and other apps' playback streams (outputs only), so
///   microphones are its input nodes and outputs its duplex ones.
///   Mellow's own streams (cpal names them `cpal-capture-<pid>` and `cpal-playback-<pid>`) are
///   nodes too: the test's or the call's microphone would list itself as one.
/// - cpal's ALSA host (no `PipeWire` daemon) adds every PCM hint and the raw `hw:` and `plughw:`
///   devices, often held exclusively; one `sysdefault:` entry per card is what a person picks,
///   plus `pulse` (the cards through a running `PulseAudio`, which holds them) and the `hdmi:`
///   outputs (screens, which `sysdefault:` leaves out).
///
/// shortcut: another app's recording stream is an input node too and stays listed while it
/// records, cpal not telling it from a microphone; filter it once cpal exposes the media class.
fn is_listed_device(
    host_name: &str,
    device_id: &str,
    direction: cpal::DeviceDirection,
    is_input: bool,
) -> bool {
    use cpal::DeviceDirection::{Duplex, Input, Output};
    match host_name {
        "PipeWire" if device_id.starts_with("cpal-") => false,
        "PipeWire" if is_input => direction == Input && device_id != "input_default",
        "PipeWire" => direction == Duplex && device_id != "sink_default",
        "ALSA" => {
            let is_card = device_id.starts_with("sysdefault:")
                || device_id == "pulse"
                || (!is_input && device_id.starts_with("hdmi:"));
            is_card
                && match direction {
                    Input => is_input,
                    Output => !is_input,
                    _ => true,
                }
        }
        _ => true,
    }
}

pub use system::SystemAudioDevices;

mod system {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
    use cpal::{FromSample, SampleFormat, SizedSample};

    use super::{
        AudioDeviceDescription, AudioDeviceError, AudioDevices, CaptureStream, DeviceListing,
        PlaybackStream, RunningStream, StreamFormat, is_listed_device, ring_buffer_capacity,
    };

    /// The system's audio devices: `CoreAudio`, WASAPI, `PipeWire` (ALSA without its daemon).
    #[derive(Debug, Default, Clone, Copy)]
    pub struct SystemAudioDevices;

    /// Owns the stream only so dropping it stops the device.
    struct CpalStream(#[allow(dead_code)] cpal::Stream);

    impl RunningStream for CpalStream {}

    fn system_error(error: &impl std::fmt::Display) -> AudioDeviceError {
        AudioDeviceError::System(error.to_string())
    }

    fn describe(
        devices: impl Iterator<Item = cpal::Device>,
        default_id: Option<&cpal::DeviceId>,
        is_input: bool,
    ) -> Vec<AudioDeviceDescription> {
        devices
            .filter_map(|device| {
                let id = device.id().ok()?;
                if !is_listed_device(id.host().name(), id.id(), direction_of(&device), is_input) {
                    return None;
                }
                let description = device.description().ok();
                // ALSA describes a device on two lines: its name, then what kind of PCM it is.
                let name = description.map_or_else(
                    || device.to_string(),
                    |description| {
                        description
                            .name()
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .to_owned()
                    },
                );
                Some(AudioDeviceDescription {
                    is_default: default_id == Some(&id),
                    id: id.to_string(),
                    name,
                })
            })
            .collect()
    }

    fn direction_of(device: &cpal::Device) -> cpal::DeviceDirection {
        device
            .description()
            .map_or(cpal::DeviceDirection::Unknown, |description| {
                description.direction()
            })
    }

    /// The device with `device_id`, or the default when absent, gone (unplugged since) or one the
    /// settings do not list: a stored sink monitor would otherwise send what the computer plays
    /// while the settings read "Unplugged device".
    fn chosen_device(
        host: &cpal::Host,
        device_id: Option<&str>,
        is_input: bool,
        default: impl FnOnce() -> Option<cpal::Device>,
    ) -> Result<cpal::Device, AudioDeviceError> {
        device_id
            .and_then(|id| id.parse::<cpal::DeviceId>().ok())
            .and_then(|id| {
                host.device_by_id(&id).filter(|device| {
                    is_listed_device(id.host().name(), id.id(), direction_of(device), is_input)
                })
            })
            .or_else(default)
            .ok_or(AudioDeviceError::NoDevice)
    }

    fn error_flag() -> (Arc<AtomicBool>, impl FnMut(cpal::Error) + Send + 'static) {
        let has_failed = Arc::new(AtomicBool::new(false));
        let flag = has_failed.clone();
        (has_failed, move |error: cpal::Error| {
            // Xruns are glitches, not failures; anything else means the device is unusable.
            if error.kind() != cpal::ErrorKind::Xrun {
                flag.store(true, Ordering::Relaxed);
            }
        })
    }

    fn build_capture<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
    ) -> Result<CaptureStream, AudioDeviceError>
    where
        T: SizedSample,
        f32: FromSample<T>,
    {
        let format = StreamFormat {
            sample_rate: config.sample_rate,
            channels: config.channels,
        };
        let (mut producer, consumer) = rtrb::RingBuffer::new(ring_buffer_capacity(format));
        let (has_failed, on_error) = error_flag();
        let stream = device
            .build_input_stream(
                *config,
                move |data: &[T], _: &cpal::InputCallbackInfo| {
                    // Real time: no allocation, no lock. Samples beyond the free room are lost.
                    let count = data.len().min(producer.slots());
                    if let Ok(chunk) = producer.write_chunk_uninit(count) {
                        chunk.fill_from_iter(data.iter().map(|sample| sample.to_sample::<f32>()));
                    }
                },
                on_error,
                None,
            )
            .map_err(|error| system_error(&error))?;
        stream.play().map_err(|error| system_error(&error))?;
        Ok(CaptureStream {
            format,
            samples: consumer,
            has_failed,
            running: Box::new(CpalStream(stream)),
        })
    }

    fn build_playback<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
    ) -> Result<PlaybackStream, AudioDeviceError>
    where
        T: SizedSample + FromSample<f32>,
    {
        let format = StreamFormat {
            sample_rate: config.sample_rate,
            channels: config.channels,
        };
        let (producer, mut consumer) = rtrb::RingBuffer::new(ring_buffer_capacity(format));
        let (has_failed, on_error) = error_flag();
        let period_samples = Arc::new(AtomicUsize::new(0));
        let device_period = period_samples.clone();
        let stream = device
            .build_output_stream(
                *config,
                move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                    // Real time: no allocation, no lock. Missing samples play as silence.
                    device_period.store(data.len(), Ordering::Relaxed);
                    let count = data.len().min(consumer.slots());
                    let (played, missing) = data.split_at_mut(count);
                    if let Ok(chunk) = consumer.read_chunk(count) {
                        for (output, sample) in played.iter_mut().zip(chunk) {
                            *output = T::from_sample(sample);
                        }
                    }
                    missing.fill(T::EQUILIBRIUM);
                },
                on_error,
                None,
            )
            .map_err(|error| system_error(&error))?;
        stream.play().map_err(|error| system_error(&error))?;
        Ok(PlaybackStream {
            format,
            samples: producer,
            has_failed,
            period_samples,
            running: Box::new(CpalStream(stream)),
        })
    }

    fn input_devices_of(host: &cpal::Host) -> DeviceListing {
        let default_id = host
            .default_input_device()
            .and_then(|device| device.id().ok());
        let devices = host.input_devices().map_err(|error| system_error(&error))?;
        Ok(describe(devices, default_id.as_ref(), true))
    }

    fn output_devices_of(host: &cpal::Host) -> DeviceListing {
        let default_id = host
            .default_output_device()
            .and_then(|device| device.id().ok());
        let devices = host
            .output_devices()
            .map_err(|error| system_error(&error))?;
        Ok(describe(devices, default_id.as_ref(), false))
    }

    impl AudioDevices for SystemAudioDevices {
        fn input_devices(&self) -> DeviceListing {
            input_devices_of(&cpal::default_host())
        }

        fn output_devices(&self) -> DeviceListing {
            output_devices_of(&cpal::default_host())
        }

        fn input_and_output_devices(&self) -> (DeviceListing, DeviceListing) {
            let host = cpal::default_host();
            (input_devices_of(&host), output_devices_of(&host))
        }

        fn open_capture(&self, device_id: Option<&str>) -> Result<CaptureStream, AudioDeviceError> {
            let host = cpal::default_host();
            let device = chosen_device(&host, device_id, true, || host.default_input_device())?;
            let supported = device
                .default_input_config()
                .map_err(|error| system_error(&error))?;
            let config = supported.config();
            match supported.sample_format() {
                SampleFormat::F32 => build_capture::<f32>(&device, &config),
                SampleFormat::I16 => build_capture::<i16>(&device, &config),
                SampleFormat::I32 => build_capture::<i32>(&device, &config),
                SampleFormat::U16 => build_capture::<u16>(&device, &config),
                _ => Err(AudioDeviceError::UnsupportedFormat),
            }
        }

        fn open_playback(
            &self,
            device_id: Option<&str>,
        ) -> Result<PlaybackStream, AudioDeviceError> {
            let host = cpal::default_host();
            let device = chosen_device(&host, device_id, false, || host.default_output_device())?;
            let supported = device
                .default_output_config()
                .map_err(|error| system_error(&error))?;
            let config = supported.config();
            match supported.sample_format() {
                SampleFormat::F32 => build_playback::<f32>(&device, &config),
                SampleFormat::I16 => build_playback::<i16>(&device, &config),
                SampleFormat::I32 => build_playback::<i32>(&device, &config),
                SampleFormat::U16 => build_playback::<u16>(&device, &config),
                _ => Err(AudioDeviceError::UnsupportedFormat),
            }
        }
    }
}

pub use synthetic::{PlaybackProbe, SyntheticAudioDevices, SyntheticSound};

mod synthetic {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use super::{
        AudioDeviceDescription, AudioDeviceError, AudioDevices, CaptureStream, PlaybackStream,
        RunningStream, StreamFormat, ring_buffer_capacity,
    };

    /// How often the synthetic devices move audio, like a sound card's period.
    const PERIOD: Duration = Duration::from_millis(10);
    /// A played sample louder than this counts as heard.
    const AUDIBLE_AMPLITUDE: f32 = 0.05;

    /// What the synthetic microphone hears.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum SyntheticSound {
        Silence,
        /// A sine wave at `frequency` hertz and `amplitude` (full scale is 1).
        Sine {
            frequency: f32,
            amplitude: f32,
        },
    }

    /// What the synthetic output played, readable from any thread.
    #[derive(Debug, Default)]
    pub struct PlaybackProbe {
        played_samples: AtomicU64,
        audible_samples: AtomicU64,
    }

    impl PlaybackProbe {
        #[must_use]
        pub fn played_samples(&self) -> u64 {
            self.played_samples.load(Ordering::Relaxed)
        }

        /// Samples played louder than a whisper: proof that decoded audio came out.
        #[must_use]
        pub fn audible_samples(&self) -> u64 {
            self.audible_samples.load(Ordering::Relaxed)
        }
    }

    /// A microphone playing a synthetic sound and an output that measures what it plays, both
    /// paced in real time by their own threads: the engine runs as with real devices, without
    /// any.
    #[derive(Debug, Clone)]
    pub struct SyntheticAudioDevices {
        sound: SyntheticSound,
        capture_format: StreamFormat,
        playback_format: StreamFormat,
        probe: Arc<PlaybackProbe>,
    }

    impl SyntheticAudioDevices {
        #[must_use]
        pub fn new(
            sound: SyntheticSound,
            capture_format: StreamFormat,
            playback_format: StreamFormat,
        ) -> Self {
            Self {
                sound,
                capture_format,
                playback_format,
                probe: Arc::new(PlaybackProbe::default()),
            }
        }

        #[must_use]
        pub fn probe(&self) -> Arc<PlaybackProbe> {
            self.probe.clone()
        }
    }

    /// A pacing thread, stopped and joined on drop.
    struct PacedThread {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl RunningStream for PacedThread {}

    impl Drop for PacedThread {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                // A panicked pacing thread has nothing left to clean up.
                let _ = thread.join();
            }
        }
    }

    /// Runs `period` every `PERIOD` on its own thread, on a fixed schedule so it does not drift.
    fn paced(mut period: impl FnMut() + Send + 'static) -> PacedThread {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut next = Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    period();
                    next += PERIOD;
                    std::thread::sleep(next.saturating_duration_since(Instant::now()));
                }
            })
        };
        PacedThread {
            stop,
            thread: Some(thread),
        }
    }

    fn frames_per_period(format: StreamFormat) -> usize {
        usize::try_from(format.sample_rate / 100).unwrap_or(0)
    }

    impl AudioDevices for SyntheticAudioDevices {
        fn input_devices(&self) -> Result<Vec<AudioDeviceDescription>, AudioDeviceError> {
            Ok(vec![AudioDeviceDescription {
                id: "synthetic:input".into(),
                name: "Synthetic microphone".into(),
                is_default: true,
            }])
        }

        fn output_devices(&self) -> Result<Vec<AudioDeviceDescription>, AudioDeviceError> {
            Ok(vec![AudioDeviceDescription {
                id: "synthetic:output".into(),
                name: "Synthetic output".into(),
                is_default: true,
            }])
        }

        fn open_capture(&self, _: Option<&str>) -> Result<CaptureStream, AudioDeviceError> {
            let format = self.capture_format;
            let (mut producer, consumer) = rtrb::RingBuffer::new(ring_buffer_capacity(format));
            let sound = self.sound;
            let frames = frames_per_period(format);
            let channels = usize::from(format.channels);
            #[allow(clippy::cast_precision_loss)] // sample rates are far below 2^24
            let sample_rate = format.sample_rate as f32;
            let mut phase: f32 = 0.0;
            let running = paced(move || {
                for _ in 0..frames {
                    let sample = match sound {
                        SyntheticSound::Silence => 0.0,
                        SyntheticSound::Sine {
                            frequency,
                            amplitude,
                        } => {
                            phase = (phase + std::f32::consts::TAU * frequency / sample_rate)
                                % std::f32::consts::TAU;
                            phase.sin() * amplitude
                        }
                    };
                    for _ in 0..channels {
                        // A full ring buffer drops samples, as a real device overrun would.
                        let _ = producer.push(sample);
                    }
                }
            });
            Ok(CaptureStream {
                format,
                samples: consumer,
                has_failed: Arc::new(AtomicBool::new(false)),
                running: Box::new(running),
            })
        }

        fn open_playback(&self, _: Option<&str>) -> Result<PlaybackStream, AudioDeviceError> {
            let format = self.playback_format;
            let (producer, mut consumer) = rtrb::RingBuffer::new(ring_buffer_capacity(format));
            let samples_per_period = frames_per_period(format) * usize::from(format.channels);
            let probe = self.probe.clone();
            let running = paced(move || {
                let count = samples_per_period.min(consumer.slots());
                let Ok(chunk) = consumer.read_chunk(count) else {
                    return;
                };
                let audible = chunk
                    .into_iter()
                    .filter(|sample: &f32| sample.abs() > AUDIBLE_AMPLITUDE)
                    .count();
                probe
                    .played_samples
                    .fetch_add(u64::try_from(count).unwrap_or(0), Ordering::Relaxed);
                probe
                    .audible_samples
                    .fetch_add(u64::try_from(audible).unwrap_or(0), Ordering::Relaxed);
            });
            Ok(PlaybackStream {
                format,
                samples: producer,
                has_failed: Arc::new(AtomicBool::new(false)),
                period_samples: Arc::new(AtomicUsize::new(samples_per_period)),
                running: Box::new(running),
            })
        }
    }
}

/// Whether a stream reported a failure; for the engine's reopen logic.
pub(crate) fn has_failed(flag: &AtomicBool) -> bool {
    flag.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use cpal::DeviceDirection::{Duplex, Input, Output};

    use super::{AudioDevices as _, DeviceListing, SystemAudioDevices, is_listed_device};

    #[test]
    fn pipewire_lists_its_sources_as_microphones_and_its_sinks_as_outputs() {
        let microphone = "alsa_input.usb-Blue_Yeti-00.analog-stereo";
        let sink = "alsa_output.pci-0000_00_1f.3.analog-stereo";
        assert!(is_listed_device("PipeWire", microphone, Input, true));
        assert!(!is_listed_device("PipeWire", microphone, Input, false));
        assert!(is_listed_device("PipeWire", sink, Duplex, false));
        // A sink as a capture source records what it plays: not a microphone.
        assert!(!is_listed_device("PipeWire", sink, Duplex, true));
        // Another app's playback stream.
        assert!(!is_listed_device("PipeWire", "Firefox", Output, false));
    }

    #[test]
    fn pipewire_leaves_out_mellow_s_own_streams() {
        assert!(!is_listed_device(
            "PipeWire",
            "cpal-capture-4242",
            Input,
            true
        ));
        assert!(!is_listed_device(
            "PipeWire",
            "cpal-playback-4242",
            Output,
            false
        ));
    }

    #[test]
    fn pipewire_s_default_nodes_are_the_settings_default_entry() {
        assert!(!is_listed_device("PipeWire", "input_default", Input, true));
        assert!(!is_listed_device("PipeWire", "sink_default", Duplex, false));
        assert!(!is_listed_device(
            "PipeWire",
            "output_default",
            Output,
            false
        ));
    }

    #[test]
    fn alsa_lists_one_entry_per_card() {
        assert!(is_listed_device(
            "ALSA",
            "sysdefault:CARD=PCH",
            Duplex,
            true
        ));
        assert!(is_listed_device(
            "ALSA",
            "sysdefault:CARD=PCH",
            Duplex,
            false
        ));
        assert!(!is_listed_device(
            "ALSA",
            "sysdefault:CARD=Mic",
            Input,
            false
        ));
        assert!(is_listed_device("ALSA", "pulse", Duplex, true));
        assert!(is_listed_device("ALSA", "pulse", Duplex, false));
        assert!(is_listed_device(
            "ALSA",
            "hdmi:CARD=HDMI,DEV=0",
            Output,
            false
        ));
        assert!(!is_listed_device(
            "ALSA",
            "hdmi:CARD=HDMI,DEV=0",
            Duplex,
            true
        ));
        for raw in [
            "hw:CARD=0,DEV=0",
            "plughw:CARD=0,DEV=0",
            "front:CARD=PCH,DEV=0",
            "default",
        ] {
            assert!(!is_listed_device("ALSA", raw, Duplex, false), "{raw}");
        }
    }

    #[test]
    fn other_hosts_list_everything() {
        assert!(is_listed_device(
            "CoreAudio",
            "BuiltInMicrophoneDevice",
            Input,
            true
        ));
        assert!(is_listed_device(
            "WASAPI",
            "{0.0.1.00000000}",
            Duplex,
            false
        ));
    }

    /// Runs the system's own listing (WASAPI on the Windows CI runner, which has no sound card and
    /// may have no audio service): whatever it finds, it answers without panicking, every id parses
    /// back as a `cpal::DeviceId`, and at most one entry is the default. The runner most likely
    /// lists no device, so there this mostly proves the listing does not panic.
    #[test]
    fn the_system_listing_answers_with_well_formed_entries() {
        fn check(listing: DeviceListing) {
            let Ok(devices) = listing else {
                return;
            };
            assert!(devices.iter().filter(|device| device.is_default).count() <= 1);
            for device in &devices {
                assert!(device.id.parse::<cpal::DeviceId>().is_ok(), "{}", device.id);
            }
        }
        let (inputs, outputs) = SystemAudioDevices.input_and_output_devices();
        check(inputs);
        check(outputs);
    }
}
