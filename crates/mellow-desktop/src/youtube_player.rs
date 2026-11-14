//! YouTube videos play in Mellow's own player window (ADR-0061), a process of its own
//! (`mellow-player`, next to this executable) that shows YouTube's privacy-enhanced embedded
//! player, as the web client does in place. One window at most: a click on another video plays it
//! there and brings the window to the front. The window learns the end of the app from its
//! standard input closing, so it never outlives Mellow, killed or not.
//!
//! The card asks [`youtube_player_video`] whether its video is the one playing (to say "Playing in
//! the player window") and [`youtube_player_fallback`] whether the window could not open for it
//! (to say why the browser opened instead). [`set_youtube_player_keep_on_top`] and
//! [`does_youtube_player_keep_on_top`] are the window's "Keep on top", also toggled by Ctrl+T (⌘T)
//! in the window itself. Every change redraws the windows.
//!
//! When the window cannot play (no `mellow-player` beside the app, `WebKitGTK` or `GStreamer`'s
//! plugins missing on Linux or its sandbox refused, `WebView2`'s runtime missing on Windows), the
//! video opens in the browser, as before, and the card says so.
//!
//! When Mellow quits with the window open, the window is asked to close and its place is saved,
//! waiting [`CLOSE_ON_QUIT_TIMEOUT`] at most.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use gpui::App;
use gpui::Global;
use mellow_client_core::shared_logic::link_previews::YouTubeEmbed;
use mellow_player::{
    BoundedLine, EXIT_NO_WEB_VIEW, EXIT_WEB_VIEW_FAILED, LaunchOptions, PLAYER_EXECUTABLE_NAME,
    ParentCommand, PlayRequest, PlayerEvent, WindowPlacement, browser_link, read_bounded_line,
};

use crate::messages::open_message_link;
use crate::preferences::{change_preferences, preferences, save_preferences_now};
use crate::theme::theme;

/// The web view's own folder, under the app's local data (`MELLOW_PROFILE` respected).
const WEB_VIEW_DIRECTORY: &str = "player-web-view";
/// The window says a handful of things per video; more waiting means the app is busy, and the
/// reader thread waits.
const EVENT_QUEUE_LENGTH: usize = 32;
/// A line from the window longer than this is not one of its events.
const MAXIMUM_EVENT_BYTES: usize = 16 * 1024;
/// The window may open one YouTube page in the browser per second: a page clicking links by
/// itself cannot flood the browser with tabs.
const MINIMUM_TIME_BETWEEN_BROWSER_OPENS: Duration = Duration::from_secs(1);
/// How long a quitting Mellow waits for the window to say where it was.
pub const CLOSE_ON_QUIT_TIMEOUT: Duration = Duration::from_millis(100);
/// Asks `WebKitGTK` to run its content processes in bubblewrap even when it would not by default:
/// a hole in the engine then does not reach the user's files, nor Mellow's session.
#[cfg(target_os = "linux")]
const FORCE_SANDBOX_VARIABLE: &str = "WEBKIT_FORCE_SANDBOX";

/// The video a card asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AskedVideo {
    video_id: String,
    /// The link as posted, opened in the browser if the window cannot play it.
    link: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WindowPhase {
    Closed,
    /// Started, not yet showing a web view: an exit now means it could not play.
    Starting {
        generation: u64,
        video: AskedVideo,
    },
    Open {
        generation: u64,
        video: AskedVideo,
    },
}

/// What the app does in answer.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PlayerAction {
    Start {
        generation: u64,
        request: PlayRequest,
    },
    Send(ParentCommand),
    OpenInBrowser(String),
    SavePlacement(WindowPlacement),
    SaveKeepOnTop(bool),
}

/// The window's life as the app sees it, apart from the process, so it can be tested.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PlayerLife {
    phase: WindowPhase,
    /// Bumped by each start, so the messages of a window that already ended are ignored.
    generation: u64,
    keep_on_top: bool,
    /// The video the window last failed to open, which the browser shows instead.
    fallback_video_id: Option<String>,
    /// When the window last had a YouTube page opened in the browser.
    last_browser_open: Option<Instant>,
}

