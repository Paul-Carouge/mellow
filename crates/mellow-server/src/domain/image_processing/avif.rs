//! Still AVIF pictures through `rav1e` (pure Rust), set up as `ravif` 0.13 sets it at speed 8,
//! which the spike measured (ADR-0033). `ravif` itself is not used: it neither signals Display P3
//! nor returns the encoder's reconstruction, which is exactly what a decoder shows and what the
//! quality check scores, so no AV1 decoder is needed.

use avif_serialize::constants::{
    ColorPrimaries as ContainerPrimaries, MatrixCoefficients as ContainerMatrix,
    TransferCharacteristics as ContainerTransfer,
};
use image::{Rgba, RgbaImage};
use rav1e::prelude::*;

use super::ImageProcessingError;

/// rav1e's speed preset: speed 6 saved only 1–3 % of bytes for twice the time, 10 lost 10–60 %.
const SPEED_PRESET: u8 = 8;
/// Ten bits even for 8-bit sources: smaller files at the same quality, as `ravif` defaults to.
const BIT_DEPTH: usize = 10;
const MAXIMUM_TEN_BIT_VALUE: f32 = 1_023.0;
const MAXIMUM_EIGHT_BIT_VALUE: f32 = 255.0;
/// Zero chroma in 10-bit full range: `round(1023 / 2)`.
const CHROMA_OFFSET: f32 = 512.0;
/// BT.601 luma weights of red and blue, the matrix `ravif` uses and the container announces.
const RED_WEIGHT: f32 = 0.299;
const BLUE_WEIGHT: f32 = 0.114;
const GREEN_WEIGHT: f32 = 1.0 - RED_WEIGHT - BLUE_WEIGHT;

/// The colour primaries the pixels are in; the transfer is sRGB's in both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Primaries {
    Srgb,
    /// Display P3 (SMPTE EG 432-1), as iPhone photos are.
    DisplayP3,
}

#[derive(Debug)]
pub(super) struct EncodedAvif {
    pub(super) bytes: Vec<u8>,
    /// The picture as any decoder shows it, alpha included.
    pub(super) decoded: RgbaImage,
}

/// `ravif`'s mapping from a 1–100 quality to rav1e's 0–255 quantizer.
fn quantizer_of_quality(quality: u8) -> usize {
    let fraction = f32::from(quality.clamp(1, 100)) / 100.0;
    let quantizer = if fraction >= 0.82 {
        (1.0 - fraction) * 2.6
    } else if fraction > 0.25 {
        fraction.mul_add(-0.5, 1.0 - 0.125)
    } else {
        1.0 - fraction
    };
    // Within 0..=255 by construction: `fraction` is in 0.01..=1.
    to_unsigned((quantizer * 255.0).round())
}

/// A non-negative float already rounded and bounded by the caller.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "callers clamp to the target range first"
)]
fn to_unsigned(value: f32) -> usize {
    value.max(0.0) as usize
}

/// `ravif` 0.13's speed tweaks at speed 8, whose comparisons are kept as written there.
fn speed_settings(quantizer: usize) -> SpeedSettings {
    let is_quantizer_below_that_of_quality_55 = quantizer < quantizer_of_quality(55);
    let is_quantizer_above_that_of_quality_80 = quantizer > quantizer_of_quality(80);
    let mut settings = SpeedSettings::from_preset(SPEED_PRESET);
    settings.multiref = false;
    settings.rdo_lookahead_frames = 1;
    settings.scene_detection_mode = SceneDetectionSpeed::None;
    settings.motion.include_near_mvs = false;
    settings.motion.use_satd_subpel = false;
    settings.fast_deblock = !is_quantizer_above_that_of_quality_80;
    settings.transform.reduced_tx_set = false;
    settings.transform.tx_domain_rate = false;
    settings.transform.rdo_tx_decision = false;
    settings.transform.enable_inter_tx_split = false;
    settings.partition.encode_bottomup = false;
    settings.partition.partition_range =
        PartitionRange::new(BlockSize::BLOCK_8X8, BlockSize::BLOCK_16X16);
    settings.cdef = is_quantizer_below_that_of_quality_55;
    settings.lrf = is_quantizer_below_that_of_quality_55;
    settings.sgr_complexity = SGRComplexityLevel::Reduced;
    settings.prediction.prediction_modes = PredictionModesSetting::Simple;
    settings.prediction.fine_directional_intra = false;
    settings
}

