//! `mellow-player`: the desktop app's YouTube player window (ADR-0061). Started by the app only,
//! never by hand: see the library for the command line and the messages. One window, one web
//! view showing `https://www.youtube-nocookie.com/embed/<id>` and nothing else; no cookie or
//! storage kept between runs, no developer tools, no message channel from the page, no download,
//! no permission granted. Escape closes the window, Ctrl+T (⌘T on macOS) keeps it on top.
// The window is a GUI app: no console opens behind it on Windows.
#![windows_subsystem = "windows"]

use std::io::Write as _;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use mellow_player::{
    BoundedLine, EXIT_INVALID_START, EXIT_NO_WEB_VIEW, EXIT_WEB_VIEW_FAILED, LaunchOptions,
    MINIMUM_SHORT_SIDE, NavigationVerdict, ParentCommand, PlayRequest, PlayerEvent, ScreenArea,
    WindowPlacement, browser_link, navigation_verdict, read_bounded_line, restored_placement,
};
use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{ElementState, Event, KeyEvent, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy};
use tao::keyboard::{Key, KeyCode, ModifiersState};
use tao::window::{Icon, Window, WindowBuilder};
use wry::http::HeaderMap;
use wry::{NewWindowResponse, PermissionResponse, WebContext, WebView, WebViewBuilder};

/// The app's icon, as the web app's.
const ICON_PNG: &[u8] = include_bytes!("../../../web/public/icons/mellow-192.png");
/// The app's identifier (its macOS bundle's, its Linux desktop entry's) as an origin.
const APP_REFERRER: &str = "https://org.mellow.desktop/";
/// A command line from the app is a few hundred bytes; a longer one is not the app's.
const MAXIMUM_COMMAND_BYTES: usize = 16 * 1024;
/// Debug builds only: plays every video muted, so a test run never makes a sound.
const MUTED_FOR_TESTS_VARIABLE: &str = "MELLOW_PLAYER_MUTED";
/// `WebKitGTK` hands a key the page left alone back to the window, so tao sees it a second time a
/// few milliseconds later; no one presses a shortcut twice that fast.
const SAME_KEY_PRESS_WINDOW: std::time::Duration = std::time::Duration::from_millis(150);

#[derive(Debug)]
enum PlayerMessage {
    Command(ParentCommand),
    /// The app's end of the standard input closed: the app quit, crashed or was killed.
    ParentGone,
    /// Escape or the keep-on-top shortcut, caught by the web view's own window (Windows).
    #[cfg_attr(not(windows), allow(dead_code))]
    Shortcut(Shortcut),
    /// The page entered or left full screen (Windows: the window follows it).
    #[cfg_attr(not(windows), allow(dead_code))]
    PageFullScreen(bool),
    /// The web view's content process ended: nothing will play any more (Linux).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    WebViewFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shortcut {
    Close,
    ToggleKeepOnTop,
}

/// Where the events go: a private copy of the standard output, which the web engine's helper
/// processes do not inherit (Linux, macOS), so nothing but this process writes the protocol.
static EVENTS: OnceLock<Mutex<Box<dyn std::io::Write + Send>>> = OnceLock::new();

fn main() -> ExitCode {
    let options = match LaunchOptions::from_arguments(std::env::args_os().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            report(&format!("mellow-player: {error}"));
            return exit_code(EXIT_INVALID_START);
        }
    };
    keep_the_protocol_private();
    let mut input = std::io::BufReader::new(std::io::stdin());
    // The app writes the first video right after starting the window.
    let Some(ParentCommand::Play(first)) = read_command(&mut input) else {
        report("mellow-player: the first command must play a video");
        return exit_code(EXIT_INVALID_START);
    };
    let event_loop = EventLoopBuilder::<PlayerMessage>::with_user_event().build();
    let player = match Player::open(&options, &first, &event_loop) {
        Ok(player) => player,
        Err(problem) => {
            report(&format!("mellow-player: {problem}"));
            send(&PlayerEvent::Unavailable);
            return exit_code(EXIT_NO_WEB_VIEW);
        }
    };
    send(&PlayerEvent::Ready);
    send(&PlayerEvent::Playing {
        video_id: first.video_id,
    });
    let proxy = event_loop.create_proxy();
    std::thread::spawn(move || read_commands(&mut input, &proxy));
    run(event_loop, player)
}

