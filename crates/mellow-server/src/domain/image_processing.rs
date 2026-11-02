//! Compression of uploaded pictures, visually lossless (ADR-0033). Pure functions over bytes, run
//! on a blocking thread by the caller: untrusted bytes are decoded in Rust (`image`), within limits
//! checked first; libwebp and rav1e only ever encode decoded pixels.

mod avif;
mod colour;
mod encoding;
mod quality;

use std::io::Cursor;
use std::time::{Duration, Instant};

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{
    AnimationDecoder as _, DynamicImage, ImageDecoder as _, ImageFormat, ImageReader, Limits,
    RgbaImage,
};

use colour::ColourSpace;
use encoding::{Encoded, last_resort_still, smallest_animation, smallest_still};

/// Larger pictures are kept as uploaded and never processed (decompression bombs). Checked from
/// the header, before any pixel is allocated; the total bounds the memory of one job.
const MAXIMUM_SIDE_PIXELS: u32 = 8_192;
const MAXIMUM_TOTAL_PIXELS: u64 = 24_000_000;
const MAXIMUM_FRAMES: usize = 600;
/// Stored sizes: three times the largest CSS size each is shown at, for high-density screens
/// (ADR-0033). Avatars show at most at 80 px (profile popover), banners in a 300 × 120 px card
/// that covers its box, so a taller banner only ever shows its middle band.
const AVATAR_SIDE_PIXELS: u32 = 256;
const BANNER_WIDTH_PIXELS: u32 = 960;
const BANNER_HEIGHT_PIXELS: u32 = 384;
/// Icons are shown small (server rail, channel row), so they are stored small: three times their
/// largest display size, for high-density screens.
const GUILD_ICON_SIDE_PIXELS: u32 = 256;
const CHANNEL_ICON_SIDE_PIXELS: u32 = 96;
/// Custom emoji show at most at 48 px (jumbo, alone in a message), so they are stored at 128 px,
/// as on Discord (ADR-0028).
const EMOJI_SIDE_PIXELS: u32 = 128;
/// Larger attachments are kept as uploaded (a JPEG stripped of its metadata), so one job stays
/// near 1 GB. Peak memory of one job, measured in release: 0.67 GB for a 12.2 MP JPEG photo,
/// 0.99 GB for it as PNG (lossless WebP is tried too), 0.84 GB at 16 MP as JPEG, 1.23 GB as PNG,
/// and 1.73 GB at 24 MP, the decode limit. 16 MP keeps the 13–15 MP photos of the measured
/// corpus compressed.
const MAXIMUM_COMPRESSED_PICTURE_PIXELS: u32 = 16_000_000;
/// Attachments show in the message list in a box of at most 400 × 300 CSS px
/// (`--mw-component-attachment-image-max-width` and `-height`); their previews fit twice that, for
/// high-density screens. Twice rather than three times as for avatars: a preview gives way to the
/// full picture as soon as it is opened, and three times would weigh about 2.25 times as much.
const PREVIEW_WIDTH_PIXELS: u32 = 800;
const PREVIEW_HEIGHT_PIXELS: u32 = 600;
/// Pictures past this are only ever encoded by the single attachment worker, so two of them never
/// run at once.
const LARGE_PICTURE_PIXELS: u32 = 1_000_000;
// Every avatar, banner and icon is under it once fitted, so processing them while uploading
// never adds a second large job, nor takes long.
const _: () = assert!(
    BANNER_WIDTH_PIXELS * BANNER_HEIGHT_PIXELS <= LARGE_PICTURE_PIXELS
        && GUILD_ICON_SIDE_PIXELS * GUILD_ICON_SIDE_PIXELS <= LARGE_PICTURE_PIXELS
        && LARGE_PICTURE_PIXELS < MAXIMUM_COMPRESSED_PICTURE_PIXELS
);
/// Searching qualities stops starting encodes past this, for pictures processed while uploading:
/// the upload itself gives up after 20 s.
const UPLOAD_SEARCH_TIME_BUDGET: Duration = Duration::from_secs(10);
/// Memory allowed to read a picture's header and its metadata, before any pixel.
const HEADER_READ_MAXIMUM_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum ImageProcessingError {
    #[error("not a picture this server reads")]
    Unsupported,
    #[error("the picture is too large to process")]
    TooLarge,
    #[error("the picture could not be encoded")]
    Encoding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfileImageKind {
    Avatar,
    Banner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IconKind {
    /// May be animated.
    Guild,
    /// Always still: an animated source keeps its first frame.
    Channel,
    /// A custom emoji (ADR-0028); may be animated.
    Emoji,
}

/// The square of a picture an icon keeps, in pixels of the picture as shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SquareCrop {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) size: u32,
}

/// A picture ready to store, with its served type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessedImage {
    pub(crate) bytes: Vec<u8>,
    pub(crate) content_type: &'static str,
    pub(crate) is_animated: bool,
}

/// An avatar or banner to store: the picture, and its first frame alone when it is animated.
/// Each file's type is read back from its bytes when served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcessedProfileImage {
    pub(crate) picture: ProcessedImage,
    pub(crate) still: Option<Vec<u8>>,
}

/// A decoded upload: its frames with their duration in milliseconds, already fitted.
#[derive(Debug)]
struct DecodedPicture {
    frames: Vec<(RgbaImage, u32)>,
    colour: ColourSpace,
    /// A JPEG: lossless WebP is never smaller for a photo, so it is not tried.
    is_photo: bool,
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAXIMUM_SIDE_PIXELS);
    limits.max_image_height = Some(MAXIMUM_SIDE_PIXELS);
    // RGBA, 4 bytes per pixel, for one frame at a time.
    limits.max_alloc = Some(MAXIMUM_TOTAL_PIXELS * 4);
    limits
}

/// Refuses a picture whose header announces more than the limits, before decoding it.
fn check_dimensions(width: u32, height: u32) -> Result<(), ImageProcessingError> {
    if width > MAXIMUM_SIDE_PIXELS
        || height > MAXIMUM_SIDE_PIXELS
        || u64::from(width) * u64::from(height) > MAXIMUM_TOTAL_PIXELS
    {
        return Err(ImageProcessingError::TooLarge);
    }
    Ok(())
}

