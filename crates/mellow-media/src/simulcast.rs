//! The three simulcast layers a camera or screen sends (ADR-0035, ADR-0037), with the web
//! client's sizes and bit rates (`web/src/lib/camera.ts`, `screen-share.ts`): one VP8 encoder per
//! layer, layers nobody watches paused as the server says, keyframes per layer on request.

use mellow_protocol::SimulcastLayer;

use crate::video_frame::{I420Picture, I420View, fit_within};
use crate::vp8::{VideoContent, Vp8Encoder, Vp8EncoderSettings, Vp8Error};

/// Cameras and screens alike, as on the web.
pub(crate) const FRAMES_PER_SECOND: u32 = 30;
const QUARTER_BITS_PER_SECOND: u32 = 150_000;
const LOW_BITS_PER_SECOND: u32 = 500_000;
const CAMERA_TOP_BITS_PER_SECOND: u32 = 1_500_000;
const CAMERA_TOP_SIZE: (u32, u32) = (1_280, 720);

/// Lowest first, as the offer lists them.
pub(crate) const LAYERS: [SimulcastLayer; 3] = [
    SimulcastLayer::Quarter,
    SimulcastLayer::Low,
    SimulcastLayer::High,
];

/// The picture size and top bit rate of a shared screen (`SCREEN_SHARE_PROFILES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScreenShareQuality {
    /// 1280 × 720 at 2.5 Mbit/s.
    Standard,
    /// 1920 × 1080 at 5 Mbit/s, the web's default.
    #[default]
    High,
    /// 2560 × 1440 at 8 Mbit/s.
    Highest,
}

impl ScreenShareQuality {
    const fn top_size(self) -> (u32, u32) {
        match self {
            Self::Standard => (1_280, 720),
            Self::High => (1_920, 1_080),
            Self::Highest => (2_560, 1_440),
        }
    }

    const fn top_bits_per_second(self) -> u32 {
        match self {
            Self::Standard => 2_500_000,
            Self::High => 5_000_000,
            Self::Highest => 8_000_000,
        }
    }
}

/// What the participant's one video source is: a camera or a screen, never both (owner's rule,
/// ADR-0037).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoSourceKind {
    Camera,
    Screen(ScreenShareQuality),
}

impl VideoSourceKind {
    fn content(self) -> VideoContent {
        match self {
            Self::Camera => VideoContent::Camera,
            Self::Screen(_) => VideoContent::Screen,
        }
    }

    fn top_size(self) -> (u32, u32) {
        match self {
            Self::Camera => CAMERA_TOP_SIZE,
            Self::Screen(quality) => quality.top_size(),
        }
    }

    /// The bit rate of `layer`: `q` and `l` keep the instance's budgets whatever the source,
    /// `h` follows the source.
    pub(crate) fn bits_per_second(self, layer: SimulcastLayer) -> u32 {
        match (layer, self) {
            (SimulcastLayer::Quarter, _) => QUARTER_BITS_PER_SECOND,
            (SimulcastLayer::Low, _) => LOW_BITS_PER_SECOND,
            (SimulcastLayer::High, Self::Camera) => CAMERA_TOP_BITS_PER_SECOND,
            (SimulcastLayer::High, Self::Screen(quality)) => quality.top_bits_per_second(),
        }
    }
}

/// `scaleResolutionDownBy` of each layer.
const fn scale_down_by(layer: SimulcastLayer) -> u32 {
    match layer {
        SimulcastLayer::Quarter => 4,
        SimulcastLayer::Low => 2,
        SimulcastLayer::High => 1,
    }
}

/// The size of `layer` for a top layer of `top`: even sides, at least 2.
pub(crate) fn layer_size(top: (u32, u32), layer: SimulcastLayer) -> (u32, u32) {
    let divisor = scale_down_by(layer);
    let even = |length: u32| ((length / divisor) & !1).max(2);
    (even(top.0), even(top.1))
}

