//! The smallest encoding that still looks like its source (ADR-0033): candidates in AVIF and WebP,
//! lossy ones at the lowest quality that keeps the SSIMULACRA2 score, found by a bounded binary
//! search.

use std::io::Cursor;
use std::time::Instant;

use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder as _, RgbaImage};
use webp_animation::{EncoderOptions, EncodingConfig, EncodingType, LossyEncodingConfig};

use super::colour::ColourSpace;
use super::{ImageProcessingError, avif, quality};

pub(super) const AVIF_CONTENT_TYPE: &str = "image/avif";
pub(super) const WEBP_CONTENT_TYPE: &str = "image/webp";
/// The qualities searched; below 60 no measured picture kept a score of 85.
const LOWEST_QUALITY: u8 = 60;
const HIGHEST_QUALITY: u8 = 95;
/// Where the search starts: the median quality that kept a score of 85 in the spike.
const AVIF_FIRST_QUALITY: u8 = 88;
const LOSSY_WEBP_FIRST_QUALITY: u8 = 90;
/// Encodes per format and picture: each AVIF of a 15 MP photo takes 20–30 s of one core.
const MAXIMUM_ENCODES_PER_FORMAT: usize = 4;
/// AVIF at its highest quality: stored when nothing else passes and lossless WebP cannot carry
/// the picture's colours.
const LAST_RESORT_AVIF_QUALITY: u8 = 100;
/// libwebp's effort for lossless animations: 0 (fast) to 100 (smallest); the pixels are the same
/// whatever the effort. Measured in release on a 480 × 480, 60-frame GIF: 2.4 s at 60, 2.2 s at
/// 25, 1.5 s at 10, decoding included. libwebp's method stays its default, 4: `webp-animation`
/// 0.10 never passes `EncodingConfig::method` on.
const ANIMATION_LOSSLESS_EFFORT: f32 = 25.0;
/// A still is one frame, so a high effort stays cheap: a 1 920 × 1 080 chat screenshot takes
/// 107 KB at 25 and about 23 KB at 90.
const STILL_LOSSLESS_EFFORT: f32 = 90.0;
/// Frames of an animation scored, evenly spaced: the metric is the slow part.
const SCORED_ANIMATION_FRAMES: usize = 12;

/// A picture as stored, with its served type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Encoded {
    pub(super) bytes: Vec<u8>,
    pub(super) content_type: &'static str,
}

/// Frames of the same size, each with its duration in milliseconds.
pub(super) type Frames = [(RgbaImage, u32)];

fn smaller(best: Option<Encoded>, candidate: Option<Encoded>) -> Option<Encoded> {
    match (best, candidate) {
        (Some(best), Some(candidate)) if candidate.bytes.len() < best.bytes.len() => {
            Some(candidate)
        }
        (Some(best), _) => Some(best),
        (None, candidate) => candidate,
    }
}

/// The smallest passing result of `encode_if_passing` over qualities in
/// `LOWEST_QUALITY..=HIGHEST_QUALITY`: a binary search from `first`, at most
/// `MAXIMUM_ENCODES_PER_FORMAT` encodes, none started past `deadline`. The best result so far is
/// kept, so stopping early still answers a valid one.
fn search_quality(
    first: u8,
    deadline: Instant,
    mut encode_if_passing: impl FnMut(u8) -> Result<Option<Vec<u8>>, ImageProcessingError>,
) -> Result<Option<Vec<u8>>, ImageProcessingError> {
    let (mut lowest, mut highest) = (LOWEST_QUALITY, HIGHEST_QUALITY);
    let mut quality = first;
    let mut best: Option<Vec<u8>> = None;
    for _ in 0..MAXIMUM_ENCODES_PER_FORMAT {
        if Instant::now() >= deadline {
            break;
        }
        match encode_if_passing(quality)? {
            Some(bytes) => {
                if best.as_ref().is_none_or(|best| bytes.len() < best.len()) {
                    best = Some(bytes);
                }
                highest = quality.saturating_sub(1);
            }
            None => lowest = quality.saturating_add(1),
        }
        if lowest > highest {
            break;
        }
        quality = lowest + (highest - lowest) / 2;
    }
    Ok(best)
}

