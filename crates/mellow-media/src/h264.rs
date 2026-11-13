//! Received H.264 (ADR-0052): Constrained Baseline in Annex B form, as `str0m` hands frames over
//! and as native senders' hardware encoders write them. Decoded by the system: `VideoToolbox` on
//! macOS, FFmpeg's software decoder on Linux (the system FFmpeg the Linux desktop already links,
//! ADR-0051's amendment), Media Foundation's decoder on Windows. Where none opens, the offer
//! leaves H.264 out, so the server never sends H.264 pictures and the interface says the
//! picture is not supported.

use mellow_protocol::VideoCodec;

#[cfg(target_os = "linux")]
pub(crate) use crate::h264_ffmpeg::H264Decoder;
#[cfg(windows)]
pub(crate) use crate::h264_media_foundation::H264Decoder;
#[cfg(target_os = "macos")]
pub(crate) use crate::h264_videotoolbox::H264Decoder;

/// Whether this system decodes received H.264, so its video receive lines offer it. The system
/// FFmpeg may lack the decoder (built without it, or a distribution's patent-free build), and
/// Windows N editions lack Media Foundation, so there a decoder is opened once to find out.
/// `VideoToolbox` always has one.
pub(crate) fn decodes_h264() -> bool {
    #[cfg(any(target_os = "linux", windows))]
    {
        static DECODES_H264: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *DECODES_H264.get_or_init(|| match H264Decoder::new() {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(%error, "H.264 is not offered: the system cannot decode it");
                false
            }
        })
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        cfg!(target_os = "macos")
    }
}

/// The largest picture a sender's highest layer may have (ADR-0037's 1440p `h` layer): the
/// server refuses larger ones, and the decoders refuse them again before allocating.
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub(crate) const MAXIMUM_DECODED_WIDTH: u32 = 2_560;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub(crate) const MAXIMUM_DECODED_HEIGHT: u32 = 1_440;

/// NAL unit types (ITU-T H.264 table 7-1).
const IDR_SLICE: u8 = 5;
#[cfg(target_os = "macos")]
pub(crate) const SEQUENCE_PARAMETER_SET: u8 = 7;
#[cfg(target_os = "macos")]
pub(crate) const PICTURE_PARAMETER_SET: u8 = 8;

/// Whether this app can show a picture sent in `codec`, as a voice state names it (absent: VP8).
#[must_use]
pub fn decodes_video_codec(codec: Option<VideoCodec>) -> bool {
    match codec {
        None | Some(VideoCodec::Vp8) => true,
        Some(VideoCodec::H264) => decodes_h264(),
        Some(VideoCodec::Unknown) => false,
    }
}

/// An H.264 decoder failed: no decoder on this system, a picture too large, or a corrupt frame.
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[derive(Debug, thiserror::Error)]
#[error("H.264 decoding failed: {0}")]
pub(crate) struct H264Error(pub(crate) String);

/// The type of a NAL unit, from its header byte.
pub(crate) fn nal_unit_type(nal_unit: &[u8]) -> Option<u8> {
    nal_unit.first().map(|header| header & 0x1f)
}

/// The NAL units of an Annex B stream, without their start codes (3 or 4 bytes) or the zero
/// bytes that may trail them.
pub(crate) fn nal_units(stream: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut rest = stream;
    std::iter::from_fn(move || {
        let start = find_start_code(rest)? + 3;
        let after = &rest[start..];
        let end = find_start_code(after).unwrap_or(after.len());
        rest = &after[end..];
        // Zero bytes before the next start code belong to it (a four-byte start code or
        // `trailing_zero_8bits`), never to the unit: a NAL unit never ends with a zero byte.
        let unit = &after[..end];
        let length = unit
            .iter()
            .rposition(|byte| *byte != 0)
            .map_or(0, |last| last + 1);
        Some(&unit[..length])
    })
    .filter(|unit| !unit.is_empty())
}