fn header_dimensions(
    bytes: &[u8],
    format: ImageFormat,
) -> Result<(u32, u32), ImageProcessingError> {
    let mut header_limits = limits();
    // Headers can hold compressed metadata (PNG iCCP) inflated while reading them.
    header_limits.max_alloc = Some(HEADER_READ_MAXIMUM_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(header_limits);
    reader.into_dimensions().map_err(|error| match error {
        image::ImageError::Limits(_) => ImageProcessingError::TooLarge,
        _ => ImageProcessingError::Unsupported,
    })
}

/// The frames of an animation, each through `shrink` as soon as it is decoded, within the frame
/// and total pixel limits.
fn collect_frames(
    frames: image::Frames<'_>,
    (width, height): (u32, u32),
    shrink: impl Fn(RgbaImage) -> RgbaImage,
) -> Result<Vec<(RgbaImage, u32)>, ImageProcessingError> {
    check_dimensions(width, height)?;
    let mut collected = Vec::new();
    let mut total_pixels = 0u64;
    for frame in frames {
        let frame = match frame {
            Ok(frame) => frame,
            Err(image::ImageError::Limits(_)) => return Err(ImageProcessingError::TooLarge),
            // A truncated file keeps the frames read so far, as browsers show it.
            Err(_) if !collected.is_empty() => break,
            Err(_) => return Err(ImageProcessingError::Unsupported),
        };
        total_pixels += u64::from(width) * u64::from(height);
        if collected.len() >= MAXIMUM_FRAMES || total_pixels > MAXIMUM_TOTAL_PIXELS {
            return Err(ImageProcessingError::TooLarge);
        }
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = numerator.checked_div(denominator).unwrap_or(0).max(20);
        collected.push((shrink(frame.into_buffer()), delay));
    }
    if collected.is_empty() {
        return Err(ImageProcessingError::Unsupported);
    }
    Ok(collected)
}

/// Reads only the frame headers of a GIF and refuses it when a frame lies outside the canvas or
/// the frames' own areas add up past the limits: each frame is decompressed at its own size,
/// whatever the canvas, so counting canvas pixels alone would not bound the work.
fn check_gif_frames(bytes: &[u8]) -> Result<(), ImageProcessingError> {
    let mut options = gif::DecodeOptions::new();
    options.skip_frame_decoding(true);
    let mut decoder = options
        .read_info(Cursor::new(bytes))
        .map_err(|_| ImageProcessingError::Unsupported)?;
    let (canvas_width, canvas_height) = (u32::from(decoder.width()), u32::from(decoder.height()));
    check_dimensions(canvas_width, canvas_height)?;
    let mut frame_count = 0usize;
    let mut total_pixels = 0u64;
    loop {
        let frame = match decoder.next_frame_info() {
            Ok(Some(frame)) => frame,
            Ok(None) => return Ok(()),
            // Truncated: the frames before the cut are what will be decoded.
            Err(_) if frame_count > 0 => return Ok(()),
            Err(_) => return Err(ImageProcessingError::Unsupported),
        };
        let is_inside_canvas = u32::from(frame.left) + u32::from(frame.width) <= canvas_width
            && u32::from(frame.top) + u32::from(frame.height) <= canvas_height;
        frame_count += 1;
        total_pixels += u64::from(frame.width) * u64::from(frame.height);
        if !is_inside_canvas || frame_count > MAXIMUM_FRAMES || total_pixels > MAXIMUM_TOTAL_PIXELS
        {
            return Err(ImageProcessingError::TooLarge);
        }
    }
}

/// The frames of a picture with their duration in milliseconds; one frame for still pictures,
/// turned the way its EXIF orientation says. Each frame goes through `shrink` as soon as it is
/// decoded, so only small frames are kept. Animated PNG (APNG) is read as its first frame.
fn decode_picture(
    bytes: &[u8],
    shrink: impl Fn(RgbaImage) -> RgbaImage,
) -> Result<DecodedPicture, ImageProcessingError> {
    let format = image::guess_format(bytes).map_err(|_| ImageProcessingError::Unsupported)?;
    let animation = |frames| DecodedPicture {
        frames,
        // GIF has no colour profile; an animated WebP's is not read (see below).
        colour: ColourSpace::Srgb,
        is_photo: false,
    };
    match format {
        ImageFormat::Gif => {
            check_gif_frames(bytes)?;
            let mut decoder = GifDecoder::new(Cursor::new(bytes))
                .map_err(|_| ImageProcessingError::Unsupported)?;
            decoder
                .set_limits(limits())
                .map_err(|_| ImageProcessingError::TooLarge)?;
            let dimensions = decoder.dimensions();
            return Ok(animation(collect_frames(
                decoder.into_frames(),
                dimensions,
                shrink,
            )?));
        }
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(Cursor::new(bytes))
                .map_err(|_| ImageProcessingError::Unsupported)?;
            if decoder.has_animation() {
                decoder
                    .set_limits(limits())
                    .map_err(|_| ImageProcessingError::TooLarge)?;
                let dimensions = decoder.dimensions();
                return Ok(animation(collect_frames(
                    decoder.into_frames(),
                    dimensions,
                    shrink,
                )?));
            }
        }
        ImageFormat::Png | ImageFormat::Jpeg => {}
        _ => return Err(ImageProcessingError::Unsupported),
    }
    let (width, height) = header_dimensions(bytes, format)?;
    check_dimensions(width, height)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits());
    let mut decoder = reader
        .into_decoder()
        .map_err(|error| as_processing_error(&error))?;
    // Only where reading metadata is bounded: the WebP decoder sizes its EXIF and ICC buffers from
    // the declared chunk length, without any limit.
    // shortcut: a still WebP's colour profile is ignored; read it once image-webp bounds it.
    let (orientation, icc_profile) = if matches!(format, ImageFormat::Jpeg | ImageFormat::Png) {
        (
            decoder
                .orientation()
                .map_err(|error| as_processing_error(&error))?,
            decoder
                .icc_profile()
                .map_err(|error| as_processing_error(&error))?,
        )
    } else {
        (Orientation::NoTransforms, None)
    };
    let mut image =
        DynamicImage::from_decoder(decoder).map_err(|error| as_processing_error(&error))?;
    image.apply_orientation(orientation);
    Ok(DecodedPicture {
        frames: vec![(shrink(image.into_rgba8()), 0)],
        colour: colour::colour_space_of(icc_profile.as_deref()),
        is_photo: format == ImageFormat::Jpeg,
    })
}

fn as_processing_error(error: &image::ImageError) -> ImageProcessingError {
    match error {
        image::ImageError::Limits(_) => ImageProcessingError::TooLarge,
        _ => ImageProcessingError::Unsupported,
    }
}

/// The middle band of a banner taller than its card, the only part the card shows.
fn shown_band_of_banner(image: RgbaImage) -> RgbaImage {
    let (width, height) = image.dimensions();
    let shown_height = u32::try_from(
        (u64::from(width) * u64::from(BANNER_HEIGHT_PIXELS))
            .div_ceil(u64::from(BANNER_WIDTH_PIXELS)),
    )
    .unwrap_or(u32::MAX);
    if height <= shown_height {
        return image;
    }
    image::imageops::crop_imm(&image, 0, (height - shown_height) / 2, width, shown_height)
        .to_image()
}

/// Down to the display size, never up; Lanczos keeps edges sharp.
fn fit(image: RgbaImage, kind: ProfileImageKind) -> RgbaImage {
    let (image, maximum_width, maximum_height) = match kind {
        ProfileImageKind::Avatar => (image, AVATAR_SIDE_PIXELS, AVATAR_SIDE_PIXELS),
        ProfileImageKind::Banner => (
            shown_band_of_banner(image),
            BANNER_WIDTH_PIXELS,
            BANNER_HEIGHT_PIXELS,
        ),
    };
    let (width, height) = image.dimensions();
    if width <= maximum_width && height <= maximum_height {
        return image;
    }
    DynamicImage::ImageRgba8(image)
        .resize(maximum_width, maximum_height, FilterType::Lanczos3)
        .into_rgba8()
}

/// The `crop` square, cut at the picture's edges, or the centred square when there is none or it
/// starts outside; then down to `maximum_side`, never up.
fn crop_square_and_shrink(
    image: &RgbaImage,
    crop: Option<SquareCrop>,
    maximum_side: u32,
) -> RgbaImage {
    let (width, height) = image.dimensions();
    let (x, y, size) = match crop {
        Some(crop) if crop.size > 0 && crop.x < width && crop.y < height => {
            let size = crop.size.min(width - crop.x).min(height - crop.y);
            (crop.x, crop.y, size)
        }
        _ => {
            let size = width.min(height);
            ((width - size) / 2, (height - size) / 2, size)
        }
    };
    let cropped = image::imageops::crop_imm(image, x, y, size, size).to_image();
    if size <= maximum_side {
        return cropped;
    }
    image::imageops::resize(&cropped, maximum_side, maximum_side, FilterType::Lanczos3)
}

/// An avatar or a banner: fitted to its display size and stored in the smallest form that looks
/// the same, animated when the source is, with its first frame as a still.
pub(crate) fn process_profile_image(
    bytes: &[u8],
    kind: ProfileImageKind,
) -> Result<ProcessedProfileImage, ImageProcessingError> {
    encode_with_still(&decode_picture(bytes, |frame| fit(frame, kind))?)
}

/// The still that must be stored: the smallest that passes, or the last resort.
fn required_still(
    frame: &RgbaImage,
    picture: &DecodedPicture,
    deadline: Instant,
) -> Result<Encoded, ImageProcessingError> {
    match smallest_still(frame, &picture.colour, !picture.is_photo, deadline)? {
        Some(encoded) => Ok(encoded),
        None => last_resort_still(frame, &picture.colour),
    }
}

/// The frames as one picture, and their first alone when there are several.
fn encode_with_still(
    picture: &DecodedPicture,
) -> Result<ProcessedProfileImage, ImageProcessingError> {
    let deadline = Instant::now() + UPLOAD_SEARCH_TIME_BUDGET;
    let Some((first, _)) = picture.frames.first() else {
        return Err(ImageProcessingError::Unsupported);
    };
    let still = required_still(first, picture, deadline)?;
    if picture.frames.len() < 2 {
        return Ok(ProcessedProfileImage {
            picture: ProcessedImage {
                bytes: still.bytes,
                content_type: still.content_type,
                is_animated: false,
            },
            still: None,
        });
    }
    let animation = smallest_animation(&picture.frames, deadline)?;
    Ok(ProcessedProfileImage {
        picture: ProcessedImage {
            bytes: animation.bytes,
            content_type: animation.content_type,
            is_animated: true,
        },
        still: Some(still.bytes),
    })
}

/// Whether processing may find several frames, from the header alone: only GIF and WebP are kept
/// animated (APNG is read as its first frame).
pub(crate) fn may_be_animated(bytes: &[u8]) -> bool {
    matches!(
        image::guess_format(bytes),
        Ok(ImageFormat::Gif | ImageFormat::WebP)
    )
}

