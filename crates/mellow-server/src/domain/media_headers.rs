//! Bounded header walks over uploaded audio and video (ADR-0038): the size a video shows at, so
//! clients hold its place, and no metadata in MP4 and `QuickTime` files (phones write the place a
//! video was filmed there). Box and element headers only: no decoding, no allocation per box.

use crate::domain::message_rules::MessageRuleViolation;

/// Movie headers hold the sample tables: a few hundred KiB for a long phone video. Counts the
/// bytes walked in `moov`, not the metadata blanked there (an audiobook's cover art is megabytes).
const MAXIMUM_MOVIE_HEADER_BYTES: usize = 4 * 1024 * 1024;
/// Boxes walked through nest three deep (`moov`, `trak`, `mdia`); anything deeper is malformed.
const MAXIMUM_BOX_DEPTH: usize = 8;
/// `WebM` puts its tracks before its first cluster, in the first few KiB.
const MAXIMUM_WEBM_HEADER_BYTES: usize = 4 * 1024 * 1024;

/// The `uuid` box type Adobe tools and exiftool write XMP in (it may hold `exif:GPSLatitude`).
const XMP_BOX_UUID: [u8; 16] = [
    0xBE, 0x7A, 0xCF, 0xCB, 0x97, 0xA9, 0x42, 0xE8, 0x9C, 0x71, 0x99, 0x94, 0x91, 0xE3, 0xAF, 0xAC,
];

const BOX_HEADER_BYTES: usize = 8;
const LARGE_BOX_HEADER_BYTES: usize = 16;
/// 16.16 fixed point, as `tkhd` writes its matrix, width and height.
const FIXED_POINT_FRACTION_BITS: u32 = 16;

/// What one `trak` box says, gathered while walking it.
#[derive(Default)]
struct TrackFacts {
    is_video: bool,
    dimensions: Option<(u32, u32)>,
}

#[derive(Default)]
struct MovieWalk {
    movie_header_bytes_read: usize,
    track: Option<TrackFacts>,
    first_video_dimensions: Option<(u32, u32)>,
}

/// Blanks every `udta`, `meta` and XMP `uuid` box of an MP4 or `QuickTime` file (at the top, in
/// `moov`, in each `trak` and `mdia`): renamed `free` and zeroed in place, so players skip them
/// and their bytes are gone. Returns the first video track's size as shown, a quarter turn of its matrix applied.
///
/// # Errors
/// `Media` for a box size that does not fit its parent, boxes nested deeper than
/// `MAXIMUM_BOX_DEPTH`, or more than `MAXIMUM_MOVIE_HEADER_BYTES` of `moov` walked: its metadata could
/// not be removed, so the file is refused rather than published with it.
pub(crate) fn blank_metadata_and_read_video_dimensions(
    file: &mut [u8],
) -> Result<Option<(u32, u32)>, MessageRuleViolation> {
    let mut walk = MovieWalk::default();
    walk.walk_boxes(file, 0)?;
    Ok(walk.first_video_dimensions)
}

impl MovieWalk {
    fn walk_boxes(&mut self, boxes: &mut [u8], depth: usize) -> Result<(), MessageRuleViolation> {
        if depth > MAXIMUM_BOX_DEPTH {
            return Err(MessageRuleViolation::Media);
        }
        let is_top_level = depth == 0;
        let mut offset = 0;
        while offset < boxes.len() {
            let Some((kind, header_bytes, box_bytes)) = box_header(&boxes[offset..], is_top_level)?
            else {
                // Fewer bytes than a header after the last top-level box: players ignore them.
                break;
            };
            let end = offset + box_bytes;
            let payload = &mut boxes[offset + header_bytes..end];
            let is_metadata = matches!(&kind, b"udta" | b"meta")
                || (&kind == b"uuid" && payload.starts_with(&XMP_BOX_UUID));
            if is_metadata {
                // A linear fill, not a walk: not charged to the movie header budget.
                payload.fill(0);
                boxes[offset + 4..offset + 8].copy_from_slice(b"free");
                offset = end;
                continue;
            }
            // `trak` and `mdia` only inside a `moov`, so everything below the top is in its budget:
            // a box walked into costs its header, any other its whole length.
            let is_walked_into = match &kind {
                b"moov" => true,
                b"trak" | b"mdia" => !is_top_level,
                _ => false,
            };
            if is_walked_into {
                self.charge_movie_header_bytes(header_bytes)?;
            } else if !is_top_level {
                self.charge_movie_header_bytes(box_bytes)?;
            }
            match &kind {
                b"moov" => self.walk_boxes(payload, depth + 1)?,
                b"trak" if !is_top_level => {
                    let outer = self.track.replace(TrackFacts::default());
                    self.walk_boxes(payload, depth + 1)?;
                    let track = std::mem::replace(&mut self.track, outer);
                    if let Some(TrackFacts {
                        is_video: true,
                        dimensions: Some(dimensions),
                    }) = track
                    {
                        self.first_video_dimensions.get_or_insert(dimensions);
                    }
                }
                b"mdia" if !is_top_level => self.walk_boxes(payload, depth + 1)?,
                b"tkhd" => {
                    if let Some(track) = &mut self.track {
                        track.dimensions = displayed_track_dimensions(payload);
                    }
                }
                b"hdlr" => {
                    if let Some(track) = &mut self.track {
                        // Version and flags, `pre_defined` (`QuickTime`'s component type), then
                        // the handler type.
                        track.is_video = payload.get(8..12) == Some(b"vide");
                    }
                }
                _ => {}
            }
            offset = end;
        }
        Ok(())
    }