fn exit_code(code: i32) -> ExitCode {
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

/// Gives the protocol a private, close-on-exec copy of the standard output, then points the
/// standard output at the standard error, which the web engine's processes inherit instead.
#[cfg(unix)]
#[allow(unsafe_code)] // dup2 has no safe std equivalent.
fn keep_the_protocol_private() {
    use std::os::fd::AsFd as _;
    let private = std::io::stdout()
        .as_fd()
        .try_clone_to_owned()
        .map(std::fs::File::from);
    match private {
        Ok(private) => {
            let _ = EVENTS.set(Mutex::new(Box::new(private)));
            // SAFETY: both descriptors are this process's standard ones, open for its lifetime.
            if unsafe { libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO) } < 0 {
                report("mellow-player: the standard output stays shared");
            }
        }
        Err(error) => report(&format!("mellow-player: no private output: {error}")),
    }
}

/// `WebView2`'s processes are started by its runtime, not by this one, and inherit nothing.
#[cfg(not(unix))]
fn keep_the_protocol_private() {}

fn read_command<R: std::io::BufRead>(reader: &mut R) -> Option<ParentCommand> {
    loop {
        match read_bounded_line(reader, MAXIMUM_COMMAND_BYTES) {
            Ok(BoundedLine::Line(line)) => match ParentCommand::from_line(&line) {
                Some(command) => return Some(command),
                None => report("mellow-player: a command was refused"),
            },
            Ok(BoundedLine::Refused) => report("mellow-player: a command was refused"),
            Ok(BoundedLine::End) | Err(_) => return None,
        }
    }
}

fn read_commands<R: std::io::BufRead>(input: &mut R, proxy: &EventLoopProxy<PlayerMessage>) {
    while let Some(command) = read_command(input) {
        if proxy.send_event(PlayerMessage::Command(command)).is_err() {
            return;
        }
    }
    let _ = proxy.send_event(PlayerMessage::ParentGone);
}

fn send(event: &PlayerEvent) {
    let line = event.to_line();
    // A closed output means the app is gone; the end of the input closes the window then.
    if let Some(events) = EVENTS.get() {
        let mut events = events.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = writeln!(events, "{line}").and_then(|()| events.flush());
    } else {
        let mut output = std::io::stdout().lock();
        let _ = writeln!(output, "{line}").and_then(|()| output.flush());
    }
}

/// Goes to the standard error the app passes on to its own (the terminal it runs in, or the
/// system's journal); never a title, an address or anything the reader chose.
#[allow(clippy::print_stderr)]
fn report(message: &str) {
    eprintln!("{message}");
}

fn is_muted_for_tests() -> bool {
    cfg!(debug_assertions) && std::env::var_os(MUTED_FOR_TESTS_VARIABLE).is_some()
}

/// YouTube's embedded player refuses to play without a referrer (its error 153), and asks apps
/// to name themselves by their identifier as an HTTPS origin; the web view, which loads the
/// player as its top document, sends none of its own.
fn embed_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        wry::http::header::REFERER,
        wry::http::HeaderValue::from_static(APP_REFERRER),
    );
    headers
}

/// The address to load for `request`, or `None` when the window already shows it: a second click
/// on the same card (or a double click) brings the window forward without starting over.
fn page_to_load(loaded: &str, request: &PlayRequest, is_muted: bool) -> Option<String> {
    let wanted = request.embed_url(is_muted);
    (wanted != loaded).then_some(wanted)
}

fn background_rgba(background: u32) -> (u8, u8, u8, u8) {
    let [_, red, green, blue] = background.to_be_bytes();
    (red, green, blue, u8::MAX)
}

fn window_icon() -> Option<Icon> {
    let picture = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let (width, height) = picture.dimensions();
    Icon::from_rgba(picture.into_raw(), width, height).ok()
}

fn screens(event_loop: &EventLoop<PlayerMessage>) -> Vec<ScreenArea> {
    event_loop
        .available_monitors()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            ScreenArea {
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
                scale_factor: monitor.scale_factor(),
            }
        })
        .collect()
}

