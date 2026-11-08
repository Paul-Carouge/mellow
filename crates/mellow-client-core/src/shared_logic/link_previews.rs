//! How a link's preview shows under its message (`web/src/components/link-previews.ts` and
//! `embeds/provider-cards.ts`, ADR-0041, ADR-0044, `docs/design/web-link-previews.md` and
//! `web-provider-embeds.md`): which layout a preview takes, the YouTube video a link names, the
//! page's colour, and the facts of a provider's card (compact counts, a language's colour, an
//! issue's badge, a wiki's language). Every text of a preview is plain: clients never read it as
//! markup.

use mellow_protocol::{
    GitHubIssueState, LinkEmbed, LinkPreview, MusicItem, MusicProvider, TwitchItem,
};
use url::Url;

use super::format::Locale;
use super::linguist_colours::LINGUIST_LANGUAGE_COLOURS;
use super::markdown::safe_http_url;

/// How a preview shows (`LinkPreviewLayout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkPreviewLayout {
    /// The link is itself a picture, shown alone like an attached one.
    PictureOnly,
    /// The card with a 16:9 poster (9:16 for a Short) and a play key.
    YouTube,
    /// A YouTube Music track the instance named the artist of: cover, title, artist, Listen.
    YouTubeMusic,
    /// The card with a wide picture under its text.
    LargePicture,
    /// The card with a small or square picture at its trailing top.
    Thumbnail,
    TextOnly,
    /// Nothing worth a card, or a link that is not http(s).
    None,
}

/// A landscape picture at least this much wider than high reads as a banner, shown large.
pub const MINIMUM_LARGE_PICTURE_ASPECT_RATIO: f64 = 1.6;
/// Narrower than this, a picture would be stretched past its pixels across the card.
pub const MINIMUM_LARGE_PICTURE_WIDTH_PIXELS: u32 = 300;

#[must_use]
pub fn link_preview_layout(
    preview: &LinkPreview,
    youtube: Option<&YouTubeEmbed>,
    embed: Option<&LinkEmbed>,
) -> LinkPreviewLayout {
    if safe_http_url(&preview.url).is_none() {
        return LinkPreviewLayout::None;
    }
    if let Some(youtube) = youtube {
        // The video's id comes from the link, checked strictly; the embed only adds the artist.
        let is_same_track = matches!(
            embed,
            Some(LinkEmbed::YouTubeMusicTrack(track)) if track.video_id == youtube.video_id
        );
        return if is_same_track {
            LinkPreviewLayout::YouTubeMusic
        } else {
            LinkPreviewLayout::YouTube
        };
    }
    let has_text = preview
        .title
        .as_deref()
        .is_some_and(|title| !title.is_empty())
        || preview
            .description
            .as_deref()
            .is_some_and(|description| !description.is_empty());
    let Some(image) = preview
        .image
        .filter(|image| image.width > 0 && image.height > 0)
    else {
        return if has_text {
            LinkPreviewLayout::TextOnly
        } else {
            LinkPreviewLayout::None
        };
    };
    if !has_text {
        return LinkPreviewLayout::PictureOnly;
    }
    let is_wide =
        f64::from(image.width) / f64::from(image.height) >= MINIMUM_LARGE_PICTURE_ASPECT_RATIO;
    if is_wide && image.width >= MINIMUM_LARGE_PICTURE_WIDTH_PIXELS {
        LinkPreviewLayout::LargePicture
    } else {
        LinkPreviewLayout::Thumbnail
    }
}

/// The preview's embed when this client knows its kind and every enumerated value in it: a
/// newer server's kinds and values get the plain card (ADR-0044).
#[must_use]
pub fn known_link_embed_of(preview: &LinkPreview) -> Option<&LinkEmbed> {
    let embed = preview.embed.as_ref()?;
    let is_known = match embed {
        LinkEmbed::Unknown => false,
        LinkEmbed::GitHubIssue(issue) | LinkEmbed::GitHubPullRequest(issue) => {
            issue.state != GitHubIssueState::Unknown
        }
        LinkEmbed::Music(music) => {
            music.provider != MusicProvider::Unknown && music.item != MusicItem::Unknown
        }
        LinkEmbed::Twitch(twitch) => twitch.item != TwitchItem::Unknown,
        LinkEmbed::YouTubeVideo(_)
        | LinkEmbed::YouTubeMusicTrack(_)
        | LinkEmbed::GitHubRepository(_)
        | LinkEmbed::VimeoVideo(_)
        | LinkEmbed::WikipediaArticle(_) => true,
    };
    is_known.then_some(embed)
}

