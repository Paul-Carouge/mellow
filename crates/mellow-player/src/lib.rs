//! What the desktop app and its YouTube player window say to each other, and what the window may
//! load (ADR-0061). The window is a process of its own (`mellow-player`), started by the app with
//! [`LaunchOptions`] on its command line; the app then writes one [`ParentCommand`] per line of
//! JSON on the window's standard input and reads one [`PlayerEvent`] per line from its standard
//! output. The end of the standard input is the end of the app: the window closes, so it never
//! outlives Mellow, even when Mellow is killed.
//!
//! Everything here is pure and validated on both sides: the app checks what it asks for, and the
//! window checks it again before anything reaches the web view, which only ever loads
//! `https://www.youtube-nocookie.com/embed/<video id>`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use url::Url;

/// The binary's file name, next to the app's own.
pub const PLAYER_EXECUTABLE_NAME: &str = if cfg!(windows) {
    "mellow-player.exe"
} else {
    "mellow-player"
};
/// YouTube's privacy-enhanced player, as on the web (ADR-0041).
pub const EMBED_ORIGIN: &str = "https://www.youtube-nocookie.com";
const EMBED_HOST: &str = "www.youtube-nocookie.com";
const EMBED_PATH_PREFIX: &str = "/embed/";
/// Links the player itself offers (its logo, the title, "Watch on YouTube"): they open in the
/// system browser, never in the window.
const BROWSER_HOSTS: [&str; 5] = [
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "music.youtube.com",
    "youtu.be",
];
const VIDEO_ID_LENGTH: usize = 11;
/// A public playlist (`PL…`, `OLAK5uy_…`, `UU…`), as client-core's link previews keep them.
const PLAYLIST_ID_LENGTHS: std::ops::RangeInclusive<usize> = 10..=64;
/// A window title longer than this is cut: the title comes from the linked page.
const MAXIMUM_TITLE_CHARACTERS: usize = 200;
/// A command line longer than this is not Mellow's.
const MAXIMUM_COMMAND_LINE_BYTES: usize = 4096;

/// The smallest window, in logical pixels along its short side, and the largest along its long side.
pub const MINIMUM_SHORT_SIDE: u32 = 144;
pub const MAXIMUM_LONG_SIDE: u32 = 16_384;
/// A video's window when nothing is remembered: YouTube's 480p, 16:9.
pub const DEFAULT_VIDEO_SIZE: (u32, u32) = (854, 480);
/// A Short's: 9:16, the height of a 720p video.
pub const DEFAULT_SHORT_SIZE: (u32, u32) = (360, 640);

/// One video to play. Every field is checked by [`PlayRequest::validated`] on both sides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayRequest {
    pub video_id: String,
    pub playlist_id: Option<String>,
    pub start_seconds: Option<u32>,
    /// A Short: the window is upright (9:16).
    pub is_short: bool,
    /// The window's title: the linked page's title, plain text.
    pub title: String,
}

impl PlayRequest {
    /// The request with a clean title, or `None` when an id is not YouTube's form.
    #[must_use]
    pub fn validated(self) -> Option<Self> {
        let is_valid = is_video_id(&self.video_id)
            && self.playlist_id.as_deref().is_none_or(is_playlist_id)
            && self.start_seconds != Some(0);
        is_valid.then(|| Self {
            title: window_title(&self.title),
            ..self
        })
    }

    /// The player's address: autoplay, then the start and the playlist, as the web's
    /// `youTubeEmbedUrl` without its page-to-player messaging, which the window does not use.
    /// `is_muted` is for tests only, so a test run never makes a sound.
    #[must_use]
    pub fn embed_url(&self, is_muted: bool) -> String {
        use std::fmt::Write as _;
        // Writing to a `String` cannot fail.
        let mut url = format!(
            "{EMBED_ORIGIN}{EMBED_PATH_PREFIX}{}?autoplay=1",
            self.video_id
        );
        if let Some(start) = self.start_seconds {
            let _ = write!(url, "&start={start}");
        }
        if let Some(playlist) = &self.playlist_id {
            let _ = write!(url, "&list={playlist}");
        }
        if is_muted {
            url.push_str("&mute=1");
        }
        url
    }
}