fn encoder_config(
    (width, height): (usize, usize),
    quantizer: usize,
    chroma_sampling: ChromaSampling,
    color_description: Option<ColorDescription>,
) -> Config {
    let minimum_tile_side = if quantizer > quantizer_of_quality(80) {
        256
    } else {
        128
    };
    Config::new()
        .with_encoder_config(EncoderConfig {
            width,
            height,
            time_base: Rational::new(1, 1),
            sample_aspect_ratio: Rational::new(1, 1),
            bit_depth: BIT_DEPTH,
            chroma_sampling,
            chroma_sample_position: ChromaSamplePosition::Unknown,
            pixel_range: PixelRange::Full,
            color_description,
            mastering_display: None,
            content_light: None,
            enable_timing_info: false,
            still_picture: true,
            error_resilient: false,
            switch_frame_interval: 0,
            min_key_frame_interval: 0,
            max_key_frame_interval: 0,
            reservoir_frame_delay: None,
            low_latency: false,
            quantizer,
            min_quantizer: u8::try_from(quantizer).unwrap_or(u8::MAX),
            bitrate: 0,
            tune: Tune::Psychovisual,
            tile_cols: 0,
            tile_rows: 0,
            // One thread per job: one tile, or rav1e's own choice for tiny pictures.
            tiles: 1.min(width * height / (minimum_tile_side * minimum_tile_side)),
            film_grain_params: None,
            level_idx: None,
            speed_settings: speed_settings(quantizer),
        })
        .with_threads(1)
}

/// One AV1 key frame: its bitstream and the frame a decoder rebuilds from it.
fn encode_frame(
    config: &Config,
    fill: impl FnOnce(&mut Frame<u16>),
) -> Result<(Vec<u8>, std::sync::Arc<Frame<u16>>), ImageProcessingError> {
    let mut context: Context<u16> = config
        .new_context()
        .map_err(|_| ImageProcessingError::Encoding)?;
    let mut frame = context.new_frame();
    fill(&mut frame);
    context
        .send_frame(frame)
        .map_err(|_| ImageProcessingError::Encoding)?;
    context.flush();
    let mut bitstream = Vec::new();
    let mut reconstruction = None;
    loop {
        match context.receive_packet() {
            Ok(mut packet) if packet.frame_type == FrameType::KEY => {
                bitstream.append(&mut packet.data);
                reconstruction = packet.rec.take().or(reconstruction);
            }
            Ok(_) | Err(EncoderStatus::Encoded) => {}
            Err(EncoderStatus::LimitReached) => break,
            Err(_) => return Err(ImageProcessingError::Encoding),
        }
    }
    let reconstruction = reconstruction.ok_or(ImageProcessingError::Encoding)?;
    Ok((bitstream, reconstruction))
}

fn ten_bit(value: u8) -> u16 {
    (u16::from(value) << 2) | (u16::from(value) >> 6)
}

fn eight_bit(value: f32) -> u8 {
    let scaled = (value * MAXIMUM_EIGHT_BIT_VALUE / MAXIMUM_TEN_BIT_VALUE).round();
    u8::try_from(to_unsigned(scaled.clamp(0.0, MAXIMUM_EIGHT_BIT_VALUE))).unwrap_or(u8::MAX)
}