/// The window and its web view.
// Independent states of the window, each its own flag; no state machine hides in them.
#[allow(clippy::struct_excessive_bools)]
struct Player {
    window: Window,
    webview: WebView,
    loaded_url: String,
    is_short: bool,
    keep_on_top: bool,
    is_muted: bool,
    modifiers: ModifiersState,
    /// The last shortcut pressed on the keyboard, and when.
    last_key_shortcut: Option<(Shortcut, std::time::Instant)>,
    /// The page is full screen (Linux and Windows tell; see `is_full_screen` for macOS).
    is_page_full_screen: Arc<AtomicBool>,
    has_reported_close: bool,
}

impl Player {
    fn open(
        options: &LaunchOptions,
        first: &PlayRequest,
        event_loop: &EventLoop<PlayerMessage>,
    ) -> Result<Self, String> {
        let background = background_rgba(options.background);
        let wanted = WindowPlacement::size_for(options.placement.as_ref(), first.is_short);
        let (position, (width, height)) = restored_placement(
            options.placement.and_then(|placement| placement.position),
            wanted,
            &screens(event_loop),
        );
        let mut window_builder = WindowBuilder::new()
            .with_title(&first.title)
            .with_inner_size(LogicalSize::new(width, height))
            .with_min_inner_size(LogicalSize::new(MINIMUM_SHORT_SIDE, MINIMUM_SHORT_SIDE))
            .with_always_on_top(options.keep_on_top)
            .with_window_icon(window_icon())
            .with_background_color(background);
        if let Some((x, y)) = position {
            window_builder = window_builder.with_position(PhysicalPosition::new(x, y));
        }
        let window = window_builder
            .build(event_loop)
            .map_err(|error| format!("cannot open the window: {error}"))?;
        let is_muted = is_muted_for_tests();
        // Incognito: nothing the page stores outlives the window. The context's folder only
        // holds what the engine itself needs to run, inside the app's profile.
        let mut web_context = WebContext::new(Some(options.data_directory.clone()));
        let loaded_url = first.embed_url(is_muted);
        let webview = build_web_view(&window, &mut web_context, &loaded_url, background)
            .map_err(|error| format!("no web view: {error}"))?;
        let is_page_full_screen = Arc::new(AtomicBool::new(false));
        #[cfg(windows)]
        forward_web_view_events(
            &webview,
            event_loop.create_proxy(),
            Arc::clone(&is_page_full_screen),
        );
        #[cfg(target_os = "linux")]
        watch_web_view(
            &webview,
            event_loop.create_proxy(),
            Arc::clone(&is_page_full_screen),
        );
        Ok(Self {
            window,
            webview,
            loaded_url,
            is_short: first.is_short,
            keep_on_top: options.keep_on_top,
            is_muted,
            modifiers: ModifiersState::empty(),
            last_key_shortcut: None,
            is_page_full_screen,
            has_reported_close: false,
        })
    }

    fn play(&mut self, request: PlayRequest) {
        if let Some(url) = page_to_load(&self.loaded_url, &request, self.is_muted) {
            if request.is_short != self.is_short {
                let (width, height) = WindowPlacement::size_for(None, request.is_short);
                self.window.set_inner_size(LogicalSize::new(width, height));
                self.is_short = request.is_short;
            }
            self.window.set_title(&request.title);
            match self.webview.load_url_with_headers(&url, embed_headers()) {
                Ok(()) => self.loaded_url = url,
                Err(error) => report(&format!("mellow-player: cannot load the video: {error}")),
            }
        }
        self.window.set_minimized(false);
        self.window.set_visible(true);
        self.window.set_focus();
        send(&PlayerEvent::Playing {
            video_id: request.video_id,
        });
    }

    fn set_keep_on_top(&mut self, is_on: bool) {
        self.keep_on_top = is_on;
        self.window.set_always_on_top(is_on);
        send(&PlayerEvent::KeepOnTop { is_on });
    }

    /// Escape then leaves full screen, as in a browser, rather than closing the window. macOS
    /// tells nothing: there the window filling its screen stands for it.
    fn is_full_screen(&self) -> bool {
        self.is_page_full_screen.load(Ordering::Relaxed)
            || self.window.fullscreen().is_some()
            || (cfg!(target_os = "macos")
                && self
                    .window
                    .current_monitor()
                    .is_some_and(|monitor| self.window.outer_size() == monitor.size()))
    }