// ---- YouTube ----

/// What a YouTube link plays, every part checked strictly (`YouTubeEmbed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YouTubeEmbed {
    pub video_id: String,
    /// A public playlist the video plays in.
    pub playlist_id: Option<String>,
    /// Where the video starts, in whole seconds above zero.
    pub start_seconds: Option<u32>,
    /// A Shorts link: the video is vertical.
    pub is_short: bool,
}

const YOUTUBE_WATCH_HOSTS: [&str; 6] = [
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "music.youtube.com",
    "youtube-nocookie.com",
    "www.youtube-nocookie.com",
];
const YOUTUBE_SHORT_LINK_HOSTS: [&str; 2] = ["youtu.be", "www.youtu.be"];
/// Paths `/<kind>/<video id>`.
const YOUTUBE_VIDEO_PATH_KINDS: [&str; 4] = ["shorts", "live", "embed", "v"];
const YOUTUBE_VIDEO_ID_LENGTH: usize = 11;
const YOUTUBE_PLAYLIST_ID_LENGTHS: std::ops::RangeInclusive<usize> = 10..=64;
const SECONDS_PER_MINUTE: u32 = 60;
const SECONDS_PER_HOUR: u32 = 3600;
/// `t=` parts as YouTube's share button writes them: at most 3 digits of hours, 5 of minutes and
/// 7 of seconds; `start=` takes the seconds' bound too.
const MAXIMUM_HOUR_DIGITS: usize = 3;
const MAXIMUM_MINUTE_DIGITS: usize = 5;
const MAXIMUM_SECOND_DIGITS: usize = 7;

fn is_youtube_id_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_' || character == '-'
}

fn is_youtube_video_id(candidate: &str) -> bool {
    candidate.len() == YOUTUBE_VIDEO_ID_LENGTH && candidate.chars().all(is_youtube_id_character)
}

/// A public playlist (`PL…`, `OLAK5uy_…`, `UU…`). Shorter ids are a member's own lists (`LL`,
/// `WL`), and `RD…` are mixes made for one viewer: the player cannot show either.
fn is_public_playlist_id(candidate: &str) -> bool {
    YOUTUBE_PLAYLIST_ID_LENGTHS.contains(&candidate.len())
        && !candidate.starts_with("RD")
        && candidate.chars().all(is_youtube_id_character)
}

/// The video of a YouTube watch, short, Shorts, live, embed or `/v/` link (YouTube Music's and
/// `youtube-nocookie.com` too), with its playlist and start; `None` for anything else.
#[must_use]
pub fn youtube_embed_of(link: &str) -> Option<YouTubeEmbed> {
    let url = Url::parse(&safe_http_url(link)?).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let path_parts: Vec<&str> = url
        .path()
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let first_query_value = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    let is_short_link_host = YOUTUBE_SHORT_LINK_HOSTS.contains(&host.as_str());
    let video_id = match path_parts.as_slice() {
        [id] if is_short_link_host => Some((*id).to_owned()),
        ["watch"] if YOUTUBE_WATCH_HOSTS.contains(&host.as_str()) => first_query_value("v"),
        [kind, id]
            if YOUTUBE_WATCH_HOSTS.contains(&host.as_str())
                && YOUTUBE_VIDEO_PATH_KINDS.contains(kind) =>
        {
            Some((*id).to_owned())
        }
        _ => None,
    }
    .filter(|id| is_youtube_video_id(id))?;
    Some(YouTubeEmbed {
        video_id,
        playlist_id: first_query_value("list").filter(|id| is_public_playlist_id(id)),
        start_seconds: youtube_start_seconds(
            first_query_value("t").as_deref(),
            first_query_value("start").as_deref(),
        ),
        is_short: !is_short_link_host && path_parts.first() == Some(&"shorts"),
    })
}