impl PlayerLife {
    fn new(keep_on_top: bool) -> Self {
        Self {
            phase: WindowPhase::Closed,
            generation: 0,
            keep_on_top,
            fallback_video_id: None,
            last_browser_open: None,
        }
    }

    fn play(&mut self, request: PlayRequest, link: String) -> Vec<PlayerAction> {
        self.fallback_video_id = None;
        let video = AskedVideo {
            video_id: request.video_id.clone(),
            link,
        };
        match &mut self.phase {
            WindowPhase::Starting { video: playing, .. }
            | WindowPhase::Open { video: playing, .. } => {
                *playing = video;
                vec![PlayerAction::Send(ParentCommand::Play(request))]
            }
            WindowPhase::Closed => {
                self.generation += 1;
                self.phase = WindowPhase::Starting {
                    generation: self.generation,
                    video,
                };
                vec![PlayerAction::Start {
                    generation: self.generation,
                    request,
                }]
            }
        }
    }

    fn current_generation(&self) -> Option<u64> {
        match &self.phase {
            WindowPhase::Closed => None,
            WindowPhase::Starting { generation, .. } | WindowPhase::Open { generation, .. } => {
                Some(*generation)
            }
        }
    }

    #[cfg(test)]
    fn on_event(&mut self, generation: u64, event: PlayerEvent) -> Vec<PlayerAction> {
        self.on_event_at(generation, event, Instant::now())
    }

    /// `event` from the window of `generation`, received at `now`.
    fn on_event_at(
        &mut self,
        generation: u64,
        event: PlayerEvent,
        now: Instant,
    ) -> Vec<PlayerAction> {
        let is_current = self.current_generation() == Some(generation);
        match event {
            // Saved whichever window it was: it is where the reader left it last. Checked like a
            // stored one, since the window is another process.
            PlayerEvent::Closed {
                placement: Some(placement),
            } => placement
                .validated()
                .map(PlayerAction::SavePlacement)
                .into_iter()
                .collect(),
            _ if !is_current => Vec::new(),
            PlayerEvent::Ready => {
                if let WindowPhase::Starting { generation, video } = &self.phase {
                    self.phase = WindowPhase::Open {
                        generation: *generation,
                        video: video.clone(),
                    };
                }
                Vec::new()
            }
            PlayerEvent::KeepOnTop { is_on } => {
                self.keep_on_top = is_on;
                vec![PlayerAction::SaveKeepOnTop(is_on)]
            }
            PlayerEvent::OpenInBrowser { url } => {
                let is_too_soon = self.last_browser_open.is_some_and(|last| {
                    now.saturating_duration_since(last) < MINIMUM_TIME_BETWEEN_BROWSER_OPENS
                });
                match browser_link(&url) {
                    Some(link) if !is_too_soon => {
                        self.last_browser_open = Some(now);
                        vec![PlayerAction::OpenInBrowser(link)]
                    }
                    _ => Vec::new(),
                }
            }
            PlayerEvent::Unavailable => self.fall_back(),
            PlayerEvent::Playing { .. } | PlayerEvent::Closed { placement: None } => Vec::new(),
        }
    }

    /// `command` could not reach the window, which is closing (the reader closed it just before
    /// clicking): a video it was asked to play opens a new window.
    fn on_lost_window(&mut self, generation: u64, command: ParentCommand) -> Vec<PlayerAction> {
        let replay = match (&self.phase, command) {
            (
                WindowPhase::Open {
                    generation: current,
                    video,
                },
                ParentCommand::Play(request),
            ) if *current == generation => Some((request, video.link.clone())),
            _ => None,
        };
        let actions = self.on_exit(generation, None);
        match replay {
            Some((request, link)) => self.play(request, link),
            None => actions,
        }
    }

    /// The process ended (or could not start, or stopped reading), with `exit_code` when it
    /// exited by itself. A window that never showed its web view, or says by its code that the web
    /// view failed, falls back to the browser, even if its last events were lost.
    fn on_exit(&mut self, generation: u64, exit_code: Option<i32>) -> Vec<PlayerAction> {
        if self.current_generation() != Some(generation) {
            return Vec::new();
        }
        let has_web_view_failed =
            matches!(exit_code, Some(EXIT_NO_WEB_VIEW | EXIT_WEB_VIEW_FAILED));
        if has_web_view_failed || matches!(self.phase, WindowPhase::Starting { .. }) {
            return self.fall_back();
        }
        self.phase = WindowPhase::Closed;
        Vec::new()
    }