    /// Where the window is, unless it is maximised, minimised or full screen: then the place it
    /// had before is the one to keep.
    fn placement(&self) -> Option<WindowPlacement> {
        if self.window.is_maximized() || self.window.is_minimized() || self.is_full_screen() {
            return None;
        }
        let size: LogicalSize<u32> = self
            .window
            .inner_size()
            .to_logical(self.window.scale_factor());
        WindowPlacement {
            position: self
                .window
                .outer_position()
                .ok()
                .map(|position| (position.x, position.y)),
            width: size.width,
            height: size.height,
        }
        .validated()
    }

    /// Tells the app the window closes, once, with its placement.
    fn report_close(&mut self) {
        if !self.has_reported_close {
            self.has_reported_close = true;
            send(&PlayerEvent::Closed {
                placement: self.placement(),
            });
        }
    }

    fn close(&mut self, control_flow: &mut ControlFlow) {
        self.report_close();
        *control_flow = ControlFlow::Exit;
    }

    fn set_page_full_screen(&mut self, is_full_screen: bool) {
        self.is_page_full_screen
            .store(is_full_screen, Ordering::Relaxed);
        self.window
            .set_fullscreen(is_full_screen.then_some(tao::window::Fullscreen::Borderless(None)));
    }

    /// Applies a message or a window event; the shortcut it stands for, if any.
    fn handle(
        &mut self,
        event: Event<'_, PlayerMessage>,
        control_flow: &mut ControlFlow,
    ) -> Option<Shortcut> {
        match event {
            Event::UserEvent(PlayerMessage::Command(command)) => match command {
                ParentCommand::Play(request) => self.play(request),
                ParentCommand::KeepOnTop { is_on } => self.set_keep_on_top(is_on),
                ParentCommand::Close => self.close(control_flow),
            },
            Event::UserEvent(PlayerMessage::ParentGone) => self.close(control_flow),
            Event::UserEvent(PlayerMessage::WebViewFailed) => {
                report("mellow-player: the web view's content process ended");
                send(&PlayerEvent::Unavailable);
                self.report_close();
                *control_flow = ControlFlow::ExitWithCode(EXIT_WEB_VIEW_FAILED);
            }
            Event::UserEvent(PlayerMessage::PageFullScreen(is_full_screen)) => {
                self.set_page_full_screen(is_full_screen);
            }
            Event::UserEvent(PlayerMessage::Shortcut(shortcut)) => return Some(shortcut),
            Event::LoopDestroyed => self.report_close(),
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => self.close(control_flow),
                WindowEvent::ModifiersChanged(state) => self.modifiers = state,
                WindowEvent::KeyboardInput {
                    event:
                        KeyEvent {
                            logical_key,
                            physical_key,
                            state: ElementState::Pressed,
                            repeat: false,
                            ..
                        },
                    ..
                } => {
                    let shortcut = shortcut_of(&logical_key, physical_key, self.modifiers)?;
                    let now = std::time::Instant::now();
                    let is_echo = self.last_key_shortcut.is_some_and(|(last, at)| {
                        last == shortcut && now.duration_since(at) < SAME_KEY_PRESS_WINDOW
                    });
                    self.last_key_shortcut = Some((shortcut, now));
                    return (!is_echo).then_some(shortcut);
                }
                _ => {}
            },
            _ => {}
        }
        None
    }
}

fn run(event_loop: EventLoop<PlayerMessage>, mut player: Player) -> ! {
    event_loop.run(move |event, _, control_flow| {
        if !matches!(control_flow, ControlFlow::ExitWithCode(_)) {
            *control_flow = ControlFlow::Wait;
        }
        match player.handle(event, control_flow) {
            Some(Shortcut::Close) if !player.is_full_screen() => player.close(control_flow),
            Some(Shortcut::ToggleKeepOnTop) => player.set_keep_on_top(!player.keep_on_top),
            _ => {}
        }
    })
}

fn shortcut_of(
    key: &Key<'_>,
    physical_key: KeyCode,
    modifiers: ModifiersState,
) -> Option<Shortcut> {
    let command_key = if cfg!(target_os = "macos") {
        modifiers.super_key()
    } else {
        modifiers.control_key()
    };
    match key {
        Key::Escape => Some(Shortcut::Close),
        _ if physical_key == KeyCode::KeyT && command_key && !modifiers.alt_key() => {
            Some(Shortcut::ToggleKeepOnTop)
        }
        _ => None,
    }
}

