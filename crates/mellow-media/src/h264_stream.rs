//! Reading an H.264 Annex B stream just enough to send it over WebRTC (ADR-0052, ADR-0054):
//! where its NAL units are, whether a sequence parameter set is Constrained Baseline, and making
//! sure every IDR access unit carries the SPS and PPS it needs, as the server requires.

use std::borrow::Cow;

/// `nal_unit_type` values (H.264 table 7-1).
const NAL_IDR_SLICE: u8 = 5;
const NAL_SEQUENCE_PARAMETER_SET: u8 = 7;
const NAL_PICTURE_PARAMETER_SET: u8 = 8;
/// `profile_idc` of Baseline; Constrained Baseline adds `constraint_set1_flag`.
const BASELINE_PROFILE: u8 = 66;
const CONSTRAINT_SET1_FLAG: u8 = 0x40;
const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// The NAL units of an Annex B stream, without their start codes.
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

/// The unit's `nal_unit_type`.
pub(crate) fn nal_type(unit: &[u8]) -> Option<u8> {
    unit.first().map(|header| header & 0x1f)
}

/// Whether `unit` is a sequence parameter set of the Constrained Baseline profile, the only one
/// the server forwards.
pub(crate) fn is_constrained_baseline_sequence_parameter_set(unit: &[u8]) -> bool {
    nal_type(unit) == Some(NAL_SEQUENCE_PARAMETER_SET)
        && unit.get(1) == Some(&BASELINE_PROFILE)
        && unit
            .get(2)
            .is_some_and(|flags| flags & CONSTRAINT_SET1_FLAG != 0)
}

/// The last SPS and PPS of a stream, put back before an IDR that lacks them.
#[derive(Debug, Default)]
pub(crate) struct ParameterSets {
    /// Annex B: every SPS then every PPS of the last access unit that had both.
    annex_b: Vec<u8>,
}

impl ParameterSets {
    /// `access_unit` with the parameter sets before it when it holds an IDR slice without them;
    /// unchanged otherwise. Remembers the sets of any access unit that carries both.
    pub(crate) fn complete<'a>(&mut self, access_unit: &'a [u8]) -> Cow<'a, [u8]> {
        let types = || nal_units(access_unit).filter_map(nal_type);
        let has_idr = types().any(|kind| kind == NAL_IDR_SLICE);
        let has_sequence_set = types().any(|kind| kind == NAL_SEQUENCE_PARAMETER_SET);
        let has_picture_set = types().any(|kind| kind == NAL_PICTURE_PARAMETER_SET);
        if has_sequence_set && has_picture_set {
            self.annex_b.clear();
            for wanted in [NAL_SEQUENCE_PARAMETER_SET, NAL_PICTURE_PARAMETER_SET] {
                for unit in nal_units(access_unit).filter(|unit| nal_type(unit) == Some(wanted)) {
                    self.annex_b.extend_from_slice(&START_CODE);
                    self.annex_b.extend_from_slice(unit);
                }
            }
            return Cow::Borrowed(access_unit);
        }
        if !has_idr || self.annex_b.is_empty() {
            return Cow::Borrowed(access_unit);
        }
        let mut completed = Vec::with_capacity(self.annex_b.len() + access_unit.len());
        completed.extend_from_slice(&self.annex_b);
        completed.extend_from_slice(access_unit);
        Cow::Owned(completed)
    }
}

