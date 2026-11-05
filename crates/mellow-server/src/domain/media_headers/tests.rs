use super::*;

const IDENTITY: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];
/// Phones' portrait videos: a landscape frame turned 90°.
const TURNED_90: [u32; 9] = [0, 0x0001_0000, 0, 0xFFFF_0000, 0, 0, 0, 0, 0x4000_0000];
const TURNED_270: [u32; 9] = [0, 0xFFFF_0000, 0, 0x0001_0000, 0, 0, 0, 0, 0x4000_0000];
const PLACE: &[u8] = b"+48.8584+002.2945/";

fn boxed(kind: &[u8], payload: &[u8]) -> Vec<u8> {
    let size = u32::try_from(payload.len() + 8).unwrap();
    [&size.to_be_bytes()[..], kind, payload].concat()
}

fn track_header(version: u8, width: u32, height: u32, matrix: [u32; 9]) -> Vec<u8> {
    let mut payload = vec![version, 0, 0, 7];
    payload.extend(vec![0; if version == 1 { 32 } else { 20 }]);
    payload.extend([0; 16]);
    for word in matrix {
        payload.extend(word.to_be_bytes());
    }
    payload.extend((width << 16).to_be_bytes());
    payload.extend((height << 16).to_be_bytes());
    boxed(b"tkhd", &payload)
}

fn handler(kind: &[u8]) -> Vec<u8> {
    boxed(
        b"mdia",
        &boxed(b"hdlr", &[&[0; 8][..], kind, &[0; 12]].concat()),
    )
}

fn track(header: Vec<u8>, kind: &[u8], extra: &[u8]) -> Vec<u8> {
    boxed(b"trak", &[header, handler(kind), extra.to_vec()].concat())
}

fn movie(boxes_in_movie: &[Vec<u8>]) -> Vec<u8> {
    [
        boxed(b"ftyp", b"isom\0\0\x02\0isommp41"),
        boxed(b"moov", &boxes_in_movie.concat()),
        boxed(b"mdat", b"frames"),
    ]
    .concat()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn a_landscape_video_reads_its_first_video_track() {
    let mut file = movie(&[
        boxed(b"mvhd", &[0; 100]),
        track(track_header(0, 0, 0, IDENTITY), b"soun", &[]),
        track(track_header(0, 1920, 1080, IDENTITY), b"vide", &[]),
        track(track_header(1, 640, 480, IDENTITY), b"vide", &[]),
    ]);
    let original = file.clone();
    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut file),
        Ok(Some((1920, 1080)))
    );
    assert_eq!(file, original, "nothing to blank");
}

#[test]
fn a_portrait_video_turned_a_quarter_shows_portrait() {
    for (version, matrix) in [(0, TURNED_90), (1, TURNED_270)] {
        let mut file = movie(&[track(
            track_header(version, 1920, 1080, matrix),
            b"vide",
            &[],
        )]);
        assert_eq!(
            blank_metadata_and_read_video_dimensions(&mut file),
            Ok(Some((1080, 1920))),
            "version {version}"
        );
    }
}

#[test]
fn audio_alone_has_no_dimensions() {
    let mut file = movie(&[track(track_header(0, 0, 0, IDENTITY), b"soun", &[])]);
    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut file),
        Ok(None)
    );
}

#[test]
fn a_track_outside_the_movie_is_not_walked() {
    // Not counted in the movie header budget, so not trusted either.
    let mut file = [
        boxed(b"ftyp", b"isom\0\0\0\0"),
        track(track_header(0, 1920, 1080, IDENTITY), b"vide", &[]),
    ]
    .concat();
    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut file),
        Ok(None)
    );
}

#[test]
fn quicktime_location_boxes_are_blanked_in_movie_and_tracks() {
    // `©xyz` in the movie's user data, and Apple's ISO 6709 key in movie and track metadata.
    let user_data = boxed(
        b"udta",
        &boxed(b"\xa9xyz", &[&[0, 18, 0, 0][..], PLACE].concat()),
    );
    let keys = boxed(
        b"keys",
        &[
            &[0; 8][..],
            b"mdta",
            b"com.apple.quicktime.location.ISO6709",
        ]
        .concat(),
    );
    let metadata = boxed(b"meta", &[keys, boxed(b"ilst", PLACE)].concat());
    let mut file = movie(&[
        user_data.clone(),
        metadata.clone(),
        track(
            track_header(0, 1920, 1080, TURNED_90),
            b"vide",
            &[user_data, metadata].concat(),
        ),
    ]);
    let length = file.len();

    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut file),
        Ok(Some((1080, 1920)))
    );
    assert_eq!(file.len(), length, "offsets into `mdat` stay right");
    assert!(!contains(&file, PLACE));
    assert!(!contains(&file, b"ISO6709"));
    assert!(!contains(&file, b"udta"));
    assert!(!contains(&file, b"meta"));
    assert!(contains(&file, b"free"));
    // Blanked boxes are still boxes: the file walks again the same way.
    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut file),
        Ok(Some((1080, 1920)))
    );
}