fn build_web_view(
    window: &Window,
    web_context: &mut WebContext,
    url: &str,
    background: (u8, u8, u8, u8),
) -> wry::Result<WebView> {
    let builder = WebViewBuilder::new_with_web_context(web_context)
        .with_url_and_headers(url, embed_headers())
        .with_incognito(true)
        .with_devtools(false)
        .with_autoplay(true)
        .with_hotkeys_zoom(false)
        .with_background_color(background)
        .with_navigation_handler(|address| {
            let verdict = navigation_verdict(&address);
            if cfg!(debug_assertions) {
                report(&format!("mellow-player: navigation {verdict:?} {address}"));
            }
            match verdict {
                NavigationVerdict::Allow => true,
                NavigationVerdict::OpenInBrowser(url) => {
                    send(&PlayerEvent::OpenInBrowser { url });
                    false
                }
                NavigationVerdict::Block => false,
            }
        })
        .with_new_window_req_handler(|address, _| {
            if let Some(url) = browser_link(&address) {
                send(&PlayerEvent::OpenInBrowser { url });
            }
            NewWindowResponse::Deny
        })
        .with_download_started_handler(|_, _| false)
        .with_permission_handler(|_| PermissionResponse::Deny);
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix as _;
        use wry::WebViewBuilderExtUnix as _;
        let container = window.default_vbox().ok_or(wry::Error::MessageSender)?;
        builder.build_gtk(container)
    }
    #[cfg(not(target_os = "linux"))]
    builder.build(window)
}

/// `WebKitGTK`'s content process ends when `GStreamer`'s base and good plugins are missing (no
/// `autoaudiosink`) or its sandbox cannot start, and leaves a black window: the app is told, and
/// opens the browser instead. Its full screen is followed so Escape leaves it.
#[cfg(target_os = "linux")]
fn watch_web_view(
    webview: &WebView,
    proxy: EventLoopProxy<PlayerMessage>,
    is_page_full_screen: Arc<AtomicBool>,
) {
    use webkit2gtk::WebViewExt as _;
    use wry::WebViewExtUnix as _;
    let view = webview.webview();
    view.connect_web_process_terminated(move |_, _| {
        let _ = proxy.send_event(PlayerMessage::WebViewFailed);
    });
    let entered = Arc::clone(&is_page_full_screen);
    // `false`: WebKitGTK still makes the window full screen itself.
    view.connect_enter_fullscreen(move |_| {
        entered.store(true, Ordering::Relaxed);
        false
    });
    view.connect_leave_fullscreen(move |_| {
        is_page_full_screen.store(false, Ordering::Relaxed);
        false
    });
}

