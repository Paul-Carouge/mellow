//! YouTube's tracks through yt-dlp, when it is installed (ADR-0063): the most up-to-date
//! extractor, tried before the built-in `rustypipe`. It runs as a process of its own with no
//! configuration, no cookie (neither a file nor a browser's) and no cache, for one video address
//! built here from its checked id, within a time and an output size limit; its JavaScript
//! runtime's cache goes to a temporary folder removed right after. It is looked for only in known
//! absolute folders, never in the working directory.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use mellow_media::RemoteTrack;
use serde_json::Value;
use tokio::io::AsyncReadExt as _;

use crate::youtube_video::{
    MAXIMUM_STREAM_LINES, PictureCandidate, PictureCodec, SoundCandidate, YouTubeFailure,
    YouTubeTracks, choose_picture_track, choose_sound_track,
};

const EXECUTABLE: &str = "yt-dlp";
/// The folders yt-dlp is looked for in: the system's and the user's own (pip, pipx).
const SYSTEM_FOLDERS: [&str; 2] = ["/usr/bin", "/usr/local/bin"];
const USER_FOLDER: &str = ".local/bin";
/// yt-dlp answers in a few seconds (more when it solves YouTube's challenges in JavaScript).
const TIMEOUT: Duration = Duration::from_secs(45);
/// One video's description with every format and caption list: a few megabytes at most.
const MAXIMUM_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;
/// YouTube refuses a fresh address for its first seconds, and keeps refusing one asked too early:
/// yt-dlp itself waits this long (its `playback_wait`) before downloading.
const PLAYBACK_WAIT: Duration = Duration::from_secs(6);
/// Enough of its errors to tell why it failed.
const MAXIMUM_ERROR_BYTES: u64 = 64 * 1024;

/// Where yt-dlp is installed, if it is.
fn executable() -> Option<PathBuf> {
    let user_folder = dirs::home_dir().map(|home| home.join(USER_FOLDER));
    SYSTEM_FOLDERS
        .iter()
        .map(PathBuf::from)
        .chain(user_folder)
        .map(|folder| folder.join(EXECUTABLE))
        .find(|candidate| candidate.is_file())
}

/// Asks yt-dlp for `video_id`'s tracks; `None` when it is not installed.
pub(crate) async fn yt_dlp_tracks(video_id: &str) -> Option<Result<YouTubeTracks, YouTubeFailure>> {
    let executable = executable()?;
    Some(run(executable, video_id).await)
}

async fn run(executable: PathBuf, video_id: &str) -> Result<YouTubeTracks, YouTubeFailure> {
    let address = format!("https://www.youtube.com/watch?v={video_id}");
    // yt-dlp's JavaScript runtime (Deno) keeps a cache of its own: pointed at a folder removed
    // once yt-dlp has answered, so nothing it writes stays on the reader's disk.
    let scratch = tempfile::Builder::new()
        .prefix("mellow-yt-dlp-")
        .tempdir()
        .map_err(|_| YouTubeFailure::CannotRead)?;
    let mut child = tokio::process::Command::new(executable)
        .args([
            "--ignore-config",
            "--no-cookies",
            "--no-cookies-from-browser",
            "--no-cache-dir",
            "--no-playlist",
            "--no-warnings",
            "-J",
            "--",
            &address,
        ])
        .env("XDG_CACHE_HOME", scratch.path())
        .env("DENO_DIR", scratch.path().join("deno"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            tracing::info!(%error, "cannot start yt-dlp");
            YouTubeFailure::CannotRead
        })?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(YouTubeFailure::CannotRead);
    };
    let reading = async {
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let (mut stdout, mut stderr) = (
            stdout.take(MAXIMUM_OUTPUT_BYTES),
            stderr.take(MAXIMUM_ERROR_BYTES),
        );
        let (read_output, read_errors) = tokio::join!(
            stdout.read_to_end(&mut output),
            stderr.read_to_end(&mut errors),
        );
        let status = child.wait().await;
        (read_output.and(read_errors).and(status), output, errors)
    };
    let (status, output, errors) = tokio::time::timeout(TIMEOUT, reading).await.map_err(|_| {
        tracing::info!("yt-dlp took too long");
        YouTubeFailure::CannotRead
    })?;
    match status {
        Ok(status) if status.success() => {
            let description: Value =
                serde_json::from_slice(&output).map_err(|_| YouTubeFailure::CannotRead)?;
            let tracks = tracks_of(&description)?;
            tokio::time::sleep(PLAYBACK_WAIT).await;
            Ok(tracks)
        }
        _ => {
            let failure = failure_of(&String::from_utf8_lossy(&errors));
            tracing::info!(?failure, "yt-dlp could not read a video");
            Err(failure)
        }
    }
}

