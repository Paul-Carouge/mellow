//! Pictures: what capture hands over (BGRA, NV12, I420), the I420 the VP8 codec reads and
//! writes, and what the interface draws (BGRA, or NV12 for a macOS `CVPixelBuffer`). Colours are
//! BT.601 limited range, what VP8 decoders assume without signalling (browsers included).

use yuv::{
    BufferStoreMut, YuvConversionMode, YuvPlanarImage, YuvPlanarImageMut, YuvRange,
    YuvStandardMatrix,
};

const RANGE: YuvRange = YuvRange::Limited;
const MATRIX: YuvStandardMatrix = YuvStandardMatrix::Bt601;
const BGRA_BYTES_PER_PIXEL: usize = 4;

/// A chroma plane's width or height for a luma one: half, rounded up.
const fn chroma_length(luma_length: u32) -> u32 {
    luma_length.div_ceil(2)
}

fn to_usize(value: u32) -> usize {
    // u32 always fits in usize on the 32- and 64-bit targets Mellow supports.
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// How captured pixels are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapturedPixelFormat {
    /// 4 bytes per pixel, blue first: screens, and cameras on some systems.
    Bgra,
    /// A luma plane then one interleaved blue-red chroma plane at half size: most cameras.
    Nv12,
    /// A luma plane then the blue and the red chroma planes at half size.
    I420,
}

/// One captured picture, its planes packed without padding (BGRA rows of `width × 4` bytes;
/// chroma planes of `⌈width / 2⌉ × ⌈height / 2⌉`). Capture backends copy into this layout.
#[derive(Clone, PartialEq, Eq)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub format: CapturedPixelFormat,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for CapturedFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CapturedFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

impl CapturedFrame {
    /// The byte count of a packed picture of this size and format.
    #[must_use]
    pub fn expected_length(width: u32, height: u32, format: CapturedPixelFormat) -> usize {
        let luma = to_usize(width) * to_usize(height);
        let chroma = to_usize(chroma_length(width)) * to_usize(chroma_length(height));
        match format {
            CapturedPixelFormat::Bgra => luma * BGRA_BYTES_PER_PIXEL,
            CapturedPixelFormat::Nv12 | CapturedPixelFormat::I420 => luma + 2 * chroma,
        }
    }

    /// The picture as I420, or `None` when its bytes do not match its size and format (a
    /// capture backend's bug: the frame is dropped).
    pub(crate) fn to_i420(&self) -> Option<I420Picture> {
        if self.width == 0
            || self.height == 0
            || self.bytes.len() != Self::expected_length(self.width, self.height, self.format)
        {
            return None;
        }
        match self.format {
            CapturedPixelFormat::Bgra => {
                I420Picture::from_bgra(self.width, self.height, &self.bytes)
            }
            CapturedPixelFormat::Nv12 => {
                I420Picture::from_nv12(self.width, self.height, &self.bytes)
            }
            CapturedPixelFormat::I420 => {
                let mut picture = I420Picture::black(self.width, self.height);
                let (luma, chroma) = self.bytes.split_at(picture.luma.len());
                let (blue, red) = chroma.split_at(picture.blue_chroma.len());
                picture.luma.copy_from_slice(luma);
                picture.blue_chroma.copy_from_slice(blue);
                picture.red_chroma.copy_from_slice(red);
                Some(picture)
            }
        }
    }
}

/// A borrowed I420 picture whose planes are known to hold `stride × rows` bytes each.
#[derive(Debug, Clone, Copy)]
pub(crate) struct I420View<'a> {
    width: u32,
    height: u32,
    luma: &'a [u8],
    luma_stride: usize,
    blue_chroma: &'a [u8],
    red_chroma: &'a [u8],
    chroma_stride: usize,
}