/// `t=` when present and not empty, else `start=`; `None` when absent, zero or malformed.
fn youtube_start_seconds(time: Option<&str>, start: Option<&str>) -> Option<u32> {
    let seconds = match (time, start) {
        (Some(time), _) if !time.is_empty() => share_time_seconds(time),
        (_, Some(start))
            if (1..=MAXIMUM_SECOND_DIGITS).contains(&start.len())
                && start.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            start.parse().ok()
        }
        _ => None,
    };
    seconds.filter(|seconds| *seconds > 0)
}

/// `90`, `90s`, `2m`, `1h2m3s` (`^(?:(\d{1,3})h)?(?:(\d{1,5})m)?(?:(\d{1,7})s?)?$`).
fn share_time_seconds(time: &str) -> Option<u32> {
    // Hours, then minutes, then seconds: each part may only follow the ones before it.
    let mut next_part = 0_u8;
    let mut total = 0_u32;
    let mut rest = time;
    while !rest.is_empty() {
        let digit_count = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digit_count == 0 || next_part > 2 {
            return None;
        }
        let (digits, after) = rest.split_at(digit_count);
        let value: u32 = digits.parse().ok()?;
        let (multiplier, part, unit_length) = match after.bytes().next() {
            Some(b'h') if next_part == 0 && digit_count <= MAXIMUM_HOUR_DIGITS => {
                (SECONDS_PER_HOUR, 1, 1)
            }
            Some(b'm') if next_part <= 1 && digit_count <= MAXIMUM_MINUTE_DIGITS => {
                (SECONDS_PER_MINUTE, 2, 1)
            }
            Some(b's') if digit_count <= MAXIMUM_SECOND_DIGITS => (1, 3, 1),
            None if digit_count <= MAXIMUM_SECOND_DIGITS => (1, 3, 0),
            _ => return None,
        };
        total = total.checked_add(value.checked_mul(multiplier)?)?;
        next_part = part;
        rest = &after[unit_length..];
    }
    Some(total)
}

/// Whole hours, minutes and seconds of a duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurationParts {
    pub hours: u32,
    pub minutes: u32,
    pub seconds: u32,
}

#[must_use]
pub fn duration_parts(total_seconds: u32) -> DurationParts {
    DurationParts {
        hours: total_seconds / SECONDS_PER_HOUR,
        minutes: total_seconds % SECONDS_PER_HOUR / SECONDS_PER_MINUTE,
        seconds: total_seconds % SECONDS_PER_MINUTE,
    }
}

/// "1:30", "1:02:03": a start time as players show it.
#[must_use]
pub fn clock_duration_label(total_seconds: u32) -> String {
    let DurationParts {
        hours,
        minutes,
        seconds,
    } = duration_parts(total_seconds);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

// ---- The card ----

/// "example.org" for a link, so a picture or card without a title still has a name.
#[must_use]
pub fn link_host_name(link: &str) -> String {
    safe_http_url(link)
        .and_then(|normalised| Url::parse(&normalised).ok())
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| link.to_owned())
}

/// Lighter than this (WCAG relative luminance), a page's colour is a white page background, not
/// a brand colour: it would vanish on the light card.
pub const MAXIMUM_BAR_COLOUR_LUMINANCE: f64 = 0.85;
const MAXIMUM_RGB: u32 = 0x00FF_FFFF;

/// The page's `theme-color` as `0xRRGGBB` for the card's pill, or `None` (the token's dusk) when
/// absent, out of range or near white (`linkPreviewBarColor`).
#[must_use]
pub fn link_preview_bar_colour(theme_colour: Option<u32>) -> Option<u32> {
    theme_colour.filter(|colour| {
        *colour <= MAXIMUM_RGB && relative_luminance(*colour) <= MAXIMUM_BAR_COLOUR_LUMINANCE
    })
}

