//! The one form a provider's link is fetched, cached and shown in (ADR-0044). Links to the same
//! thing that differ only by tracking parameters (`?si=`, `feature=`, `pp=`) share one preview row,
//! and the first poster's tracking id never reaches readers of another channel. Each provider
//! slice adds its links here; any other link stays as it is, so its cache key is unchanged.

use url::Url;

/// Hosts whose `/watch?v=` and `/<kind>/<id>` links name a video (the web client's
/// `youTubeEmbedOf` accepts the same).
const YOUTUBE_WATCH_HOSTS: [&str; 6] = [
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "music.youtube.com",
    "youtube-nocookie.com",
    "www.youtube-nocookie.com",
];
const YOUTUBE_MUSIC_HOST: &str = "music.youtube.com";
const YOUTUBE_SHORT_LINK_HOSTS: [&str; 2] = ["youtu.be", "www.youtu.be"];
const YOUTUBE_VIDEO_PATH_KINDS: [&str; 4] = ["shorts", "live", "embed", "v"];
const YOUTUBE_SHORTS_PATH_KIND: &str = "shorts";
const YOUTUBE_VIDEO_ID_LENGTH: usize = 11;
const YOUTUBE_PLAYLIST_ID_LENGTHS: std::ops::RangeInclusive<usize> = 10..=64;
/// Mixes made for one viewer, which the embedded player cannot show.
const YOUTUBE_PERSONAL_MIX_PREFIX: &str = "RD";
/// Digits allowed per part of `t=` (`1h2m3s`) and in `start=`, as in the web client.
const MAXIMUM_HOUR_DIGITS: usize = 3;
const MAXIMUM_MINUTE_DIGITS: usize = 5;
const MAXIMUM_SECOND_DIGITS: usize = 7;
const SECONDS_PER_MINUTE: u32 = 60;
const SECONDS_PER_HOUR: u32 = 3_600;

/// The link in the form its preview is fetched, cached under and shown with.
pub(crate) fn canonical_link(link: &Url) -> Url {
    canonical_youtube_link(link)
        .or_else(|| super::providers::canonical_link(link))
        .unwrap_or_else(|| link.clone())
}

fn is_youtube_id_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '-' || character == '_'
}

pub(crate) fn is_youtube_video_id(id: &str) -> bool {
    id.len() == YOUTUBE_VIDEO_ID_LENGTH && id.chars().all(is_youtube_id_character)
}

pub(crate) fn is_public_youtube_playlist_id(id: &str) -> bool {
    YOUTUBE_PLAYLIST_ID_LENGTHS.contains(&id.len())
        && !id.starts_with(YOUTUBE_PERSONAL_MIX_PREFIX)
        && id.chars().all(is_youtube_id_character)
}

/// `https://www.youtube.com/watch?v=<id>` (`/shorts/<id>` for a Short, `music.youtube.com` kept
/// for YouTube Music), with only the playlist and the start time of the link.
fn canonical_youtube_link(link: &Url) -> Option<Url> {
    let host = link.host_str()?;
    let path_parts: Vec<&str> = link
        .path_segments()?
        .filter(|part| !part.is_empty())
        .collect();
    let query_value = |name: &str| {
        link.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    };
    let (video_id, is_short) = if YOUTUBE_SHORT_LINK_HOSTS.contains(&host) {
        match path_parts.as_slice() {
            [id] => ((*id).to_owned(), false),
            _ => return None,
        }
    } else if YOUTUBE_WATCH_HOSTS.contains(&host) {
        match path_parts.as_slice() {
            ["watch"] => (query_value("v")?.into_owned(), false),
            [kind, id] if YOUTUBE_VIDEO_PATH_KINDS.contains(kind) => {
                ((*id).to_owned(), *kind == YOUTUBE_SHORTS_PATH_KIND)
            }
            _ => return None,
        }
    } else {
        return None;
    };
    if !is_youtube_video_id(&video_id) {
        return None;
    }
    let mut canonical = if host == YOUTUBE_MUSIC_HOST {
        let mut music = Url::parse("https://music.youtube.com/watch").ok()?;
        music.query_pairs_mut().append_pair("v", &video_id);
        music
    } else if is_short {
        Url::parse(&format!("https://www.youtube.com/shorts/{video_id}")).ok()?
    } else {
        let mut watch = Url::parse("https://www.youtube.com/watch").ok()?;
        watch.query_pairs_mut().append_pair("v", &video_id);
        watch
    };
    if let Some(playlist_id) = query_value("list").filter(|id| is_public_youtube_playlist_id(id)) {
        canonical
            .query_pairs_mut()
            .append_pair("list", &playlist_id);
    }
    if let Some(seconds) =
        youtube_start_seconds(query_value("t").as_deref(), query_value("start").as_deref())
    {
        canonical
            .query_pairs_mut()
            .append_pair("t", &seconds.to_string());
    }
    Some(canonical)
}

