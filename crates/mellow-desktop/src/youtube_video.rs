//! YouTube videos played in their card by Mellow's own player (ADR-0063): the owner's choice,
//! knowing it departs from YouTube's terms. `rustypipe` asks YouTube's `InnerTube` API for the
//! video's separate picture and sound tracks, `mellow-media` streams them in byte ranges and the
//! attachment player plays them like a video attachment (one player at a time, the same controls,
//! "Enlarge" opening the media viewer, stopped when the row leaves the list).
//!
//! A card's video is an [`Attachment`] of its own kind ([`YOUTUBE_VIDEO_TYPE`]) whose id is made
//! here from the card (top bit set: never an instance's id), so every surface of the attachment
//! player works unchanged; [`youtube_video_of`] gives back what it plays. Nothing goes to YouTube
//! before someone presses play; no cookie of the user's and nothing of the Mellow session goes
//! with any request. When YouTube refuses (a "confirm you're not a robot" check, a private, age
//! restricted or live video) or the tracks cannot be read, the card says why and offers the video
//! on YouTube. Only Linux plays them so far; elsewhere the card opens the link in the browser.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash as _, Hasher as _};
use std::time::Duration;

use gpui::{App, ElementId, Global};
use mellow_client_core::shared_logic::link_previews::YouTubeEmbed;
use mellow_media::RemoteTrack;
use mellow_protocol::{Attachment, Snowflake};

/// The kind of a card's video; an instance never serves it (its types are sniffed, ADR-0038).
pub(crate) const YOUTUBE_VIDEO_TYPE: &str = "video/x-mellow-youtube";
/// Set in every id made here: an instance's ids are timestamps, far below it.
const YOUTUBE_ID_BIT: u64 = 1 << 63;
/// The tallest picture track played: pictures are shown at 720 lines at most
/// (`MAXIMUM_SHOWN_VIDEO_SIZE`), so a taller one would only cost decoding.
pub(crate) const MAXIMUM_STREAM_LINES: u32 = 720;
/// Cards remembered for the session; the oldest is forgotten first.
const MAXIMUM_REMEMBERED_VIDEOS: usize = 500;

/// Whether this system plays YouTube videos in their card.
#[must_use]
pub(crate) const fn plays_youtube_here() -> bool {
    cfg!(target_os = "linux")
}

/// What a card's video plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct YouTubeVideo {
    /// The link as posted, for "Open on YouTube" and "Copy link".
    pub(crate) link: String,
    pub(crate) video_id: String,
    pub(crate) start_seconds: Option<u32>,
}

/// Why YouTube's video could not play here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum YouTubeFailure {
    /// YouTube asks to confirm one is not a robot (or refuses this address).
    RobotCheck,
    Private,
    AgeRestricted,
    Live,
    /// Gone, blocked here, paid, members only…
    Unavailable,
    /// The network, or a change on YouTube's side.
    CannotRead,
}

#[derive(Default)]
struct YouTubeVideos {
    videos: HashMap<Snowflake, YouTubeVideo>,
    order: VecDeque<Snowflake>,
    failures: HashMap<Snowflake, YouTubeFailure>,
}

impl Global for YouTubeVideos {}

/// The id of the video a card shows: the same for the same card and link across frames.
fn youtube_id(card_id: &ElementId, link: &str) -> Snowflake {
    // A key drawn once per run: an instance cannot compute a card's id ahead to forge one.
    static KEY: std::sync::OnceLock<std::collections::hash_map::RandomState> =
        std::sync::OnceLock::new();
    let mut hasher = std::hash::BuildHasher::build_hasher(KEY.get_or_init(Default::default));
    card_id.hash(&mut hasher);
    link.hash(&mut hasher);
    Snowflake::from_raw(hasher.finish() | YOUTUBE_ID_BIT)
}

