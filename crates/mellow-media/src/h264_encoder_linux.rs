//! A shared screen's three H.264 layers encoded on the GPU through VA-API (Vesper's
//! `vesper-encode` in its real-time mode, ADR-0054): DMA-BUF pictures from the portal stay on
//! the GPU, the top layer is encoded at the quality's size and the two lower ones are scaled from
//! it on the GPU. Runs on its own thread, fed the newest picture through a one-slot queue, so the
//! capture callback never waits for it.

use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mellow_protocol::SimulcastLayer;
use vesper_core::VideoFrame;
use vesper_encode::Encoder;

use crate::h264::{is_h264_keyframe, nal_unit_type, nal_units};
use crate::h264_stream::{ParameterSets, is_constrained_baseline_sequence_parameter_set};
use crate::screen_capture::ScreenShareContext;
use crate::session::ScreenShareEnd;
use crate::simulcast::{FRAMES_PER_SECOND, LAYERS, VideoSourceKind, layer_size};
use crate::video_frame::fit_within;
use crate::video_sender::EncodedVideoInput;

/// Without a new picture (the portal sends one only when the screen changes), the last one is
/// encoded again this often, as the VP8 path does: late watchers get a picture and the server
/// never sees the screen as silent.
const LONGEST_INTERVAL_BETWEEN_FRAMES: Duration = Duration::from_secs(1);
/// How often the thread looks at the server's requests while no picture comes.
const REQUEST_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// The size and rate of the probe that tells whether the GPU encodes Constrained Baseline.
const PROBE_SIZE: (u32, u32) = (640, 360);
const PROBE_BITS_PER_SECOND: u64 = 500_000;
const SEQUENCE_PARAMETER_SET: u8 = 7;

/// Whether this machine's VA-API driver opens a real-time Constrained Baseline H.264 encoder.
pub(crate) fn can_encode_h264() -> bool {
    match Encoder::new_real_time_h264(
        PROBE_SIZE.0,
        PROBE_SIZE.1,
        FRAMES_PER_SECOND,
        PROBE_BITS_PER_SECOND,
    ) {
        Ok(_) => true,
        Err(error) => {
            tracing::info!(%error, "no VA-API H.264 encoder: screens are shared in VP8");
            false
        }
    }
}

/// Where the capture puts its pictures: the newest waits, any other is dropped (and goes back
/// to the compositor at once).
pub(crate) struct H264PictureSlot {
    sender: SyncSender<VideoFrame>,
}

impl H264PictureSlot {
    /// Never blocks.
    pub(crate) fn offer(&self, frame: VideoFrame) {
        // A full slot or a stopped encoder drops the picture, which releases its buffer.
        let _ = self.sender.try_send(frame);
    }
}

/// The encoding thread; dropping it waits for it, once its input closed.
pub(crate) struct H264ScreenEncoder {
    thread: Option<JoinHandle<()>>,
}

impl Drop for H264ScreenEncoder {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the screen's H.264 encoding thread panicked");
        }
    }
}

/// Starts encoding into `input` what arrives in the returned slot, until `input` closes or the
/// slot is dropped. A failure ends the share through `share`.
pub(crate) fn start(
    input: EncodedVideoInput,
    share: ScreenShareContext,
) -> std::io::Result<(H264ScreenEncoder, H264PictureSlot)> {
    let (sender, pictures) = std::sync::mpsc::sync_channel(1);
    let thread = std::thread::Builder::new()
        .name("mellow-screen-h264".into())
        .spawn(move || {
            if let Err(error) = run(input, &pictures) {
                share.end(ScreenShareEnd::Failed(error));
            }
        })?;
    Ok((
        H264ScreenEncoder {
            thread: Some(thread),
        },
        H264PictureSlot { sender },
    ))
}

/// One layer's encoder and what it needs to send its frames.
struct LayerEncoder {
    layer: SimulcastLayer,
    encoder: Encoder,
    parameter_sets: ParameterSets,
    is_active: bool,
    /// Whether this layer's first SPS was checked to be Constrained Baseline.
    is_profile_checked: bool,
}

/// The three layers of one source size and quality.
struct Layers {
    kind: VideoSourceKind,
    source_size: (u32, u32),
    /// Top first: the lower ones scale its last picture.
    encoders: Vec<LayerEncoder>,
}

impl Layers {
    fn open(kind: VideoSourceKind, source_size: (u32, u32)) -> Result<Self, String> {
        let VideoSourceKind::Screen(quality) = kind else {
            return Err("only screens are encoded in H.264".into());
        };
        let top = fit_within(source_size, quality.top_size());
        let open = |layer: SimulcastLayer, encoder: Encoder| LayerEncoder {
            layer,
            encoder,
            parameter_sets: ParameterSets::default(),
            is_active: true,
            is_profile_checked: false,
        };
        let top_encoder = Encoder::new_real_time_h264(
            top.0,
            top.1,
            FRAMES_PER_SECOND,
            u64::from(kind.bits_per_second(SimulcastLayer::High)),
        )
        .map_err(|error| error.to_string())?;
        let mut encoders = Vec::with_capacity(LAYERS.len());
        for layer in [SimulcastLayer::Low, SimulcastLayer::Quarter] {
            let size = layer_size(top, layer);
            let encoder = top_encoder
                .new_scaled_real_time_h264(
                    size.0,
                    size.1,
                    FRAMES_PER_SECOND,
                    u64::from(kind.bits_per_second(layer)),
                )
                .map_err(|error| error.to_string())?;
            encoders.push(open(layer, encoder));
        }
        encoders.insert(0, open(SimulcastLayer::High, top_encoder));
        Ok(Self {
            kind,
            source_size,
            encoders,
        })
    }