#[test]
fn a_large_size_box_and_a_cut_last_box_are_read() {
    let mut large_movie = vec![0, 0, 0, 1];
    large_movie.extend(b"moov");
    let movie_payload = track(track_header(0, 320, 240, IDENTITY), b"vide", &[]);
    large_movie.extend(
        u64::try_from(movie_payload.len() + 16)
            .unwrap()
            .to_be_bytes(),
    );
    large_movie.extend(movie_payload);
    // An `mdat` claiming more than the file holds, then nothing: a cut recording.
    let mut file = [
        boxed(b"ftyp", b"isom\0\0\0\0"),
        large_movie,
        [&0x00FF_FFFF_u32.to_be_bytes()[..], b"mdat", b"cut"].concat(),
    ]
    .concat();
    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut file),
        Ok(Some((320, 240)))
    );
}

#[test]
fn malformed_box_sizes_are_refused() {
    let too_small = [&4_u32.to_be_bytes()[..], b"tkhd"].concat();
    let past_its_parent = [&64_u32.to_be_bytes()[..], b"tkhd", &[0; 8]].concat();
    let zero_inside = [&0_u32.to_be_bytes()[..], b"udta"].concat();
    for inner in [too_small, past_its_parent, zero_inside] {
        let mut file = movie(&[boxed(b"trak", &inner)]);
        assert_eq!(
            blank_metadata_and_read_video_dimensions(&mut file),
            Err(MessageRuleViolation::Media),
            "{inner:?}"
        );
    }
    let mut top_level_too_small = [&3_u32.to_be_bytes()[..], b"moov"].concat();
    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut top_level_too_small),
        Err(MessageRuleViolation::Media)
    );
}

#[test]
fn boxes_nested_deeper_than_the_limit_are_refused() {
    let nest = |levels: usize| {
        let mut inner = boxed(b"udta", PLACE);
        for _ in 0..levels {
            inner = boxed(b"mdia", &inner);
        }
        boxed(b"moov", &inner)
    };
    // `moov` is depth 0: its payload is walked at 1, so seven `mdia` reach depth 8.
    let mut deepest_allowed = nest(7);
    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut deepest_allowed),
        Ok(None)
    );
    assert!(!contains(&deepest_allowed, PLACE));
    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut nest(8)),
        Err(MessageRuleViolation::Media)
    );
    // Depth is bounded, not the stack: a hostile file thousands of boxes deep stops at once.
    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut nest(1000)),
        Err(MessageRuleViolation::Media)
    );
}

#[test]
fn a_movie_header_over_the_limit_is_refused() {
    let padding = |bytes: usize| boxed(b"stbl", &vec![0; bytes - 8]);
    let mut at_the_limit = boxed(b"moov", &padding(MAXIMUM_MOVIE_HEADER_BYTES - 8));
    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut at_the_limit),
        Ok(None)
    );
    let mut over_the_limit = boxed(b"moov", &padding(MAXIMUM_MOVIE_HEADER_BYTES - 7));
    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut over_the_limit),
        Err(MessageRuleViolation::Media)
    );
    // Two movie headers count together.
    let half = boxed(b"moov", &padding(MAXIMUM_MOVIE_HEADER_BYTES / 2));
    let mut two_halves = [half.clone(), half].concat();
    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut two_halves),
        Err(MessageRuleViolation::Media)
    );
}

/// An EBML element with a one-byte size (bodies under 127 bytes).
fn element(id: &[u8], body: &[u8]) -> Vec<u8> {
    let size = 0x80 | u8::try_from(body.len()).unwrap();
    [id, &[size], body].concat()
}

/// A browser's recording: segment and cluster of unknown size.
const UNKNOWN_EBML_SIZE: [u8; 8] = [0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];

fn webm(entries: &[Vec<u8>]) -> Vec<u8> {
    webm_with_segment_size(entries, UNKNOWN_EBML_SIZE)
}