/// Full-range 10-bit BT.601 YCbCr, as `ravif` computes it.
fn luma_and_chroma(pixel: Rgba<u8>) -> [u16; 3] {
    let scale = MAXIMUM_TEN_BIT_VALUE / MAXIMUM_EIGHT_BIT_VALUE;
    let [red, green, blue] = [pixel[0], pixel[1], pixel[2]].map(f32::from);
    let luma = (scale * BLUE_WEIGHT).mul_add(
        blue,
        (scale * RED_WEIGHT).mul_add(red, scale * GREEN_WEIGHT * green),
    );
    let blue_difference = blue
        .mul_add(scale, -luma)
        .mul_add(0.5 / (1.0 - BLUE_WEIGHT), CHROMA_OFFSET);
    let red_difference = red
        .mul_add(scale, -luma)
        .mul_add(0.5 / (1.0 - RED_WEIGHT), CHROMA_OFFSET);
    [luma, blue_difference, red_difference].map(|value| {
        u16::try_from(to_unsigned(value.round().clamp(0.0, MAXIMUM_TEN_BIT_VALUE)))
            .unwrap_or(u16::MAX)
    })
}

/// The inverse of `luma_and_chroma`, to 8-bit RGB as browsers show it.
fn red_green_blue(luma: u16, blue_difference: u16, red_difference: u16) -> [u8; 3] {
    let luma = f32::from(luma);
    let blue =
        (f32::from(blue_difference) - CHROMA_OFFSET).mul_add(2.0 * (1.0 - BLUE_WEIGHT), luma);
    let red = (f32::from(red_difference) - CHROMA_OFFSET).mul_add(2.0 * (1.0 - RED_WEIGHT), luma);
    let green = (luma - RED_WEIGHT * red - BLUE_WEIGHT * blue) / GREEN_WEIGHT;
    [eight_bit(red), eight_bit(green), eight_bit(blue)]
}

/// Visible rows of a plane, each cut to the picture's width.
fn visible_rows(plane: &Plane<u16>, (width, height): (usize, usize)) -> Vec<&[u16]> {
    plane
        .data_origin()
        .chunks(plane.cfg.stride)
        .take(height)
        .map(|row| &row[..width])
        .collect()
}

