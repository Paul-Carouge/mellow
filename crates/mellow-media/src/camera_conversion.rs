//! Raw camera layouts (YUY2, NV12 with row padding) to the packed limited-range I420 the VP8
//! encoder reads (`video_frame`). Pure functions, so they are tested on every platform; Linux
//! cameras (`camera_linux`) use them on their own thread, never in PipeWire's callback.

/// How a camera quantises its samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SampleRange {
    /// Luma 16 to 235, chroma 16 to 240: what VP8 assumes, copied as is.
    Limited,
    /// 0 to 255: every MJPEG picture (JFIF), squeezed into the limited range.
    Full,
}

/// Maps a full-range sample onto `16 ..= 16 + span`, rounded.
const fn full_to_limited_table(span: u32) -> [u8; 256] {
    let mut table = [0; 256];
    let mut sample = 0;
    while sample < 256 {
        // At most 16 + 224: always a byte.
        #[allow(clippy::cast_possible_truncation)]
        let limited = (16 + (sample * span + 127) / 255) as u8;
        table[sample as usize] = limited;
        sample += 1;
    }
    table
}

const LUMA_SPAN: u32 = 235 - 16;
const CHROMA_SPAN: u32 = 240 - 16;
static FULL_TO_LIMITED_LUMA: [u8; 256] = full_to_limited_table(LUMA_SPAN);
static FULL_TO_LIMITED_CHROMA: [u8; 256] = full_to_limited_table(CHROMA_SPAN);

fn luma_sample(sample: u8, range: SampleRange) -> u8 {
    match range {
        SampleRange::Limited => sample,
        SampleRange::Full => FULL_TO_LIMITED_LUMA[usize::from(sample)],
    }
}

fn chroma_sample(sample: u8, range: SampleRange) -> u8 {
    match range {
        SampleRange::Limited => sample,
        SampleRange::Full => FULL_TO_LIMITED_CHROMA[usize::from(sample)],
    }
}

fn to_usize(value: u32) -> usize {
    // u32 always fits in usize on the 32- and 64-bit targets Mellow supports.
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// Whether `rows` rows of `row_bytes` fit in `bytes` with `stride` bytes per row.
fn holds_rows(bytes: &[u8], stride: usize, row_bytes: usize, rows: usize) -> bool {
    stride >= row_bytes
        && rows > 0
        && stride
            .checked_mul(rows - 1)
            .and_then(|start| start.checked_add(row_bytes))
            .is_some_and(|needed| needed <= bytes.len())
}

/// A packed I420 picture (`width × height` luma, then two `⌈width / 2⌉ × ⌈height / 2⌉` chroma
/// planes) from YUY2 (`Y0 U Y1 V` per two pixels) with `stride` bytes per row. Its 4:2:2 chroma
/// is averaged over each pair of rows. `None` when the bytes are too short for the size.
pub(crate) fn i420_from_yuy2(
    width: u32,
    height: u32,
    (bytes, stride): (&[u8], usize),
    range: SampleRange,
) -> Option<Vec<u8>> {
    let (width, height) = (to_usize(width), to_usize(height));
    let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
    if width == 0 || !holds_rows(bytes, stride, chroma_width * 4, height) {
        return None;
    }
    let mut picture = vec![0; width * height + 2 * chroma_width * chroma_height];
    let (luma, chroma) = picture.split_at_mut(width * height);
    let (blue, red) = chroma.split_at_mut(chroma_width * chroma_height);
    let row = |index: usize| &bytes[index * stride..index * stride + chroma_width * 4];
    for (index, target) in luma.chunks_exact_mut(width).enumerate() {
        // Each group of four bytes holds two luma samples; an odd width drops the last one.
        let samples = row(index)
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|group| [group[0], group[2]]);
        for (sample, source) in target.iter_mut().zip(samples) {
            *sample = luma_sample(source, range);
        }
    }
    for (index, (blue_row, red_row)) in blue
        .chunks_exact_mut(chroma_width)
        .zip(red.chunks_exact_mut(chroma_width))
        .enumerate()
    {
        let upper = row(2 * index);
        // An odd height repeats its last row.
        let lower = row((2 * index + 1).min(height - 1));
        for (((blue_sample, red_sample), upper), lower) in blue_row
            .iter_mut()
            .zip(red_row.iter_mut())
            .zip(upper.as_chunks::<4>().0)
            .zip(lower.as_chunks::<4>().0)
        {
            let average = |a: u8, b: u8| u8::try_from((u16::from(a) + u16::from(b)).div_ceil(2));
            *blue_sample = chroma_sample(average(upper[1], lower[1]).ok()?, range);
            *red_sample = chroma_sample(average(upper[3], lower[3]).ok()?, range);
        }
    }
    Some(picture)
}