/// What the app asks of the window, one per line of JSON on its standard input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum ParentCommand {
    /// Plays this video instead of the current one and brings the window to the front.
    Play(PlayRequest),
    KeepOnTop {
        is_on: bool,
    },
    Close,
}

/// What the window tells the app, one per line of JSON on its standard output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum PlayerEvent {
    /// The web view exists: from now on the window plays the videos it is given.
    Ready,
    Playing {
        video_id: String,
    },
    KeepOnTop {
        is_on: bool,
    },
    /// A link the player offers was clicked; the app opens it if [`browser_link`] accepts it.
    OpenInBrowser {
        url: String,
    },
    /// The system has no web view (`WebView2`'s runtime missing on Windows): the app opens the
    /// video in the browser. A missing `WebKitGTK` stops the process before it can say anything.
    Unavailable,
    /// The window closed; the app keeps its place for next time.
    Closed {
        placement: Option<WindowPlacement>,
    },
}

impl PlayerEvent {
    /// The event's line, without its line feed.
    #[must_use]
    pub fn to_line(&self) -> String {
        // A plain enum of strings, numbers and booleans always serialises.
        serde_json::to_string(self).unwrap_or_default()
    }

    #[must_use]
    pub fn from_line(line: &str) -> Option<Self> {
        serde_json::from_str(line).ok()
    }
}

impl ParentCommand {
    #[must_use]
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// The command, with a [`ParentCommand::Play`] validated; `None` for anything else.
    #[must_use]
    pub fn from_line(line: &str) -> Option<Self> {
        match serde_json::from_str(line).ok()? {
            Self::Play(request) => request.validated().map(Self::Play),
            other => Some(other),
        }
    }
}

/// Where the window was and its inner size in logical pixels. The position is in physical pixels;
/// the window leaves it out when it closes maximised, minimised or full screen, and it means
/// nothing on Wayland, whose compositor places windows itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub position: Option<(i32, i32)>,
    pub width: u32,
    pub height: u32,
}

impl WindowPlacement {
    /// `854x480` or `854x480@120,-40`, as kept in the preferences and passed on the command line.
    #[must_use]
    pub fn to_text(&self) -> String {
        match self.position {
            None => format!("{}x{}", self.width, self.height),
            Some((x, y)) => format!("{}x{}@{x},{y}", self.width, self.height),
        }
    }

    /// Anything but [`WindowPlacement::to_text`]'s form, or a size out of bounds, gives `None`.
    #[must_use]
    pub fn from_text(text: &str) -> Option<Self> {
        let (size, position) = match text.split_once('@') {
            Some((size, position)) => (size, Some(position)),
            None => (text, None),
        };
        let (width, height) = size.split_once('x')?;
        let width: u32 = width.parse().ok()?;
        let height: u32 = height.parse().ok()?;
        let is_size_valid =
            width.min(height) >= MINIMUM_SHORT_SIDE && width.max(height) <= MAXIMUM_LONG_SIDE;
        let position = match position {
            None => None,
            Some(position) => {
                let (x, y) = position.split_once(',')?;
                Some((x.parse().ok()?, y.parse().ok()?))
            }
        };
        is_size_valid.then_some(Self {
            position,
            width,
            height,
        })
    }

    /// The placement with every field back within bounds, as a stored one would be read; `None`
    /// for one Mellow would not keep.
    #[must_use]
    pub fn validated(&self) -> Option<Self> {
        Self::from_text(&self.to_text())
    }

    /// The inner size for a video (`is_short` false, landscape) or a Short (portrait): the
    /// remembered one when it has that orientation, the default otherwise.
    #[must_use]
    pub fn size_for(placement: Option<&Self>, is_short: bool) -> (u32, u32) {
        match placement {
            Some(placement) if (placement.height > placement.width) == is_short => {
                (placement.width, placement.height)
            }
            _ if is_short => DEFAULT_SHORT_SIZE,
            _ => DEFAULT_VIDEO_SIZE,
        }
    }
}