/// A still AVIF at `quality` (1–100), with its alpha when any pixel is not opaque.
pub(super) fn encode(
    image: &RgbaImage,
    quality: u8,
    primaries: Primaries,
) -> Result<EncodedAvif, ImageProcessingError> {
    let (width, height) = image.dimensions();
    let size = (
        usize::try_from(width).map_err(|_| ImageProcessingError::TooLarge)?,
        usize::try_from(height).map_err(|_| ImageProcessingError::TooLarge)?,
    );
    let quantizer = quantizer_of_quality(quality);
    let color_description = ColorDescription {
        color_primaries: match primaries {
            Primaries::Srgb => ColorPrimaries::BT709,
            Primaries::DisplayP3 => ColorPrimaries::SMPTE432,
        },
        transfer_characteristics: TransferCharacteristics::SRGB,
        matrix_coefficients: MatrixCoefficients::BT601,
    };
    let fill_rows = |plane: &mut Plane<u16>, value: &dyn Fn(Rgba<u8>) -> u16| {
        let stride = plane.cfg.stride;
        for (row, pixels) in plane.data_origin_mut().chunks_mut(stride).zip(image.rows()) {
            for (sample, pixel) in row.iter_mut().zip(pixels) {
                *sample = value(*pixel);
            }
        }
    };
    let (color, color_frame) = encode_frame(
        &encoder_config(
            size,
            quantizer,
            ChromaSampling::Cs444,
            Some(color_description),
        ),
        |frame| {
            for (index, plane) in frame.planes.iter_mut().enumerate() {
                fill_rows(plane, &|pixel| luma_and_chroma(pixel)[index]);
            }
        },
    )?;
    let has_alpha = image.pixels().any(|pixel| pixel[3] != u8::MAX);
    let alpha = has_alpha
        .then(|| {
            encode_frame(
                &encoder_config(size, quantizer, ChromaSampling::Cs400, None),
                |frame| fill_rows(&mut frame.planes[0], &|pixel| ten_bit(pixel[3])),
            )
        })
        .transpose()?;

    let [luma, blue_difference, red_difference] =
        [0, 1, 2].map(|index| visible_rows(&color_frame.planes[index], size));
    let alpha_rows = alpha
        .as_ref()
        .map(|(_, frame)| visible_rows(&frame.planes[0], size));
    let decoded = RgbaImage::from_fn(width, height, |x, y| {
        let (column, row) = (x as usize, y as usize);
        let [red, green, blue] = red_green_blue(
            luma[row][column],
            blue_difference[row][column],
            red_difference[row][column],
        );
        let opacity = alpha_rows
            .as_ref()
            .map_or(u8::MAX, |rows| eight_bit(f32::from(rows[row][column])));
        Rgba([red, green, blue, opacity])
    });

    let mut container = avif_serialize::Aviffy::new();
    container
        .set_color_primaries(match primaries {
            Primaries::Srgb => ContainerPrimaries::Bt709,
            Primaries::DisplayP3 => ContainerPrimaries::DisplayP3,
        })
        .set_transfer_characteristics(ContainerTransfer::Srgb)
        .set_matrix_coefficients(ContainerMatrix::Bt601)
        .set_full_color_range(true);
    let bytes = container.to_vec(
        &color,
        alpha.as_ref().map(|(bitstream, _)| bitstream.as_slice()),
        width,
        height,
        u8::try_from(BIT_DEPTH).unwrap_or(u8::MAX),
    );
    Ok(EncodedAvif { bytes, decoded })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_conversion_round_trips_within_one_step() {
        for value in (0..=255u8).step_by(5) {
            for pixel in [
                Rgba([value, 0, 0, 255]),
                Rgba([0, value, 0, 255]),
                Rgba([0, 0, value, 255]),
                Rgba([value, value / 2, 255 - value, 255]),
            ] {
                let [luma, blue, red] = luma_and_chroma(pixel);
                let back = red_green_blue(luma, blue, red);
                for (channel, original) in back.iter().zip(&pixel.0[..3]) {
                    assert!(channel.abs_diff(*original) <= 1, "{pixel:?} -> {back:?}");
                }
            }
        }
    }

    #[test]
    fn the_reconstruction_is_the_picture_at_high_quality_and_the_container_says_p3() {
        let image = RgbaImage::from_fn(48, 32, |x, y| {
            Rgba([
                u8::try_from(x * 5).unwrap(),
                u8::try_from(y * 7).unwrap(),
                90,
                if x < 8 { 0 } else { 255 },
            ])
        });
        let encoded = encode(&image, 100, Primaries::DisplayP3).unwrap();
        assert_eq!(&encoded.bytes[4..12], b"ftypavif");
        // nclx colour box: primaries 12 (Display P3), transfer 13 (sRGB), matrix 6, full range.
        let nclx = [b'n', b'c', b'l', b'x', 0, 12, 0, 13, 0, 6, 0x80];
        assert!(
            encoded
                .bytes
                .windows(nclx.len())
                .any(|window| window == nclx)
        );
        let largest_difference = image
            .pixels()
            .zip(encoded.decoded.pixels())
            .flat_map(|(source, decoded)| {
                source
                    .0
                    .iter()
                    .zip(decoded.0)
                    .map(|(a, b)| a.abs_diff(b))
                    .collect::<Vec<_>>()
            })
            .filter(|difference| *difference > 0)
            .max()
            .unwrap_or(0);
        assert!(largest_difference <= 4, "{largest_difference}");
        assert_eq!(encoded.decoded.get_pixel(0, 0)[3], 0, "transparency kept");
    }
}