/// The RID of a layer in the SDP.
pub(crate) const fn rid_of(layer: SimulcastLayer) -> &'static str {
    match layer {
        SimulcastLayer::Quarter => "q",
        SimulcastLayer::Low => "l",
        SimulcastLayer::High => "h",
    }
}

/// One layer's encoded frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EncodedLayerFrame {
    pub(crate) layer: SimulcastLayer,
    pub(crate) bytes: Vec<u8>,
    pub(crate) is_keyframe: bool,
}

#[derive(Debug)]
struct LayerEncoder {
    layer: SimulcastLayer,
    is_active: bool,
    is_keyframe_due: bool,
    encoder: Option<Vp8Encoder>,
}

/// The three encoders of the participant's video source.
#[derive(Debug)]
pub(crate) struct SimulcastEncoder {
    kind: VideoSourceKind,
    layers: [LayerEncoder; 3],
}

impl SimulcastEncoder {
    /// Every layer starts active, as a browser's encodings do, until the server says otherwise.
    pub(crate) fn new(kind: VideoSourceKind) -> Self {
        Self {
            kind,
            layers: LAYERS.map(|layer| LayerEncoder {
                layer,
                is_active: true,
                is_keyframe_due: true,
                encoder: None,
            }),
        }
    }

    pub(crate) fn kind(&self) -> VideoSourceKind {
        self.kind
    }

    /// A new screen quality changes only the bit rates; a camera turning into a screen (or back)
    /// changes how pictures are encoded, so its encoders start again with keyframes.
    pub(crate) fn set_kind(&mut self, kind: VideoSourceKind) -> Result<(), Vp8Error> {
        let has_content_changed = kind.content() != self.kind.content();
        self.kind = kind;
        for layer in &mut self.layers {
            if has_content_changed {
                layer.encoder = None;
                layer.is_keyframe_due = true;
            } else if let Some(encoder) = &mut layer.encoder {
                encoder.set_bits_per_second(kind.bits_per_second(layer.layer))?;
            }
        }
        Ok(())
    }

    /// The layers someone watches (`VOICE_VIDEO_LAYERS`); the others stop being encoded. An
    /// empty list leaves them as they are, as the web client does: the server only sends one
    /// once it turned the video off.
    pub(crate) fn set_active_layers(&mut self, active: &[SimulcastLayer]) {
        if active.is_empty() {
            return;
        }
        for layer in &mut self.layers {
            let is_active = active.contains(&layer.layer);
            // A resumed layer's watchers wait for a keyframe: send it at once.
            layer.is_keyframe_due |= is_active && !layer.is_active;
            layer.is_active = is_active;
        }
    }

    #[cfg(test)]
    pub(crate) fn active_layers(&self) -> Vec<SimulcastLayer> {
        self.layers
            .iter()
            .filter(|layer| layer.is_active)
            .map(|layer| layer.layer)
            .collect()
    }

    /// `None` asks every layer, as a keyframe request without a RID does.
    pub(crate) fn request_keyframe(&mut self, layer: Option<SimulcastLayer>) {
        for encoder in &mut self.layers {
            if layer.is_none_or(|asked| asked == encoder.layer) {
                encoder.is_keyframe_due = true;
            }
        }
    }

    /// Encodes `picture` (any size: it is fitted to the source's top size) on every active
    /// layer, at `rtp_time` (90 kHz). Layers whose rate control dropped the frame are absent.
    pub(crate) fn encode(
        &mut self,
        picture: I420View<'_>,
        rtp_time: u64,
    ) -> Result<Vec<EncodedLayerFrame>, Vp8Error> {
        let top = fit_within((picture.width(), picture.height()), self.kind.top_size());
        let scaled_top;
        let top_picture = if top == (picture.width(), picture.height()) {
            picture
        } else {
            scaled_top = picture.scaled(top.0, top.1);
            scaled_top.view()
        };
        // Each layer is shrunk from the one above it: half the work of shrinking from the top.
        // Layers above the highest active one are skipped; those between are only shrunk.
        let Some(lowest_active) = self.layers.iter().position(|layer| layer.is_active) else {
            return Ok(Vec::new());
        };
        let kind = self.kind;
        let mut above: Option<I420Picture> = None;
        let mut frames = Vec::with_capacity(LAYERS.len());
        for layer in self.layers.iter_mut().skip(lowest_active).rev() {
            if layer.layer != SimulcastLayer::High {
                let size = layer_size(top, layer.layer);
                let source = above.as_ref().map_or(top_picture, I420Picture::view);
                above = Some(source.scaled(size.0, size.1));
            }
            if layer.is_active {
                let view = above.as_ref().map_or(top_picture, I420Picture::view);
                if let Some(frame) = layer.encode(view, kind, rtp_time)? {
                    frames.push(frame);
                }
            }
        }
        frames.reverse();
        Ok(frames)
    }
}

