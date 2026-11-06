//! What the SFU reads of an H.264 keyframe (ADR-0052): the picture size its sequence parameter set
//! (SPS) declares, so a keyframe larger than its simulcast layer reaches no one, as with VP8
//! (ADR-0035). Only Constrained Baseline is accepted: it has no B-frames, which an SFU forwarding
//! frames in arrival order needs, and every browser decodes it. The server never decodes the
//! picture itself; it reads a few dozen bits of the SPS.

/// The NAL unit type of a sequence parameter set (ITU-T H.264, table 7-1).
const SEQUENCE_PARAMETER_SET_NAL_UNIT_TYPE: u8 = 7;
const NAL_UNIT_TYPE_MASK: u8 = 0x1f;
/// `profile_idc` of the Baseline profiles; Constrained Baseline adds `constraint_set1_flag`.
const BASELINE_PROFILE_IDC: u8 = 66;
const CONSTRAINT_SET1_FLAG: u8 = 0x40;
/// Pictures are coded in macroblocks of 16 × 16 samples.
const MACROBLOCK_SAMPLES: u64 = 16;
/// Cropping counts in pairs of samples each way for 4:2:0, the only chroma format of Baseline.
const CROP_UNIT_SAMPLES: u64 = 2;
/// An exp-Golomb code of an SPS field fits 32 bits; a longer run of zeros is malformed.
const MAXIMUM_EXP_GOLOMB_LEADING_ZEROS: u32 = 31;
/// `num_ref_frames_in_pic_order_cnt_cycle` is at most 255 (ITU-T H.264, 7.4.2.1.1).
const MAXIMUM_REFERENCE_FRAMES_IN_ORDER_COUNT_CYCLE: u32 = 255;

/// The picture size, in samples, declared by the first SPS of an access unit in Annex B form (the
/// form `str0m` hands depacketised H.264 frames in), after cropping. `None` when the access unit
/// has no SPS, when the SPS is not Constrained Baseline, or when it is malformed.
#[must_use]
pub(crate) fn constrained_baseline_picture_size(access_unit: &[u8]) -> Option<(u64, u64)> {
    let sequence_parameter_set = nal_units(access_unit).find(|nal_unit| {
        nal_unit.first().is_some_and(|header| {
            header & NAL_UNIT_TYPE_MASK == SEQUENCE_PARAMETER_SET_NAL_UNIT_TYPE
        })
    })?;
    let payload = without_emulation_prevention(sequence_parameter_set.get(1..)?);
    picture_size_of_sequence_parameter_set(&payload)
}

/// The NAL units of an Annex B access unit: what follows each `00 00 01` start code (the four-byte
/// form adds a zero, which ends the previous unit and is trimmed).
fn nal_units(access_unit: &[u8]) -> impl Iterator<Item = &[u8]> {
    const START_CODE: [u8; 3] = [0, 0, 1];
    let starts: Vec<usize> = access_unit
        .windows(START_CODE.len())
        .enumerate()
        .filter(|(_, window)| *window == START_CODE)
        .map(|(index, _)| index + START_CODE.len())
        .collect();
    let ends: Vec<usize> = starts
        .iter()
        .skip(1)
        .map(|next_start| next_start - START_CODE.len())
        .chain(std::iter::once(access_unit.len()))
        .collect();
    starts
        .into_iter()
        .zip(ends)
        .filter_map(move |(start, end)| {
            let nal_unit = access_unit.get(start..end)?;
            let length = nal_unit
                .iter()
                .rposition(|byte| *byte != 0)
                .map_or(0, |last| last + 1);
            nal_unit.get(..length)
        })
}

/// The raw bits of a NAL unit's payload: an encoder inserts `03` after two zero bytes so the
/// payload never looks like a start code.
fn without_emulation_prevention(payload: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(payload.len());
    let mut zeros_in_a_row = 0_usize;
    for &byte in payload {
        if zeros_in_a_row >= 2 && byte == 3 {
            zeros_in_a_row = 0;
            continue;
        }
        zeros_in_a_row = if byte == 0 { zeros_in_a_row + 1 } else { 0 };
        raw.push(byte);
    }
    raw
}