    /// The window cannot play: the browser opens the video it was asked for last.
    fn fall_back(&mut self) -> Vec<PlayerAction> {
        let phase = std::mem::replace(&mut self.phase, WindowPhase::Closed);
        match phase {
            WindowPhase::Starting { video, .. } | WindowPhase::Open { video, .. } => {
                self.fallback_video_id = Some(video.video_id);
                vec![PlayerAction::OpenInBrowser(video.link)]
            }
            WindowPhase::Closed => Vec::new(),
        }
    }

    fn link(&self) -> Option<&str> {
        match &self.phase {
            WindowPhase::Closed => None,
            WindowPhase::Starting { video, .. } | WindowPhase::Open { video, .. } => {
                Some(&video.link)
            }
        }
    }

    fn video_id(&self) -> Option<&str> {
        match &self.phase {
            WindowPhase::Closed => None,
            WindowPhase::Starting { video, .. } | WindowPhase::Open { video, .. } => {
                Some(&video.video_id)
            }
        }
    }
}

/// The running window's input, and where its reader thread hands its last placement (for the
/// quit, which cannot wait for the task bringing its messages here).
struct RunningWindow {
    generation: u64,
    input: ChildStdin,
    closed: std::sync::mpsc::Receiver<Option<WindowPlacement>>,
}

struct YouTubePlayer {
    life: PlayerLife,
    window: Option<RunningWindow>,
}

impl Global for YouTubePlayer {}

/// What the window's two watcher threads bring to the app.
#[derive(Debug)]
enum WindowMessage {
    Event(PlayerEvent),
    /// With the exit code, if the process exited by itself.
    Exited(Option<i32>),
}

fn player(cx: &mut App) -> &mut YouTubePlayer {
    if !cx.has_global::<YouTubePlayer>() {
        let keep_on_top =
            preferences(cx).is_some_and(|preferences| preferences.does_youtube_player_keep_on_top);
        cx.set_global(YouTubePlayer {
            life: PlayerLife::new(keep_on_top),
            window: None,
        });
    }
    cx.global_mut::<YouTubePlayer>()
}

/// Plays `video` (from the card of `link`, titled `title`) in the player window, opening it or
/// bringing it to the front. A request YouTube's player could not take opens the link instead.
pub fn play_in_youtube_player(link: &str, video: &YouTubeEmbed, title: &str, cx: &mut App) {
    let request = PlayRequest {
        video_id: video.video_id.clone(),
        playlist_id: video.playlist_id.clone(),
        start_seconds: video.start_seconds,
        is_short: video.is_short,
        title: title.to_owned(),
    }
    .validated();
    let Some(request) = request else {
        open_message_link(link, cx);
        return;
    };
    let actions = player(cx).life.play(request, link.to_owned());
    run(actions, cx);
}

/// The video the player window shows (or is opening with), if it is open.
#[must_use]
pub fn youtube_player_video(cx: &App) -> Option<&str> {
    cx.try_global::<YouTubePlayer>()
        .and_then(|player| player.life.video_id())
}

/// The link (with its start and playlist) the player window shows, if it is open.
#[must_use]
pub fn youtube_player_link(cx: &App) -> Option<&str> {
    cx.try_global::<YouTubePlayer>()
        .and_then(|player| player.life.link())
}

/// Whether the player window could not open for `video_id`, which the browser showed instead.
#[must_use]
pub fn youtube_player_fallback(video_id: &str, cx: &App) -> bool {
    cx.try_global::<YouTubePlayer>()
        .is_some_and(|player| player.life.fallback_video_id.as_deref() == Some(video_id))
}

/// Whether the player window stays above the others (now, or when it next opens).
#[must_use]
pub fn does_youtube_player_keep_on_top(cx: &App) -> bool {
    cx.try_global::<YouTubePlayer>().map_or_else(
        || preferences(cx).is_some_and(|preferences| preferences.does_youtube_player_keep_on_top),
        |player| player.life.keep_on_top,
    )
}