/// The attachment the card `card_id` plays for `video` (from `link`, titled `title`).
pub(crate) fn youtube_attachment(
    card_id: &ElementId,
    link: &str,
    video: &YouTubeEmbed,
    title: &str,
    cx: &mut App,
) -> Attachment {
    let id = youtube_id(card_id, link);
    let store = cx.default_global::<YouTubeVideos>();
    if !store.videos.contains_key(&id) {
        store.order.push_back(id);
        store.videos.insert(
            id,
            YouTubeVideo {
                link: link.to_owned(),
                video_id: video.video_id.clone(),
                start_seconds: video.start_seconds,
            },
        );
        while store.order.len() > MAXIMUM_REMEMBERED_VIDEOS {
            if let Some(oldest) = store.order.pop_front() {
                store.videos.remove(&oldest);
                store.failures.remove(&oldest);
            }
        }
    }
    let (width, height) = if video.is_short { (9, 16) } else { (16, 9) };
    Attachment {
        id,
        file_name: title.to_owned(),
        content_type: YOUTUBE_VIDEO_TYPE.to_owned(),
        size_bytes: 0,
        width: Some(width),
        height: Some(height),
    }
}

#[must_use]
pub(crate) fn is_youtube_attachment(attachment: &Attachment) -> bool {
    attachment.content_type == YOUTUBE_VIDEO_TYPE && attachment.id.raw() & YOUTUBE_ID_BIT != 0
}

/// What the attachment `id` plays, when it is a card's YouTube video.
#[must_use]
pub(crate) fn youtube_video_of(id: Snowflake, cx: &App) -> Option<YouTubeVideo> {
    cx.try_global::<YouTubeVideos>()?.videos.get(&id).cloned()
}

/// Why the attachment `id`'s video could not play, if it could not.
#[must_use]
pub(crate) fn youtube_failure_of(id: Snowflake, cx: &App) -> Option<YouTubeFailure> {
    cx.try_global::<YouTubeVideos>()?.failures.get(&id).copied()
}

pub(crate) fn set_youtube_failure(id: Snowflake, failure: Option<YouTubeFailure>, cx: &mut App) {
    let failures = &mut cx.default_global::<YouTubeVideos>().failures;
    match failure {
        Some(failure) => failures.insert(id, failure),
        None => failures.remove(&id),
    };
}

/// Where a video starts: its link's `t=`.
#[must_use]
pub(crate) fn start_of(video: &YouTubeVideo) -> Option<Duration> {
    video
        .start_seconds
        .filter(|seconds| *seconds > 0)
        .map(|seconds| Duration::from_secs(u64::from(seconds)))
}

/// A video's two tracks, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct YouTubeTracks {
    pub(crate) pictures: RemoteTrack,
    pub(crate) sound: RemoteTrack,
}

/// A picture codec this system's decoder reads, best first: VP9 (small, quick to decode), H.264,
/// then AV1 (the heaviest to decode in software).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PictureCodec {
    Vp9,
    H264,
    Av1,
}

/// A picture track YouTube offers, as far as choosing one goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PictureCandidate {
    pub(crate) codec: Option<PictureCodec>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// HDR, or protected by digital restrictions: not played.
    pub(crate) is_excluded: bool,
}

impl PictureCandidate {
    /// Its quality's lines: the short side, so a vertical Short counts as a landscape video.
    fn lines(&self) -> u32 {
        self.width.min(self.height)
    }
}

/// The picture track to play: the most lines up to `maximum_lines`, the best codec among equals;
/// when every track is taller, the smallest.
#[must_use]
pub(crate) fn choose_picture_track(
    candidates: &[PictureCandidate],
    maximum_lines: u32,
) -> Option<usize> {
    let playable = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| !candidate.is_excluded && candidate.lines() > 0)
        .filter_map(|(index, candidate)| candidate.codec.map(|codec| (index, candidate, codec)));
    let within = playable
        .clone()
        .filter(|(_, candidate, _)| candidate.lines() <= maximum_lines)
        .max_by(|(_, a, a_codec), (_, b, b_codec)| {
            a.lines().cmp(&b.lines()).then(b_codec.cmp(a_codec))
        });
    within
        .or_else(|| {
            playable.min_by(|(_, a, a_codec), (_, b, b_codec)| {
                a.lines().cmp(&b.lines()).then(a_codec.cmp(b_codec))
            })
        })
        .map(|(index, _, _)| index)
}