/// How the app starts the window: `mellow-player --data-directory <dir> [--background #rrggbb]
/// [--placement <placement>] [--keep-on-top]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchOptions {
    /// The web view's own folder, inside the app's profile (`MELLOW_PROFILE`).
    pub data_directory: PathBuf,
    /// Around the player and before it draws: the theme's player background, `0xRRGGBB`.
    pub background: u32,
    pub placement: Option<WindowPlacement>,
    pub keep_on_top: bool,
}

/// Why a command line was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchOptionsError {
    MissingDataDirectory,
    RelativeDataDirectory,
    InvalidValue(&'static str),
    UnknownArgument,
    TooLong,
}

impl std::fmt::Display for LaunchOptionsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingDataDirectory => formatter.write_str("--data-directory is required"),
            Self::RelativeDataDirectory => {
                formatter.write_str("--data-directory must be an absolute path")
            }
            Self::InvalidValue(option) => write!(formatter, "invalid value for {option}"),
            Self::UnknownArgument => formatter.write_str("unknown argument"),
            Self::TooLong => formatter.write_str("the command line is too long"),
        }
    }
}

impl std::error::Error for LaunchOptionsError {}

const DATA_DIRECTORY_OPTION: &str = "--data-directory";
const BACKGROUND_OPTION: &str = "--background";
const PLACEMENT_OPTION: &str = "--placement";
const KEEP_ON_TOP_OPTION: &str = "--keep-on-top";

impl LaunchOptions {
    /// The arguments after the program's name.
    #[must_use]
    pub fn to_arguments(&self) -> Vec<std::ffi::OsString> {
        let mut arguments = vec![
            DATA_DIRECTORY_OPTION.into(),
            self.data_directory.clone().into_os_string(),
            BACKGROUND_OPTION.into(),
            format!("#{:06x}", self.background & 0x00FF_FFFF).into(),
        ];
        if let Some(placement) = &self.placement {
            arguments.push(PLACEMENT_OPTION.into());
            arguments.push(placement.to_text().into());
        }
        if self.keep_on_top {
            arguments.push(KEEP_ON_TOP_OPTION.into());
        }
        arguments
    }

    /// Reads the arguments after the program's name.
    ///
    /// # Errors
    /// A missing or relative data directory, a malformed value, an unknown argument, or a command
    /// line too long to be Mellow's.
    pub fn from_arguments(
        arguments: impl IntoIterator<Item = std::ffi::OsString>,
    ) -> Result<Self, LaunchOptionsError> {
        let mut data_directory = None;
        let mut background = DEFAULT_BACKGROUND;
        let mut placement = None;
        let mut keep_on_top = false;
        let arguments: Vec<std::ffi::OsString> = arguments.into_iter().collect();
        let length: usize = arguments.iter().map(|argument| argument.len()).sum();
        if length > MAXIMUM_COMMAND_LINE_BYTES {
            return Err(LaunchOptionsError::TooLong);
        }
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            let mut value = |option: &'static str| {
                arguments
                    .next()
                    .ok_or(LaunchOptionsError::InvalidValue(option))
            };
            match argument.to_str() {
                Some(DATA_DIRECTORY_OPTION) => {
                    let path = PathBuf::from(value(DATA_DIRECTORY_OPTION)?);
                    if !path.is_absolute() {
                        return Err(LaunchOptionsError::RelativeDataDirectory);
                    }
                    data_directory = Some(path);
                }
                Some(BACKGROUND_OPTION) => {
                    background = value(BACKGROUND_OPTION)?
                        .to_str()
                        .and_then(parse_hex_colour)
                        .ok_or(LaunchOptionsError::InvalidValue(BACKGROUND_OPTION))?;
                }
                Some(PLACEMENT_OPTION) => {
                    placement = Some(
                        value(PLACEMENT_OPTION)?
                            .to_str()
                            .and_then(WindowPlacement::from_text)
                            .ok_or(LaunchOptionsError::InvalidValue(PLACEMENT_OPTION))?,
                    );
                }
                Some(KEEP_ON_TOP_OPTION) => keep_on_top = true,
                _ => return Err(LaunchOptionsError::UnknownArgument),
            }
        }
        Ok(Self {
            data_directory: data_directory.ok_or(LaunchOptionsError::MissingDataDirectory)?,
            background,
            placement,
            keep_on_top,
        })
    }
}