pub fn set_youtube_player_keep_on_top(is_on: bool, cx: &mut App) {
    player(cx).life.keep_on_top = is_on;
    run(
        vec![
            PlayerAction::Send(ParentCommand::KeepOnTop { is_on }),
            PlayerAction::SaveKeepOnTop(is_on),
        ],
        cx,
    );
}

/// Closes the player window, if it is open.
pub fn close_youtube_player(cx: &mut App) {
    if cx.has_global::<YouTubePlayer>() {
        run(vec![PlayerAction::Send(ParentCommand::Close)], cx);
    }
}

/// Closes the window when Mellow quits, however it quits, and saves where it was: the window
/// says so in answer to `close`, read here directly from its reader thread for at most
/// [`CLOSE_ON_QUIT_TIMEOUT`]. Without an answer, the end of the process closes it anyway.
pub fn close_youtube_player_when_the_app_quits(cx: &mut App) {
    cx.on_app_quit(|cx| {
        let window = cx
            .try_global::<YouTubePlayer>()
            .is_some_and(|player| player.window.is_some());
        if window {
            close_youtube_player(cx);
            let placement = cx
                .global::<YouTubePlayer>()
                .window
                .as_ref()
                .and_then(|window| window.closed.recv_timeout(CLOSE_ON_QUIT_TIMEOUT).ok())
                .flatten()
                .and_then(|placement| placement.validated());
            if let Some(placement) = placement {
                change_preferences(cx, move |preferences| {
                    preferences.youtube_player_placement = Some(placement);
                });
                save_preferences_now(cx);
            }
        }
        std::future::ready(())
    })
    .detach();
}

fn run(actions: Vec<PlayerAction>, cx: &mut App) {
    for action in actions {
        match action {
            PlayerAction::Start {
                generation,
                request,
            } => start_window(generation, &request, cx),
            PlayerAction::Send(command) => {
                let player = player(cx);
                let Some(window) = player.window.as_mut() else {
                    continue;
                };
                let generation = window.generation;
                let is_sent = writeln!(window.input, "{}", command.to_line())
                    .and_then(|()| window.input.flush())
                    .is_ok();
                if !is_sent {
                    // It stopped reading: it is ending, or ended.
                    player.window = None;
                    let actions = player.life.on_lost_window(generation, command);
                    run(actions, cx);
                }
            }
            PlayerAction::OpenInBrowser(url) => open_message_link(&url, cx),
            PlayerAction::SavePlacement(placement) => change_preferences(cx, move |preferences| {
                preferences.youtube_player_placement = Some(placement);
            }),
            PlayerAction::SaveKeepOnTop(is_on) => change_preferences(cx, move |preferences| {
                preferences.does_youtube_player_keep_on_top = is_on;
            }),
        }
    }
    cx.refresh_windows();
}

fn web_view_directory() -> Option<PathBuf> {
    Some(
        dirs::data_local_dir()?
            .join(&*crate::profile::with_profile_suffix(
                crate::pictures::disk_cache::APPLICATION_DIRECTORY,
            ))
            .join(WEB_VIEW_DIRECTORY),
    )
}

fn player_executable() -> Option<PathBuf> {
    Some(
        std::env::current_exe()
            .ok()?
            .parent()?
            .join(PLAYER_EXECUTABLE_NAME),
    )
}