/// Why yt-dlp failed, from its error message.
fn failure_of(errors: &str) -> YouTubeFailure {
    let errors = errors.to_ascii_lowercase();
    if errors.contains("not a bot")
        || errors.contains("captcha")
        || errors.contains("http error 429")
    {
        YouTubeFailure::RobotCheck
    } else if errors.contains("private video") {
        YouTubeFailure::Private
    } else if errors.contains("age")
        && (errors.contains("restricted") || errors.contains("confirm your age"))
    {
        YouTubeFailure::AgeRestricted
    } else if errors.contains("live event")
        || errors.contains("is live")
        || errors.contains("premieres")
    {
        YouTubeFailure::Live
    } else if errors.contains("video unavailable") || errors.contains("not available") {
        YouTubeFailure::Unavailable
    } else {
        YouTubeFailure::CannotRead
    }
}

/// One of yt-dlp's formats, as far as choosing one goes.
struct Format<'a> {
    url: &'a str,
    mime_type: &'static str,
    headers: Vec<(&'a str, &'a str)>,
}

fn text<'a>(format: &'a Value, key: &str) -> Option<&'a str> {
    format.get(key).and_then(Value::as_str)
}

fn whole(format: &Value, key: &str) -> u32 {
    format
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|number| u32::try_from(number).ok())
        .unwrap_or(0)
}

/// The picture and sound tracks to play from yt-dlp's description of a video (`-J`): direct
/// HTTPS formats only, chosen as `rustypipe`'s are.
pub(crate) fn tracks_of(description: &Value) -> Result<YouTubeTracks, YouTubeFailure> {
    let is_live = description.get("is_live").and_then(Value::as_bool) == Some(true)
        || matches!(
            text(description, "live_status"),
            Some("is_live" | "is_upcoming")
        );
    if is_live {
        return Err(YouTubeFailure::Live);
    }
    let formats: Vec<&Value> = description
        .get("formats")
        .and_then(Value::as_array)
        .map(|formats| formats.iter().collect())
        .unwrap_or_default();
    let is_direct = |format: &&Value| {
        text(format, "protocol") == Some("https") && text(format, "url").is_some()
    };
    let is_excluded = |format: &Value| {
        format.get("has_drm").and_then(Value::as_bool) == Some(true)
            || text(format, "dynamic_range").is_some_and(|range| range != "SDR")
    };
    let pictures: Vec<&Value> = formats
        .iter()
        .copied()
        .filter(is_direct)
        .filter(|format| {
            text(format, "acodec") == Some("none")
                && text(format, "vcodec").is_some_and(|codec| codec != "none")
        })
        .collect();
    let sounds: Vec<&Value> = formats
        .iter()
        .copied()
        .filter(is_direct)
        .filter(|format| {
            text(format, "vcodec") == Some("none")
                && text(format, "acodec").is_some_and(|codec| codec != "none")
        })
        .collect();
    let picture_candidates: Vec<PictureCandidate> = pictures
        .iter()
        .map(|format| PictureCandidate {
            codec: text(format, "vcodec").and_then(|codec| {
                if codec.starts_with("vp9") || codec.starts_with("vp09") {
                    Some(PictureCodec::Vp9)
                } else if codec.starts_with("avc1") {
                    Some(PictureCodec::H264)
                } else if codec.starts_with("av01") {
                    Some(PictureCodec::Av1)
                } else {
                    None
                }
            }),
            width: whole(format, "width"),
            height: whole(format, "height"),
            is_excluded: is_excluded(format) || picture_mime(format).is_none(),
        })
        .collect();
    let sound_candidates: Vec<SoundCandidate> = sounds
        .iter()
        .map(|format| {
            let codec = text(format, "acodec").unwrap_or_default();
            SoundCandidate {
                // yt-dlp ranks the video's own language 10, a dubbed one -1; a range-compressed
                // (DRC) copy of the same comes after the plain one.
                language_rank: format
                    .get("language_preference")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    * 2
                    - i64::from(text(format, "format_id").is_some_and(|id| id.contains("drc"))),
                is_playable: codec == "opus" || codec.starts_with("mp4a"),
                is_opus: codec == "opus",
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                // Kilobits per second, a few hundred at most.
                bitrate: (format.get("abr").and_then(Value::as_f64).unwrap_or(0.0) * 1_000.0) as u32,
                is_excluded: is_excluded(format) || sound_mime(format).is_none(),
            }
        })
        .collect();
    let picture = choose_picture_track(&picture_candidates, MAXIMUM_STREAM_LINES)
        .and_then(|index| pictures.get(index))
        .and_then(|format| {
            Some(Format {
                url: text(format, "url")?,
                mime_type: picture_mime(format)?,
                headers: headers_of(format),
            })
        })
        .ok_or(YouTubeFailure::Unavailable)?;
    let sound = choose_sound_track(&sound_candidates)
        .and_then(|index| sounds.get(index))
        .and_then(|format| {
            Some(Format {
                url: text(format, "url")?,
                mime_type: sound_mime(format)?,
                headers: headers_of(format),
            })
        })
        .ok_or(YouTubeFailure::Unavailable)?;
    Ok(YouTubeTracks {
        pictures: track_of(&picture)?,
        sound: track_of(&sound)?,
    })
}