/// A sound track YouTube offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SoundCandidate {
    /// Above zero for the video's own language (its default track), below for a dubbed one.
    pub(crate) language_rank: i64,
    /// Opus or AAC; anything else (Dolby) is not played.
    pub(crate) is_playable: bool,
    pub(crate) is_opus: bool,
    pub(crate) bitrate: u32,
    pub(crate) is_excluded: bool,
}

/// The sound track to play: the video's own language before a dubbed one, then Opus before AAC,
/// the richest of them.
#[must_use]
pub(crate) fn choose_sound_track(candidates: &[SoundCandidate]) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| candidate.is_playable && !candidate.is_excluded)
        .max_by_key(|(_, candidate)| {
            (
                candidate.language_rank,
                candidate.is_opus,
                candidate.bitrate,
            )
        })
        .map(|(index, _)| index)
}

#[cfg(target_os = "linux")]
mod extraction {
    use std::sync::OnceLock;

    use rustypipe::client::RustyPipe;
    use rustypipe::error::{Error, ExtractionError, UnavailabilityReason};
    use rustypipe::model::{AudioCodec, VideoCodec};

    use super::{
        MAXIMUM_STREAM_LINES, PictureCandidate, PictureCodec, SoundCandidate, YouTubeFailure,
        YouTubeTracks, choose_picture_track, choose_sound_track,
    };
    use mellow_media::RemoteTrack;

    /// `rustypipe-botguard`, when installed, answers YouTube's bot checks (PO tokens); found on
    /// `PATH` only, never in the working directory as `rustypipe` would look by default.
    const BOTGUARD_EXECUTABLE: &str = "rustypipe-botguard";

    fn on_path(name: &str) -> Option<std::path::PathBuf> {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .filter(|directory| directory.is_absolute())
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    }