/// Where the next `00 00 01` starts.
fn find_start_code(bytes: &[u8]) -> Option<usize> {
    bytes.windows(3).position(|window| window == [0, 0, 1])
}

/// Whether `frame` holds an IDR slice a decoder can start from.
pub(crate) fn is_h264_keyframe(frame: &[u8]) -> bool {
    nal_units(frame).any(|nal_unit| nal_unit_type(nal_unit) == Some(IDR_SLICE))
}

/// Whether a decoded picture fits the largest layer.
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub(crate) fn is_within_maximum_size(width: u32, height: u32) -> bool {
    width > 0 && height > 0 && width <= MAXIMUM_DECODED_WIDTH && height <= MAXIMUM_DECODED_HEIGHT
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Four frames of a 320 × 180 Constrained Baseline stream (x264, one access unit delimiter
    /// before each, SPS and PPS before the IDR).
    pub(crate) const FIXTURE: &[u8] =
        include_bytes!("../tests/fixtures/constrained-baseline-320x180.h264");
    const ACCESS_UNIT_DELIMITER: u8 = 9;

    /// The fixture's access units, each with its NAL units behind 4-byte start codes, as `str0m`
    /// hands them over.
    pub(crate) fn fixture_frames() -> Vec<Vec<u8>> {
        let mut frames: Vec<Vec<u8>> = Vec::new();
        for nal_unit in nal_units(FIXTURE) {
            if nal_unit_type(nal_unit) == Some(ACCESS_UNIT_DELIMITER) {
                frames.push(Vec::new());
                continue;
            }
            if let Some(frame) = frames.last_mut() {
                frame.extend_from_slice(&[0, 0, 0, 1]);
                frame.extend_from_slice(nal_unit);
            }
        }
        frames
    }

    #[test]
    fn nal_units_are_split_on_three_and_four_byte_start_codes() {
        let stream = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 0, 1, 0x65, 4,
        ];
        let units: Vec<&[u8]> = nal_units(&stream).collect();
        assert_eq!(units, [&[0x67, 1, 2][..], &[0x68, 3], &[0x65, 4]]);
        assert_eq!(nal_units(&[1, 2, 3]).count(), 0, "no start code");
        assert_eq!(nal_units(&[0, 0, 1]).count(), 0, "nothing after it");
    }

    #[test]
    fn only_an_access_unit_with_an_idr_slice_is_a_keyframe() {
        let frames = fixture_frames();
        assert_eq!(frames.len(), 4);
        assert!(is_h264_keyframe(&frames[0]));
        assert!(frames[1..].iter().all(|frame| !is_h264_keyframe(frame)));
        assert!(!is_h264_keyframe(&[]));
    }

    #[test]
    fn the_codecs_this_platform_shows_are_named() {
        assert!(decodes_video_codec(None), "absent means VP8");
        assert!(decodes_video_codec(Some(VideoCodec::Vp8)));
        assert_eq!(decodes_video_codec(Some(VideoCodec::H264)), decodes_h264());
        assert!(!decodes_video_codec(Some(VideoCodec::Unknown)));
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    fn pictures_larger_than_the_highest_layer_are_refused() {
        assert!(is_within_maximum_size(2_560, 1_440));
        assert!(!is_within_maximum_size(2_561, 1_440));
        assert!(!is_within_maximum_size(0, 10));
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn the_fixture_decodes_one_picture_per_access_unit_at_its_size() {
        let mut decoder = H264Decoder::new().unwrap();
        let mut picture_count = 0;
        for frame in fixture_frames() {
            if let Some(picture) = decoder.decode(&frame).unwrap() {
                assert_eq!((picture.width(), picture.height()), (320, 180));
                // The test pattern is not black: the luma plane carries it.
                assert!(picture.luma().iter().any(|sample| *sample > 64));
                picture_count += 1;
            }
        }
        assert_eq!(
            picture_count, 4,
            "one picture per access unit, none held back"
        );
    }
}