/// Dusk's player background (`component.link-preview.player-background`), when none is given.
const DEFAULT_BACKGROUND: u32 = 0x0011_0B11;

/// `#rrggbb` as `0xRRGGBB`.
#[must_use]
pub fn parse_hex_colour(text: &str) -> Option<u32> {
    let digits = text.strip_prefix('#')?;
    (digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(digits, 16).ok())
        .flatten()
}

/// What the window does with a navigation its web view is about to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationVerdict {
    /// The player itself, or one of its own empty frames.
    Allow,
    /// A YouTube page the player links to: the system browser opens it, the window stays.
    OpenInBrowser(String),
    Block,
}

/// The only addresses the web view may show: the player for a valid video id, over HTTPS on the
/// default port, without credentials; and the empty frames its page makes. A YouTube link the
/// player offers goes to the browser; anything else is blocked.
#[must_use]
pub fn navigation_verdict(address: &str) -> NavigationVerdict {
    if matches!(address, "about:blank" | "about:srcdoc") {
        return NavigationVerdict::Allow;
    }
    let Ok(url) = Url::parse(address) else {
        return NavigationVerdict::Block;
    };
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return NavigationVerdict::Block;
    }
    let host = url.host_str().unwrap_or_default();
    if host == EMBED_HOST
        && url
            .path()
            .strip_prefix(EMBED_PATH_PREFIX)
            .is_some_and(is_video_id)
    {
        return NavigationVerdict::Allow;
    }
    match browser_link(address) {
        Some(link) => NavigationVerdict::OpenInBrowser(link),
        None => NavigationVerdict::Block,
    }
}

/// `address` as the browser should get it, when it is an HTTPS page of YouTube without
/// credentials nor an explicit port; `None` otherwise. The app checks it again before opening.
#[must_use]
pub fn browser_link(address: &str) -> Option<String> {
    let url = Url::parse(address).ok()?;
    let is_youtube_page = url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .host_str()
            .is_some_and(|host| BROWSER_HOSTS.contains(&host));
    is_youtube_page.then(|| url.as_str().to_owned())
}

/// YouTube's video id: 11 of `[A-Za-z0-9_-]`.
#[must_use]
pub fn is_video_id(candidate: &str) -> bool {
    candidate.len() == VIDEO_ID_LENGTH && candidate.bytes().all(is_id_byte)
}

/// A public playlist the embedded player can show; a viewer's own mixes (`RD…`) it cannot.
#[must_use]
pub fn is_playlist_id(candidate: &str) -> bool {
    PLAYLIST_ID_LENGTHS.contains(&candidate.len())
        && !candidate.starts_with("RD")
        && candidate.bytes().all(is_id_byte)
}

fn is_id_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

/// The page's title on one line, without control characters, cut to a bounded length; "YouTube"
/// when nothing is left.
#[must_use]
pub fn window_title(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .filter(|character| !is_invisible_formatting(*character))
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAXIMUM_TITLE_CHARACTERS)
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        "YouTube".to_owned()
    } else {
        cleaned
    }
}

/// Zero-width characters, direction marks, embeddings, overrides and isolates, and the byte
/// order mark: a title could hide or reverse what it shows with them.
fn is_invisible_formatting(character: char) -> bool {
    matches!(
        character,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
    )
}

/// What [`read_bounded_line`] read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundedLine {
    /// A line, without its line feed.
    Line(String),
    /// A line longer than the bound (or not UTF-8): it was read to its end and dropped.
    Refused,
    End,
}