    /// `<user cache>/mellow[-profile]/youtube/bg_snapshot.bin`, its folder private to the user.
    fn botguard_snapshot_file() -> Option<std::path::PathBuf> {
        let directory = dirs::cache_dir()?
            .join(&*crate::profile::with_profile_suffix(
                crate::pictures::disk_cache::APPLICATION_DIRECTORY,
            ))
            .join("youtube");
        std::fs::create_dir_all(&directory).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).ok()?;
        }
        Some(directory.join("bg_snapshot.bin"))
    }

    fn client() -> Option<&'static RustyPipe> {
        static CLIENT: OnceLock<Option<RustyPipe>> = OnceLock::new();
        CLIENT
            .get_or_init(|| {
                // Nothing on disk (no cache, no failure reports), no account.
                let builder = RustyPipe::builder().no_storage().no_reporter();
                // `no_storage` leaves botguard's snapshot relative to the working directory,
                // where a planted file would be loaded: keep it in Mellow's own cache folder.
                let builder = match (on_path(BOTGUARD_EXECUTABLE), botguard_snapshot_file()) {
                    (Some(path), Some(snapshot)) => {
                        builder.botguard_bin(path).botguard_snapshot_file(snapshot)
                    }
                    _ => builder.no_botguard(),
                };
                builder
                    .build()
                    .map_err(|error| tracing::warn!(%error, "cannot start the YouTube client"))
                    .ok()
            })
            .as_ref()
    }

    fn failure_of(error: &Error) -> YouTubeFailure {
        match error {
            Error::Extraction(ExtractionError::Unavailable { reason, .. }) => match reason {
                UnavailabilityReason::Captcha
                | UnavailabilityReason::IpBan
                | UnavailabilityReason::VpnBan => YouTubeFailure::RobotCheck,
                UnavailabilityReason::Private => YouTubeFailure::Private,
                UnavailabilityReason::AgeRestricted => YouTubeFailure::AgeRestricted,
                UnavailabilityReason::OfflineLivestream => YouTubeFailure::Live,
                _ => YouTubeFailure::Unavailable,
            },
            Error::Extraction(ExtractionError::Botguard(_)) => YouTubeFailure::RobotCheck,
            Error::Extraction(ExtractionError::NotFound { .. }) => YouTubeFailure::Unavailable,
            _ => YouTubeFailure::CannotRead,
        }
    }

    /// `video_id`'s two tracks to play: yt-dlp's when it is installed and answers (the most up
    /// to date), else `rustypipe`'s. When both fail, yt-dlp's reason wins when it names one.
    /// Runs on the network runtime.
    pub(crate) async fn youtube_tracks(video_id: String) -> Result<YouTubeTracks, YouTubeFailure> {
        let yt_dlp_failure = match crate::youtube_yt_dlp::yt_dlp_tracks(&video_id).await {
            Some(Ok(tracks)) => {
                tracing::info!("a YouTube video's tracks came from yt-dlp");
                return Ok(tracks);
            }
            Some(Err(failure)) => Some(failure),
            None => None,
        };
        rustypipe_tracks(&video_id).await.map_err(|failure| {
            yt_dlp_failure
                .filter(|failure| *failure != YouTubeFailure::CannotRead)
                .unwrap_or(failure)
        })
    }

    async fn rustypipe_tracks(video_id: &str) -> Result<YouTubeTracks, YouTubeFailure> {
        let client = client().ok_or(YouTubeFailure::CannotRead)?;
        let player = client.query().player(video_id).await.map_err(|error| {
            tracing::info!(failure = ?failure_of(&error), "YouTube refused a video");
            failure_of(&error)
        })?;
        if player.details.is_live {
            return Err(YouTubeFailure::Live);
        }
        let pictures: Vec<PictureCandidate> = player
            .video_only_streams
            .iter()
            .map(|stream| PictureCandidate {
                codec: match stream.codec {
                    VideoCodec::Vp9 => Some(PictureCodec::Vp9),
                    VideoCodec::Avc1 => Some(PictureCodec::H264),
                    VideoCodec::Av01 => Some(PictureCodec::Av1),
                    _ => None,
                },
                width: stream.width,
                height: stream.height,
                is_excluded: stream.hdr || stream.drm_track_type.is_some(),
            })
            .collect();
        let sounds: Vec<SoundCandidate> = player
            .audio_streams
            .iter()
            .map(|stream| SoundCandidate {
                language_rank: match &stream.track {
                    Some(track) if track.is_default => 1,
                    Some(_) => -1,
                    None => 0,
                },
                is_playable: matches!(stream.codec, AudioCodec::Opus | AudioCodec::Mp4a),
                is_opus: stream.codec == AudioCodec::Opus,
                bitrate: stream.bitrate,
                is_excluded: stream.drm_track_type.is_some(),
            })
            .collect();
        let picture = choose_picture_track(&pictures, MAXIMUM_STREAM_LINES)
            .and_then(|index| player.video_only_streams.get(index))
            .ok_or(YouTubeFailure::Unavailable)?;
        let sound = choose_sound_track(&sounds)
            .and_then(|index| player.audio_streams.get(index))
            .ok_or(YouTubeFailure::Unavailable)?;
        // The addresses come from a third party: checked here, and again at each redirect.
        Ok(YouTubeTracks {
            pictures: RemoteTrack::new(&picture.url, &picture.mime)
                .ok_or(YouTubeFailure::CannotRead)?,
            sound: RemoteTrack::new(&sound.url, &sound.mime).ok_or(YouTubeFailure::CannotRead)?,
        })
    }
}

#[cfg(target_os = "linux")]
pub(crate) use extraction::youtube_tracks;