fn webm_with_segment_size(entries: &[Vec<u8>], segment_size: [u8; 8]) -> Vec<u8> {
    let header = element(&[0x1A, 0x45, 0xDF, 0xA3], &element(&[0x42, 0x82], b"webm"));
    let tracks = element(&[0x16, 0x54, 0xAE, 0x6B], &entries.concat());
    let cluster = [&[0x1F, 0x43, 0xB6, 0x75][..], &UNKNOWN_EBML_SIZE, b"frames"].concat();
    [
        header,
        vec![0x18, 0x53, 0x80, 0x67],
        segment_size.to_vec(),
        element(&[0x15, 0x49, 0xA9, 0x66], b"info"),
        tracks,
        cluster,
    ]
    .concat()
}

fn webm_track(video: Option<(&[u8], &[u8])>) -> Vec<u8> {
    let number = element(&[0xD7], &[1]);
    let video = video.map_or_else(Vec::new, |(width, height)| {
        element(
            &[0xE0],
            &[element(&[0xB0], width), element(&[0xBA], height)].concat(),
        )
    });
    element(&[0xAE], &[number, video].concat())
}

#[test]
fn a_webm_file_reads_its_first_video_track() {
    let file = webm(&[
        webm_track(None),
        webm_track(Some((&[0x02, 0x80], &[0x01, 0x68]))),
        webm_track(Some((&[0x10], &[0x10]))),
    ]);
    assert_eq!(webm_video_dimensions(&file), Some((640, 360)));
}

#[test]
fn a_webm_file_without_a_readable_video_track_has_no_dimensions() {
    assert_eq!(webm_video_dimensions(&webm(&[webm_track(None)])), None);
    assert_eq!(
        webm_video_dimensions(&webm(&[webm_track(Some((&[0], &[0x10])))])),
        None
    );
    // A size running past the end of the file.
    let mut cut = webm(&[webm_track(Some((&[0x02, 0x80], &[0x01, 0x68])))]);
    let tracks_size_at = cut
        .windows(4)
        .position(|w| w == [0x16, 0x54, 0xAE, 0x6B])
        .unwrap()
        + 4;
    cut[tracks_size_at] = 0xFE;
    assert_eq!(webm_video_dimensions(&cut), None);
    assert_eq!(webm_video_dimensions(b""), None);
    assert_eq!(webm_video_dimensions(&[0x00; 16]), None);
}

#[test]
fn a_webm_file_with_a_known_segment_size_past_the_bytes_read_reads_its_tracks() {
    // ffmpeg and other seekable muxers write the real segment size: 256 MiB here, far past the
    // header window and the bytes given.
    let file = webm_with_segment_size(
        &[webm_track(Some((&[0x02, 0x80], &[0x01, 0x68])))],
        [0x01, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00],
    );
    assert_eq!(webm_video_dimensions(&file), Some((640, 360)));
}

#[test]
fn metadata_larger_than_the_movie_header_limit_is_blanked_not_refused() {
    // An audiobook's cover art in `moov/udta/meta/ilst/covr`.
    let cover = boxed(b"covr", &vec![7; MAXIMUM_MOVIE_HEADER_BYTES]);
    let mut file = movie(&[
        boxed(b"mvhd", &[0; 100]),
        boxed(b"udta", &boxed(b"meta", &boxed(b"ilst", &cover))),
    ]);
    let length = file.len();
    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut file),
        Ok(None)
    );
    assert_eq!(file.len(), length);
    assert!(!file.contains(&7));
}

#[test]
fn xmp_boxes_are_blanked_at_the_top_and_in_the_movie() {
    let xmp = boxed(
        b"uuid",
        &[
            &XMP_BOX_UUID[..],
            b"<exif:GPSLatitude>48,51.5N</exif:GPSLatitude>",
        ]
        .concat(),
    );
    let other_uuid = boxed(b"uuid", &[&[0xAB; 16][..], b"kept"].concat());
    let mut file = [
        movie(&[
            xmp.clone(),
            other_uuid,
            track(track_header(0, 1920, 1080, IDENTITY), b"vide", &[]),
        ]),
        xmp,
    ]
    .concat();
    let length = file.len();
    assert_eq!(
        blank_metadata_and_read_video_dimensions(&mut file),
        Ok(Some((1920, 1080)))
    );
    assert_eq!(file.len(), length);
    assert!(!contains(&file, b"GPSLatitude"));
    assert!(!contains(&file, &XMP_BOX_UUID));
    assert!(
        contains(&file, b"kept"),
        "other `uuid` boxes are left alone"
    );
}