/// A packed I420 picture from NV12 planes with their own strides: the luma plane, then the
/// interleaved blue-red chroma plane at half size. `None` when a plane is too short.
pub(crate) fn i420_from_nv12(
    width: u32,
    height: u32,
    (luma_bytes, luma_stride): (&[u8], usize),
    (chroma_bytes, chroma_stride): (&[u8], usize),
    range: SampleRange,
) -> Option<Vec<u8>> {
    let (width, height) = (to_usize(width), to_usize(height));
    let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
    if width == 0
        || !holds_rows(luma_bytes, luma_stride, width, height)
        || !holds_rows(chroma_bytes, chroma_stride, chroma_width * 2, chroma_height)
    {
        return None;
    }
    let mut picture = vec![0; width * height + 2 * chroma_width * chroma_height];
    let (luma, chroma) = picture.split_at_mut(width * height);
    let (blue, red) = chroma.split_at_mut(chroma_width * chroma_height);
    for (target, source) in luma
        .chunks_exact_mut(width)
        .zip(luma_bytes.chunks(luma_stride))
    {
        for (sample, source) in target.iter_mut().zip(source) {
            *sample = luma_sample(*source, range);
        }
    }
    for ((blue_row, red_row), source) in blue
        .chunks_exact_mut(chroma_width)
        .zip(red.chunks_exact_mut(chroma_width))
        .zip(chroma_bytes.chunks(chroma_stride))
    {
        for ((blue_sample, red_sample), pair) in blue_row
            .iter_mut()
            .zip(red_row.iter_mut())
            .zip(source.as_chunks::<2>().0)
        {
            *blue_sample = chroma_sample(pair[0], range);
            *red_sample = chroma_sample(pair[1], range);
        }
    }
    Some(picture)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video_frame::{CapturedFrame, CapturedPixelFormat};

    #[test]
    fn full_range_squeezes_into_the_limited_range() {
        assert_eq!(luma_sample(0, SampleRange::Full), 16);
        assert_eq!(luma_sample(255, SampleRange::Full), 235);
        assert_eq!(chroma_sample(0, SampleRange::Full), 16);
        assert_eq!(chroma_sample(128, SampleRange::Full), 128);
        assert_eq!(chroma_sample(255, SampleRange::Full), 240);
        assert_eq!(luma_sample(0, SampleRange::Limited), 0);
    }

    #[test]
    fn yuy2_rows_with_padding_become_i420_with_chroma_averaged_over_two_rows() {
        // 4 × 2 pixels, rows of 8 bytes padded to 10.
        let bytes = [
            10, 100, 11, 200, 12, 102, 13, 202, 0, 0, //
            20, 110, 21, 210, 22, 112, 23, 212, 0, 0,
        ];
        let picture = i420_from_yuy2(4, 2, (&bytes, 10), SampleRange::Limited).unwrap();
        assert_eq!(
            picture,
            [
                10, 11, 12, 13, 20, 21, 22, 23, // luma
                105, 107, // blue: (100 + 110) / 2, (102 + 112) / 2
                205, 207, // red
            ]
        );
        assert_eq!(
            picture.len(),
            CapturedFrame::expected_length(4, 2, CapturedPixelFormat::I420)
        );
    }

    #[test]
    fn odd_yuy2_sizes_keep_whole_chroma_samples() {
        // 3 × 3: two groups per row, the last luma sample of each row dropped.
        let row = [50, 60, 51, 70, 52, 61, 53, 71];
        let bytes = [row, row, row].concat();
        let picture = i420_from_yuy2(3, 3, (&bytes, 8), SampleRange::Limited).unwrap();
        assert_eq!(
            picture.len(),
            CapturedFrame::expected_length(3, 3, CapturedPixelFormat::I420)
        );
        assert_eq!(&picture[..3], &[50, 51, 52]);
        assert_eq!(&picture[9..], &[60, 61, 60, 61, 70, 71, 70, 71]);
    }

    #[test]
    fn nv12_planes_with_their_own_strides_become_i420() {
        let luma = [1, 2, 0, 3, 4, 0];
        let chroma = [100, 200, 9];
        let picture = i420_from_nv12(2, 2, (&luma, 3), (&chroma, 3), SampleRange::Full).unwrap();
        assert_eq!(
            picture,
            [
                luma_sample(1, SampleRange::Full),
                luma_sample(2, SampleRange::Full),
                luma_sample(3, SampleRange::Full),
                luma_sample(4, SampleRange::Full),
                chroma_sample(100, SampleRange::Full),
                chroma_sample(200, SampleRange::Full),
            ]
        );
    }

    #[test]
    fn planes_too_short_for_the_size_are_refused() {
        assert!(i420_from_yuy2(4, 2, (&[0; 15], 8), SampleRange::Limited).is_none());
        assert!(i420_from_yuy2(4, 2, (&[0; 16], 6), SampleRange::Limited).is_none());
        assert!(i420_from_yuy2(0, 2, (&[0; 16], 8), SampleRange::Limited).is_none());
        assert!(i420_from_nv12(2, 2, (&[0; 3], 2), (&[0; 2], 2), SampleRange::Limited).is_none());
        assert!(i420_from_nv12(2, 2, (&[0; 4], 2), (&[0; 1], 2), SampleRange::Limited).is_none());
        assert!(i420_from_nv12(2, 2, (&[0; 4], 2), (&[0; 2], 2), SampleRange::Limited).is_some());
    }
}