/// WCAG 2.2's relative luminance of `0xRRGGBB`.
fn relative_luminance(colour: u32) -> f64 {
    let linear = |shift: u32| {
        let value = f64::from((colour >> shift) & 0xFF) / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0)
}

// ---- Provider cards ----

/// A repository card shows the first five topics; the page has them all.
pub const MAXIMUM_SHOWN_TOPICS: usize = 5;

/// Compact notations by power of a thousand: English and French (CLDR's short forms).
const COMPACT_SUFFIXES: [(&str, &str); 4] = [("K", "k"), ("M", "M"), ("B", "Md"), ("T", "Bn")];
const COMPACT_SIGNIFICANT_DIGITS: i32 = 3;
const THOUSAND: f64 = 1000.0;

/// "104K", « 104 k », "13.6K", « 13,6 k »: a count at a glance, three significant digits at most
/// (`Intl.NumberFormat` with `notation: "compact"`); the exact one is for screen readers.
#[must_use]
pub fn compact_count(locale: Locale, value: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    // A count far beyond 2^53 would only lose its last digits.
    let mut scaled = value as f64;
    let mut power = 0_usize;
    while scaled >= THOUSAND && power < COMPACT_SUFFIXES.len() {
        scaled /= THOUSAND;
        power += 1;
    }
    if power == 0 {
        return value.to_string();
    }
    #[allow(clippy::cast_possible_truncation)] // A few digits.
    let integer_digits = scaled.log10().floor() as i32 + 1;
    let decimals = (COMPACT_SIGNIFICANT_DIGITS - integer_digits).max(0);
    let factor = 10_f64.powi(decimals);
    let mut rounded = (scaled * factor).round() / factor;
    // 999 950 rounds to "1000K": the next unit says it better.
    if rounded >= THOUSAND && power < COMPACT_SUFFIXES.len() {
        rounded /= THOUSAND;
        power += 1;
    }
    #[allow(clippy::cast_sign_loss)] // At most two decimals.
    let shown = format!("{rounded:.*}", decimals as usize);
    let shown = if shown.contains('.') {
        shown.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        shown
    };
    let (english, french) = COMPACT_SUFFIXES[power - 1];
    match locale {
        Locale::English => format!("{shown}{english}"),
        Locale::French => format!("{}\u{a0}{french}", shown.replace('.', ",")),
    }
}

/// The dot's `0xRRGGBB` beside a repository's language: Linguist's, else the instance's, else
/// `None` for the token's neutral dot.
#[must_use]
pub fn language_colour_of(language: Option<&str>, instance_colour: Option<u32>) -> Option<u32> {
    let language = language?;
    LINGUIST_LANGUAGE_COLOURS
        .iter()
        .find(|(name, _)| *name == language)
        .map(|(_, colour)| *colour)
        .or_else(|| link_preview_bar_colour(instance_colour))
}

/// The glyph of an issue's or pull request's state badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueStateGlyph {
    /// A dot in a ring: an open issue.
    Record,
    GitPullRequest,
    GitMerge,
    CheckCircle,
    XCircle,
}

/// How a state's badge looks: draft is the only outlined one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueStateTone {
    Open,
    Draft,
    Merged,
    Completed,
    Closed,
}

/// Every state its own glyph, coloured in Mellow's tones, never GitHub's (`issueStateBadge`).
/// `None` for a state from a newer server, which [`known_link_embed_of`] already turns away.
#[must_use]
pub fn issue_state_badge(
    state: GitHubIssueState,
    is_pull_request: bool,
) -> Option<(IssueStateGlyph, IssueStateTone)> {
    Some(match state {
        GitHubIssueState::Open if is_pull_request => {
            (IssueStateGlyph::GitPullRequest, IssueStateTone::Open)
        }
        GitHubIssueState::Open => (IssueStateGlyph::Record, IssueStateTone::Open),
        GitHubIssueState::Draft => (IssueStateGlyph::GitPullRequest, IssueStateTone::Draft),
        GitHubIssueState::Merged => (IssueStateGlyph::GitMerge, IssueStateTone::Merged),
        GitHubIssueState::Completed => (IssueStateGlyph::CheckCircle, IssueStateTone::Completed),
        GitHubIssueState::NotPlanned | GitHubIssueState::Closed => {
            (IssueStateGlyph::XCircle, IssueStateTone::Closed)
        }
        GitHubIssueState::Unknown => return None,
    })
}