    fn charge_movie_header_bytes(&mut self, bytes: usize) -> Result<(), MessageRuleViolation> {
        self.movie_header_bytes_read = self.movie_header_bytes_read.saturating_add(bytes);
        if self.movie_header_bytes_read > MAXIMUM_MOVIE_HEADER_BYTES {
            return Err(MessageRuleViolation::Media);
        }
        Ok(())
    }
}

/// The type, header length and whole length of the box `bytes` starts with. At the top level, a
/// box running past the end of the file (a cut recording) ends with it, and `None` means fewer
/// bytes than a header are left; inside a box, both are malformed.
fn box_header(
    bytes: &[u8],
    is_top_level: bool,
) -> Result<Option<([u8; 4], usize, usize)>, MessageRuleViolation> {
    let Some(header) = bytes.first_chunk::<BOX_HEADER_BYTES>() else {
        return if is_top_level {
            Ok(None)
        } else {
            Err(MessageRuleViolation::Media)
        };
    };
    let [s0, s1, s2, s3, k0, k1, k2, k3] = *header;
    let kind = [k0, k1, k2, k3];
    let (header_bytes, declared) = match u32::from_be_bytes([s0, s1, s2, s3]) {
        // Up to the end of the file: only its last top-level box may say so.
        0 if is_top_level => (
            BOX_HEADER_BYTES,
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        ),
        1 => {
            let large = bytes
                .get(BOX_HEADER_BYTES..LARGE_BOX_HEADER_BYTES)
                .and_then(|large| <[u8; 8]>::try_from(large).ok())
                .ok_or(MessageRuleViolation::Media)?;
            (LARGE_BOX_HEADER_BYTES, u64::from_be_bytes(large))
        }
        size => (BOX_HEADER_BYTES, u64::from(size)),
    };
    let declared = usize::try_from(declared).unwrap_or(usize::MAX);
    if declared < header_bytes {
        return Err(MessageRuleViolation::Media);
    }
    let box_bytes = if declared <= bytes.len() {
        declared
    } else if is_top_level {
        bytes.len()
    } else {
        return Err(MessageRuleViolation::Media);
    };
    Ok(Some((kind, header_bytes, box_bytes)))
}

/// A `tkhd` payload's width and height as the track shows: swapped when its matrix turns it a
/// quarter (phones film portrait videos as landscape frames turned 90° or 270°). `None` for a
/// track without a picture (audio) or a payload too short.
fn displayed_track_dimensions(payload: &[u8]) -> Option<(u32, u32)> {
    // After version and flags: times, track id, reserved and duration, 64-bit times in version 1.
    let matrix_offset = match payload.first()? {
        0 => 40,
        1 => 52,
        _ => return None,
    };
    let word = |index: usize| {
        let start = matrix_offset + 4 * index;
        payload
            .get(start..start + 4)
            .and_then(|word| <[u8; 4]>::try_from(word).ok())
            .map(u32::from_be_bytes)
    };
    // Matrix `a b u / c d v / x y w`, then width and height.
    let (a, b, c, d) = (word(0)?, word(1)?, word(3)?, word(4)?);
    let width = word(9)? >> FIXED_POINT_FRACTION_BITS;
    let height = word(10)? >> FIXED_POINT_FRACTION_BITS;
    if width == 0 || height == 0 {
        return None;
    }
    let is_turned_a_quarter = a == 0 && d == 0 && b != 0 && c != 0;
    Some(if is_turned_a_quarter {
        (height, width)
    } else {
        (width, height)
    })
}