fn start_window(generation: u64, request: &PlayRequest, cx: &mut App) {
    let (placement, keep_on_top) = preferences(cx).map_or((None, false), |preferences| {
        (
            preferences.youtube_player_placement,
            preferences.does_youtube_player_keep_on_top,
        )
    });
    let options = web_view_directory().map(|data_directory| LaunchOptions {
        data_directory,
        background: theme(cx).component_link_preview_player_background >> 8,
        placement,
        keep_on_top: keep_on_top || player(cx).life.keep_on_top,
    });
    let started = options
        .zip(player_executable())
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
        .and_then(|(options, executable)| {
            let mut command = Command::new(executable);
            #[cfg(target_os = "linux")]
            command.env(FORCE_SANDBOX_VARIABLE, "1");
            command
                .args(options.to_arguments())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
        });
    let mut child = match started {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!(error = %error.kind(), "cannot start the YouTube player window");
            let actions = player(cx).life.on_exit(generation, None);
            run(actions, cx);
            return;
        }
    };
    let (Some(mut input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
        // Both are piped above.
        let actions = player(cx).life.on_exit(generation, None);
        run(actions, cx);
        return;
    };
    let first = ParentCommand::Play(request.clone()).to_line();
    if let Err(error) = writeln!(input, "{first}").and_then(|()| input.flush()) {
        tracing::warn!(error = %error.kind(), "the YouTube player window stopped at its start");
    }
    let (sender, mut receiver) = tokio::sync::mpsc::channel(EVENT_QUEUE_LENGTH);
    let event_sender = sender.clone();
    let (closed_sender, closed) = std::sync::mpsc::sync_channel(1);
    // The window's output, read line by line within bounds; its end is not the window's end, which
    // is watched apart.
    std::thread::spawn(move || read_window_output(output, &event_sender, &closed_sender));
    std::thread::spawn(move || {
        // Reaps the process; its exit code tells whether its web view failed.
        let exit_code = child.wait().ok().map(exit_code_of);
        let _ = sender.blocking_send(WindowMessage::Exited(exit_code));
    });
    // Detached: it ends with the channel, once both threads have, so the window's last events
    // and its exit always arrive, even after a newer window replaced it.
    cx.spawn(async move |cx| {
        while let Some(message) = receiver.recv().await {
            cx.update(|cx| {
                let player = player(cx);
                let actions = match message {
                    WindowMessage::Event(event) => {
                        player.life.on_event_at(generation, event, Instant::now())
                    }
                    WindowMessage::Exited(exit_code) => {
                        if player
                            .window
                            .as_ref()
                            .is_some_and(|window| window.generation == generation)
                        {
                            player.window = None;
                        }
                        player.life.on_exit(generation, exit_code)
                    }
                };
                run(actions, cx);
            });
        }
    })
    .detach();
    player(cx).window = Some(RunningWindow {
        generation,
        input,
        closed,
    });
}

/// The window's exit code; a window killed by a signal crashed (`WebKitGTK` aborts it when its
/// sandbox cannot start) and counts as a web view that failed, so the browser takes the video.
fn exit_code_of(status: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        if status.signal().is_some() {
            return EXIT_WEB_VIEW_FAILED;
        }
    }
    status.code().unwrap_or(EXIT_WEB_VIEW_FAILED)
}