/// A guild or channel icon or a custom emoji: the square kept, fitted to its display size and
/// stored in the smallest form that looks the same; a guild's icon and an emoji animated when the
/// source is, with its first frame as a still.
pub(crate) fn process_icon(
    bytes: &[u8],
    kind: IconKind,
    crop: Option<SquareCrop>,
) -> Result<ProcessedProfileImage, ImageProcessingError> {
    let side = match kind {
        IconKind::Guild => GUILD_ICON_SIDE_PIXELS,
        IconKind::Channel => CHANNEL_ICON_SIDE_PIXELS,
        IconKind::Emoji => EMOJI_SIDE_PIXELS,
    };
    let mut picture = decode_picture(bytes, |frame| crop_square_and_shrink(&frame, crop, side))?;
    if kind == IconKind::Channel {
        // shortcut: every frame is decoded and shrunk before the first is kept; stop decoding
        // after it if long GIFs as channel icons hit the processing time budget.
        picture.frames.truncate(1);
    }
    encode_with_still(&picture)
}

/// The content types `optimize_attachment` may make smaller.
pub(crate) const OPTIMIZABLE_CONTENT_TYPES: [&str; 4] =
    ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// An attachment's smallest form that looks the same (ADR-0033): EXIF orientation applied to the
/// pixels, no metadata left, animations kept animated; no encode starts past `deadline`. `None`
/// when nothing passed or nothing came out smaller than `bytes` (a JPEG already stripped of its
/// metadata by `strip_jpeg_metadata` at upload), for an animated PNG, for frames over
/// `MAXIMUM_COMPRESSED_PICTURE_PIXELS`, and when the stored form could not show the same colours.
pub(crate) fn optimize_attachment(
    bytes: &[u8],
    content_type: &str,
    deadline: Instant,
) -> Result<Option<ProcessedImage>, ImageProcessingError> {
    if !OPTIMIZABLE_CONTENT_TYPES.contains(&content_type) || is_animated_png(bytes)? {
        return Ok(None);
    }
    let picture = decode_picture(bytes, |frame| frame)?;
    let Some((first, _)) = picture.frames.first() else {
        return Err(ImageProcessingError::Unsupported);
    };
    if first.width().saturating_mul(first.height()) > MAXIMUM_COMPRESSED_PICTURE_PIXELS
        || !picture.colour.keeps_colours(first)
    {
        return Ok(None);
    }
    let is_animated = picture.frames.len() > 1;
    let encoded = if is_animated {
        Some(smallest_animation(&picture.frames, deadline)?)
    } else {
        smallest_still(first, &picture.colour, !picture.is_photo, deadline)?
    };
    Ok(encoded
        .filter(|encoded| encoded.bytes.len() < bytes.len())
        .map(|encoded| ProcessedImage {
            bytes: encoded.bytes,
            content_type: encoded.content_type,
            is_animated,
        }))
}

/// A smaller copy of an attachment for the message list (ADR-0033): fitted within
/// `PREVIEW_WIDTH_PIXELS` × `PREVIEW_HEIGHT_PIXELS`, stored in the smallest form that scores as
/// visually lossless against the fitted picture, animated when the source is; no encode starts
/// past `deadline`. `None` when the picture already fits, when nothing passes, for an animated
/// PNG, and when the stored form could not show the same colours.
pub(crate) fn attachment_preview(
    bytes: &[u8],
    content_type: &str,
    deadline: Instant,
) -> Result<Option<ProcessedImage>, ImageProcessingError> {
    if !OPTIMIZABLE_CONTENT_TYPES.contains(&content_type) || is_animated_png(bytes)? {
        return Ok(None);
    }
    let Some((width, height)) = displayed_dimensions(bytes) else {
        return Err(ImageProcessingError::Unsupported);
    };
    if width <= PREVIEW_WIDTH_PIXELS && height <= PREVIEW_HEIGHT_PIXELS {
        return Ok(None);
    }
    let picture = decode_picture(bytes, |frame| fitted_preview(&frame).unwrap_or(frame))?;
    let Some((first, _)) = picture.frames.first() else {
        return Err(ImageProcessingError::Unsupported);
    };
    if !picture.colour.keeps_colours(first) {
        return Ok(None);
    }
    let is_animated = picture.frames.len() > 1;
    let encoded = if is_animated {
        Some(smallest_animation(&picture.frames, deadline)?)
    } else {
        smallest_still(first, &picture.colour, !picture.is_photo, deadline)?
    };
    Ok(encoded.map(|encoded| ProcessedImage {
        bytes: encoded.bytes,
        content_type: encoded.content_type,
        is_animated,
    }))
}

/// The picture shrunk to fit the preview size, its ratio kept; `None` when it already fits.
fn fitted_preview(image: &RgbaImage) -> Option<RgbaImage> {
    let (width, height) = image.dimensions();
    if width <= PREVIEW_WIDTH_PIXELS && height <= PREVIEW_HEIGHT_PIXELS {
        return None;
    }
    let (width, height) = (u64::from(width), u64::from(height));
    let (maximum_width, maximum_height) = (
        u64::from(PREVIEW_WIDTH_PIXELS),
        u64::from(PREVIEW_HEIGHT_PIXELS),
    );
    // Rounded to the nearest pixel, at least one: both results stay within the preview size.
    let (fitted_width, fitted_height) = if width * maximum_height >= height * maximum_width {
        (maximum_width, (height * maximum_width + width / 2) / width)
    } else {
        (
            (width * maximum_height + height / 2) / height,
            maximum_height,
        )
    };
    let side = |value: u64| u32::try_from(value.max(1)).unwrap_or(1);
    Some(image::imageops::resize(
        image,
        side(fitted_width),
        side(fitted_height),
        FilterType::Lanczos3,
    ))
}

/// Whether a PNG is animated (APNG), from its header: decoding would keep only its first frame.
fn is_animated_png(bytes: &[u8]) -> Result<bool, ImageProcessingError> {
    if image::guess_format(bytes).ok() != Some(ImageFormat::Png) {
        return Ok(false);
    }
    let mut header_limits = limits();
    header_limits.max_alloc = Some(HEADER_READ_MAXIMUM_BYTES);
    let decoder = PngDecoder::with_limits(Cursor::new(bytes), header_limits)
        .map_err(|error| as_processing_error(&error))?;
    decoder
        .is_apng()
        .map_err(|error| as_processing_error(&error))
}
/// The size an attachment as stored shows at, from its header alone, so clients can hold its
/// place: turned by the EXIF orientation of a JPEG (`strip_jpeg_metadata` keeps it alone) or a
/// PNG. `optimize_attachment` turns the pixels the same way, so this stays true after it. `None`
/// for anything that is not a picture this server reads.
pub(crate) fn displayed_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let format = image::guess_format(bytes).ok()?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP
    ) {
        return None;
    }
    // No side limit: only the header is read, and huge photos still deserve their place held.
    let mut header_limits = Limits::default();
    header_limits.max_alloc = Some(HEADER_READ_MAXIMUM_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(header_limits);
    let mut decoder = reader.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    // Only where reading EXIF is bounded, as in `decode_picture`.
    let orientation = if matches!(format, ImageFormat::Jpeg | ImageFormat::Png) {
        decoder.orientation().ok()?
    } else {
        Orientation::NoTransforms
    };
    // EXIF orientations 5 to 8 turn the picture a quarter.
    let is_turned_a_quarter = matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    );
    Some(if is_turned_a_quarter {
        (height, width)
    } else {
        (width, height)
    })
}

const JPEG_START_OF_IMAGE: [u8; 2] = [0xFF, 0xD8];
const MARKER_APP0: u8 = 0xE0;
const MARKER_APP1: u8 = 0xE1;
const MARKER_APP2: u8 = 0xE2;
const MARKER_APP14: u8 = 0xEE;
const MARKER_APP15: u8 = 0xEF;
const MARKER_COMMENT: u8 = 0xFE;
const MARKER_START_OF_SCAN: u8 = 0xDA;
const MARKER_END_OF_IMAGE: u8 = 0xD9;
const MARKER_TEMPORARY: u8 = 0x01;
const ICC_PROFILE_HEADER: &[u8] = b"ICC_PROFILE\0";
const EXIF_HEADER: &[u8] = b"Exif\0\0";
const ORIENTATION_TAG: u16 = 0x0112;

/// Markers with no length that may appear anywhere: restart markers and TEM.
fn is_standalone_marker(marker: u8) -> bool {
    marker == MARKER_TEMPORARY || (0xD0..=0xD7).contains(&marker)
}

/// Metadata segments to drop: every `APPn` but JFIF (APP0), ICC profiles (APP2) and Adobe colour
/// markers (APP14), which colours depend on; and comments. EXIF, XMP, IPTC, MPF and vendor
/// segments hold GPS positions, camera serials and other pictures (depth maps, thumbnails).
fn is_dropped_metadata(marker: u8, payload: &[u8]) -> bool {
    match marker {
        MARKER_APP0 | MARKER_APP14 => false,
        MARKER_APP2 => !payload.starts_with(ICC_PROFILE_HEADER),
        MARKER_APP1..=MARKER_APP15 | MARKER_COMMENT => true,
        _ => false,
    }
}