/// The smallest passing AVIF: `search_quality` from `AVIF_FIRST_QUALITY`, bounded by the size of
/// the lossless candidate when there is one. Bytes shrink as quality falls, so the lowest quality
/// is probed first: when even it is no smaller than lossless, no quality can win and nothing is
/// scored; when it passes, nothing smaller exists. A candidate at least as large as the bound is
/// not scored (scoring is the slow part) and sends the search lower. Every bytes answered passed.
fn smallest_passing_avif<Decoded>(
    lossless_size: Option<usize>,
    deadline: Instant,
    mut encode: impl FnMut(u8) -> Result<(Vec<u8>, Decoded), ImageProcessingError>,
    mut passes: impl FnMut(&Decoded) -> bool,
) -> Result<Option<Vec<u8>>, ImageProcessingError> {
    let is_smaller = |bytes: &[u8]| lossless_size.is_none_or(|bound| bytes.len() < bound);
    if lossless_size.is_some() && Instant::now() < deadline {
        let (bytes, decoded) = encode(LOWEST_QUALITY)?;
        if !is_smaller(&bytes) {
            return Ok(None);
        }
        if passes(&decoded) {
            return Ok(Some(bytes));
        }
    }
    let best = search_quality(AVIF_FIRST_QUALITY, deadline, |quality| {
        let (bytes, decoded) = encode(quality)?;
        if !is_smaller(&bytes) {
            // Answered as passing only to search lower: filtered out below.
            return Ok(Some(bytes));
        }
        Ok(passes(&decoded).then_some(bytes))
    })?;
    Ok(best.filter(|bytes| is_smaller(bytes)))
}

fn webp_options(encoding_type: EncodingType, quality: f32) -> EncoderOptions {
    EncoderOptions {
        encoding_config: Some(EncodingConfig {
            encoding_type,
            quality,
            ..EncodingConfig::default()
        }),
        ..EncoderOptions::default()
    }
}

/// Frames borrowed, so a still is encoded without copying its pixels.
fn encode_webp<'a>(
    frames: impl IntoIterator<Item = (&'a RgbaImage, u32)>,
    options: EncoderOptions,
) -> Result<Vec<u8>, ImageProcessingError> {
    let mut frames = frames.into_iter().peekable();
    let Some((first, _)) = frames.peek() else {
        return Err(ImageProcessingError::Encoding);
    };
    let mut encoder = webp_animation::Encoder::new_with_options(first.dimensions(), options)
        .map_err(|_| ImageProcessingError::Encoding)?;
    let mut timestamp: i32 = 0;
    for (frame, delay) in frames {
        encoder
            .add_frame(frame.as_raw(), timestamp)
            .map_err(|_| ImageProcessingError::Encoding)?;
        timestamp = timestamp.saturating_add(i32::try_from(delay.max(1)).unwrap_or(i32::MAX));
    }
    let bytes = encoder
        .finalize(timestamp)
        .map_err(|_| ImageProcessingError::Encoding)?;
    Ok(bytes.to_vec())
}

/// One still or animated lossless WebP: always passes, its pixels being the source's.
pub(super) fn lossless_webp(frames: &Frames) -> Result<Vec<u8>, ImageProcessingError> {
    encode_webp(
        frames.iter().map(|(frame, delay)| (frame, *delay)),
        webp_options(EncodingType::Lossless, ANIMATION_LOSSLESS_EFFORT),
    )
}

fn lossless_still_webp(image: &RgbaImage) -> Result<Encoded, ImageProcessingError> {
    Ok(Encoded {
        bytes: encode_webp(
            [(image, 0)],
            webp_options(EncodingType::Lossless, STILL_LOSSLESS_EFFORT),
        )?,
        content_type: WEBP_CONTENT_TYPE,
    })
}

fn lossy_webp(frames: &Frames, quality: u8) -> Result<Vec<u8>, ImageProcessingError> {
    let config = LossyEncodingConfig {
        // Sharper colour edges for the 4:2:0 colour every lossy WebP has; alpha stays lossless.
        use_sharp_yuv: true,
        ..LossyEncodingConfig::default()
    };
    encode_webp(
        frames.iter().map(|(frame, delay)| (frame, *delay)),
        webp_options(EncodingType::Lossy(config), f32::from(quality)),
    )
}

/// The smallest still that passes: AVIF searched, and lossless WebP when `is_lossless_tried` and
/// the colours have an sRGB form (WebP is written without a profile). `None` when none passes.
pub(super) fn smallest_still(
    source: &RgbaImage,
    colour: &ColourSpace,
    is_lossless_tried: bool,
    deadline: Instant,
) -> Result<Option<Encoded>, ImageProcessingError> {
    // Cheap and always valid: first, so a deadline still leaves it.
    let lossless = if is_lossless_tried {
        colour
            .srgb_form(source)
            .map(|pixels| lossless_still_webp(&pixels))
            .transpose()?
    } else {
        None
    };
    let stored = colour
        .stored(source)
        .ok_or(ImageProcessingError::Encoding)?;
    let primaries = colour.primaries();
    let avif = smallest_passing_avif(
        lossless.as_ref().map(|lossless| lossless.bytes.len()),
        deadline,
        |quality| {
            let encoded = avif::encode(&stored, quality, primaries)?;
            Ok((encoded.bytes, encoded.decoded))
        },
        |decoded| quality::passes(&[(&stored, decoded)], primaries),
    )?
    .map(|bytes| Encoded {
        bytes,
        content_type: AVIF_CONTENT_TYPE,
    });
    Ok(smaller(lossless, avif))
}