/// Whether the access unit holds an IDR slice.
pub(crate) fn has_idr_slice(access_unit: &[u8]) -> bool {
    nal_units(access_unit).any(|unit| nal_type(unit) == Some(NAL_IDR_SLICE))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// libx264, Constrained Baseline, 320 × 180, six pictures, an IDR every three with its
    /// parameter sets (`repeat-headers`).
    const CONSTRAINED_BASELINE: &[u8] =
        include_bytes!("../tests/fixtures/h264_constrained_baseline.h264");
    /// libx264, Main profile with two B-frames between anchors.
    const MAIN_WITH_B_FRAMES: &[u8] =
        include_bytes!("../tests/fixtures/h264_main_with_b_frames.h264");

    /// `slice_type` of a slice's header (H.264 7.3.3), emulation prevention removed: the second
    /// unsigned exp-Golomb code after the one-byte NAL header.
    fn slice_type(unit: &[u8]) -> Option<u32> {
        let mut payload = Vec::new();
        let mut zeros = 0;
        for &byte in unit.get(1..)? {
            if zeros >= 2 && byte == 3 {
                zeros = 0;
                continue;
            }
            zeros = if byte == 0 { zeros + 1 } else { 0 };
            payload.push(byte);
        }
        let mut bit = 0_usize;
        let mut read_bit = || {
            let value = payload.get(bit / 8).map(|byte| (byte >> (7 - bit % 8)) & 1);
            bit += 1;
            value
        };
        let mut read_unsigned_exp_golomb = || {
            let mut leading_zeros = 0;
            while read_bit()? == 0 {
                leading_zeros += 1;
            }
            let mut suffix = 0_u32;
            for _ in 0..leading_zeros {
                suffix = (suffix << 1) | u32::from(read_bit()?);
            }
            Some((1 << leading_zeros) - 1 + suffix)
        };
        let _first_macroblock = read_unsigned_exp_golomb()?;
        read_unsigned_exp_golomb()
    }

    /// B slices are `slice_type` 1 or 6.
    fn has_bidirectional_slices(stream: &[u8]) -> bool {
        nal_units(stream)
            .filter(|unit| matches!(nal_type(unit), Some(1 | 5)))
            .filter_map(slice_type)
            .any(|kind| kind % 5 == 1)
    }

    /// The access units of a stream: each begins at its parameter sets or at a slice whose
    /// `first_mb_in_slice` is 0 (one slice per picture here).
    fn access_units(stream: &[u8]) -> Vec<Vec<u8>> {
        let mut units: Vec<Vec<u8>> = Vec::new();
        let mut is_picture_started = false;
        for unit in nal_units(stream) {
            let kind = nal_type(unit);
            let starts_new = match kind {
                Some(NAL_SEQUENCE_PARAMETER_SET | 6 | 9 | 1 | NAL_IDR_SLICE) => is_picture_started,
                _ => false,
            };
            if starts_new || units.is_empty() {
                units.push(Vec::new());
                is_picture_started = false;
            }
            if matches!(kind, Some(1 | NAL_IDR_SLICE)) {
                is_picture_started = true;
            }
            let current = units.last_mut().unwrap();
            current.extend_from_slice(&START_CODE);
            current.extend_from_slice(unit);
        }
        units
    }

    #[test]
    fn the_recorded_sample_is_constrained_baseline_without_b_frames() {
        let sequence_sets: Vec<&[u8]> = nal_units(CONSTRAINED_BASELINE)
            .filter(|unit| nal_type(unit) == Some(NAL_SEQUENCE_PARAMETER_SET))
            .collect();
        assert_eq!(sequence_sets.len(), 2, "one before each IDR");
        assert!(
            sequence_sets
                .iter()
                .all(|unit| is_constrained_baseline_sequence_parameter_set(unit))
        );
        assert!(!has_bidirectional_slices(CONSTRAINED_BASELINE));
    }

    #[test]
    fn main_profile_and_b_frames_are_told_apart() {
        let sequence_set = nal_units(MAIN_WITH_B_FRAMES)
            .find(|unit| nal_type(unit) == Some(NAL_SEQUENCE_PARAMETER_SET))
            .unwrap();
        assert!(!is_constrained_baseline_sequence_parameter_set(
            sequence_set
        ));
        assert!(has_bidirectional_slices(MAIN_WITH_B_FRAMES));
    }

    #[test]
    fn every_idr_of_the_sample_carries_its_parameter_sets() {
        let units = access_units(CONSTRAINED_BASELINE);
        assert_eq!(units.len(), 6);
        let mut sets = ParameterSets::default();
        for unit in &units {
            assert!(matches!(sets.complete(unit), Cow::Borrowed(_)));
        }
        assert_eq!(units.iter().filter(|unit| has_idr_slice(unit)).count(), 2);
    }

    #[test]
    fn an_idr_without_parameter_sets_gets_the_last_ones() {
        let units = access_units(CONSTRAINED_BASELINE);
        let mut sets = ParameterSets::default();
        sets.complete(&units[0]);
        // The second IDR (the fourth picture) with its SPS and PPS taken out.
        let stripped: Vec<u8> = nal_units(&units[3])
            .filter(|unit| {
                !matches!(
                    nal_type(unit),
                    Some(NAL_SEQUENCE_PARAMETER_SET | NAL_PICTURE_PARAMETER_SET)
                )
            })
            .flat_map(|unit| START_CODE.iter().chain(unit).copied())
            .collect();
        assert!(has_idr_slice(&stripped));
        let completed = sets.complete(&stripped);
        let kinds: Vec<u8> = nal_units(&completed).filter_map(nal_type).collect();
        assert_eq!(
            &kinds[..2],
            [NAL_SEQUENCE_PARAMETER_SET, NAL_PICTURE_PARAMETER_SET]
        );
        assert!(kinds.contains(&NAL_IDR_SLICE));
        // A delta picture is never touched.
        assert!(matches!(sets.complete(&units[1]), Cow::Borrowed(_)));
    }

    #[test]
    fn start_codes_of_three_and_four_bytes_and_trailing_zeros_are_read() {
        let stream = [
            0, 0, 0, 1, 0x67, 66, 0xc0, 0, 0, 0, 1, 0x68, 1, 0, 0, 0, 1, 0x65, 9, 0,
        ];
        let units: Vec<&[u8]> = nal_units(&stream).collect();
        assert_eq!(units, [&[0x67, 66, 0xc0][..], &[0x68, 1], &[0x65, 9]]);
        assert!(is_constrained_baseline_sequence_parameter_set(units[0]));
        assert!(nal_units(&[0, 0, 0]).next().is_none());
    }
}