/// `WebView2` runs in a window of its own, which keeps the keys from tao: its accelerator event
/// hands Escape and Ctrl+T over, except Escape while the page is full screen, which leaves it.
/// Its full screen only fills the web view: the window follows it.
#[cfg(windows)]
#[allow(unsafe_code)] // WebView2's COM calls; each pointer comes from WebView2 itself.
fn forward_web_view_events(
    webview: &WebView,
    proxy: EventLoopProxy<PlayerMessage>,
    is_page_full_screen: Arc<AtomicBool>,
) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
        COREWEBVIEW2_PHYSICAL_KEY_STATUS,
    };
    use webview2_com::{
        AcceleratorKeyPressedEventHandler, ContainsFullScreenElementChangedEventHandler,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_T,
    };
    use wry::WebViewExtWindows as _;

    let full_screen_proxy = proxy.clone();
    let is_full_screen_for_keys = Arc::clone(&is_page_full_screen);
    let keys = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, arguments| {
        let Some(arguments) = arguments else {
            return Ok(());
        };
        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
        let mut key = 0_u32;
        let mut status = COREWEBVIEW2_PHYSICAL_KEY_STATUS::default();
        // SAFETY: `arguments` is the live event's, and the out-pointers are locals.
        unsafe {
            arguments.KeyEventKind(&raw mut kind)?;
            arguments.VirtualKey(&raw mut key)?;
            arguments.PhysicalKeyStatus(&raw mut status)?;
        }
        // A key held down repeats: one press, one shortcut.
        if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN || status.WasKeyDown.as_bool() {
            return Ok(());
        }
        // SAFETY: reads this thread's keyboard state, which the event was raised on.
        let is_down = |virtual_key: u16| unsafe { GetKeyState(i32::from(virtual_key)) } < 0;
        let shortcut = if key == u32::from(VK_ESCAPE.0) {
            (!is_full_screen_for_keys.load(Ordering::Relaxed)).then_some(Shortcut::Close)
        } else if key == u32::from(VK_T.0) && is_down(VK_CONTROL.0) && !is_down(VK_MENU.0) {
            Some(Shortcut::ToggleKeepOnTop)
        } else {
            None
        };
        if let Some(shortcut) = shortcut {
            let _ = proxy.send_event(PlayerMessage::Shortcut(shortcut));
            // SAFETY: as above.
            unsafe { arguments.SetHandled(true)? };
        }
        Ok(())
    }));
    let full_screen =
        ContainsFullScreenElementChangedEventHandler::create(Box::new(move |core, _| {
            let Some(core) = core else {
                return Ok(());
            };
            let mut contains = windows::core::BOOL::default();
            // SAFETY: `core` is the event's web view, and the out-pointer is a local.
            unsafe { core.ContainsFullScreenElement(&raw mut contains)? };
            is_page_full_screen.store(contains.as_bool(), Ordering::Relaxed);
            let _ = full_screen_proxy.send_event(PlayerMessage::PageFullScreen(contains.as_bool()));
            Ok(())
        }));
    let mut token = 0_i64;
    // SAFETY: the controller and the web view are this window's, alive for the whole process.
    let registered = unsafe {
        webview
            .controller()
            .add_AcceleratorKeyPressed(&keys, &raw mut token)
            .and_then(|()| {
                webview
                    .webview()
                    .add_ContainsFullScreenElementChanged(&full_screen, &raw mut token)
            })
    };
    if let Err(error) = registered {
        report(&format!("mellow-player: no keyboard shortcuts: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(video_id: &str) -> PlayRequest {
        PlayRequest {
            video_id: video_id.to_owned(),
            playlist_id: None,
            start_seconds: None,
            is_short: false,
            title: "A".to_owned(),
        }
    }

    #[test]
    fn the_input_gives_commands_and_skips_the_rest() {
        let play = ParentCommand::Play(request("dQw4w9WgXcQ"));
        let too_long = "x".repeat(MAXIMUM_COMMAND_BYTES * 2);
        let text = format!(
            "{}\nnot json\n{too_long}\n{}\n",
            play.to_line(),
            ParentCommand::Close.to_line()
        );
        let mut input = std::io::Cursor::new(text);
        assert_eq!(read_command(&mut input), Some(play));
        assert_eq!(read_command(&mut input), Some(ParentCommand::Close));
        assert_eq!(read_command(&mut input), None);
    }

    #[test]
    fn the_video_already_shown_is_not_loaded_again() {
        let shown = request("dQw4w9WgXcQ");
        let loaded = shown.embed_url(false);
        assert_eq!(page_to_load(&loaded, &shown, false), None);
        assert_eq!(
            page_to_load(&loaded, &request("a_b-C1d2E3f"), false),
            Some(request("a_b-C1d2E3f").embed_url(false))
        );
        let later = PlayRequest {
            start_seconds: Some(30),
            ..shown
        };
        assert_eq!(
            page_to_load(&loaded, &later, false),
            Some(later.embed_url(false))
        );
    }

    #[test]
    fn escape_closes_and_the_command_key_with_t_keeps_on_top() {
        assert!(matches!(
            shortcut_of(&Key::Escape, KeyCode::Escape, ModifiersState::empty()),
            Some(Shortcut::Close)
        ));
        let command_key = if cfg!(target_os = "macos") {
            ModifiersState::SUPER
        } else {
            ModifiersState::CONTROL
        };
        assert!(matches!(
            shortcut_of(&Key::Character("t"), KeyCode::KeyT, command_key),
            Some(Shortcut::ToggleKeepOnTop)
        ));
        assert!(
            shortcut_of(&Key::Character("t"), KeyCode::KeyT, ModifiersState::empty()).is_none()
        );
        assert!(
            shortcut_of(
                &Key::Character("t"),
                KeyCode::KeyT,
                command_key | ModifiersState::ALT
            )
            .is_none()
        );
    }
}