impl<'a> I420View<'a> {
    /// `None` when a plane is too short for its stride and rows, or a stride for the width.
    pub(crate) fn new(
        width: u32,
        height: u32,
        (luma, luma_stride): (&'a [u8], usize),
        (blue_chroma, red_chroma, chroma_stride): (&'a [u8], &'a [u8], usize),
    ) -> Option<Self> {
        let chroma_rows = to_usize(chroma_length(height));
        let is_valid = width > 0
            && height > 0
            && luma_stride >= to_usize(width)
            && chroma_stride >= to_usize(chroma_length(width))
            && luma.len() >= luma_stride.checked_mul(to_usize(height))?
            && blue_chroma.len() >= chroma_stride.checked_mul(chroma_rows)?
            && red_chroma.len() >= chroma_stride.checked_mul(chroma_rows)?;
        is_valid.then_some(Self {
            width,
            height,
            luma,
            luma_stride,
            blue_chroma,
            red_chroma,
            chroma_stride,
        })
    }

    pub(crate) fn width(&self) -> u32 {
        self.width
    }

    pub(crate) fn height(&self) -> u32 {
        self.height
    }

    pub(crate) fn luma(&self) -> &'a [u8] {
        self.luma
    }

    pub(crate) fn luma_stride(&self) -> usize {
        self.luma_stride
    }

    pub(crate) fn blue_chroma(&self) -> &'a [u8] {
        self.blue_chroma
    }

    pub(crate) fn red_chroma(&self) -> &'a [u8] {
        self.red_chroma
    }

    pub(crate) fn chroma_stride(&self) -> usize {
        self.chroma_stride
    }

    /// The picture shrunk to `width × height` by averaging (never enlarged: a larger target
    /// keeps nearest pixels).
    pub(crate) fn scaled(&self, width: u32, height: u32) -> I420Picture {
        let mut picture = I420Picture::black(width, height);
        scale_plane(
            (self.luma, self.luma_stride, self.width, self.height),
            (&mut picture.luma, width, height),
        );
        let (chroma_width, chroma_height) = (chroma_length(width), chroma_length(height));
        let (source_chroma_width, source_chroma_height) =
            (chroma_length(self.width), chroma_length(self.height));
        for (source, target) in [
            (self.blue_chroma, &mut picture.blue_chroma),
            (self.red_chroma, &mut picture.red_chroma),
        ] {
            scale_plane(
                (
                    source,
                    self.chroma_stride,
                    source_chroma_width,
                    source_chroma_height,
                ),
                (target, chroma_width, chroma_height),
            );
        }
        picture
    }

    /// Packed BGRA, `width × 4` bytes per row.
    pub(crate) fn to_bgra(self) -> Option<Vec<u8>> {
        let row_bytes = to_usize(self.width) * BGRA_BYTES_PER_PIXEL;
        let mut bgra = vec![0; row_bytes * to_usize(self.height)];
        let planar = YuvPlanarImage {
            y_plane: self.luma,
            y_stride: u32::try_from(self.luma_stride).ok()?,
            u_plane: self.blue_chroma,
            u_stride: u32::try_from(self.chroma_stride).ok()?,
            v_plane: self.red_chroma,
            v_stride: u32::try_from(self.chroma_stride).ok()?,
            width: self.width,
            height: self.height,
        };
        yuv::yuv420_to_bgra(
            &planar,
            &mut bgra,
            u32::try_from(row_bytes).ok()?,
            RANGE,
            MATRIX,
        )
        .ok()?;
        Some(bgra)
    }

    /// Packed NV12: the luma plane, then blue and red chroma interleaved.
    pub(crate) fn to_nv12(self) -> Vec<u8> {
        let (width, height) = (to_usize(self.width), to_usize(self.height));
        let chroma_width = to_usize(chroma_length(self.width));
        let chroma_height = to_usize(chroma_length(self.height));
        let mut nv12 = Vec::with_capacity(width * height + 2 * chroma_width * chroma_height);
        for row in self.luma.chunks(self.luma_stride).take(height) {
            nv12.extend_from_slice(&row[..width]);
        }
        for (blue_row, red_row) in self
            .blue_chroma
            .chunks(self.chroma_stride)
            .zip(self.red_chroma.chunks(self.chroma_stride))
            .take(chroma_height)
        {
            for (blue, red) in blue_row[..chroma_width]
                .iter()
                .zip(&red_row[..chroma_width])
            {
                nv12.extend_from_slice(&[*blue, *red]);
            }
        }
        nv12
    }
}