/// What a still that must be stored is when nothing passed: lossless when its colours allow,
/// else AVIF at its highest quality.
pub(super) fn last_resort_still(
    source: &RgbaImage,
    colour: &ColourSpace,
) -> Result<Encoded, ImageProcessingError> {
    if let Some(pixels) = colour.srgb_form(source) {
        return lossless_still_webp(&pixels);
    }
    let stored = colour
        .stored(source)
        .ok_or(ImageProcessingError::Encoding)?;
    Ok(Encoded {
        bytes: avif::encode(&stored, LAST_RESORT_AVIF_QUALITY, colour.primaries())?.bytes,
        content_type: AVIF_CONTENT_TYPE,
    })
}

/// The frames of an animated WebP with the time each starts at, in milliseconds.
fn decoded_animation(bytes: &[u8]) -> Option<Vec<(u64, RgbaImage)>> {
    let decoder = WebPDecoder::new(Cursor::new(bytes)).ok()?;
    let mut start = 0u64;
    decoder
        .into_frames()
        .map(|frame| {
            let frame = frame.ok()?;
            let (numerator, denominator) = frame.delay().numer_denom_ms();
            let frame_start = start;
            start += u64::from(numerator.checked_div(denominator).unwrap_or(0));
            Some((frame_start, frame.into_buffer()))
        })
        .collect()
}

/// Whether a lossy animation passes on up to `SCORED_ANIMATION_FRAMES` frames, each compared with
/// the frame shown at the same time: the encoder may merge frames that look alike.
fn animation_passes(frames: &Frames, encoded: &[u8]) -> bool {
    let Some(decoded) = decoded_animation(encoded) else {
        return false;
    };
    let step = frames.len().div_ceil(SCORED_ANIMATION_FRAMES).max(1);
    let mut start = 0u64;
    let mut starts = Vec::with_capacity(frames.len());
    for (_, delay) in frames {
        starts.push(start);
        start += u64::from((*delay).max(1));
    }
    let pairs: Option<Vec<(&RgbaImage, &RgbaImage)>> = frames
        .iter()
        .zip(starts)
        .step_by(step)
        .map(|((source, _), frame_start)| {
            let shown = decoded
                .iter()
                .rev()
                .find(|(decoded_start, _)| *decoded_start <= frame_start)?;
            Some((source, &shown.1))
        })
        .collect();
    pairs.is_some_and(|pairs| quality::passes(&pairs, avif::Primaries::Srgb))
}

/// The smaller of a lossless and a passing lossy animated WebP; pure Rust has no animated AVIF
/// encoder.
pub(super) fn smallest_animation(
    frames: &Frames,
    deadline: Instant,
) -> Result<Encoded, ImageProcessingError> {
    let lossless = Encoded {
        bytes: lossless_webp(frames)?,
        content_type: WEBP_CONTENT_TYPE,
    };
    let lossy = search_quality(LOSSY_WEBP_FIRST_QUALITY, deadline, |quality| {
        let bytes = lossy_webp(frames, quality)?;
        Ok(animation_passes(frames, &bytes).then_some(bytes))
    })?
    .map(|bytes| Encoded {
        bytes,
        content_type: WEBP_CONTENT_TYPE,
    });
    Ok(smaller(Some(lossless.clone()), lossy).unwrap_or(lossless))
}

#[cfg(test)]
mod tests {
    use image::Rgba;

    use super::*;