impl LayerEncoder {
    fn encode(
        &mut self,
        picture: I420View<'_>,
        kind: VideoSourceKind,
        rtp_time: u64,
    ) -> Result<Option<EncodedLayerFrame>, Vp8Error> {
        let settings = Vp8EncoderSettings {
            width: picture.width(),
            height: picture.height(),
            bits_per_second: kind.bits_per_second(self.layer),
            frames_per_second: FRAMES_PER_SECOND,
            content: kind.content(),
        };
        // A new size (another camera, a resized window) needs a new encoder.
        if self
            .encoder
            .as_ref()
            .is_none_or(|encoder| encoder.settings() != settings)
        {
            self.encoder = Some(Vp8Encoder::new(settings)?);
            self.is_keyframe_due = true;
        }
        let Some(encoder) = &mut self.encoder else {
            return Ok(None);
        };
        let frame = encoder.encode(picture, rtp_time, self.is_keyframe_due)?;
        let Some(frame) = frame else {
            return Ok(None);
        };
        if frame.is_keyframe {
            self.is_keyframe_due = false;
        }
        Ok(Some(EncodedLayerFrame {
            layer: self.layer,
            bytes: frame.bytes,
            is_keyframe: frame.is_keyframe,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vp8::Vp8Decoder;

    /// A picture with detail the encoder must spend bits on.
    fn pattern(width: u32, height: u32, shift: u32) -> I420Picture {
        let mut picture = I420Picture::black(width, height);
        let (luma, blue, red) = picture.planes_mut();
        for (index, sample) in luma.iter_mut().enumerate() {
            let index = u32::try_from(index).unwrap();
            let (x, y) = (index % width, index / width);
            *sample = u8::try_from((x + y * 3 + shift * 4) % 220 + 16).unwrap();
        }
        blue.fill(90);
        red.fill(170);
        picture
    }

    /// Width and height from a VP8 keyframe's header (RFC 6386 §9.1).
    fn keyframe_size(frame: &[u8]) -> (u32, u32) {
        assert_eq!(frame[0] & 1, 0, "a keyframe");
        assert_eq!(&frame[3..6], &[0x9d, 0x01, 0x2a], "the start code");
        let width = u32::from(u16::from_le_bytes([frame[6], frame[7]]) & 0x3fff);
        let height = u32::from(u16::from_le_bytes([frame[8], frame[9]]) & 0x3fff);
        (width, height)
    }

    #[test]
    fn a_camera_sends_the_web_s_three_layers_starting_with_keyframes() {
        let mut encoder = SimulcastEncoder::new(VideoSourceKind::Camera);
        let picture = pattern(1_920, 1_080, 0);
        let frames = encoder.encode(picture.view(), 0).unwrap();
        let layers: Vec<_> = frames.iter().map(|frame| frame.layer).collect();
        assert_eq!(layers, LAYERS);
        let sizes: Vec<_> = frames
            .iter()
            .map(|frame| keyframe_size(&frame.bytes))
            .collect();
        assert_eq!(sizes, [(320, 180), (640, 360), (1_280, 720)]);
        assert!(frames.iter().all(|frame| frame.is_keyframe));
    }

    #[test]
    fn a_1440p_screen_keeps_its_size_and_the_server_s_layer_limits() {
        let mut encoder =
            SimulcastEncoder::new(VideoSourceKind::Screen(ScreenShareQuality::Highest));
        let frames = encoder.encode(pattern(2_560, 1_440, 0).view(), 0).unwrap();
        let sizes: Vec<_> = frames
            .iter()
            .map(|frame| keyframe_size(&frame.bytes))
            .collect();
        // At most 640 × 360, 1280 × 720 and 2560 × 1440 (`video_rules.rs`).
        assert_eq!(sizes, [(640, 360), (1_280, 720), (2_560, 1_440)]);
    }

    #[test]
    fn paused_layers_are_not_encoded_and_a_resumed_one_starts_with_a_keyframe() {
        let mut encoder = SimulcastEncoder::new(VideoSourceKind::Camera);
        encoder.encode(pattern(1_280, 720, 0).view(), 0).unwrap();
        encoder.set_active_layers(&[SimulcastLayer::Quarter]);
        let frames = encoder
            .encode(pattern(1_280, 720, 1).view(), 3_000)
            .unwrap();
        assert!(
            frames
                .iter()
                .all(|frame| frame.layer == SimulcastLayer::Quarter && !frame.is_keyframe),
            "{frames:?}"
        );
        encoder.set_active_layers(&[]);
        assert_eq!(
            encoder.active_layers(),
            [SimulcastLayer::Quarter],
            "unchanged"
        );

        encoder.set_active_layers(&[SimulcastLayer::Quarter, SimulcastLayer::High]);
        let frames = encoder
            .encode(pattern(1_280, 720, 2).view(), 6_000)
            .unwrap();
        let high = frames
            .iter()
            .find(|frame| frame.layer == SimulcastLayer::High)
            .unwrap();
        assert!(high.is_keyframe);
        assert!(
            !frames
                .iter()
                .any(|frame| frame.layer == SimulcastLayer::Low)
        );
    }

    #[test]
    fn a_keyframe_request_names_one_layer_or_all() {
        let mut encoder = SimulcastEncoder::new(VideoSourceKind::Camera);
        for index in 0..3 {
            encoder
                .encode(pattern(640, 360, index).view(), u64::from(index) * 3_000)
                .unwrap();
        }
        encoder.request_keyframe(Some(SimulcastLayer::Low));
        let frames = encoder.encode(pattern(640, 360, 3).view(), 9_000).unwrap();
        let keyframes: Vec<_> = frames
            .iter()
            .filter(|frame| frame.is_keyframe)
            .map(|frame| frame.layer)
            .collect();
        assert_eq!(keyframes, [SimulcastLayer::Low]);
        encoder.request_keyframe(None);
        let frames = encoder.encode(pattern(640, 360, 4).view(), 12_000).unwrap();
        assert!(frames.iter().all(|frame| frame.is_keyframe));
    }

    #[test]
    fn a_layer_decodes_back_to_its_picture() {
        let mut encoder = SimulcastEncoder::new(VideoSourceKind::Camera);
        let mut decoder = Vp8Decoder::new(1).unwrap();
        for index in 0..5 {
            let picture = pattern(1_280, 720, index);
            let frames = encoder
                .encode(picture.view(), u64::from(index) * 3_000)
                .unwrap();
            let high = frames
                .iter()
                .find(|frame| frame.layer == SimulcastLayer::High)
                .unwrap();
            let picture_back = decoder.decode(&high.bytes).unwrap().unwrap();
            assert_eq!((picture_back.width(), picture_back.height()), (1_280, 720));
            let error: u64 = picture_back
                .luma()
                .chunks(picture_back.luma_stride())
                .zip(picture.view().luma().chunks(1_280))
                .flat_map(|(decoded_row, original_row)| {
                    decoded_row[..1_280]
                        .iter()
                        .zip(original_row)
                        .map(|(a, b)| u64::from(a.abs_diff(*b)))
                })
                .sum();
            let mean_error = error / (1_280 * 720);
            assert!(mean_error <= 6, "frame {index}: mean error {mean_error}");
        }
    }

    #[test]
    fn a_corrupt_frame_is_a_decoding_error() {
        let mut decoder = Vp8Decoder::new(1).unwrap();
        assert_eq!(decoder.decode(&[0xff; 40]).unwrap_err(), Vp8Error::Codec);
    }

    #[test]
    fn a_quality_change_keeps_the_encoders_and_a_camera_to_screen_change_restarts_them() {
        let mut encoder = SimulcastEncoder::new(VideoSourceKind::Screen(ScreenShareQuality::High));
        encoder.encode(pattern(1_280, 720, 0).view(), 0).unwrap();
        encoder
            .set_kind(VideoSourceKind::Screen(ScreenShareQuality::Highest))
            .unwrap();
        let frames = encoder
            .encode(pattern(1_280, 720, 1).view(), 3_000)
            .unwrap();
        assert!(frames.iter().all(|frame| !frame.is_keyframe));
        encoder.set_kind(VideoSourceKind::Camera).unwrap();
        let frames = encoder
            .encode(pattern(1_280, 720, 2).view(), 6_000)
            .unwrap();
        assert!(frames.iter().all(|frame| frame.is_keyframe));
    }

    /// The cost of a 720p camera picture, from a captured NV12 frame to the three layers'
    /// VP8 frames. A measurement, not a check: run it in release with
    /// `cargo test --release -p mellow-media --lib -- --ignored --nocapture measure` (under
    /// `/usr/bin/time` for the CPU time across libvpx's threads).
    #[test]
    #[ignore = "a measurement for the pull request, not a check"]
    #[allow(clippy::print_stderr)]
    fn measure_the_encoding_of_a_720p_camera() {
        use crate::camera::synthetic_frame;
        use crate::video_frame::{CapturedFrame, CapturedPixelFormat};

        const PICTURES: u32 = 300;
        let captured: Vec<CapturedFrame> = (0..PICTURES)
            .map(|index| {
                let picture = synthetic_frame(1_280, 720, index).to_i420().unwrap();
                CapturedFrame {
                    width: 1_280,
                    height: 720,
                    format: CapturedPixelFormat::Nv12,
                    bytes: picture.view().to_nv12(),
                }
            })
            .collect();
        let milliseconds_per_picture = |work: &mut dyn FnMut(usize)| {
            let started = std::time::Instant::now();
            for index in 0..captured.len() {
                work(index);
            }
            started.elapsed().as_secs_f64() * 1_000.0 / f64::from(PICTURES)
        };
        let conversion = milliseconds_per_picture(&mut |index| {
            std::hint::black_box(captured[index].to_i420());
        });
        let pictures: Vec<I420Picture> = captured
            .iter()
            .map(|frame| frame.to_i420().unwrap())
            .collect();
        let shrinking = milliseconds_per_picture(&mut |index| {
            let low = pictures[index].view().scaled(640, 360);
            std::hint::black_box(low.view().scaled(320, 180));
        });
        let mut bytes_sent = 0_usize;
        let mut encode_with = |active: &[SimulcastLayer]| {
            let mut encoder = SimulcastEncoder::new(VideoSourceKind::Camera);
            encoder.set_active_layers(active);
            milliseconds_per_picture(&mut |index| {
                let rtp_time = u64::try_from(index).unwrap() * 3_000;
                for frame in encoder.encode(pictures[index].view(), rtp_time).unwrap() {
                    bytes_sent += frame.bytes.len();
                }
            })
        };
        let top_layer = encode_with(&[SimulcastLayer::High]);
        let all_layers = encode_with(&LAYERS);
        eprintln!(
            "720p camera, milliseconds of wall time per picture: NV12 to I420 {conversion:.2}, \
             shrinking to 360p and 180p {shrinking:.2}, VP8 top layer {top_layer:.2}, three \
             layers with shrinking {all_layers:.2}; {} kbit/s sent over both runs",
            bytes_sent * 8 * 30 / usize::try_from(PICTURES * 2).unwrap() / 1_000,
        );
    }
}