fn picture_mime(format: &Value) -> Option<&'static str> {
    match text(format, "ext")? {
        "webm" => Some("video/webm"),
        "mp4" => Some("video/mp4"),
        _ => None,
    }
}

fn sound_mime(format: &Value) -> Option<&'static str> {
    match text(format, "ext")? {
        "webm" => Some("audio/webm"),
        "m4a" | "mp4" => Some("audio/mp4"),
        _ => None,
    }
}

fn headers_of(format: &Value) -> Vec<(&str, &str)> {
    format
        .get("http_headers")
        .and_then(Value::as_object)
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(name, value)| Some((name.as_str(), value.as_str()?)))
                .collect()
        })
        .unwrap_or_default()
}

/// The address comes from a third party: checked, with only its safe headers kept.
fn track_of(format: &Format<'_>) -> Result<RemoteTrack, YouTubeFailure> {
    RemoteTrack::new(format.url, format.mime_type)
        .map(|track| track.with_headers(format.headers.iter().copied()))
        .ok_or(YouTubeFailure::CannotRead)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trimmed `yt-dlp -J` answer: a storyboard, a combined format, an HLS one, pictures and
    /// sounds of several kinds.
    const DESCRIPTION: &str = r#"{
        "id": "dQw4w9WgXcQ", "is_live": false, "live_status": "not_live",
        "formats": [
            {"format_id": "sb0", "protocol": "mhtml", "ext": "mhtml", "vcodec": "none", "acodec": "none",
             "url": "https://i.ytimg.com/sb/x"},
            {"format_id": "18", "protocol": "https", "ext": "mp4", "vcodec": "avc1.42001E", "acodec": "mp4a.40.2",
             "width": 640, "height": 360, "url": "https://rr1---sn-a.googlevideo.com/videoplayback?itag=18"},
            {"format_id": "233", "protocol": "m3u8_native", "ext": "mp4", "vcodec": "none", "acodec": "mp4a.40.5",
             "url": "https://manifest.googlevideo.com/api/manifest/hls"},
            {"format_id": "140", "protocol": "https", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2",
             "abr": 129.5, "language_preference": 10, "url": "https://rr1---sn-a.googlevideo.com/videoplayback?itag=140"},
            {"format_id": "251", "protocol": "https", "ext": "webm", "vcodec": "none", "acodec": "opus",
             "abr": 135.2, "language_preference": 10, "url": "https://rr1---sn-a.googlevideo.com/videoplayback?itag=251",
             "http_headers": {"User-Agent": "Mozilla/5.0", "Accept": "*/*", "Cookie": "SID=x"}},
            {"format_id": "251-3", "protocol": "https", "ext": "webm", "vcodec": "none", "acodec": "opus",
             "abr": 140.0, "language_preference": -1, "url": "https://rr1---sn-a.googlevideo.com/videoplayback?itag=251&xtags=vi"},
            {"format_id": "251-drc", "protocol": "https", "ext": "webm", "vcodec": "none", "acodec": "opus",
             "abr": 136.0, "language_preference": 10, "url": "https://rr1---sn-a.googlevideo.com/videoplayback?itag=251&xtags=drc"},
            {"format_id": "136", "protocol": "https", "ext": "mp4", "vcodec": "avc1.4d401f", "acodec": "none",
             "width": 1280, "height": 720, "dynamic_range": "SDR", "url": "https://rr1---sn-a.googlevideo.com/videoplayback?itag=136"},
            {"format_id": "247", "protocol": "https", "ext": "webm", "vcodec": "vp9", "acodec": "none",
             "width": 1280, "height": 720, "dynamic_range": "SDR", "url": "https://rr1---sn-a.googlevideo.com/videoplayback?itag=247",
             "http_headers": {"User-Agent": "Mozilla/5.0"}},
            {"format_id": "334", "protocol": "https", "ext": "webm", "vcodec": "vp09.02.30.10", "acodec": "none",
             "width": 1280, "height": 720, "dynamic_range": "HDR10", "url": "https://rr1---sn-a.googlevideo.com/videoplayback?itag=334"},
            {"format_id": "248", "protocol": "https", "ext": "webm", "vcodec": "vp9", "acodec": "none",
             "width": 1920, "height": 1080, "url": "https://rr1---sn-a.googlevideo.com/videoplayback?itag=248"}
        ]
    }"#;

    #[test]
    fn the_tracks_are_the_720p_vp9_pictures_and_the_opus_sound() {
        let description: Value = serde_json::from_str(DESCRIPTION).unwrap();
        let tracks = tracks_of(&description).unwrap();
        let expected_pictures = RemoteTrack::new(
            "https://rr1---sn-a.googlevideo.com/videoplayback?itag=247",
            "video/webm",
        )
        .unwrap()
        .with_headers([("User-Agent", "Mozilla/5.0")]);
        assert_eq!(tracks.pictures, expected_pictures);
        // Only the user agent: never the cookie, nor the browser page's `Accept`.
        let expected_sound = RemoteTrack::new(
            "https://rr1---sn-a.googlevideo.com/videoplayback?itag=251",
            "audio/webm",
        )
        .unwrap()
        .with_headers([("User-Agent", "Mozilla/5.0")]);
        assert_eq!(tracks.sound, expected_sound);
    }

    #[test]
    fn a_live_video_or_one_without_direct_tracks_is_refused() {
        let live: Value = serde_json::from_str(r#"{"is_live": true, "formats": []}"#).unwrap();
        assert_eq!(tracks_of(&live), Err(YouTubeFailure::Live));
        let hls_only: Value = serde_json::from_str(
            r#"{"formats": [{"protocol": "m3u8_native", "ext": "mp4", "vcodec": "avc1", "acodec": "none",
                "width": 1280, "height": 720, "url": "https://manifest.googlevideo.com/x"}]}"#,
        )
        .unwrap();
        assert_eq!(tracks_of(&hls_only), Err(YouTubeFailure::Unavailable));
    }

    #[test]
    fn an_address_outside_youtubes_servers_is_refused() {
        let description: Value = serde_json::from_str(
            &DESCRIPTION.replace("rr1---sn-a.googlevideo.com", "evil.example"),
        )
        .unwrap();
        assert_eq!(tracks_of(&description), Err(YouTubeFailure::CannotRead));
    }

    #[test]
    fn its_errors_say_why() {
        assert_eq!(
            failure_of("ERROR: [youtube] x: Sign in to confirm you’re not a bot."),
            YouTubeFailure::RobotCheck
        );
        assert_eq!(
            failure_of("ERROR: [youtube] x: Private video"),
            YouTubeFailure::Private
        );
        assert_eq!(
            failure_of("ERROR: [youtube] x: Sign in to confirm your age"),
            YouTubeFailure::AgeRestricted
        );
        assert_eq!(
            failure_of("ERROR: [youtube] x: Video unavailable"),
            YouTubeFailure::Unavailable
        );
        assert_eq!(
            failure_of("ERROR: something else"),
            YouTubeFailure::CannotRead
        );
    }
}