/// An owned I420 picture, planes packed (strides equal widths).
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct I420Picture {
    width: u32,
    height: u32,
    luma: Vec<u8>,
    blue_chroma: Vec<u8>,
    red_chroma: Vec<u8>,
}

impl std::fmt::Debug for I420Picture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("I420Picture")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

/// Limited-range black: luma 16, neutral chroma.
const BLACK_LUMA: u8 = 16;
const NEUTRAL_CHROMA: u8 = 128;

impl I420Picture {
    pub(crate) fn black(width: u32, height: u32) -> Self {
        let chroma = to_usize(chroma_length(width)) * to_usize(chroma_length(height));
        Self {
            width,
            height,
            luma: vec![BLACK_LUMA; to_usize(width) * to_usize(height)],
            blue_chroma: vec![NEUTRAL_CHROMA; chroma],
            red_chroma: vec![NEUTRAL_CHROMA; chroma],
        }
    }

    pub(crate) fn view(&self) -> I420View<'_> {
        I420View {
            width: self.width,
            height: self.height,
            luma: &self.luma,
            luma_stride: to_usize(self.width),
            blue_chroma: &self.blue_chroma,
            red_chroma: &self.red_chroma,
            chroma_stride: to_usize(chroma_length(self.width)),
        }
    }

    /// Mutable planes, for tests and synthetic sources: luma, blue chroma, red chroma.
    #[cfg(test)]
    pub(crate) fn planes_mut(&mut self) -> (&mut [u8], &mut [u8], &mut [u8]) {
        (&mut self.luma, &mut self.blue_chroma, &mut self.red_chroma)
    }

    fn from_bgra(width: u32, height: u32, bgra: &[u8]) -> Option<Self> {
        let mut picture = Self::black(width, height);
        let chroma_stride = chroma_length(width);
        let mut planar = YuvPlanarImageMut {
            y_plane: BufferStoreMut::Borrowed(&mut picture.luma),
            y_stride: width,
            u_plane: BufferStoreMut::Borrowed(&mut picture.blue_chroma),
            u_stride: chroma_stride,
            v_plane: BufferStoreMut::Borrowed(&mut picture.red_chroma),
            v_stride: chroma_stride,
            width,
            height,
        };
        yuv::bgra_to_yuv420(
            &mut planar,
            bgra,
            width.checked_mul(4)?,
            RANGE,
            MATRIX,
            YuvConversionMode::Balanced,
        )
        .ok()?;
        Some(picture)
    }

    fn from_nv12(width: u32, height: u32, nv12: &[u8]) -> Option<Self> {
        let mut picture = Self::black(width, height);
        let (luma, interleaved) = nv12.split_at_checked(picture.luma.len())?;
        picture.luma.copy_from_slice(luma);
        for (([blue_sample, red_sample], blue), red) in interleaved
            .as_chunks::<2>()
            .0
            .iter()
            .zip(picture.blue_chroma.iter_mut())
            .zip(picture.red_chroma.iter_mut())
        {
            *blue = *blue_sample;
            *red = *red_sample;
        }
        Some(picture)
    }
}

/// The largest size within `maximum` with the source's aspect ratio, never larger than the
/// source, both sides even (whole chroma samples) and at least 2.
pub(crate) fn fit_within(
    (width, height): (u32, u32),
    (maximum_width, maximum_height): (u32, u32),
) -> (u32, u32) {
    let (width_64, height_64) = (u64::from(width.max(1)), u64::from(height.max(1)));
    // Scale by the tighter of the two limits, in integers: w × mh ≤ mw × h decides which.
    let (fitted_width, fitted_height) = if width <= maximum_width && height <= maximum_height {
        (width_64, height_64)
    } else if width_64 * u64::from(maximum_height) >= u64::from(maximum_width) * height_64 {
        (
            u64::from(maximum_width),
            height_64 * u64::from(maximum_width) / width_64,
        )
    } else {
        (
            width_64 * u64::from(maximum_height) / height_64,
            u64::from(maximum_height),
        )
    };
    let even = |length: u64| u32::try_from(length & !1).unwrap_or(u32::MAX).max(2);
    (even(fitted_width), even(fitted_height))
}