/// `t=` (`90`, `90s`, `2m`, `1h2m3s`), else `start=` (whole seconds); none when absent, zero or
/// malformed.
fn youtube_start_seconds(time: Option<&str>, start: Option<&str>) -> Option<u32> {
    let seconds = match time.filter(|time| !time.is_empty()) {
        Some(time) => youtube_time_seconds(time)?,
        None => digits_value(start?, MAXIMUM_SECOND_DIGITS)?,
    };
    (seconds > 0).then_some(seconds)
}

fn youtube_time_seconds(time: &str) -> Option<u32> {
    let (hours, rest) = number_before(time, 'h', MAXIMUM_HOUR_DIGITS)?;
    let (minutes, rest) = number_before(rest, 'm', MAXIMUM_MINUTE_DIGITS)?;
    let seconds = match rest.strip_suffix('s').unwrap_or(rest) {
        "" => 0,
        digits => digits_value(digits, MAXIMUM_SECOND_DIGITS)?,
    };
    hours
        .checked_mul(SECONDS_PER_HOUR)?
        .checked_add(minutes.checked_mul(SECONDS_PER_MINUTE)?)?
        .checked_add(seconds)
}

/// The number written before `unit` at the start of `text` (0 when `unit` is absent) and what
/// follows; none when that number is malformed.
fn number_before(text: &str, unit: char, maximum_digits: usize) -> Option<(u32, &str)> {
    match text.split_once(unit) {
        Some((digits, rest)) => Some((digits_value(digits, maximum_digits)?, rest)),
        None => Some((0, text)),
    }
}

fn digits_value(digits: &str, maximum_digits: usize) -> Option<u32> {
    let is_well_formed = (1..=maximum_digits).contains(&digits.len())
        && digits.bytes().all(|byte| byte.is_ascii_digit());
    is_well_formed.then(|| digits.parse().ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical(link: &str) -> String {
        canonical_link(&Url::parse(link).unwrap()).to_string()
    }

    #[test]
    fn youtube_links_to_one_video_share_one_form_without_tracking_parameters() {
        let watch = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        for link in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&si=abc&feature=share",
            "https://youtu.be/dQw4w9WgXcQ?si=other",
            "https://m.youtube.com/watch?feature=x&v=dQw4w9WgXcQ",
            "https://www.youtube.com/live/dQw4w9WgXcQ?si=x",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
            "https://www.youtube-nocookie.com/v/dQw4w9WgXcQ",
        ] {
            assert_eq!(canonical(link), watch, "{link}");
        }
        assert_eq!(
            canonical("https://youtube.com/shorts/aqz-KE-bpKQ?si=Xy"),
            "https://www.youtube.com/shorts/aqz-KE-bpKQ"
        );
        assert_eq!(
            canonical("https://music.youtube.com/watch?v=lYBUbBu4W08&si=a&list=OLAK5uy_abcdefghij"),
            "https://music.youtube.com/watch?v=lYBUbBu4W08&list=OLAK5uy_abcdefghij"
        );
    }

    #[test]
    fn the_start_time_and_a_public_playlist_are_kept_in_one_spelling() {
        for (link, start) in [
            ("https://youtu.be/dQw4w9WgXcQ?t=90", Some("90")),
            ("https://youtu.be/dQw4w9WgXcQ?t=90s", Some("90")),
            ("https://youtu.be/dQw4w9WgXcQ?t=1m30s", Some("90")),
            ("https://youtu.be/dQw4w9WgXcQ?t=1h2m3s", Some("3723")),
            ("https://youtu.be/dQw4w9WgXcQ?t=2m", Some("120")),
            ("https://youtu.be/dQw4w9WgXcQ?start=45", Some("45")),
            ("https://youtu.be/dQw4w9WgXcQ?t=0", None),
            ("https://youtu.be/dQw4w9WgXcQ?t=1x", None),
            ("https://youtu.be/dQw4w9WgXcQ?t=12345678", None),
            ("https://youtu.be/dQw4w9WgXcQ?t=-5", None),
        ] {
            let expected = start.map_or_else(
                || "https://www.youtube.com/watch?v=dQw4w9WgXcQ".to_owned(),
                |start| format!("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t={start}"),
            );
            assert_eq!(canonical(link), expected, "{link}");
        }
        assert_eq!(
            canonical("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL0123456789&index=3"),
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL0123456789"
        );
        assert_eq!(
            canonical("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=RDdQw4w9WgXcQ"),
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "a mix made for one viewer"
        );
    }

    #[test]
    fn other_links_and_malformed_or_look_alike_youtube_links_stay_as_they_are() {
        for link in [
            "https://example.org/watch?v=dQw4w9WgXcQ&si=a",
            "https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/watch?v=dQw4w9WgXc%22",
            "https://www.youtube.com/@channel",
            "https://www.youtube.com/playlist?list=PL0123456789",
            "https://youtu.be/dQw4w9WgXcQ/extra",
        ] {
            assert_eq!(
                canonical(link),
                Url::parse(link).unwrap().to_string(),
                "{link}"
            );
        }
    }
}