/// Reads an SPS payload (after its NAL header) up to its cropping (ITU-T H.264, 7.3.2.1.1).
fn picture_size_of_sequence_parameter_set(payload: &[u8]) -> Option<(u64, u64)> {
    let mut bits = BitReader::new(payload);
    let profile_idc = u8::try_from(bits.read_bits(8)?).ok()?;
    let constraint_flags = u8::try_from(bits.read_bits(8)?).ok()?;
    if profile_idc != BASELINE_PROFILE_IDC || constraint_flags & CONSTRAINT_SET1_FLAG == 0 {
        return None;
    }
    let _level_idc = bits.read_bits(8)?;
    let _seq_parameter_set_id = bits.read_exp_golomb()?;
    let _log2_max_frame_num_minus4 = bits.read_exp_golomb()?;
    match bits.read_exp_golomb()? {
        0 => {
            let _log2_max_pic_order_cnt_lsb_minus4 = bits.read_exp_golomb()?;
        }
        1 => {
            let _delta_pic_order_always_zero_flag = bits.read_bits(1)?;
            let _offset_for_non_ref_pic = bits.read_exp_golomb()?;
            let _offset_for_top_to_bottom_field = bits.read_exp_golomb()?;
            let cycle = bits.read_exp_golomb()?;
            if cycle > MAXIMUM_REFERENCE_FRAMES_IN_ORDER_COUNT_CYCLE {
                return None;
            }
            for _ in 0..cycle {
                let _offset_for_ref_frame = bits.read_exp_golomb()?;
            }
        }
        2 => {}
        _ => return None,
    }
    let _max_num_ref_frames = bits.read_exp_golomb()?;
    let _gaps_in_frame_num_value_allowed_flag = bits.read_bits(1)?;
    let width_in_macroblocks = u64::from(bits.read_exp_golomb()?) + 1;
    let height_in_map_units = u64::from(bits.read_exp_golomb()?) + 1;
    let is_frame_only = bits.read_bits(1)? == 1;
    if !is_frame_only {
        let _mb_adaptive_frame_field_flag = bits.read_bits(1)?;
    }
    let _direct_8x8_inference_flag = bits.read_bits(1)?;
    // Interlaced pictures count map units as field macroblock pairs.
    let fields_per_frame = if is_frame_only { 1 } else { 2 };
    let mut width = width_in_macroblocks * MACROBLOCK_SAMPLES;
    let mut height = fields_per_frame * height_in_map_units * MACROBLOCK_SAMPLES;
    if bits.read_bits(1)? == 1 {
        let mut crop = || bits.read_exp_golomb().map(u64::from);
        let (left, right, top, bottom) = (crop()?, crop()?, crop()?, crop()?);
        width = width.checked_sub(CROP_UNIT_SAMPLES * (left + right))?;
        height = height.checked_sub(CROP_UNIT_SAMPLES * fields_per_frame * (top + bottom))?;
    }
    (width > 0 && height > 0).then_some((width, height))
}

/// Reads bits most significant first.
struct BitReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> BitReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn read_bit(&mut self) -> Option<u32> {
        let byte = self.bytes.get(self.position / 8)?;
        let bit = (byte >> (7 - self.position % 8)) & 1;
        self.position += 1;
        Some(u32::from(bit))
    }

    fn read_bits(&mut self, count: u32) -> Option<u32> {
        (0..count).try_fold(0_u32, |value, _| Some((value << 1) | self.read_bit()?))
    }

    /// An unsigned exp-Golomb code, `ue(v)`. A signed one, `se(v)`, has the same length, which is
    /// all the fields skipped here need.
    fn read_exp_golomb(&mut self) -> Option<u32> {
        let mut leading_zeros = 0;
        while self.read_bit()? == 0 {
            leading_zeros += 1;
            if leading_zeros > MAXIMUM_EXP_GOLOMB_LEADING_ZEROS {
                return None;
            }
        }
        let suffix = self.read_bits(leading_zeros)?;
        // At most 31 leading zeros: the value fits a u32 once 1 is taken off.
        ((1_u64 << leading_zeros) - 1 + u64::from(suffix))
            .try_into()
            .ok()
    }
}

/// Builds H.264 test streams: an encoder's bit writer, enough for the fields read above.
#[cfg(test)]
pub(crate) mod test_streams {
    struct BitWriter {
        bytes: Vec<u8>,
        bit_count: usize,
    }

    impl BitWriter {
        fn bits(&mut self, value: u32, count: u32) {
            for shift in (0..count).rev() {
                if self.bit_count.is_multiple_of(8) {
                    self.bytes.push(0);
                }
                let bit = u8::try_from((value >> shift) & 1).unwrap();
                *self.bytes.last_mut().unwrap() |= bit << (7 - self.bit_count % 8);
                self.bit_count += 1;
            }
        }

        fn exp_golomb(&mut self, value: u32) {
            let code = value + 1;
            let length = 32 - code.leading_zeros();
            self.bits(0, length - 1);
            self.bits(code, length);
        }
    }