/// The next line of `reader`, at most `maximum_bytes` long. A longer line is skipped to its end
/// in buffer-sized steps, never held whole and never read again as the next line.
///
/// # Errors
/// The reader's own.
pub fn read_bounded_line<R: std::io::BufRead>(
    reader: &mut R,
    maximum_bytes: usize,
) -> std::io::Result<BoundedLine> {
    let mut line = Vec::new();
    let mut is_too_long = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(if line.is_empty() && !is_too_long {
                BoundedLine::End
            } else if is_too_long {
                BoundedLine::Refused
            } else {
                text_line(line)
            });
        }
        let (taken, found_end) = match available.iter().position(|byte| *byte == b'\n') {
            Some(index) => (index + 1, true),
            None => (available.len(), false),
        };
        let content = &available[..if found_end { taken - 1 } else { taken }];
        if !is_too_long && line.len() + content.len() <= maximum_bytes {
            line.extend_from_slice(content);
        } else {
            is_too_long = true;
            line = Vec::new();
        }
        reader.consume(taken);
        if found_end {
            return Ok(if is_too_long {
                BoundedLine::Refused
            } else {
                text_line(line)
            });
        }
    }
}

fn text_line(line: Vec<u8>) -> BoundedLine {
    String::from_utf8(line).map_or(BoundedLine::Refused, |text| {
        BoundedLine::Line(text.trim_end_matches('\r').to_owned())
    })
}

/// A screen, in physical pixels, with its scale factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
}

/// A remembered window must keep this much of itself on a screen, in physical pixels, to be put
/// back where it was: enough to grab its title bar.
pub const MINIMUM_VISIBLE_SIDE: i64 = 48;

/// Where to open the window, of `wanted` logical size, given where it was and the screens now
/// connected: its old position when at least 48 × 48 pixels of it fall on one screen, and its size
/// cut to that screen (the first one when the position is dropped).
#[must_use]
pub fn restored_placement(
    position: Option<(i32, i32)>,
    wanted: (u32, u32),
    screens: &[ScreenArea],
) -> (Option<(i32, i32)>, (u32, u32)) {
    let overlap_with = |screen: &ScreenArea, (x, y): (i32, i32)| {
        let (width, height) = physical_size(wanted, screen.scale_factor);
        let left = i64::from(x).max(i64::from(screen.x));
        let top = i64::from(y).max(i64::from(screen.y));
        let right = (i64::from(x) + width).min(i64::from(screen.x) + i64::from(screen.width));
        let bottom = (i64::from(y) + height).min(i64::from(screen.y) + i64::from(screen.height));
        (right - left).min(bottom - top)
    };
    let kept = position.and_then(|position| {
        screens
            .iter()
            .find(|screen| overlap_with(screen, position) >= MINIMUM_VISIBLE_SIDE)
            .map(|screen| (position, screen))
    });
    let screen = kept.map(|(_, screen)| screen).or(screens.first());
    let size = screen.map_or(wanted, |screen| {
        let logical = |side: u32| {
            // A screen is far below u32::MAX pixels; the cast cannot truncate.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let side = (f64::from(side) / screen.scale_factor.max(1.0)).floor() as u32;
            side.max(MINIMUM_SHORT_SIDE)
        };
        (
            wanted.0.min(logical(screen.width)),
            wanted.1.min(logical(screen.height)),
        )
    });
    (kept.map(|(position, _)| position), size)
}

/// `size` in logical pixels as physical pixels on a screen of `scale_factor`.
fn physical_size(size: (u32, u32), scale_factor: f64) -> (i64, i64) {
    // Window sides stay below MAXIMUM_LONG_SIDE times a scale of a few units.
    #[allow(clippy::cast_possible_truncation)]
    let scaled = |side: u32| (f64::from(side) * scale_factor).round() as i64;
    (scaled(size.0), scaled(size.1))
}