/// Elsewhere nothing is extracted: the card opens the link instead.
#[cfg(not(target_os = "linux"))]
pub(crate) async fn youtube_tracks(_video_id: String) -> Result<YouTubeTracks, YouTubeFailure> {
    Err(YouTubeFailure::CannotRead)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(codec: Option<PictureCodec>, width: u32, height: u32) -> PictureCandidate {
        PictureCandidate {
            codec,
            width,
            height,
            is_excluded: false,
        }
    }

    #[test]
    fn the_picture_track_is_the_tallest_within_the_limit_in_the_best_codec() {
        let offered = [
            picture(Some(PictureCodec::H264), 640, 360),
            picture(Some(PictureCodec::H264), 1280, 720),
            picture(Some(PictureCodec::Vp9), 1280, 720),
            picture(Some(PictureCodec::Av1), 1280, 720),
            picture(Some(PictureCodec::Vp9), 1920, 1080),
            picture(None, 1280, 720),
        ];
        assert_eq!(choose_picture_track(&offered, 720), Some(2));
        assert_eq!(choose_picture_track(&offered, 1080), Some(4));
        assert_eq!(choose_picture_track(&offered, 480), Some(0));
    }

    #[test]
    fn a_short_counts_its_short_side_and_taller_tracks_fall_back_to_the_smallest() {
        let short = [
            picture(Some(PictureCodec::Vp9), 1080, 1920),
            picture(Some(PictureCodec::Vp9), 720, 1280),
        ];
        assert_eq!(choose_picture_track(&short, 720), Some(1));
        let only_tall = [
            picture(Some(PictureCodec::H264), 3840, 2160),
            picture(Some(PictureCodec::Av1), 1920, 1080),
        ];
        assert_eq!(choose_picture_track(&only_tall, 720), Some(1));
    }

    #[test]
    fn excluded_or_unknown_tracks_are_never_chosen() {
        let offered = [
            PictureCandidate {
                is_excluded: true,
                ..picture(Some(PictureCodec::Vp9), 1280, 720)
            },
            picture(None, 640, 360),
        ];
        assert_eq!(choose_picture_track(&offered, 720), None);
    }

    #[test]
    fn the_sound_track_is_the_richest_opus_then_aac() {
        let sound = |is_opus, bitrate| SoundCandidate {
            language_rank: 0,
            is_playable: true,
            is_opus,
            bitrate,
            is_excluded: false,
        };
        let offered = [
            sound(false, 130_000),
            sound(true, 50_000),
            sound(true, 136_000),
            SoundCandidate {
                is_playable: false,
                ..sound(false, 384_000)
            },
        ];
        assert_eq!(choose_sound_track(&offered), Some(2));
        assert_eq!(choose_sound_track(&offered[..1]), Some(0));
        assert_eq!(choose_sound_track(&offered[3..]), None);
        // A richer dubbed track never wins over the video's own.
        let dubbed = [
            SoundCandidate {
                language_rank: -1,
                ..sound(true, 132_000)
            },
            SoundCandidate {
                language_rank: 10,
                ..sound(true, 125_000)
            },
        ];
        assert_eq!(choose_sound_track(&dubbed), Some(1));
    }

    #[test]
    fn a_video_starts_at_its_links_time() {
        let video = |start_seconds| YouTubeVideo {
            link: "https://youtu.be/dQw4w9WgXcQ?t=95".to_owned(),
            video_id: "dQw4w9WgXcQ".to_owned(),
            start_seconds,
        };
        assert_eq!(start_of(&video(Some(95))), Some(Duration::from_secs(95)));
        assert_eq!(start_of(&video(None)), None);
        assert_eq!(start_of(&video(Some(0))), None);
    }

    #[test]
    fn a_cards_video_has_an_id_no_instance_gives() {
        let card: ElementId = "card".into();
        let link = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        let id = youtube_id(&card, link);
        assert_eq!(id, youtube_id(&card, link));
        assert_ne!(id, youtube_id(&"other".into(), link));
        assert!(id.raw() & YOUTUBE_ID_BIT != 0);
        let attachment = Attachment {
            id,
            file_name: "Title".to_owned(),
            content_type: YOUTUBE_VIDEO_TYPE.to_owned(),
            size_bytes: 0,
            width: None,
            height: None,
        };
        assert!(is_youtube_attachment(&attachment));
        // An instance serving the same type under one of its own ids is not taken for one.
        assert!(!is_youtube_attachment(&Attachment {
            id: Snowflake::from_raw(44),
            ..attachment
        }));
    }
}