/// The largest wikis' languages as each interface names them (code, English, French).
/// shortcut: the web asks `Intl.DisplayNames` for any code; other wikis show their code, as the
/// web's fallback does. Add a row when a wiki people share is missing.
const WIKI_LANGUAGE_NAMES: [(&str, &str, &str); 16] = [
    ("ar", "Arabic", "arabe"),
    ("ca", "Catalan", "catalan"),
    ("de", "German", "allemand"),
    ("en", "English", "anglais"),
    ("es", "Spanish", "espagnol"),
    ("fr", "French", "français"),
    ("it", "Italian", "italien"),
    ("ja", "Japanese", "japonais"),
    ("nl", "Dutch", "néerlandais"),
    ("pl", "Polish", "polonais"),
    ("pt", "Portuguese", "portugais"),
    ("ru", "Russian", "russe"),
    ("sv", "Swedish", "suédois"),
    ("uk", "Ukrainian", "ukrainien"),
    ("vi", "Vietnamese", "vietnamien"),
    ("zh", "Chinese", "chinois"),
];

/// The wiki's language as the interface names it ("French", « anglais »), or `None` when it is
/// the interface's own, so the card says plainly "Wikipedia" (`wikipediaLanguageName`).
#[must_use]
pub fn wikipedia_language_name(wiki_language: &str, interface: Locale) -> Option<String> {
    let interface_code = match interface {
        Locale::English => "en",
        Locale::French => "fr",
    };
    if wiki_language == interface_code {
        return None;
    }
    let name = WIKI_LANGUAGE_NAMES
        .iter()
        .find(|(code, _, _)| *code == wiki_language)
        .map_or(wiki_language, |(_, english, french)| match interface {
            Locale::English => english,
            Locale::French => french,
        });
    Some(name.to_owned())
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{LinkPreviewImage, Snowflake};

    use super::*;

    // Ported from `link-previews.test.ts` and `embeds/provider-cards.test.ts`.

    fn article() -> LinkPreview {
        LinkPreview {
            id: Snowflake::from_raw(900),
            url: "https://example.org/article".to_owned(),
            title: Some("An article".to_owned()),
            description: Some("What it says".to_owned()),
            site_name: None,
            theme_color: None,
            image: None,
            embed: None,
        }
    }

    fn with_image(width: u32, height: u32) -> LinkPreview {
        LinkPreview {
            image: Some(LinkPreviewImage { width, height }),
            ..article()
        }
    }

    const PLAYLIST_ID: &str = "PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI";

    fn video() -> YouTubeEmbed {
        YouTubeEmbed {
            video_id: "dQw4w9WgXcQ".to_owned(),
            playlist_id: None,
            start_seconds: None,
            is_short: false,
        }
    }

    fn short() -> YouTubeEmbed {
        YouTubeEmbed {
            video_id: "a_b-C1d2E3f".to_owned(),
            is_short: true,
            ..video()
        }
    }

    #[test]
    fn reads_every_youtube_link_form() {
        for link in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://m.youtube.com/watch?feature=share&v=dQw4w9WgXcQ",
            "http://WWW.YouTube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ?si=abc",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/live/dQw4w9WgXcQ?si=abc",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
            "https://www.youtube.com/v/dQw4w9WgXcQ",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
            "https://youtube-nocookie.com/embed/dQw4w9WgXcQ",
        ] {
            assert_eq!(youtube_embed_of(link), Some(video()), "{link}");
        }
        for link in [
            "https://www.youtube.com/shorts/a_b-C1d2E3f",
            "https://youtube.com/shorts/a_b-C1d2E3f?si=Xy-9_aBcDeFgHiJk",
            "https://m.youtube.com/shorts/a_b-C1d2E3f",
        ] {
            assert_eq!(youtube_embed_of(link), Some(short()), "{link}");
        }
    }

    #[test]
    fn starts_where_the_link_says() {
        for (query, seconds) in [
            ("t=90", 90),
            ("t=90s", 90),
            ("t=2m", 120),
            ("t=1h", 3600),
            ("t=1h2m3s", 3723),
            ("t=2m5", 125),
            ("start=42", 42),
            ("t=10&start=42", 10),
        ] {
            for base in [
                "https://www.youtube.com/watch?v=dQw4w9WgXcQ&",
                "https://youtu.be/dQw4w9WgXcQ?",
            ] {
                let embed = youtube_embed_of(&format!("{base}{query}"));
                assert_eq!(
                    embed.and_then(|embed| embed.start_seconds),
                    Some(seconds),
                    "{query}"
                );
            }
        }
    }

    #[test]
    fn starts_at_the_beginning_with_a_malformed_time() {
        for query in [
            "t=",
            "t=0",
            "t=0s",
            "t=abc",
            "t=1x",
            "t=-5",
            "t=s",
            "t=1m2h",
            "t=12345678",
            "start=4.5",
            "start=1e3",
            "start=",
        ] {
            let link = format!("https://www.youtube.com/watch?v=dQw4w9WgXcQ&{query}");
            assert_eq!(youtube_embed_of(&link), Some(video()), "{query}");
        }
    }

    #[test]
    fn keeps_a_public_playlist_and_drops_the_ones_the_player_cannot_show() {
        let link = format!("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list={PLAYLIST_ID}");
        assert_eq!(
            youtube_embed_of(&link).and_then(|embed| embed.playlist_id),
            Some(PLAYLIST_ID.to_owned())
        );
        let link = format!("https://youtu.be/dQw4w9WgXcQ?list={PLAYLIST_ID}&t=5");
        assert_eq!(
            youtube_embed_of(&link),
            Some(YouTubeEmbed {
                playlist_id: Some(PLAYLIST_ID.to_owned()),
                start_seconds: Some(5),
                ..video()
            })
        );
        let too_long = "x".repeat(65);
        for list in [
            "RD",
            "RDdQw4w9WgXcQ",
            "LL",
            "WL",
            "PL%3Cscript%3Exxxxxxx",
            &format!("{PLAYLIST_ID}%26"),
            &too_long,
        ] {
            let link = format!("https://music.youtube.com/watch?v=dQw4w9WgXcQ&list={list}");
            assert_eq!(youtube_embed_of(&link), Some(video()), "{list}");
        }
    }

    #[test]
    fn refuses_other_ids_pages_and_look_alike_hosts() {
        for link in [
            "https://www.youtube.com/watch?v=dQw4w9WgXc",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQQ",
            "https://youtu.be/dQw4w9WgX%2F",
            "https://www.youtube.com/watch?v=dQw4w9WgX.Q",
            "https://www.youtube.com/watch",
            "https://www.youtube.com/embed/",
            "https://www.youtube.com/@channel",
            "https://www.youtube.com/playlist?list=PL123",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ/extra",
            "https://www.youtube.com/channel/UCuAXFkgsw1L7xaCfnd5JJOw",
            "https://youtu.be/dQw4w9WgXcQ/extra",
            "https://youtu.be/shorts/dQw4w9WgXcQ",
            "https://notyoutube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com.evil.example/watch?v=dQw4w9WgXcQ",
            "https://youtube-nocookie.com.evil.example/embed/dQw4w9WgXcQ",
            "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC",
            "javascript:alert(1)//youtu.be/dQw4w9WgXcQ",
            "https://user@youtu.be/dQw4w9WgXcQ",
        ] {
            assert_eq!(youtube_embed_of(link), None, "{link}");
        }
    }

    #[test]
    fn shows_a_wide_picture_large_and_a_small_or_square_one_as_a_thumbnail() {
        let layout = |preview: &LinkPreview| link_preview_layout(preview, None, None);
        assert_eq!(
            layout(&with_image(800, 420)),
            LinkPreviewLayout::LargePicture
        );
        assert_eq!(layout(&with_image(800, 600)), LinkPreviewLayout::Thumbnail);
        assert_eq!(layout(&with_image(200, 100)), LinkPreviewLayout::Thumbnail);
        assert_eq!(layout(&with_image(300, 400)), LinkPreviewLayout::Thumbnail);
    }

    #[test]
    fn a_picture_link_shows_alone_and_an_empty_preview_shows_nothing() {
        let picture = LinkPreview {
            title: None,
            description: None,
            ..with_image(64, 64)
        };
        assert_eq!(
            link_preview_layout(&picture, None, None),
            LinkPreviewLayout::PictureOnly
        );
        assert_eq!(
            link_preview_layout(&article(), None, None),
            LinkPreviewLayout::TextOnly
        );
        let site_only = LinkPreview {
            title: None,
            description: None,
            site_name: Some("Example".to_owned()),
            ..article()
        };
        assert_eq!(
            link_preview_layout(&site_only, None, None),
            LinkPreviewLayout::None
        );
        assert_eq!(
            link_preview_layout(&with_image(0, 10), None, None),
            LinkPreviewLayout::TextOnly
        );
        let script = LinkPreview {
            url: "javascript:alert(1)".to_owned(),
            ..article()
        };
        assert_eq!(
            link_preview_layout(&script, None, None),
            LinkPreviewLayout::None
        );
    }

    #[test]
    fn every_youtube_video_gets_its_card_and_a_named_track_its_music_card() {
        assert_eq!(
            link_preview_layout(&with_image(480, 360), Some(&video()), None),
            LinkPreviewLayout::YouTube
        );
        let track = LinkPreview {
            url: "https://music.youtube.com/watch?v=dQw4w9WgXcQ".to_owned(),
            embed: Some(LinkEmbed::YouTubeMusicTrack(
                mellow_protocol::YouTubeMusicTrackEmbed {
                    video_id: "dQw4w9WgXcQ".to_owned(),
                    artist: "Noëlle Dupuis".to_owned(),
                    album: None,
                    duration_seconds: None,
                },
            )),
            ..article()
        };
        assert_eq!(
            link_preview_layout(&track, Some(&video()), known_link_embed_of(&track)),
            LinkPreviewLayout::YouTubeMusic
        );
        assert_eq!(
            link_preview_layout(&track, Some(&short()), known_link_embed_of(&track)),
            LinkPreviewLayout::YouTube
        );
    }

    #[test]
    fn a_newer_servers_kind_or_value_gets_the_plain_card() {
        let with_embed = |embed: &str| -> LinkPreview {
            serde_json::from_str(&format!(
                r#"{{"id":"1","url":"https://example.org/","title":"A","embed":{embed}}}"#
            ))
            .expect("a preview")
        };
        assert!(
            known_link_embed_of(&with_embed(
                r#"{"kind":"twitch","item":"clip","id":"Slug"}"#
            ))
            .is_some()
        );
        assert_eq!(known_link_embed_of(&article()), None);
        for embed in [
            r#"{"kind":"hologram","depth":3}"#,
            r#"{"kind":"music","provider":"tidal","item":"track","id":"1"}"#,
            r#"{"kind":"music","provider":"spotify","item":"audiobook","id":"1"}"#,
            r#"{"kind":"github_pull_request","owner":"a","repository":"b","number":1,"state":"queued","author":"c","comments":0}"#,
            r#"{"kind":"twitch","item":"stream","id":"a"}"#,
        ] {
            let preview = with_embed(embed);
            assert_eq!(known_link_embed_of(&preview), None, "{embed}");
            assert_eq!(
                link_preview_layout(&preview, None, known_link_embed_of(&preview)),
                LinkPreviewLayout::TextOnly
            );
        }
    }

    #[test]
    fn the_pill_takes_the_pages_colour_unless_missing_impossible_or_near_white() {
        assert_eq!(link_preview_bar_colour(Some(3_385_975)), Some(0x0033_aa77));
        assert_eq!(link_preview_bar_colour(Some(0)), Some(0));
        assert_eq!(link_preview_bar_colour(None), None);
        assert_eq!(link_preview_bar_colour(Some(0x0100_0000)), None);
        assert_eq!(link_preview_bar_colour(Some(0x00ff_ffff)), None);
        assert_eq!(link_preview_bar_colour(Some(0x00f9_f9f9)), None);
        assert_eq!(
            link_preview_bar_colour(Some(0x00ff_0000)),
            Some(0x00ff_0000)
        );
        assert_eq!(
            link_preview_bar_colour(Some(0x00cc_cccc)),
            Some(0x00cc_cccc)
        );
    }

    #[test]
    fn writes_durations_as_players_do() {
        for (seconds, label) in [
            (5, "0:05"),
            (90, "1:30"),
            (600, "10:00"),
            (3723, "1:02:03"),
            (86400, "24:00:00"),
        ] {
            assert_eq!(clock_duration_label(seconds), label);
        }
        assert_eq!(
            duration_parts(3723),
            DurationParts {
                hours: 1,
                minutes: 2,
                seconds: 3
            }
        );
    }

    #[test]
    fn names_the_site_of_a_link() {
        assert_eq!(
            link_host_name("https://www.rust-lang.org/learn"),
            "www.rust-lang.org"
        );
        assert_eq!(link_host_name("not a link"), "not a link");
    }

    #[test]
    fn rounds_large_counts_to_three_digits_in_the_readers_notation() {
        assert_eq!(compact_count(Locale::English, 104_238), "104K");
        assert_eq!(compact_count(Locale::English, 13_612), "13.6K");
        assert_eq!(compact_count(Locale::French, 13_612), "13,6\u{a0}k");
        assert_eq!(compact_count(Locale::English, 1_234), "1.23K");
        assert_eq!(compact_count(Locale::English, 42), "42");
        assert_eq!(compact_count(Locale::English, 1_000), "1K");
        assert_eq!(compact_count(Locale::English, 999_999), "1M");
        assert_eq!(compact_count(Locale::French, 1_500_000), "1,5\u{a0}M");
        assert_eq!(compact_count(Locale::French, 2_000_000_000), "2\u{a0}Md");
    }

    #[test]
    fn a_language_takes_linguists_colour_then_the_instances() {
        assert_eq!(language_colour_of(Some("Rust"), None), Some(0x00de_a584));
        assert_eq!(
            language_colour_of(Some("Brainfuck"), Some(0x002f_2530)),
            Some(0x002f_2530)
        );
        assert_eq!(language_colour_of(Some("constructor"), None), None);
        assert_eq!(language_colour_of(None, Some(0x002f_2530)), None);
    }

    #[test]
    fn every_state_has_its_own_glyph_with_draft_the_only_outlined_tone() {
        use IssueStateGlyph as Glyph;
        use IssueStateTone as Tone;
        let states = [
            GitHubIssueState::Open,
            GitHubIssueState::Draft,
            GitHubIssueState::Merged,
            GitHubIssueState::Completed,
            GitHubIssueState::NotPlanned,
            GitHubIssueState::Closed,
        ];
        let badges: Vec<_> = states
            .iter()
            .map(|state| issue_state_badge(*state, true))
            .collect();
        assert_eq!(
            badges,
            [
                Some((Glyph::GitPullRequest, Tone::Open)),
                Some((Glyph::GitPullRequest, Tone::Draft)),
                Some((Glyph::GitMerge, Tone::Merged)),
                Some((Glyph::CheckCircle, Tone::Completed)),
                Some((Glyph::XCircle, Tone::Closed)),
                Some((Glyph::XCircle, Tone::Closed)),
            ]
        );
        assert_eq!(
            issue_state_badge(GitHubIssueState::Open, false),
            Some((Glyph::Record, Tone::Open))
        );
    }

    #[test]
    fn names_another_wikis_language_in_the_interfaces_and_none_for_its_own() {
        assert_eq!(
            wikipedia_language_name("fr", Locale::English),
            Some("French".to_owned())
        );
        assert_eq!(
            wikipedia_language_name("en", Locale::French),
            Some("anglais".to_owned())
        );
        assert_eq!(wikipedia_language_name("fr", Locale::French), None);
        assert_eq!(
            wikipedia_language_name("simple", Locale::English),
            Some("simple".to_owned())
        );
    }
}