/// Each simulcast layer is exactly half the one above: averaging 2 × 2 blocks in fixed-size
/// chunks lets the compiler vectorise it (the general path below is several times slower).
fn halve_plane(
    (source, source_stride): (&[u8], usize),
    (target, target_width): (&mut [u8], usize),
) {
    let mut source_rows = source.chunks(source_stride);
    for target_row in target.chunks_mut(target_width) {
        let (Some(upper), Some(lower)) = (source_rows.next(), source_rows.next()) else {
            return;
        };
        let (upper_pairs, _) = upper.as_chunks::<2>();
        let (lower_pairs, _) = lower.as_chunks::<2>();
        for ((sample, upper_pair), lower_pair) in
            target_row.iter_mut().zip(upper_pairs).zip(lower_pairs)
        {
            let sum = u16::from(upper_pair[0])
                + u16::from(upper_pair[1])
                + u16::from(lower_pair[0])
                + u16::from(lower_pair[1]);
            // Rounded average of four bytes: at most 255.
            *sample = u8::try_from((sum + 2) / 4).unwrap_or(u8::MAX);
        }
    }
}

/// Box-filter scaling of one plane: each target sample averages the source samples it covers,
/// which keeps fine detail (text) from aliasing when a picture is shrunk by a large factor.
fn scale_plane(
    (source, source_stride, source_width, source_height): (&[u8], usize, u32, u32),
    (target, target_width, target_height): (&mut [u8], u32, u32),
) {
    if source_width == 2 * target_width && source_height == 2 * target_height {
        halve_plane((source, source_stride), (target, to_usize(target_width)));
        return;
    }
    let spans = |source_length: u32, target_length: u32| -> Vec<(usize, usize)> {
        (0..u64::from(target_length))
            .map(|index| {
                let start = index * u64::from(source_length) / u64::from(target_length);
                let end = ((index + 1) * u64::from(source_length) / u64::from(target_length))
                    .max(start + 1);
                (
                    usize::try_from(start).unwrap_or(usize::MAX),
                    usize::try_from(end).unwrap_or(usize::MAX),
                )
            })
            .collect()
    };
    let columns = spans(source_width, target_width);
    let rows = spans(source_height, target_height);
    let mut sums = vec![0_u32; columns.len()];
    for (target_row, &(row_start, row_end)) in target.chunks_mut(to_usize(target_width)).zip(&rows)
    {
        sums.fill(0);
        for source_row in source
            .chunks(source_stride)
            .skip(row_start)
            .take(row_end - row_start)
        {
            for (sum, &(column_start, column_end)) in sums.iter_mut().zip(&columns) {
                *sum += source_row[column_start..column_end]
                    .iter()
                    .map(|&sample| u32::from(sample))
                    .sum::<u32>();
            }
        }
        for ((sample, sum), &(column_start, column_end)) in
            target_row.iter_mut().zip(&sums).zip(&columns)
        {
            let count = u32::try_from((row_end - row_start) * (column_end - column_start))
                .unwrap_or(u32::MAX)
                .max(1);
            // Rounded average of 8-bit samples: always fits in a byte.
            *sample = u8::try_from((sum + count / 2) / count).unwrap_or(u8::MAX);
        }
    }
}

/// How a decoded picture is handed to the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecodedPixelFormat {
    /// Packed BGRA, `width × 4` bytes per row: an image on any platform.
    #[default]
    Bgra,
    /// Packed NV12: wrapped in a `CVPixelBuffer` and drawn without conversion on macOS.
    Nv12,
}