/// The window exited on its own after the web view failed to start (`WebView2`'s runtime
/// missing) or its content process ended (`WebKitGTK` without `GStreamer`'s plugins, or its
/// sandbox refused): the app opens the browser instead.
pub const EXIT_NO_WEB_VIEW: i32 = 3;
pub const EXIT_WEB_VIEW_FAILED: i32 = 4;
/// A start the app never makes: a bad command line or a first command that is not a video.
pub const EXIT_INVALID_START: i32 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    const VIDEO_ID: &str = "dQw4w9WgXcQ";
    const PLAYLIST_ID: &str = "PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI";

    fn request() -> PlayRequest {
        PlayRequest {
            video_id: VIDEO_ID.to_owned(),
            playlist_id: None,
            start_seconds: None,
            is_short: false,
            title: "Never Gonna Give You Up".to_owned(),
        }
    }

    #[test]
    fn the_embed_url_is_the_privacy_enhanced_player_with_its_start_and_playlist() {
        assert_eq!(
            request().embed_url(false),
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1"
        );
        let request = PlayRequest {
            playlist_id: Some(PLAYLIST_ID.to_owned()),
            start_seconds: Some(3723),
            ..request()
        };
        assert_eq!(
            request.embed_url(true),
            format!(
                "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1&start=3723&list={PLAYLIST_ID}&mute=1"
            )
        );
        assert_eq!(
            navigation_verdict(&request.embed_url(false)),
            NavigationVerdict::Allow
        );
    }

    #[test]
    fn only_youtube_ids_pass_validation() {
        assert!(request().validated().is_some());
        for video_id in [
            "dQw4w9WgXc",
            "dQw4w9WgXcQQ",
            "dQw4w9WgX/Q",
            "dQw4w9WgX?Q",
            "dQw4w9WgX&Q",
            "dQw4w9WgX#Q",
            "dQw4w9WgXé",
            "",
        ] {
            let request = PlayRequest {
                video_id: video_id.to_owned(),
                ..request()
            };
            assert_eq!(request.validated(), None, "{video_id}");
        }
        for playlist_id in ["PL12", "RDdQw4w9WgXcQ", "PL&autoplay=0&x=123456"] {
            let request = PlayRequest {
                playlist_id: Some(playlist_id.to_owned()),
                ..request()
            };
            assert_eq!(request.validated(), None, "{playlist_id}");
        }
        let at_zero = PlayRequest {
            start_seconds: Some(0),
            ..request()
        };
        assert_eq!(at_zero.validated(), None);
    }

    #[test]
    fn the_title_is_one_bounded_line() {
        assert_eq!(window_title("  A\nB\u{7}\tC  "), "A B C");
        assert_eq!(
            window_title("\u{202E}evil\u{200B}\u{2066}txt\u{2069}\u{FEFF}\u{200F}"),
            "eviltxt"
        );
        assert_eq!(window_title("\n\r"), "YouTube");
        assert_eq!(
            window_title(&"é".repeat(500)).chars().count(),
            MAXIMUM_TITLE_CHARACTERS
        );
    }

    #[test]
    fn the_web_view_shows_the_player_only() {
        for allowed in [
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
            "about:blank",
            "about:srcdoc",
        ] {
            assert_eq!(
                navigation_verdict(allowed),
                NavigationVerdict::Allow,
                "{allowed}"
            );
        }
        for blocked in [
            "http://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
            "https://www.youtube-nocookie.com:8443/embed/dQw4w9WgXcQ",
            "https://user@www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXc",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ/extra",
            "https://www.youtube-nocookie.com/",
            "https://youtube-nocookie.com.evil.example/embed/dQw4w9WgXcQ",
            "https://evil.example/embed/dQw4w9WgXcQ",
            "https://googleads.g.doubleclick.net/pagead/id",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<p>hi</p>",
            "about:config",
            "not a url",
        ] {
            assert_eq!(
                navigation_verdict(blocked),
                NavigationVerdict::Block,
                "{blocked}"
            );
        }
    }

    #[test]
    fn youtube_pages_the_player_links_to_open_in_the_browser() {
        for link in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ",
        ] {
            assert_eq!(
                navigation_verdict(link),
                NavigationVerdict::OpenInBrowser(link.to_owned()),
                "{link}"
            );
        }
        for refused in [
            "http://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com:444/",
            "https://a:b@www.youtube.com/",
            "https://www.youtube.com.evil.example/",
            "https://accounts.google.com/",
        ] {
            assert_eq!(browser_link(refused), None, "{refused}");
        }
    }

    #[test]
    fn commands_and_events_go_through_one_line_of_json_each() {
        let commands = [
            ParentCommand::Play(request()),
            ParentCommand::KeepOnTop { is_on: true },
            ParentCommand::Close,
        ];
        for command in commands {
            let line = command.to_line();
            assert!(!line.contains('\n'));
            assert_eq!(ParentCommand::from_line(&line), Some(command));
        }
        let events = [
            PlayerEvent::Ready,
            PlayerEvent::Playing {
                video_id: VIDEO_ID.to_owned(),
            },
            PlayerEvent::KeepOnTop { is_on: false },
            PlayerEvent::OpenInBrowser {
                url: "https://www.youtube.com/".to_owned(),
            },
            PlayerEvent::Unavailable,
            PlayerEvent::Closed {
                placement: Some(WindowPlacement {
                    position: Some((-10, 20)),
                    width: 854,
                    height: 480,
                }),
            },
        ];
        for event in events {
            assert_eq!(PlayerEvent::from_line(&event.to_line()), Some(event));
        }
    }

    #[test]
    fn a_play_command_with_a_forged_id_is_refused() {
        let line = r#"{"command":"play","video_id":"../../x?a=b","playlist_id":null,"start_seconds":null,"is_short":false,"title":"x"}"#;
        assert_eq!(ParentCommand::from_line(line), None);
        assert_eq!(ParentCommand::from_line("garbage"), None);
        let titled = format!(
            r#"{{"command":"play","video_id":"{VIDEO_ID}","playlist_id":null,"start_seconds":null,"is_short":false,"title":"a\nb"}}"#
        );
        let Some(ParentCommand::Play(request)) = ParentCommand::from_line(&titled) else {
            panic!("a valid request is refused");
        };
        assert_eq!(request.title, "a b");
    }

    #[test]
    fn launch_options_survive_their_command_line() {
        let options = LaunchOptions {
            data_directory: std::env::temp_dir().join("mellow-player"),
            background: 0x0011_0B11,
            placement: Some(WindowPlacement {
                position: Some((-1920, 40)),
                width: 960,
                height: 540,
            }),
            keep_on_top: true,
        };
        assert_eq!(
            LaunchOptions::from_arguments(options.to_arguments()),
            Ok(options.clone())
        );
        let plain = LaunchOptions {
            placement: None,
            keep_on_top: false,
            ..options
        };
        assert_eq!(
            LaunchOptions::from_arguments(plain.to_arguments()),
            Ok(plain)
        );
    }

    #[test]
    fn launch_options_refuse_what_mellow_never_passes() {
        let parse = |arguments: &[&str]| {
            LaunchOptions::from_arguments(arguments.iter().map(std::ffi::OsString::from))
        };
        let directory = std::env::temp_dir().join("x");
        let directory = directory.to_str().unwrap_or_default();
        assert_eq!(parse(&[]), Err(LaunchOptionsError::MissingDataDirectory));
        assert_eq!(
            parse(&["--data-directory", "relative"]),
            Err(LaunchOptionsError::RelativeDataDirectory)
        );
        assert_eq!(
            parse(&[
                "--data-directory",
                directory,
                "--url",
                "https://evil.example"
            ]),
            Err(LaunchOptionsError::UnknownArgument)
        );
        assert_eq!(
            parse(&["--data-directory", directory, "--background", "red"]),
            Err(LaunchOptionsError::InvalidValue("--background"))
        );
        assert_eq!(
            parse(&["--data-directory", directory, "--placement", "10x10"]),
            Err(LaunchOptionsError::InvalidValue("--placement"))
        );
        assert_eq!(
            parse(&["--data-directory"]),
            Err(LaunchOptionsError::InvalidValue("--data-directory"))
        );
        let long = "x".repeat(MAXIMUM_COMMAND_LINE_BYTES);
        assert_eq!(
            parse(&["--data-directory", &long]),
            Err(LaunchOptionsError::TooLong)
        );
    }

    #[test]
    fn placements_keep_their_text_form_and_bounds() {
        let placement = WindowPlacement {
            position: Some((-5, 7)),
            width: 854,
            height: 480,
        };
        assert_eq!(placement.to_text(), "854x480@-5,7");
        assert_eq!(WindowPlacement::from_text("854x480@-5,7"), Some(placement));
        assert_eq!(
            WindowPlacement::from_text("854x480"),
            Some(WindowPlacement {
                position: None,
                ..placement
            })
        );
        for refused in [
            "",
            "854",
            "854x",
            "100x480",
            "854x480@",
            "854x480@1",
            "99999x480",
        ] {
            assert_eq!(WindowPlacement::from_text(refused), None, "{refused}");
        }
    }

    #[test]
    fn the_window_takes_the_orientation_of_the_video() {
        let landscape = WindowPlacement {
            position: None,
            width: 1280,
            height: 720,
        };
        assert_eq!(
            WindowPlacement::size_for(Some(&landscape), false),
            (1280, 720)
        );
        assert_eq!(
            WindowPlacement::size_for(Some(&landscape), true),
            DEFAULT_SHORT_SIZE
        );
        assert_eq!(WindowPlacement::size_for(None, false), DEFAULT_VIDEO_SIZE);
    }

    #[test]
    fn colours_are_six_hexadecimal_digits() {
        assert_eq!(parse_hex_colour("#110b11"), Some(0x0011_0B11));
        for refused in ["110b11", "#110b1", "#110b11ff", "#gg0000", "#+10b11"] {
            assert_eq!(parse_hex_colour(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_placement_from_the_window_is_checked_like_a_stored_one() {
        let kept = WindowPlacement {
            position: Some((-5, 7)),
            width: 854,
            height: 480,
        };
        assert_eq!(kept.validated(), Some(kept));
        let huge = WindowPlacement {
            width: MAXIMUM_LONG_SIDE + 1,
            ..kept
        };
        assert_eq!(huge.validated(), None);
    }

    #[test]
    fn long_lines_are_skipped_whole_and_never_read_again() {
        let text = format!("short\n{}\nnext\r\nlast", "x".repeat(100));
        // A small buffer: the long line spans several fills.
        let mut reader = std::io::BufReader::with_capacity(8, text.as_bytes());
        let mut read = || read_bounded_line(&mut reader, 16).unwrap();
        assert_eq!(read(), BoundedLine::Line("short".to_owned()));
        assert_eq!(read(), BoundedLine::Refused);
        assert_eq!(read(), BoundedLine::Line("next".to_owned()));
        assert_eq!(read(), BoundedLine::Line("last".to_owned()));
        assert_eq!(read(), BoundedLine::End);
        let mut invalid = std::io::BufReader::new(&b"\xff\xfe\n"[..]);
        assert_eq!(
            read_bounded_line(&mut invalid, 16).unwrap(),
            BoundedLine::Refused
        );
    }

    #[test]
    fn a_window_returns_where_it_was_only_if_it_can_be_grabbed_there() {
        let screens = [
            ScreenArea {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale_factor: 1.0,
            },
            ScreenArea {
                x: 1920,
                y: 0,
                width: 2560,
                height: 1440,
                scale_factor: 2.0,
            },
        ];
        assert_eq!(
            restored_placement(Some((100, 100)), (854, 480), &screens),
            (Some((100, 100)), (854, 480))
        );
        // 40 pixels left on the first screen: not enough to grab.
        assert_eq!(
            restored_placement(Some((-814, 100)), (854, 480), &screens),
            (None, (854, 480))
        );
        // 48 are.
        assert_eq!(
            restored_placement(Some((-806, 100)), (854, 480), &screens).0,
            Some((-806, 100))
        );
        // On a screen that is gone.
        assert_eq!(
            restored_placement(Some((5000, 100)), (854, 480), &screens).0,
            None
        );
        // Larger than its screen: cut to it, in that screen's logical pixels.
        assert_eq!(
            restored_placement(Some((2000, 0)), (3000, 2000), &screens),
            (Some((2000, 0)), (1280, 720))
        );
        assert_eq!(
            restored_placement(None, (3000, 2000), &screens),
            (None, (1920, 1080))
        );
        assert_eq!(
            restored_placement(None, (854, 480), &[]),
            (None, (854, 480))
        );
    }
}