    #[test]
    fn the_search_finds_the_lowest_passing_quality_within_four_encodes() {
        let far = Instant::now() + std::time::Duration::from_secs(3_600);
        let mut tried = Vec::new();
        // Passes from 80 up; a lower quality gives fewer bytes.
        let found = search_quality(AVIF_FIRST_QUALITY, far, |quality| {
            tried.push(quality);
            Ok((quality >= 80).then(|| vec![0; usize::from(quality)]))
        })
        .unwrap();
        assert_eq!(tried, [88, 73, 80, 76]);
        assert_eq!(
            found.map(|bytes| bytes.len()),
            Some(80),
            "the best passing so far"
        );

        let mut tried = Vec::new();
        let none = search_quality(AVIF_FIRST_QUALITY, far, |quality| {
            tried.push(quality);
            Ok(None)
        })
        .unwrap();
        assert_eq!((none, tried), (None, vec![88, 92, 94, 95]));

        let past = Instant::now();
        let nothing_started = search_quality(AVIF_FIRST_QUALITY, past, |_| {
            panic!("no encode starts past the deadline")
        })
        .unwrap();
        assert_eq!(nothing_started, None);
    }

    /// Encodes and scorings of `smallest_passing_avif` when an encode at quality `q` takes `q`
    /// bytes and passes from 80 up.
    fn bounded_search(lossless_size: Option<usize>) -> (Option<usize>, Vec<u8>, Vec<u8>) {
        let far = Instant::now() + std::time::Duration::from_secs(3_600);
        let (mut encoded, mut scored) = (Vec::new(), Vec::new());
        let found = smallest_passing_avif(
            lossless_size,
            far,
            |quality| {
                encoded.push(quality);
                Ok((vec![0; usize::from(quality)], quality))
            },
            |quality| {
                scored.push(*quality);
                *quality >= 80
            },
        )
        .unwrap();
        (found.map(|bytes| bytes.len()), encoded, scored)
    }

    #[test]
    fn the_avif_search_stops_as_soon_as_lossless_cannot_be_beaten() {
        assert_eq!(
            bounded_search(None),
            (Some(80), vec![88, 73, 80, 76], vec![88, 73, 80, 76]),
            "no lossless: the plain search"
        );
        assert_eq!(
            bounded_search(Some(60)),
            (None, vec![60], vec![]),
            "the lowest quality is already as large as lossless: one encode, nothing scored"
        );
        assert_eq!(
            bounded_search(Some(85)),
            (Some(80), vec![60, 88, 73, 80, 76], vec![60, 73, 80, 76]),
            "88 is larger than lossless: encoded, never scored"
        );
        assert_eq!(
            bounded_search(Some(80)),
            (None, vec![60, 88, 73, 80, 76], vec![60, 73, 76]),
            "what passes is never smaller than lossless"
        );
    }

    #[test]
    fn a_lossless_still_is_encoded_at_a_high_effort() {
        // A UI-like picture: a sidebar and a chat sheet with lines of text, words drawn by a
        // seeded hash from a vocabulary of 16, each a run of glyphs of 8 × 12 from 26.
        let hash = |value: u32| value.wrapping_mul(2_654_435_761).rotate_left(13) ^ value;
        let screenshot = RgbaImage::from_fn(960, 600, |x, y| {
            let panel = if x < 200 {
                [0x2B, 0x22, 0x2D]
            } else {
                [0x22, 0x1B, 0x23]
            };
            let (line, row) = (y / 20, y % 20);
            // Words of 6 glyphs and a space, so 56 px each.
            let (word_slot, within_word) = ((x % 200) / 56, (x % 200) % 56);
            let word = hash(line * 31 + x / 200 * 7 + word_slot) % 16;
            let (letter, column) = (within_word / 8, within_word % 8);
            let glyph = hash(word * 7 + letter) % 26;
            let is_text = (4..16).contains(&row) && letter < 6 && x % 200 < 168;
            // Anti-aliased: a glyph's pixel is the panel blended with the text in 5 steps.
            let coverage = if is_text {
                hash(glyph * 96 + (row - 4) * 8 + column) % 5
            } else {
                0
            };
            let blend = |from: u8, to: u8| {
                let (from, to) = (u32::from(from), u32::from(to));
                u8::try_from((from * (4 - coverage) + to * coverage) / 4).unwrap()
            };
            let text = [0xE8, 0xDF, 0xE6];
            Rgba([
                blend(panel[0], text[0]),
                blend(panel[1], text[1]),
                blend(panel[2], text[2]),
                255,
            ])
        });
        let at_animation_effort = encode_webp(
            [(&screenshot, 0)],
            webp_options(EncodingType::Lossless, ANIMATION_LOSSLESS_EFFORT),
        )
        .unwrap();
        let still = lossless_still_webp(&screenshot).unwrap();
        // 46 118 B at 25 and 9 662 B at 90 when written; the chat screenshot of the corpus
        // went from 107 738 B to 22 768 B.
        assert!(
            at_animation_effort.len() >= 3 * still.bytes.len(),
            "{} B at 25, {} B at 90",
            at_animation_effort.len(),
            still.bytes.len()
        );
    }
}