/// A decoded picture of someone's camera or screen, at most the size its tile asked for.
#[derive(Clone, PartialEq, Eq)]
pub struct DecodedVideoFrame {
    pub width: u32,
    pub height: u32,
    pub format: DecodedPixelFormat,
    /// Owned, so the interface wraps them into its image without copying.
    pub pixels: Vec<u8>,
}

impl std::fmt::Debug for DecodedVideoFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecodedVideoFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_fit_the_limit_keep_their_shape_and_are_never_enlarged() {
        assert_eq!(fit_within((1920, 1080), (1280, 720)), (1280, 720));
        assert_eq!(fit_within((640, 480), (1280, 720)), (640, 480));
        assert_eq!(fit_within((1440, 1080), (1280, 720)), (960, 720));
        assert_eq!(fit_within((3440, 1440), (2560, 1440)), (2560, 1070));
        assert_eq!(
            fit_within((641, 361), (1280, 720)),
            (640, 360),
            "even sides"
        );
        assert_eq!(fit_within((1, 1), (1280, 720)), (2, 2));
    }

    #[test]
    fn shrinking_averages_the_covered_samples() {
        let source = [10, 20, 30, 40, 50, 60, 70, 80];
        let mut target = [0_u8; 2];
        scale_plane((&source, 4, 4, 2), (&mut target, 2, 1));
        assert_eq!(target, [(10 + 20 + 50 + 60) / 4, (30 + 40 + 70 + 80) / 4]);
    }

    #[test]
    fn shrinking_by_another_factor_than_half_averages_too() {
        let source = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];
        let mut target = [0_u8; 2];
        scale_plane((&source, 6, 6, 2), (&mut target, 2, 1));
        assert_eq!(
            target,
            [
                50, // (10 + 20 + 30 + 70 + 80 + 90) / 6
                80, // (40 + 50 + 60 + 100 + 110 + 120) / 6
            ]
        );
    }

    #[test]
    fn bgra_survives_a_round_trip_through_i420() {
        let (width, height) = (8, 4);
        let bgra: Vec<u8> = (0..width * height)
            .flat_map(|_| [40_u8, 120, 200, 255])
            .collect();
        let picture = I420Picture::from_bgra(width, height, &bgra).unwrap();
        let back = picture.view().to_bgra().unwrap();
        for (original, converted) in bgra.iter().zip(&back) {
            assert!(
                original.abs_diff(*converted) <= 3,
                "{original} vs {converted}"
            );
        }
    }

    #[test]
    fn nv12_and_i420_hold_the_same_chroma() {
        let mut picture = I420Picture::black(4, 2);
        let (_, blue, red) = picture.planes_mut();
        blue.copy_from_slice(&[1, 2]);
        red.copy_from_slice(&[3, 4]);
        let nv12 = picture.view().to_nv12();
        assert_eq!(&nv12[8..], &[1, 3, 2, 4]);
        let frame = CapturedFrame {
            width: 4,
            height: 2,
            format: CapturedPixelFormat::Nv12,
            bytes: nv12,
        };
        assert_eq!(frame.to_i420().unwrap(), picture);
    }

    #[test]
    fn a_frame_whose_bytes_do_not_match_its_size_is_refused() {
        let frame = CapturedFrame {
            width: 4,
            height: 4,
            format: CapturedPixelFormat::Bgra,
            bytes: vec![0; 10],
        };
        assert!(frame.to_i420().is_none());
    }

    #[test]
    fn views_refuse_planes_shorter_than_their_strides() {
        let plane = [0_u8; 8];
        assert!(I420View::new(4, 2, (&plane, 4), (&plane[..2], &plane[..2], 2)).is_some());
        assert!(I420View::new(4, 2, (&plane[..7], 4), (&plane[..2], &plane[..2], 2)).is_none());
        assert!(I420View::new(4, 2, (&plane, 4), (&plane[..2], &plane[..1], 2)).is_none());
        assert!(I420View::new(4, 2, (&plane, 4), (&plane[..2], &plane[..2], 1)).is_none());
    }
}