/// Brings the window's events to the app, and its last placement to a quitting app too.
fn read_window_output(
    output: std::process::ChildStdout,
    events: &tokio::sync::mpsc::Sender<WindowMessage>,
    closed: &std::sync::mpsc::SyncSender<Option<WindowPlacement>>,
) {
    let mut output = std::io::BufReader::new(output);
    loop {
        match read_bounded_line(&mut output, MAXIMUM_EVENT_BYTES) {
            Ok(BoundedLine::End) | Err(_) => return,
            Ok(BoundedLine::Refused) => {}
            Ok(BoundedLine::Line(line)) => {
                let Some(event) = PlayerEvent::from_line(&line) else {
                    continue;
                };
                if let PlayerEvent::Closed { placement } = &event {
                    let _ = closed.try_send(*placement);
                }
                if events.blocking_send(WindowMessage::Event(event)).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINK: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
    const OTHER_LINK: &str = "https://youtu.be/a_b-C1d2E3f";

    fn request(video_id: &str) -> PlayRequest {
        PlayRequest {
            video_id: video_id.to_owned(),
            playlist_id: None,
            start_seconds: None,
            is_short: false,
            title: "A video".to_owned(),
        }
    }

    #[test]
    fn the_first_click_starts_the_window_and_the_next_ones_reuse_it() {
        let mut life = PlayerLife::new(false);
        assert_eq!(
            life.play(request("dQw4w9WgXcQ"), LINK.to_owned()),
            vec![PlayerAction::Start {
                generation: 1,
                request: request("dQw4w9WgXcQ")
            }]
        );
        assert_eq!(life.video_id(), Some("dQw4w9WgXcQ"));
        assert_eq!(life.on_event(1, PlayerEvent::Ready), Vec::new());
        // Already open: the same window plays the other video, nothing new starts.
        assert_eq!(
            life.play(request("a_b-C1d2E3f"), OTHER_LINK.to_owned()),
            vec![PlayerAction::Send(ParentCommand::Play(request(
                "a_b-C1d2E3f"
            )))]
        );
        assert_eq!(life.video_id(), Some("a_b-C1d2E3f"));
        // A click while the window is still starting goes to it as well.
        let mut starting = PlayerLife::new(false);
        starting.play(request("dQw4w9WgXcQ"), LINK.to_owned());
        assert!(matches!(
            starting.play(request("a_b-C1d2E3f"), OTHER_LINK.to_owned())[..],
            [PlayerAction::Send(_)]
        ));
    }

    #[test]
    fn a_window_that_ends_before_its_web_view_falls_back_to_the_browser() {
        let mut life = PlayerLife::new(false);
        life.play(request("dQw4w9WgXcQ"), LINK.to_owned());
        // WebKitGTK missing: the process ends before it says anything.
        assert_eq!(
            life.on_exit(1, None),
            vec![PlayerAction::OpenInBrowser(LINK.to_owned())]
        );
        assert_eq!(life.video_id(), None);
        assert_eq!(life.fallback_video_id.as_deref(), Some("dQw4w9WgXcQ"));
        // The next click tries the window again.
        assert!(matches!(
            life.play(request("dQw4w9WgXcQ"), LINK.to_owned())[..],
            [PlayerAction::Start { generation: 2, .. }]
        ));
        assert_eq!(life.fallback_video_id, None);
    }

    #[test]
    fn a_window_whose_web_view_fails_while_playing_falls_back_once() {
        let mut life = PlayerLife::new(false);
        life.play(request("dQw4w9WgXcQ"), LINK.to_owned());
        life.on_event(1, PlayerEvent::Ready);
        assert_eq!(
            life.on_event(1, PlayerEvent::Unavailable),
            vec![PlayerAction::OpenInBrowser(LINK.to_owned())]
        );
        // Its exit afterwards opens nothing more.
        assert_eq!(life.on_exit(1, None), Vec::new());
    }

    #[test]
    fn a_window_closed_by_the_reader_keeps_its_place_and_opens_nothing() {
        let mut life = PlayerLife::new(false);
        life.play(request("dQw4w9WgXcQ"), LINK.to_owned());
        life.on_event(1, PlayerEvent::Ready);
        let placement = WindowPlacement {
            position: Some((10, 20)),
            width: 854,
            height: 480,
        };
        assert_eq!(
            life.on_event(
                1,
                PlayerEvent::Closed {
                    placement: Some(placement)
                }
            ),
            vec![PlayerAction::SavePlacement(placement)]
        );
        assert_eq!(life.on_exit(1, None), Vec::new());
        assert_eq!(life.video_id(), None);
        assert_eq!(life.fallback_video_id, None);
    }

    #[test]
    fn a_click_reaching_a_closing_window_opens_a_new_one() {
        let mut life = PlayerLife::new(false);
        life.play(request("dQw4w9WgXcQ"), LINK.to_owned());
        life.on_event(1, PlayerEvent::Ready);
        let actions = life.play(request("a_b-C1d2E3f"), OTHER_LINK.to_owned());
        let [PlayerAction::Send(command)] = &actions[..] else {
            panic!("an open window gets the video");
        };
        // The window closed meanwhile and its input is gone.
        assert_eq!(
            life.on_lost_window(1, command.clone()),
            vec![PlayerAction::Start {
                generation: 2,
                request: request("a_b-C1d2E3f")
            }]
        );
        assert_eq!(life.video_id(), Some("a_b-C1d2E3f"));
        // A lost keep-on-top changes nothing more.
        let mut other = PlayerLife::new(false);
        other.play(request("dQw4w9WgXcQ"), LINK.to_owned());
        other.on_event(1, PlayerEvent::Ready);
        assert_eq!(
            other.on_lost_window(1, ParentCommand::KeepOnTop { is_on: true }),
            Vec::new()
        );
        assert_eq!(other.video_id(), None);
    }

    #[test]
    fn the_window_opens_one_page_in_the_browser_per_second_at_most() {
        let mut life = PlayerLife::new(false);
        life.play(request("dQw4w9WgXcQ"), LINK.to_owned());
        let start = Instant::now();
        let open = |life: &mut PlayerLife, after: Duration| {
            life.on_event_at(
                1,
                PlayerEvent::OpenInBrowser {
                    url: LINK.to_owned(),
                },
                start + after,
            )
        };
        assert_eq!(
            open(&mut life, Duration::ZERO),
            vec![PlayerAction::OpenInBrowser(LINK.to_owned())]
        );
        assert_eq!(open(&mut life, Duration::from_millis(999)), Vec::new());
        assert_eq!(
            open(&mut life, Duration::from_millis(1000)),
            vec![PlayerAction::OpenInBrowser(LINK.to_owned())]
        );
    }

    #[test]
    fn a_placement_out_of_bounds_from_the_window_is_not_saved() {
        let mut life = PlayerLife::new(false);
        life.play(request("dQw4w9WgXcQ"), LINK.to_owned());
        let huge = WindowPlacement {
            position: Some((0, 0)),
            width: 100_000,
            height: 480,
        };
        assert_eq!(
            life.on_event(
                1,
                PlayerEvent::Closed {
                    placement: Some(huge)
                }
            ),
            Vec::new()
        );
    }

    #[test]
    fn an_exit_code_saying_the_web_view_failed_falls_back_even_when_open() {
        // Its `unavailable` event was lost: the exit code still says it.
        for code in [EXIT_NO_WEB_VIEW, EXIT_WEB_VIEW_FAILED] {
            let mut life = PlayerLife::new(false);
            life.play(request("dQw4w9WgXcQ"), LINK.to_owned());
            life.on_event(1, PlayerEvent::Ready);
            assert_eq!(
                life.on_exit(1, Some(code)),
                vec![PlayerAction::OpenInBrowser(LINK.to_owned())],
                "{code}"
            );
        }
        let mut closed = PlayerLife::new(false);
        closed.play(request("dQw4w9WgXcQ"), LINK.to_owned());
        closed.on_event(1, PlayerEvent::Ready);
        assert_eq!(closed.on_exit(1, Some(0)), Vec::new());
    }

    #[cfg(unix)]
    #[test]
    fn a_window_killed_by_a_signal_counts_as_a_failed_web_view() {
        let crashed = Command::new("sh")
            .args(["-c", "kill -ABRT $$"])
            .status()
            .unwrap();
        assert_eq!(exit_code_of(crashed), EXIT_WEB_VIEW_FAILED);
        let closed = Command::new("sh").args(["-c", "exit 0"]).status().unwrap();
        assert_eq!(exit_code_of(closed), 0);
    }

    #[test]
    fn messages_of_an_ended_window_change_nothing() {
        let mut life = PlayerLife::new(false);
        life.play(request("dQw4w9WgXcQ"), LINK.to_owned());
        life.on_event(1, PlayerEvent::Ready);
        life.on_exit(1, None);
        life.play(request("a_b-C1d2E3f"), OTHER_LINK.to_owned());
        assert_eq!(life.on_event(1, PlayerEvent::Unavailable), Vec::new());
        assert_eq!(life.on_exit(1, None), Vec::new());
        assert_eq!(life.video_id(), Some("a_b-C1d2E3f"));
    }

    #[test]
    fn the_window_toggles_keep_on_top_and_opens_only_youtube_pages() {
        let mut life = PlayerLife::new(false);
        life.play(request("dQw4w9WgXcQ"), LINK.to_owned());
        assert_eq!(
            life.on_event(1, PlayerEvent::KeepOnTop { is_on: true }),
            vec![PlayerAction::SaveKeepOnTop(true)]
        );
        assert!(life.keep_on_top);
        assert_eq!(
            life.on_event(
                1,
                PlayerEvent::OpenInBrowser {
                    url: LINK.to_owned()
                }
            ),
            vec![PlayerAction::OpenInBrowser(LINK.to_owned())]
        );
        for refused in [
            "https://evil.example/",
            "file:///etc/passwd",
            "javascript:x",
        ] {
            assert_eq!(
                life.on_event(
                    1,
                    PlayerEvent::OpenInBrowser {
                        url: refused.to_owned()
                    }
                ),
                Vec::new(),
                "{refused}"
            );
        }
    }
}