    /// Sends only the `active` layers (given every time: encoders reopened for a new size or
    /// quality must not send a layer the server or the uplink paused) and forces the keyframes
    /// asked for.
    fn apply(&mut self, active: &[SimulcastLayer], keyframes: &[Option<SimulcastLayer>]) {
        for layer in &mut self.encoders {
            let is_active = active.contains(&layer.layer);
            // A resumed layer's watchers wait for a keyframe: send it at once.
            if is_active && !layer.is_active {
                layer.encoder.force_keyframe();
            }
            layer.is_active = is_active;
            if keyframes
                .iter()
                .any(|asked| asked.is_none_or(|asked| asked == layer.layer))
            {
                layer.encoder.force_keyframe();
            }
        }
    }

    /// Encodes `frame` (or repeats the last picture) on every active layer and sends the frames.
    /// The top layer always encodes: the lower ones scale its picture.
    fn encode(
        &mut self,
        frame: Option<VideoFrame>,
        input: &EncodedVideoInput,
    ) -> Result<(), String> {
        let rtp_timestamp = input.rtp_time_of(Instant::now());
        // The portal's pictures are stamped on `CLOCK_MONOTONIC`: repeats follow them.
        let pts = vesper_audio::monotonic_now();
        let Some((top, lower)) = self.encoders.split_first_mut() else {
            return Ok(());
        };
        let packets = match frame {
            Some(frame) => top.encoder.encode(frame),
            None => top.encoder.repeat(pts),
        }
        .map_err(|error| error.to_string())?;
        send_packets(top, packets, rtp_timestamp, input)?;
        for layer in lower.iter_mut().filter(|layer| layer.is_active) {
            let packets = layer
                .encoder
                .encode_scaled(&top.encoder, pts)
                .map_err(|error| error.to_string())?;
            send_packets(layer, packets, rtp_timestamp, input)?;
        }
        Ok(())
    }
}

/// Sends a layer's packets as access units, with the parameter sets before every IDR; refuses a
/// stream whose profile is not Constrained Baseline (the server would drop all of it).
fn send_packets(
    layer: &mut LayerEncoder,
    packets: Vec<vesper_core::EncodedPacket>,
    rtp_timestamp: u64,
    input: &EncodedVideoInput,
) -> Result<(), String> {
    for packet in packets {
        if !layer.is_profile_checked
            && let Some(sequence_set) = nal_units(&packet.data)
                .find(|unit| nal_unit_type(unit) == Some(SEQUENCE_PARAMETER_SET))
        {
            if !is_constrained_baseline_sequence_parameter_set(sequence_set) {
                return Err("the GPU encoder does not write Constrained Baseline".into());
            }
            layer.is_profile_checked = true;
        }
        let access_unit = layer.parameter_sets.complete(&packet.data);
        if !layer.is_active {
            continue;
        }
        if is_h264_keyframe(&access_unit) && !layer.is_profile_checked {
            // An IDR the receiver cannot place; the next keyframe carries its SPS.
            layer.encoder.force_keyframe();
            continue;
        }
        input.push(layer.layer, rtp_timestamp, access_unit.into_owned());
    }
    Ok(())
}

fn run(mut input: EncodedVideoInput, pictures: &Receiver<VideoFrame>) -> Result<(), String> {
    let mut layers: Option<Layers> = None;
    let mut last_encoded_at: Option<Instant> = None;
    // The server only says when its layers change: kept for encoders opened later.
    let mut active_layers = LAYERS.to_vec();
    while input.is_open() {
        let frame = match pictures.recv_timeout(REQUEST_POLL_INTERVAL) {
            Ok(frame) => Some(frame),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        };
        let Some(kind) = input.kind() else {
            return Ok(());
        };
        let requests = input.take_requests();
        if let Some(active) = requests.active_layers {
            active_layers = active;
        }
        // A new quality or source size needs encoders of other sizes.
        if let Some(frame) = &frame
            && layers.as_ref().is_none_or(|layers| {
                layers.kind != kind || layers.source_size != (frame.width, frame.height)
            })
        {
            layers = Some(Layers::open(kind, (frame.width, frame.height))?);
        }
        let Some(current) = layers.as_mut() else {
            continue;
        };
        current.apply(&active_layers, &requests.keyframes);
        let now = Instant::now();
        let is_repeat_due = last_encoded_at
            .is_some_and(|at| now.saturating_duration_since(at) >= LONGEST_INTERVAL_BETWEEN_FRAMES);
        if frame.is_none() && requests.keyframes.is_empty() && !is_repeat_due {
            continue;
        }
        current.encode(frame, &input)?;
        last_encoded_at = Some(now);
    }
    Ok(())
}