/// Removes metadata without touching the compressed image, and anything after its end (phones
/// append further pictures there). The EXIF orientation is written back alone, so the photo still
/// shows the right way up. A scan cut short (a truncated file) keeps its data, which holds no
/// marker; any other damage after the first scan ends the file there, and damage before it is
/// refused, since the metadata could not be removed.
pub(crate) fn strip_jpeg_metadata(bytes: &[u8]) -> Result<Vec<u8>, ImageProcessingError> {
    if !bytes.starts_with(&JPEG_START_OF_IMAGE) {
        return Err(ImageProcessingError::Unsupported);
    }
    let mut stripping = JpegStripping {
        output: Vec::with_capacity(bytes.len()),
        position: 2,
        has_passed_first_scan: false,
    };
    stripping.output.extend_from_slice(&JPEG_START_OF_IMAGE);
    match stripping.copy_without_metadata(bytes) {
        Ok(()) => Ok(stripping.output),
        Err(_) if stripping.has_passed_first_scan => Ok(stripping.output),
        Err(error) => Err(error),
    }
}

struct JpegStripping {
    output: Vec<u8>,
    position: usize,
    has_passed_first_scan: bool,
}

impl JpegStripping {
    fn copy_without_metadata(&mut self, bytes: &[u8]) -> Result<(), ImageProcessingError> {
        let mut orientation = None;
        loop {
            let position = self.position;
            if bytes.get(position) != Some(&0xFF) {
                return Err(ImageProcessingError::Unsupported);
            }
            let marker = *bytes
                .get(position + 1)
                .ok_or(ImageProcessingError::Unsupported)?;
            if marker == 0xFF {
                // Fill byte before a marker.
                self.position += 1;
                continue;
            }
            if marker == MARKER_END_OF_IMAGE {
                self.output.extend_from_slice(&[0xFF, MARKER_END_OF_IMAGE]);
                self.position += 2;
                return Ok(());
            }
            if is_standalone_marker(marker) {
                self.output.extend_from_slice(&[0xFF, marker]);
                self.position += 2;
                continue;
            }
            let length_bytes = bytes
                .get(position + 2..position + 4)
                .ok_or(ImageProcessingError::Unsupported)?;
            let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
            if length < 2 {
                return Err(ImageProcessingError::Unsupported);
            }
            let segment = bytes
                .get(position..position + 2 + length)
                .ok_or(ImageProcessingError::Unsupported)?;
            let payload = &segment[4..];
            if marker == MARKER_APP1 {
                orientation = orientation.or_else(|| exif_orientation(payload));
            }
            if marker == MARKER_START_OF_SCAN && !self.has_passed_first_scan {
                if let Some(orientation) = orientation.filter(|value| *value != 1) {
                    self.output
                        .extend_from_slice(&orientation_segment(orientation));
                }
                self.has_passed_first_scan = true;
            }
            if !is_dropped_metadata(marker, payload) {
                self.output.extend_from_slice(segment);
            }
            self.position += 2 + length;
            if marker == MARKER_START_OF_SCAN {
                let Ok(scan_end) = end_of_entropy_coded_data(bytes, self.position) else {
                    // Cut inside the compressed data: it holds no marker, so no metadata.
                    self.output.extend_from_slice(&bytes[self.position..]);
                    return Ok(());
                };
                self.output
                    .extend_from_slice(&bytes[self.position..scan_end]);
                self.position = scan_end;
            }
        }
    }
}

/// Where the compressed data after a scan header stops: the first `0xFF` that starts a real
/// marker (`0xFF 0x00` is an escaped byte; restart markers belong to the data).
fn end_of_entropy_coded_data(bytes: &[u8], start: usize) -> Result<usize, ImageProcessingError> {
    let mut position = start;
    loop {
        let offset = bytes
            .get(position..)
            .and_then(|rest| rest.iter().position(|byte| *byte == 0xFF))
            .ok_or(ImageProcessingError::Unsupported)?;
        position += offset;
        match bytes.get(position + 1) {
            Some(0x00 | 0xD0..=0xD7) => position += 2,
            Some(_) => return Ok(position),
            None => return Err(ImageProcessingError::Unsupported),
        }
    }
}

/// The orientation of an APP1 payload, if it is EXIF and holds one in IFD0.
fn exif_orientation(payload: &[u8]) -> Option<u16> {
    let tiff = payload.strip_prefix(EXIF_HEADER)?;
    let is_little_endian = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let read_u16 = |offset: usize| -> Option<u16> {
        let pair = [*tiff.get(offset)?, *tiff.get(offset + 1)?];
        Some(if is_little_endian {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        })
    };
    let read_u32 = |offset: usize| -> Option<u32> {
        let quad = [
            *tiff.get(offset)?,
            *tiff.get(offset + 1)?,
            *tiff.get(offset + 2)?,
            *tiff.get(offset + 3)?,
        ];
        Some(if is_little_endian {
            u32::from_le_bytes(quad)
        } else {
            u32::from_be_bytes(quad)
        })
    };
    let ifd = usize::try_from(read_u32(4)?).ok()?;
    let entries = usize::from(read_u16(ifd)?);
    (0..entries).find_map(|index| {
        let entry = ifd + 2 + index * 12;
        (read_u16(entry)? == ORIENTATION_TAG)
            .then(|| read_u16(entry + 8))
            .flatten()
            .filter(|value| (1..=8).contains(value))
    })
}

/// A minimal APP1 EXIF segment holding only the orientation.
fn orientation_segment(orientation: u16) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(EXIF_HEADER);
    payload.extend_from_slice(b"II");
    payload.extend_from_slice(&42u16.to_le_bytes());
    payload.extend_from_slice(&8u32.to_le_bytes());
    payload.extend_from_slice(&1u16.to_le_bytes());
    payload.extend_from_slice(&ORIENTATION_TAG.to_le_bytes());
    payload.extend_from_slice(&3u16.to_le_bytes()); // SHORT
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&orientation.to_le_bytes());
    payload.extend_from_slice(&[0, 0]);
    payload.extend_from_slice(&0u32.to_le_bytes());
    let mut segment = vec![0xFF, MARKER_APP1];
    // A few dozen bytes: always fits the 16-bit length.
    let length = u16::try_from(payload.len() + 2).unwrap_or(u16::MAX);
    segment.extend_from_slice(&length.to_be_bytes());
    segment.extend_from_slice(&payload);
    segment
}