const EBML_SEGMENT: u32 = 0x1853_8067;
const EBML_TRACKS: u32 = 0x1654_AE6B;
const EBML_TRACK_ENTRY: u32 = 0xAE;
const EBML_VIDEO: u32 = 0xE0;
const EBML_PIXEL_WIDTH: u32 = 0xB0;
const EBML_PIXEL_HEIGHT: u32 = 0xBA;

/// The first video track's `PixelWidth` and `PixelHeight` of a `WebM` file, read in its first
/// `MAXIMUM_WEBM_HEADER_BYTES`. `None` when they are not there or the elements are malformed.
pub(crate) fn webm_video_dimensions(file: &[u8]) -> Option<(u32, u32)> {
    let header = &file[..file.len().min(MAXIMUM_WEBM_HEADER_BYTES)];
    // Seekable muxers (ffmpeg, OBS) write the segment's real size, past the window read: the
    // top level alone may end at the window, every element inside stays strict.
    let segment = EbmlElements {
        rest: header,
        may_end_at_the_window: true,
    }
    .find(|(id, _)| *id == EBML_SEGMENT)
    .map(|(_, segment)| segment)?;
    let tracks = child_element(segment, EBML_TRACKS)?;
    EbmlElements::strict(tracks)
        .filter(|(id, _)| *id == EBML_TRACK_ENTRY)
        .find_map(|(_, entry)| {
            let video = child_element(entry, EBML_VIDEO)?;
            let width = unsigned_element(child_element(video, EBML_PIXEL_WIDTH)?)?;
            let height = unsigned_element(child_element(video, EBML_PIXEL_HEIGHT)?)?;
            (width > 0 && height > 0).then_some((width, height))
        })
}

fn child_element(body: &[u8], wanted: u32) -> Option<&[u8]> {
    EbmlElements::strict(body)
        .find(|(id, _)| *id == wanted)
        .map(|(_, child)| child)
}

/// An unsigned integer element's value, if it fits a `u32`.
fn unsigned_element(body: &[u8]) -> Option<u32> {
    if body.len() > 8 {
        return None;
    }
    let value = body
        .iter()
        .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
    u32::try_from(value).ok()
}

/// The elements of one EBML level, in order; stops at the first malformed one. An element of
/// unknown size (a live recording's segment or cluster) runs to the end of its parent.
struct EbmlElements<'a> {
    rest: &'a [u8],
    /// Whether a known size past the end of `rest` ends with it instead of stopping.
    may_end_at_the_window: bool,
}

impl<'a> EbmlElements<'a> {
    fn strict(rest: &'a [u8]) -> Self {
        Self {
            rest,
            may_end_at_the_window: false,
        }
    }
}

impl<'a> Iterator for EbmlElements<'a> {
    type Item = (u32, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let (id, id_bytes) = variable_integer(self.rest, true)?;
        let id = id?;
        let (size, size_bytes) = variable_integer(self.rest.get(id_bytes..)?, false)?;
        let body_start = id_bytes + size_bytes;
        let body_end = match size {
            None => self.rest.len(),
            Some(size) => {
                let end = body_start.checked_add(usize::try_from(size).unwrap_or(usize::MAX));
                match end {
                    Some(end) if end <= self.rest.len() => end,
                    _ if self.may_end_at_the_window => self.rest.len(),
                    _ => {
                        self.rest = &[];
                        return None;
                    }
                }
            }
        };
        let Some(body) = self.rest.get(body_start..body_end) else {
            self.rest = &[];
            return None;
        };
        self.rest = &self.rest[body_end..];
        Some((u32::try_from(id).ok()?, body))
    }
}

/// An EBML variable-length integer and its length: an element id keeps its length marker (ids are
/// written that way), a size drops it, and a size of all ones is unknown (`None`).
fn variable_integer(bytes: &[u8], is_id: bool) -> Option<(Option<u64>, usize)> {
    let first = *bytes.first()?;
    let length = usize::try_from(first.leading_zeros()).ok()? + 1;
    let maximum_length = if is_id { 4 } else { 8 };
    if length > maximum_length {
        return None;
    }
    let raw = bytes
        .get(..length)?
        .iter()
        .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
    if is_id {
        return Some((Some(raw), length));
    }
    let value_bits = 7 * u32::try_from(length).ok()?;
    let value = raw & ((1_u64 << value_bits) - 1);
    let is_unknown = value == (1_u64 << value_bits) - 1;
    Some(((!is_unknown).then_some(value), length))
}

#[cfg(test)]
mod tests;