    /// An SPS NAL unit (with its header) for a `width` × `height` picture of `profile_idc` and
    /// `constraint_flags`, with emulation prevention, as an encoder writes it.
    #[must_use]
    pub(crate) fn sequence_parameter_set(
        profile_idc: u8,
        constraint_flags: u8,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        let mut writer = BitWriter {
            bytes: Vec::new(),
            bit_count: 0,
        };
        writer.bits(u32::from(profile_idc), 8);
        writer.bits(u32::from(constraint_flags), 8);
        // Level 5.1: a 1440p screen.
        writer.bits(51, 8);
        writer.exp_golomb(0);
        writer.exp_golomb(0);
        // pic_order_cnt_type 2, as WebRTC encoders write it.
        writer.exp_golomb(2);
        writer.exp_golomb(1);
        writer.bits(0, 1);
        let width_in_macroblocks = width.div_ceil(16);
        let height_in_macroblocks = height.div_ceil(16);
        writer.exp_golomb(width_in_macroblocks - 1);
        writer.exp_golomb(height_in_macroblocks - 1);
        // frame_mbs_only_flag, direct_8x8_inference_flag.
        writer.bits(1, 1);
        writer.bits(1, 1);
        let right = (width_in_macroblocks * 16 - width) / 2;
        let bottom = (height_in_macroblocks * 16 - height) / 2;
        let is_cropped = right > 0 || bottom > 0;
        writer.bits(u32::from(is_cropped), 1);
        if is_cropped {
            writer.exp_golomb(0);
            writer.exp_golomb(right);
            writer.exp_golomb(0);
            writer.exp_golomb(bottom);
        }
        // No VUI, then the RBSP stop bit.
        writer.bits(0, 1);
        writer.bits(1, 1);
        let mut nal_unit = vec![0x67];
        let mut zeros_in_a_row = 0;
        for byte in writer.bytes {
            if zeros_in_a_row >= 2 && byte <= 3 {
                nal_unit.push(3);
                zeros_in_a_row = 0;
            }
            zeros_in_a_row = if byte == 0 { zeros_in_a_row + 1 } else { 0 };
            nal_unit.push(byte);
        }
        nal_unit
    }

    /// An IDR access unit in Annex B form: SPS, PPS, then an IDR slice ending with `slice_tail`
    /// (which must hold no two zero bytes in a row).
    #[must_use]
    pub(crate) fn keyframe(width: u32, height: u32, slice_tail: &[u8]) -> Vec<u8> {
        let mut access_unit = vec![0, 0, 0, 1];
        access_unit.extend(sequence_parameter_set(66, 0xe0, width, height));
        access_unit.extend([0, 0, 0, 1, 0x68, 0xce, 0x3c, 0x80]);
        access_unit.extend([0, 0, 0, 1, 0x65, 0x88, 0x84]);
        access_unit.extend(slice_tail);
        access_unit
    }

    /// A non-IDR access unit: one slice ending with `slice_tail`.
    #[must_use]
    pub(crate) fn delta_frame(slice_tail: &[u8]) -> Vec<u8> {
        let mut access_unit = vec![0, 0, 0, 1, 0x41, 0x9a, 0x02];
        access_unit.extend(slice_tail);
        access_unit
    }
}

#[cfg(test)]
mod tests {
    use super::test_streams::{delta_frame, keyframe, sequence_parameter_set};
    use super::*;

    #[test]
    fn reads_the_cropped_size_of_a_constrained_baseline_keyframe() {
        for (width, height) in [
            (320, 180),
            (640, 360),
            (1_280, 720),
            (1_920, 1_080),
            (2_560, 1_440),
        ] {
            assert_eq!(
                constrained_baseline_picture_size(&keyframe(width, height, &[0xaa])),
                Some((u64::from(width), u64::from(height))),
                "{width} × {height}"
            );
        }
    }

    #[test]
    fn three_byte_start_codes_are_read_too() {
        let mut access_unit = vec![0, 0, 1];
        access_unit.extend(sequence_parameter_set(66, 0x40, 640, 360));
        access_unit.extend([0, 0, 1, 0x65, 0x88]);
        assert_eq!(
            constrained_baseline_picture_size(&access_unit),
            Some((640, 360))
        );
    }

    #[test]
    fn other_profiles_and_frames_without_a_sequence_parameter_set_are_refused() {
        let with_profile = |profile_idc, constraint_flags| {
            let mut access_unit = vec![0, 0, 0, 1];
            access_unit.extend(sequence_parameter_set(
                profile_idc,
                constraint_flags,
                320,
                180,
            ));
            access_unit
        };
        assert_eq!(
            constrained_baseline_picture_size(&with_profile(66, 0)),
            None,
            "Baseline without its constraint may hold what Constrained Baseline forbids"
        );
        assert_eq!(
            constrained_baseline_picture_size(&with_profile(100, 0x40)),
            None,
            "High has B-frames"
        );
        assert_eq!(constrained_baseline_picture_size(&delta_frame(&[1])), None);
        assert_eq!(constrained_baseline_picture_size(&[]), None);
        assert_eq!(constrained_baseline_picture_size(&[0, 0, 1, 0x67]), None);
    }

    #[test]
    fn malformed_sequence_parameter_sets_are_refused_without_panicking() {
        let whole = sequence_parameter_set(66, 0xe0, 2_560, 1_440);
        for length in 0..whole.len() - 1 {
            let mut access_unit = vec![0, 0, 1];
            access_unit.extend(whole.get(..length).unwrap());
            assert_eq!(
                constrained_baseline_picture_size(&access_unit),
                None,
                "{length} bytes"
            );
        }
        // Thirty-two zero bits then more: an exp-Golomb code too long for any field.
        let endless = [
            0, 0, 1, 0x67, 66, 0x40, 51, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x80,
        ];
        assert_eq!(constrained_baseline_picture_size(&endless), None);
    }

    #[test]
    fn emulation_prevention_bytes_are_removed_before_reading() {
        assert_eq!(
            without_emulation_prevention(&[1, 0, 0, 3, 1, 0, 0, 3, 0, 3]),
            vec![1, 0, 0, 1, 0, 0, 0, 3]
        );
    }
}