#[cfg(test)]
pub(crate) mod tests {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame, ImageEncoder as _, Rgba};

    use super::*;

    /// Far enough that no test search is cut short.
    fn no_deadline() -> Instant {
        Instant::now() + Duration::from_secs(3_600)
    }

    /// A stored picture's size: AVIF from its `ispe` box (no AV1 decoder here), others decoded.
    pub(crate) fn stored_dimensions(bytes: &[u8]) -> (u32, u32) {
        if bytes.get(4..12) == Some(b"ftypavif") {
            let at = bytes
                .windows(4)
                .position(|window| window == b"ispe")
                .unwrap();
            let read = |offset: usize| {
                u32::from_be_bytes(bytes[at + offset..at + offset + 4].try_into().unwrap())
            };
            // Box type, then version and flags, then width and height.
            return (read(8), read(12));
        }
        let decoded = image::load_from_memory(bytes).unwrap();
        (decoded.width(), decoded.height())
    }

    /// The colour primaries an AVIF announces in its `nclx` box.
    fn avif_primaries(bytes: &[u8]) -> u16 {
        let at = bytes
            .windows(4)
            .position(|window| window == b"nclx")
            .unwrap();
        u16::from_be_bytes([bytes[at + 4], bytes[at + 5]])
    }

    /// A photo-like picture: smooth gradients under fine, seeded noise.
    pub(crate) fn photo(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            let noise =
                (x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503)).rotate_left(7) % 24;
            let shade = |value: u32| u8::try_from(value.min(255)).unwrap();
            Rgba([
                shade(x * 200 / width + noise),
                shade(y * 200 / height + noise),
                shade((x + y) * 100 / (width + height) + 60 + noise),
                255,
            ])
        })
    }

    pub(crate) fn encoded(
        image: &RgbaImage,
        format: ImageFormat,
        icc_profile: Option<Vec<u8>>,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        let rgb = DynamicImage::ImageRgba8(image.clone()).into_rgb8();
        let (width, height) = image.dimensions();
        if format == ImageFormat::Jpeg {
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95);
            if let Some(profile) = icc_profile {
                encoder.set_icc_profile(profile).unwrap();
            }
            encoder
                .write_image(&rgb, width, height, image::ExtendedColorType::Rgb8)
                .unwrap();
        } else {
            let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
            if let Some(profile) = icc_profile {
                encoder.set_icc_profile(profile).unwrap();
            }
            encoder
                .write_image(
                    image.as_raw(),
                    width,
                    height,
                    image::ExtendedColorType::Rgba8,
                )
                .unwrap();
        }
        bytes
    }

    pub(crate) fn animated_gif(width: u32, height: u32, frames: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder.set_repeat(Repeat::Infinite).unwrap();
            for index in 0..frames {
                let image = RgbaImage::from_fn(width, height, |x, y| {
                    let shade = u8::try_from((x + y + u32::from(index) * 8) % 256).unwrap();
                    Rgba([shade, 255 - shade, index.wrapping_mul(30), 255])
                });
                encoder
                    .encode_frame(Frame::from_parts(
                        image,
                        0,
                        0,
                        Delay::from_numer_denom_ms(80, 1),
                    ))
                    .unwrap();
            }
        }
        bytes
    }

    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        let image = RgbaImage::from_fn(width, height, |x, _| {
            Rgba([u8::try_from(x % 256).unwrap(), 40, 90, 255])
        });
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(
                image.as_raw(),
                width,
                height,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn an_animated_gif_avatar_stays_animated_fits_256_and_shrinks() {
        let gif = animated_gif(600, 600, 6);
        let avatar = process_profile_image(&gif, ProfileImageKind::Avatar).unwrap();
        assert!(avatar.picture.is_animated);
        assert_eq!(avatar.picture.content_type, "image/webp");
        assert!(
            avatar.picture.bytes.len() < gif.len(),
            "{} < {}",
            avatar.picture.bytes.len(),
            gif.len()
        );
        let decoder = webp_animation::Decoder::new(&avatar.picture.bytes).unwrap();
        assert_eq!(decoder.dimensions(), (256, 256));
        assert_eq!(decoder.into_iter().count(), 6);
        assert_eq!(stored_dimensions(&avatar.still.unwrap()), (256, 256));
    }

    #[test]
    fn icons_keep_the_centred_square_unless_told_otherwise() {
        // Columns are shaded by x, so the kept square shows in its first pixel.
        let wide = RgbaImage::from_fn(300, 100, |x, _| {
            Rgba([u8::try_from(x % 256).unwrap(), 40, 90, 255])
        });
        let centred = crop_square_and_shrink(&wide, None, 256);
        assert_eq!(centred.dimensions(), (100, 100), "never scaled up");
        assert_eq!(centred.get_pixel(0, 0)[0], 100);

        let chosen = Some(SquareCrop {
            x: 20,
            y: 0,
            size: 100,
        });
        assert_eq!(
            crop_square_and_shrink(&wide, chosen, 256).get_pixel(0, 0)[0],
            20
        );

        let past_the_edge = Some(SquareCrop {
            x: 250,
            y: 10,
            size: 100,
        });
        assert_eq!(
            crop_square_and_shrink(&wide, past_the_edge, 256).dimensions(),
            (50, 50)
        );

        for outside in [
            SquareCrop {
                x: 300,
                y: 0,
                size: 10,
            },
            SquareCrop {
                x: 0,
                y: 0,
                size: 0,
            },
        ] {
            assert_eq!(crop_square_and_shrink(&wide, Some(outside), 256), centred);
        }
    }

    #[test]
    fn guild_icons_stay_animated_and_channel_icons_keep_one_frame_both_small() {
        let gif = animated_gif(600, 400, 4);
        let guild = process_icon(&gif, IconKind::Guild, None).unwrap();
        assert!(guild.picture.is_animated);
        assert!(guild.still.is_some());
        let decoder = webp_animation::Decoder::new(&guild.picture.bytes).unwrap();
        assert_eq!(decoder.dimensions(), (256, 256));
        assert_eq!(decoder.into_iter().count(), 4);

        let channel = process_icon(&gif, IconKind::Channel, None).unwrap();
        assert!(!channel.picture.is_animated);
        assert!(channel.still.is_none());
        assert_eq!(stored_dimensions(&channel.picture.bytes), (96, 96));

        let emoji = process_icon(&gif, IconKind::Emoji, None).unwrap();
        assert!(emoji.picture.is_animated);
        assert!(emoji.still.is_some());
        let decoder = webp_animation::Decoder::new(&emoji.picture.bytes).unwrap();
        assert_eq!(decoder.dimensions(), (128, 128));
    }

    #[test]
    fn a_flat_graphic_avatar_is_kept_pixel_for_pixel_as_lossless_webp() {
        // One-pixel stripes: lossless is far smaller than any lossy encoding that keeps them.
        let stripes = RgbaImage::from_fn(64, 40, |x, _| {
            if x % 2 == 0 {
                Rgba([250, 250, 250, 255])
            } else {
                Rgba([20, 30, 200, 255])
            }
        });
        let avatar = process_profile_image(
            &encoded(&stripes, ImageFormat::Png, None),
            ProfileImageKind::Avatar,
        )
        .unwrap();
        assert!(!avatar.picture.is_animated);
        assert_eq!(avatar.picture.content_type, "image/webp");
        let decoded = image::load_from_memory(&avatar.picture.bytes)
            .unwrap()
            .to_rgba8();
        assert_eq!(decoded, stripes, "lossless");
    }

    #[test]
    fn a_photo_is_stored_as_avif_at_a_searched_quality_that_keeps_the_score() {
        let source = photo(96, 96);
        let found = smallest_still(&source, &ColourSpace::Srgb, true, no_deadline())
            .unwrap()
            .unwrap();
        assert_eq!(found.content_type, "image/avif");
        // The encoder is deterministic: find the quality the search kept, and score it again.
        let quality = (60..=95)
            .find(|quality| {
                avif::encode(&source, *quality, avif::Primaries::Srgb)
                    .unwrap()
                    .bytes
                    == found.bytes
            })
            .expect("a searched quality");
        let decoded = avif::encode(&source, quality, avif::Primaries::Srgb)
            .unwrap()
            .decoded;
        let score = quality::worst_score(&[(&source, &decoded)], avif::Primaries::Srgb).unwrap();
        assert!(score >= quality::MINIMUM_SCORE, "{score} at {quality}");
        assert!(quality < 95, "lossy enough to save space: {quality}");
        let lossless = encoding::lossless_webp(&[(source, 0)]).unwrap();
        assert!(found.bytes.len() < lossless.len());
    }

    #[test]
    fn a_transparent_emoji_keeps_its_transparency() {
        // A soft disc on a transparent square.
        let disc = RgbaImage::from_fn(160, 160, |x, y| {
            let distance = (x.abs_diff(80).pow(2) + y.abs_diff(80).pow(2)).isqrt();
            let alpha = u8::try_from(70u32.saturating_sub(distance).min(10) * 25).unwrap();
            Rgba([255, u8::try_from(x).unwrap(), 40, alpha])
        });
        let emoji = process_icon(
            &encoded(&disc, ImageFormat::Png, None),
            IconKind::Emoji,
            None,
        )
        .unwrap();
        assert_eq!(stored_dimensions(&emoji.picture.bytes), (128, 128));
        if emoji.picture.content_type == "image/avif" {
            // An alpha plane is an auxiliary image of the AVIF.
            let alpha_urn = b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha";
            assert!(
                emoji
                    .picture
                    .bytes
                    .windows(alpha_urn.len())
                    .any(|window| window == alpha_urn)
            );
        } else {
            let decoded = image::load_from_memory(&emoji.picture.bytes)
                .unwrap()
                .to_rgba8();
            assert_eq!(decoded.get_pixel(0, 0)[3], 0);
        }
    }

    #[test]
    fn banners_keep_their_ratio_and_attachments_shrink() {
        let banner = process_profile_image(&png(3_840, 1_080), ProfileImageKind::Banner).unwrap();
        assert_eq!(stored_dimensions(&banner.picture.bytes), (960, 270));

        let source = png(300, 300);
        let optimized = optimize_attachment(&source, "image/png", no_deadline())
            .unwrap()
            .unwrap();
        assert!(optimized.bytes.len() < source.len());
        assert_eq!(stored_dimensions(&optimized.bytes), (300, 300));
        assert!(
            optimize_attachment(b"just text", "application/octet-stream", no_deadline())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn attachment_previews_fit_the_message_list_at_twice_its_size() {
        let wide = encoded(&photo(1_600, 120), ImageFormat::Jpeg, None);
        let preview = attachment_preview(&wide, "image/jpeg", no_deadline())
            .unwrap()
            .unwrap();
        assert_eq!(stored_dimensions(&preview.bytes), (800, 60));
        assert!(preview.bytes.len() < wide.len());
        assert!(!preview.is_animated);

        let tall = attachment_preview(&png(100, 2_000), "image/png", no_deadline())
            .unwrap()
            .unwrap();
        assert_eq!(stored_dimensions(&tall.bytes), (30, 600));

        assert_eq!(
            attachment_preview(&png(800, 600), "image/png", no_deadline()),
            Ok(None),
            "already fits: the attachment is its own preview"
        );
        assert_eq!(
            attachment_preview(b"just text", "application/octet-stream", no_deadline()),
            Ok(None)
        );
    }

    #[test]
    fn an_animated_attachment_gets_an_animated_preview() {
        let gif = animated_gif(1_000, 100, 3);
        let preview = attachment_preview(&gif, "image/gif", no_deadline())
            .unwrap()
            .unwrap();
        assert_eq!(preview.content_type, "image/webp");
        assert!(preview.is_animated);
        let decoder = WebPDecoder::new(Cursor::new(&preview.bytes)).unwrap();
        assert_eq!(decoder.dimensions(), (800, 80));
        assert_eq!(decoder.into_frames().count(), 3);
    }

    #[test]
    fn an_attachment_whose_colours_would_change_gets_no_preview() {
        let print_export = encoded(
            &photo(1_200, 90),
            ImageFormat::Jpeg,
            Some(colour::tests::cmyk_icc_bytes()),
        );
        assert_eq!(
            attachment_preview(&print_export, "image/jpeg", no_deadline()),
            Ok(None)
        );
    }

    #[test]
    fn an_animated_gif_attachment_stays_animated_and_keeps_the_score() {
        let gif = animated_gif(120, 80, 4);
        let as_webp = optimize_attachment(&gif, "image/gif", no_deadline())
            .unwrap()
            .unwrap();
        assert_eq!(as_webp.content_type, "image/webp");
        assert!(as_webp.is_animated);
        let source = decode_picture(&gif, |frame| frame).unwrap().frames;
        let decoder = WebPDecoder::new(Cursor::new(&as_webp.bytes)).unwrap();
        let stored: Vec<RgbaImage> = decoder
            .into_frames()
            .map(|frame| frame.unwrap().into_buffer())
            .collect();
        assert_eq!(stored.len(), 4);
        let pairs: Vec<(&RgbaImage, &RgbaImage)> =
            source.iter().map(|(frame, _)| frame).zip(&stored).collect();
        let score = quality::worst_score(&pairs, avif::Primaries::Srgb).unwrap();
        assert!(score >= quality::MINIMUM_SCORE, "{score}");
    }

    #[test]
    fn an_animated_png_attachment_is_left_alone() {
        let mut apng = png(16, 16);
        // An `acTL` chunk before the image data marks an animated PNG (one frame, played once).
        let insert_at = apng
            .windows(4)
            .position(|window| window == b"IDAT")
            .unwrap()
            - 4;
        let mut animation_control = 1u32.to_be_bytes().to_vec();
        animation_control.extend_from_slice(&0u32.to_be_bytes());
        let chunk = png_chunk(*b"acTL", &animation_control);
        apng.splice(insert_at..insert_at, chunk);
        assert_eq!(
            optimize_attachment(&apng, "image/png", no_deadline()),
            Ok(None)
        );
    }

    #[test]
    fn a_turned_phone_photo_attachment_is_stored_upright_without_its_metadata() {
        let jpeg = encoded(&photo(128, 64), ImageFormat::Jpeg, None);
        let mut turned = jpeg[..2].to_vec();
        turned.extend_from_slice(&orientation_segment(6));
        turned.extend_from_slice(&jpeg[2..]);
        let stripped = strip_jpeg_metadata(&turned).unwrap();
        assert_eq!(displayed_dimensions(&stripped), Some((64, 128)));

        let optimized = optimize_attachment(&stripped, "image/jpeg", no_deadline())
            .unwrap()
            .expect("AVIF beats a quality 95 JPEG");
        assert_eq!(optimized.content_type, "image/avif");
        assert_eq!(stored_dimensions(&optimized.bytes), (64, 128));
        assert!(!optimized.bytes.windows(4).any(|window| window == b"Exif"));
    }

    #[test]
    fn wide_gamut_pictures_are_stored_in_display_p3_and_never_as_untagged_webp() {
        use moxcms::ColorProfile;

        let display_p3 = colour::tests::icc_bytes(&ColorProfile::new_display_p3());
        let iphone_photo = encoded(&photo(128, 96), ImageFormat::Jpeg, Some(display_p3));
        let optimized = optimize_attachment(&iphone_photo, "image/jpeg", no_deadline())
            .unwrap()
            .expect("AVIF beats a quality 95 JPEG");
        assert_eq!(optimized.content_type, "image/avif");
        assert_eq!(avif_primaries(&optimized.bytes), 12, "Display P3");

        // Flat stripes would be lossless WebP in sRGB, which cannot carry Adobe RGB colours.
        let stripes = RgbaImage::from_fn(64, 64, |x, _| {
            if x % 2 == 0 {
                Rgba([0, 200, 60, 255])
            } else {
                Rgba([240, 240, 240, 255])
            }
        });
        let adobe_rgb = colour::tests::icc_bytes(&ColorProfile::new_adobe_rgb());
        let avatar = process_profile_image(
            &encoded(&stripes, ImageFormat::Png, Some(adobe_rgb)),
            ProfileImageKind::Avatar,
        )
        .unwrap();
        assert_eq!(avatar.picture.content_type, "image/avif");
        assert_eq!(
            avif_primaries(&avatar.picture.bytes),
            12,
            "converted to Display P3"
        );

        let srgb = colour::tests::icc_bytes(&ColorProfile::new_srgb());
        let tagged_srgb = process_profile_image(
            &encoded(&stripes, ImageFormat::Png, Some(srgb)),
            ProfileImageKind::Avatar,
        )
        .unwrap();
        assert_eq!(
            tagged_srgb.picture.content_type, "image/webp",
            "sRGB stays lossless"
        );
    }

    #[test]
    fn an_attachment_whose_colours_would_change_is_kept_as_uploaded() {
        use moxcms::ColorProfile;

        // Browsers show a CMYK print export through its profile; the decoder ignores it.
        let print_export = encoded(
            &photo(64, 64),
            ImageFormat::Jpeg,
            Some(colour::tests::cmyk_icc_bytes()),
        );
        assert_eq!(
            optimize_attachment(&print_export, "image/jpeg", no_deadline()),
            Ok(None)
        );

        // The purest Adobe RGB green lies outside Display P3.
        let green = RgbaImage::from_pixel(64, 64, Rgba([0, 255, 0, 255]));
        let adobe_rgb = colour::tests::icc_bytes(&ColorProfile::new_adobe_rgb());
        assert_eq!(
            optimize_attachment(
                &encoded(&green, ImageFormat::Png, Some(adobe_rgb)),
                "image/png",
                no_deadline()
            ),
            Ok(None)
        );
    }

    #[test]
    fn a_display_p3_screenshot_in_srgb_colours_stays_lossless() {
        use moxcms::ColorProfile;

        // A window in greys, as macOS tags its screenshots with the display's profile.
        let window = RgbaImage::from_fn(200, 120, |x, y| {
            let grey = if y < 20 || x % 50 == 0 { 230 } else { 40 };
            Rgba([grey, grey, grey, 255])
        });
        let display_p3 = colour::tests::icc_bytes(&ColorProfile::new_display_p3());
        let screenshot = encoded(&window, ImageFormat::Png, Some(display_p3));
        let optimized = optimize_attachment(&screenshot, "image/png", no_deadline())
            .unwrap()
            .unwrap();
        assert_eq!(optimized.content_type, "image/webp");
        let decoded = image::load_from_memory(&optimized.bytes)
            .unwrap()
            .to_rgba8();
        assert!(
            decoded
                .as_raw()
                .iter()
                .zip(window.as_raw())
                .all(|(stored, source)| stored.abs_diff(*source) <= 2),
            "the same greys in sRGB"
        );
    }

    #[test]
    fn an_attachment_past_the_compression_size_is_kept_as_uploaded() {
        let side = 4_001;
        let huge = encoded(
            &RgbaImage::from_pixel(side, side, Rgba([30, 60, 90, 255])),
            ImageFormat::Png,
            None,
        );
        assert_eq!(
            optimize_attachment(&huge, "image/png", no_deadline()),
            Ok(None)
        );
    }

    #[test]
    fn a_tall_banner_keeps_the_middle_band_its_card_shows() {
        let tall = process_profile_image(&png(1_920, 8_000), ProfileImageKind::Banner).unwrap();
        assert_eq!(
            stored_dimensions(&tall.picture.bytes),
            (BANNER_WIDTH_PIXELS, BANNER_HEIGHT_PIXELS)
        );
        let narrow_and_tall =
            process_profile_image(&png(300, 2_000), ProfileImageKind::Banner).unwrap();
        assert_eq!(
            stored_dimensions(&narrow_and_tall.picture.bytes),
            (300, 120)
        );
    }

    #[test]
    fn jpeg_metadata_goes_but_orientation_and_the_image_stay() {
        // SOI, APP1 EXIF with orientation 6 and a GPS-looking comment, COM, SOS, data, EOI.
        let mut exif = EXIF_HEADER.to_vec();
        exif.extend_from_slice(
            b"MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0",
        );
        exif.extend_from_slice(b"GPS 48.85N 2.35E");
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, MARKER_APP1];
        jpeg.extend_from_slice(&u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
        jpeg.extend_from_slice(&exif);
        jpeg.extend_from_slice(&[0xFF, MARKER_COMMENT, 0x00, 0x07]);
        jpeg.extend_from_slice(b"hello");
        jpeg.extend_from_slice(&[
            0xFF,
            MARKER_START_OF_SCAN,
            0x00,
            0x02,
            0x12,
            0x34,
            0xFF,
            0xD9,
        ]);

        let stripped = strip_jpeg_metadata(&jpeg).unwrap();
        assert!(!stripped.windows(3).any(|window| window == b"GPS"));
        assert!(!stripped.windows(5).any(|window| window == b"hello"));
        assert!(stripped.ends_with(&[
            0xFF,
            MARKER_START_OF_SCAN,
            0x00,
            0x02,
            0x12,
            0x34,
            0xFF,
            0xD9
        ]));
        let kept = strip_jpeg_metadata(&stripped).unwrap();
        assert_eq!(kept, stripped, "the orientation survives a second pass");
        assert_eq!(exif_orientation(&orientation_segment(6)[4..]), Some(6));
        assert!(
            strip_jpeg_metadata(b"\xFF\xD8\xFF\xE1\xFF\xFF").is_err(),
            "truncated"
        );
    }

    #[test]
    fn a_truncated_gif_keeps_the_frames_before_the_cut() {
        let gif = animated_gif(32, 32, 3);
        // Cut inside the last frame's data: its trailer and end are gone.
        let truncated = &gif[..gif.len() - 40];
        let avatar = process_profile_image(truncated, ProfileImageKind::Avatar).unwrap();
        assert!(avatar.picture.is_animated, "two whole frames remain");
    }

    #[test]
    fn oversized_or_unreadable_pictures_are_left_alone() {
        assert_eq!(
            process_profile_image(b"<svg/>", ProfileImageKind::Avatar),
            Err(ImageProcessingError::Unsupported)
        );
        let huge = png(MAXIMUM_SIDE_PIXELS + 1, 1);
        assert_eq!(
            process_profile_image(&huge, ProfileImageKind::Avatar),
            Err(ImageProcessingError::TooLarge)
        );
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn png_chunk(kind: [u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut chunk = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
        let mut checked = kind.to_vec();
        checked.extend_from_slice(payload);
        chunk.extend_from_slice(&checked);
        chunk.extend_from_slice(&crc32(&checked).to_be_bytes());
        chunk
    }

    #[test]
    fn a_png_announcing_huge_dimensions_is_refused_before_any_allocation() {
        // 200 000 × 200 000 RGBA in about 70 bytes: inflating it would need 160 GB.
        let mut header = Vec::new();
        header.extend_from_slice(&200_000u32.to_be_bytes());
        header.extend_from_slice(&200_000u32.to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut bomb = b"\x89PNG\r\n\x1a\n".to_vec();
        bomb.extend_from_slice(&png_chunk(*b"IHDR", &header));
        bomb.extend_from_slice(&png_chunk(
            *b"IDAT",
            &[0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
        ));
        bomb.extend_from_slice(&png_chunk(*b"IEND", &[]));

        assert_eq!(
            optimize_attachment(&bomb, "image/png", no_deadline()),
            Err(ImageProcessingError::TooLarge)
        );
        assert_eq!(
            process_profile_image(&bomb, ProfileImageKind::Avatar),
            Err(ImageProcessingError::TooLarge)
        );
    }

    #[test]
    fn a_rotated_phone_photo_makes_an_upright_avatar() {
        let image = RgbaImage::from_fn(40, 20, |x, _| {
            Rgba([u8::try_from(x * 6).unwrap(), 0, 0, 255])
        });
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .write_image(
                &DynamicImage::ImageRgba8(image).into_rgb8(),
                40,
                20,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        // Orientation 6: the camera was turned a quarter; viewers rotate it upright.
        let mut rotated = jpeg[..2].to_vec();
        rotated.extend_from_slice(&orientation_segment(6));
        rotated.extend_from_slice(&jpeg[2..]);

        let avatar = process_profile_image(&rotated, ProfileImageKind::Avatar).unwrap();
        assert_eq!(stored_dimensions(&avatar.picture.bytes), (20, 40));
    }

    fn png_with_exif_orientation(width: u32, height: u32, orientation: u16) -> Vec<u8> {
        let image = RgbaImage::from_pixel(width, height, Rgba([10, 20, 30, 255]));
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new_with_quality(
            &mut bytes,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::NoFilter,
        );
        // The segment minus its marker, length and `Exif\0\0`: the TIFF payload eXIf holds.
        encoder
            .set_exif_metadata(orientation_segment(orientation)[10..].to_vec())
            .unwrap();
        encoder
            .write_image(
                image.as_raw(),
                width,
                height,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn a_png_keeps_showing_the_size_its_orientation_says_after_compression() {
        let has_exif = |bytes: &[u8]| bytes.windows(4).any(|window| window == b"eXIf");
        for orientation in 1..=8u16 {
            let original = png_with_exif_orientation(64, 32, orientation);
            let expected = if orientation >= 5 { (32, 64) } else { (64, 32) };
            assert_eq!(displayed_dimensions(&original), Some(expected));

            let optimized = optimize_attachment(&original, "image/png", no_deadline())
                .unwrap()
                .expect("a flat PNG compresses");
            // Turned in its pixels, and no EXIF left (nor any GPS position in it).
            assert_eq!(stored_dimensions(&optimized.bytes), expected);
            assert!(!has_exif(&optimized.bytes));
        }
    }

    #[test]
    fn displayed_dimensions_follow_the_stored_orientation() {
        let image = RgbaImage::from_pixel(40, 20, Rgba([10, 20, 30, 255]));
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .write_image(
                &DynamicImage::ImageRgba8(image).into_rgb8(),
                40,
                20,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        let turned = |orientation: u16| {
            let mut bytes = jpeg[..2].to_vec();
            bytes.extend_from_slice(&orientation_segment(orientation));
            bytes.extend_from_slice(&jpeg[2..]);
            strip_jpeg_metadata(&bytes).unwrap()
        };

        assert_eq!(displayed_dimensions(&jpeg), Some((40, 20)));
        for orientation in 1..=4 {
            assert_eq!(displayed_dimensions(&turned(orientation)), Some((40, 20)));
        }
        for orientation in 5..=8 {
            assert_eq!(displayed_dimensions(&turned(orientation)), Some((20, 40)));
        }
        assert_eq!(displayed_dimensions(&png(30, 10)), Some((30, 10)));
        assert_eq!(
            displayed_dimensions(&animated_gif(12, 34, 2)),
            Some((12, 34))
        );
        let as_webp = encoding::lossless_webp(
            &decode_picture(&animated_gif(12, 34, 2), |frame| frame)
                .unwrap()
                .frames,
        )
        .unwrap();
        assert_eq!(displayed_dimensions(&as_webp), Some((12, 34)));
        assert_eq!(displayed_dimensions(b"just text"), None);
    }

    #[test]
    fn an_animated_webp_avatar_stays_animated() {
        let animated_webp =
            process_profile_image(&animated_gif(64, 64, 3), ProfileImageKind::Avatar)
                .unwrap()
                .picture
                .bytes;
        let avatar = process_profile_image(&animated_webp, ProfileImageKind::Avatar).unwrap();
        assert!(avatar.picture.is_animated);
        assert!(avatar.still.is_some());
    }

    #[test]
    fn jpeg_stripping_drops_trailers_and_other_pictures_and_never_panics() {
        let scan = [
            0xFF,
            MARKER_START_OF_SCAN,
            0x00,
            0x02,
            0x12,
            0xFF,
            0x00,
            0xFF,
            0xD0,
            0x34,
        ];
        let mut jpeg = vec![0xFF, 0xD8];
        let icc = [
            &[0xFF, MARKER_APP2, 0x00, 0x11][..],
            ICC_PROFILE_HEADER,
            b"abc",
        ]
        .concat();
        let mpf = [&[0xFF, MARKER_APP2, 0x00, 0x08][..], b"MPF\0xy"].concat();
        jpeg.extend_from_slice(&icc);
        jpeg.extend_from_slice(&mpf);
        jpeg.extend_from_slice(&scan);
        jpeg.extend_from_slice(&[0xFF, MARKER_END_OF_IMAGE]);
        jpeg.extend_from_slice(b"\xFF\xD8 second picture with GPS");

        let stripped = strip_jpeg_metadata(&jpeg).unwrap();
        let expected = [&[0xFF, 0xD8][..], &icc, &scan, &[0xFF, MARKER_END_OF_IMAGE]].concat();
        assert_eq!(stripped, expected);
        assert_eq!(
            strip_jpeg_metadata(b"\xFF\xD8\xFF\xE1\x00\x00\xFF\xDA"),
            Err(ImageProcessingError::Unsupported),
            "a segment length under 2"
        );
    }

    #[test]
    fn a_still_webp_declaring_a_huge_exif_chunk_is_not_read_for_its_orientation() {
        // A real 1×1 lossless WebP, rewrapped as VP8X with an EXIF chunk announcing ~4 GiB that
        // is not there: reading it would allocate the announced size.
        let simple =
            encoding::lossless_webp(&[(RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255])), 0)])
                .unwrap();
        let lossless_chunk = &simple[12..];
        let mut extended_header = b"VP8X".to_vec();
        extended_header.extend_from_slice(&10u32.to_le_bytes());
        extended_header.extend_from_slice(&[0x08, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut body = b"WEBP".to_vec();
        body.extend_from_slice(&extended_header);
        body.extend_from_slice(lossless_chunk);
        body.extend_from_slice(b"EXIF");
        body.extend_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        let mut forged = b"RIFF".to_vec();
        forged.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        forged.extend_from_slice(&body);

        let avatar = process_profile_image(&forged, ProfileImageKind::Avatar).unwrap();
        assert!(!avatar.picture.is_animated);
    }

    #[test]
    fn gif_frames_outside_the_canvas_are_refused_before_decoding() {
        let mut gif = animated_gif(64, 64, 2);
        // Canvas shrunk to 1×1: the 64×64 frames now lie outside it.
        gif[6..10].copy_from_slice(&[1, 0, 1, 0]);
        assert_eq!(
            process_profile_image(&gif, ProfileImageKind::Avatar),
            Err(ImageProcessingError::TooLarge)
        );
        assert_eq!(
            optimize_attachment(&gif, "image/gif", no_deadline()),
            Err(ImageProcessingError::TooLarge)
        );
    }

    #[test]
    fn a_photo_cut_after_its_metadata_still_loses_it() {
        let mut exif = EXIF_HEADER.to_vec();
        exif.extend_from_slice(b"MM\0\x2a\0\0\0\x08\0\0GPS 48.85N 2.35E");
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, MARKER_APP1];
        jpeg.extend_from_slice(&u16::try_from(exif.len() + 2).unwrap().to_be_bytes());
        jpeg.extend_from_slice(&exif);
        // A scan whose data stops short: no end-of-image marker.
        jpeg.extend_from_slice(&[0xFF, MARKER_START_OF_SCAN, 0x00, 0x02, 0x12, 0x34, 0x56]);

        let stripped = strip_jpeg_metadata(&jpeg).unwrap();
        assert!(!stripped.windows(3).any(|window| window == b"GPS"));
        assert!(
            stripped.ends_with(&[0x12, 0x34, 0x56]),
            "the image data is kept"
        );
    }

    /// `cargo test --release -p mellow-server image_processing_timing -- --ignored --nocapture`
    #[test]
    #[ignore = "measurement, not a check"]
    #[allow(clippy::print_stdout)]
    fn image_processing_timing() {
        let gif = animated_gif(480, 480, 60);
        let started = Instant::now();
        let avatar = process_profile_image(&gif, ProfileImageKind::Avatar).unwrap();
        println!(
            "animated avatar: {} KB GIF -> {} KB WebP in {:?}",
            gif.len() / 1024,
            avatar.picture.bytes.len() / 1024,
            started.elapsed()
        );
        let source = encoded(&photo(1_600, 1_200), ImageFormat::Jpeg, None);
        let started = Instant::now();
        let optimized = optimize_attachment(&source, "image/jpeg", no_deadline()).unwrap();
        println!(
            "photo attachment: {} KB -> {:?} KB in {:?}",
            source.len() / 1024,
            optimized.map(|image| image.bytes.len() / 1024),
            started.elapsed()
        );
    }

    /// Sizes over a folder of pictures, as attachments, avatars and banners:
    /// `MELLOW_IMAGE_CORPUS=/path cargo test --release -p mellow-server image_corpus_sizes -- --ignored --nocapture`.
    /// With `MELLOW_IMAGE_CORPUS_OUTPUT=/path`, each sRGB still's AVIF at quality 80 and its
    /// reconstruction (PNG) are written there, to compare with another decoder.
    #[test]
    #[ignore = "measurement, not a check"]
    #[allow(clippy::print_stdout)]
    fn image_corpus_sizes() {
        let Ok(corpus) = std::env::var("MELLOW_IMAGE_CORPUS") else {
            return;
        };
        let output = std::env::var("MELLOW_IMAGE_CORPUS_OUTPUT").ok();
        let mut paths: Vec<_> = std::fs::read_dir(corpus)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        let (mut uploaded, mut stored, mut avatars, mut banners) = (0, 0, 0, 0);
        let mut shown = 0;
        for path in paths {
            let source = std::fs::read(&path).unwrap();
            let content_type = crate::domain::message_rules::sniff_content_type(&source);
            let bytes = if content_type == "image/jpeg" {
                strip_jpeg_metadata(&source).unwrap()
            } else {
                source
            };
            let started = Instant::now();
            let deadline = Instant::now() + Duration::from_secs(60);
            let optimized = optimize_attachment(&bytes, content_type, deadline).unwrap();
            let elapsed = started.elapsed();
            let avatar = process_profile_image(&bytes, ProfileImageKind::Avatar).unwrap();
            let banner = process_profile_image(&bytes, ProfileImageKind::Banner).unwrap();
            let kept = optimized
                .as_ref()
                .map_or(bytes.len(), |image| image.bytes.len());
            let preview_started = Instant::now();
            let preview = attachment_preview(
                &bytes,
                content_type,
                preview_started + Duration::from_secs(20),
            )
            .unwrap()
            .filter(|preview| preview.bytes.len() < kept);
            let preview_elapsed = preview_started.elapsed();
            // What the message list loads: the preview, or the stored file when there is none.
            let shown_bytes = preview.as_ref().map_or(kept, |preview| preview.bytes.len());
            println!(
                "{}\t{}\t{}\t{}\t{:?}\tpreview {} {:?}\tavatar {} {}\tbanner {} {}",
                path.file_name().unwrap().to_string_lossy(),
                bytes.len(),
                kept,
                optimized
                    .as_ref()
                    .map_or("kept", |image| image.content_type),
                elapsed,
                shown_bytes,
                preview_elapsed,
                avatar.picture.bytes.len(),
                avatar.picture.content_type,
                banner.picture.bytes.len(),
                banner.picture.content_type,
            );
            uploaded += bytes.len();
            stored += kept;
            shown += shown_bytes;
            avatars += avatar.picture.bytes.len();
            banners += banner.picture.bytes.len();
            let Some(output) = &output else {
                continue;
            };
            let picture = decode_picture(&bytes, |frame| frame).unwrap();
            if picture.frames.len() == 1 && matches!(picture.colour, ColourSpace::Srgb) {
                let encoded =
                    avif::encode(&picture.frames[0].0, 80, avif::Primaries::Srgb).unwrap();
                let name = path.file_stem().unwrap().to_string_lossy().into_owned();
                let folder = std::path::Path::new(output);
                std::fs::write(folder.join(format!("{name}.avif")), encoded.bytes).unwrap();
                let mut reconstruction = Vec::new();
                image::codecs::png::PngEncoder::new(&mut reconstruction)
                    .write_image(
                        encoded.decoded.as_raw(),
                        encoded.decoded.width(),
                        encoded.decoded.height(),
                        image::ExtendedColorType::Rgba8,
                    )
                    .unwrap();
                std::fs::write(
                    folder.join(format!("{name}.reconstruction.png")),
                    reconstruction,
                )
                .unwrap();
            }
        }
        println!(
            "attachments: {uploaded} B uploaded, {stored} B stored, {shown} B shown in the message list; avatars {avatars} B; banners {banners} B"
        );
    }
}
