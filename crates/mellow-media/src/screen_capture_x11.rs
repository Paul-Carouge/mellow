//! Screen sharing on an X11 session (ADR-0054): an in-app picker of the RandR monitors and the
//! managed windows (`_NET_CLIENT_LIST`, Mellow's own left out by `_NET_WM_PID`), and pictures
//! read with MIT-SHM `GetImage` into one shared segment reused for every picture (plain
//! `GetImage` without MIT-SHM), at 30 frames per second, encoded in VP8. `x11rb` speaks the
//! protocol in pure Rust; the shared segment is System V shared memory through `libc`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shm::ConnectionExt as _;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, ImageFormat, MapState, Window};
use x11rb::rust_connection::RustConnection;

use crate::screen_capture::{
    RunningScreenCapture, ScreenCaptureError, ScreenChoice, ScreenShareContext,
    ScreenSourceDescription, ScreenSourceKind, ShareSound, ShareSoundState, pack_bgra_rows,
    thumbnail_of,
};
use crate::screen_capture_linux::{RunningShareSound, binary_of_process, start_sound_for};
use crate::session::ScreenShareEnd;
use crate::simulcast::FRAMES_PER_SECOND;
use crate::video_frame::CapturedFrame;
use crate::video_sender::VideoInput;

const MONITOR_ID_PREFIX: &str = "monitor:";
const WINDOW_ID_PREFIX: &str = "window:";
/// Every bit of every pixel.
const ALL_PLANES: u32 = u32::MAX;
const BYTES_PER_PIXEL: usize = 4;
/// The depths whose `ZPixmap` pictures are 32-bit BGRX on every X server Mellow meets.
const SUPPORTED_DEPTHS: [u8; 2] = [24, 32];

fn system_error(error: impl std::fmt::Display) -> ScreenCaptureError {
    ScreenCaptureError::System(error.to_string())
}

fn connect() -> Result<(RustConnection, Window), ScreenCaptureError> {
    let (connection, screen) = x11rb::connect(None).map_err(system_error)?;
    let root = connection
        .setup()
        .roots
        .get(screen)
        .map(|screen| screen.root)
        .ok_or(ScreenCaptureError::Unavailable)?;
    Ok((connection, root))
}

/// The atoms the window list needs.
struct Atoms {
    client_list: Atom,
    window_name: Atom,
    utf8_string: Atom,
    window_pid: Atom,
}

impl Atoms {
    fn intern(connection: &RustConnection) -> Result<Self, ScreenCaptureError> {
        let intern = |name: &[u8]| -> Result<Atom, ScreenCaptureError> {
            Ok(connection
                .intern_atom(false, name)
                .map_err(system_error)?
                .reply()
                .map_err(system_error)?
                .atom)
        };
        Ok(Self {
            client_list: intern(b"_NET_CLIENT_LIST")?,
            window_name: intern(b"_NET_WM_NAME")?,
            utf8_string: intern(b"UTF8_STRING")?,
            window_pid: intern(b"_NET_WM_PID")?,
        })
    }
}

/// A rectangle of a drawable to capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Area {
    x: i16,
    y: i16,
    width: u16,
    height: u16,
}

/// What a source id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Monitor(Area),
    Window(Window),
}

fn parse_source(id: &str) -> Option<Source> {
    if let Some(window) = id.strip_prefix(WINDOW_ID_PREFIX) {
        return window.parse().ok().map(Source::Window);
    }
    let mut numbers = id.strip_prefix(MONITOR_ID_PREFIX)?.split(',');
    let mut next = || numbers.next()?.parse::<i32>().ok();
    let area = Area {
        x: i16::try_from(next()?).ok()?,
        y: i16::try_from(next()?).ok()?,
        width: u16::try_from(next()?).ok()?,
        height: u16::try_from(next()?).ok()?,
    };
    Some(Source::Monitor(area))
}

fn monitor_id(area: Area) -> String {
    format!(
        "{MONITOR_ID_PREFIX}{},{},{},{}",
        area.x, area.y, area.width, area.height
    )
}

/// The monitors as RandR lists them, or the whole root window without RandR.
fn monitors(
    connection: &RustConnection,
    root: Window,
) -> Result<Vec<(Area, String)>, ScreenCaptureError> {
    let listed = connection
        .randr_get_monitors(root, true)
        .ok()
        .and_then(|cookie| cookie.reply().ok());
    if let Some(listed) = listed {
        return Ok(listed
            .monitors
            .iter()
            .map(|monitor| {
                let name = connection
                    .get_atom_name(monitor.name)
                    .ok()
                    .and_then(|cookie| cookie.reply().ok())
                    .map(|reply| String::from_utf8_lossy(&reply.name).into_owned())
                    .unwrap_or_default();
                let area = Area {
                    x: monitor.x,
                    y: monitor.y,
                    width: monitor.width,
                    height: monitor.height,
                };
                (area, name)
            })
            .collect());
    }
    let geometry = connection
        .get_geometry(root)
        .map_err(system_error)?
        .reply()
        .map_err(system_error)?;
    Ok(vec![(
        Area {
            x: 0,
            y: 0,
            width: geometry.width,
            height: geometry.height,
        },
        String::new(),
    )])
}

/// A managed window worth offering: viewable, titled, of another process.
struct WindowDescription {
    window: Window,
    title: String,
    application_name: Option<String>,
    process: Option<u32>,
    width: u16,
    height: u16,
}

fn text_property(
    connection: &RustConnection,
    window: Window,
    property: impl Into<Atom>,
    kind: impl Into<Atom>,
) -> Option<Vec<u8>> {
    let reply = connection
        .get_property(false, window, property, kind, 0, u32::MAX / 4)
        .ok()?
        .reply()
        .ok()?;
    reply.value8().map(Iterator::collect)
}

fn windows(
    connection: &RustConnection,
    root: Window,
    atoms: &Atoms,
) -> Result<Vec<WindowDescription>, ScreenCaptureError> {
    let own_process = std::process::id();
    let listed = connection
        .get_property(
            false,
            root,
            atoms.client_list,
            AtomEnum::WINDOW,
            0,
            u32::MAX / 4,
        )
        .map_err(system_error)?
        .reply()
        .map_err(system_error)?;
    let Some(windows) = listed.value32() else {
        return Ok(Vec::new());
    };
    let mut described = Vec::new();
    for window in windows {
        let Some(attributes) = connection
            .get_window_attributes(window)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        else {
            continue;
        };
        if attributes.map_state != MapState::VIEWABLE {
            continue;
        }
        let process = connection
            .get_property(false, window, atoms.window_pid, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(|reply| reply.value32().and_then(|mut values| values.next()));
        if process == Some(own_process) {
            continue;
        }
        let title = text_property(connection, window, atoms.window_name, atoms.utf8_string)
            .or_else(|| text_property(connection, window, AtomEnum::WM_NAME, AtomEnum::STRING))
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        if title.trim().is_empty() {
            continue;
        }
        // `WM_CLASS` is "instance\0class\0": the class names the application.
        let application_name =
            text_property(connection, window, AtomEnum::WM_CLASS, AtomEnum::STRING).and_then(
                |bytes| {
                    bytes
                        .split(|byte| *byte == 0)
                        .nth(1)
                        .filter(|class| !class.is_empty())
                        .map(|class| String::from_utf8_lossy(class).into_owned())
                },
            );
        let Some(geometry) = connection
            .get_geometry(window)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        else {
            continue;
        };
        if geometry.width < 2 || geometry.height < 2 {
            continue;
        }
        described.push(WindowDescription {
            window,
            title,
            application_name,
            process,
            width: geometry.width,
            height: geometry.height,
        });
    }
    Ok(described)
}

/// One picture of `area` of `drawable` through plain `GetImage`, packed BGRA.
fn read_picture(
    connection: &RustConnection,
    drawable: Window,
    area: Area,
    bytes: Vec<u8>,
) -> Option<CapturedFrame> {
    let reply = connection
        .get_image(
            ImageFormat::Z_PIXMAP,
            drawable,
            area.x,
            area.y,
            area.width,
            area.height,
            ALL_PLANES,
        )
        .ok()?
        .reply()
        .ok()?;
    if !SUPPORTED_DEPTHS.contains(&reply.depth) {
        return None;
    }
    let width = usize::from(area.width);
    pack_bgra_rows(
        bytes,
        u32::from(area.width),
        u32::from(area.height),
        width * BYTES_PER_PIXEL,
        &reply.data,
        false,
    )
}

/// The monitors and windows of the X session.
pub(crate) fn sources(
    with_thumbnails: bool,
) -> Result<Vec<ScreenSourceDescription>, ScreenCaptureError> {
    let (connection, root) = connect()?;
    let atoms = Atoms::intern(&connection)?;
    let thumbnail = |drawable: Window, area: Area| {
        with_thumbnails
            .then(|| read_picture(&connection, drawable, area, Vec::new()))
            .flatten()
            .and_then(|picture| thumbnail_of(&picture))
    };
    let mut sources: Vec<ScreenSourceDescription> = monitors(&connection, root)?
        .into_iter()
        .map(|(area, name)| ScreenSourceDescription {
            id: monitor_id(area),
            kind: ScreenSourceKind::Display,
            title: name,
            application_name: None,
            width: u32::from(area.width),
            height: u32::from(area.height),
            thumbnail: thumbnail(root, area),
        })
        .collect();
    for window in windows(&connection, root, &atoms)? {
        let area = Area {
            x: 0,
            y: 0,
            width: window.width,
            height: window.height,
        };
        sources.push(ScreenSourceDescription {
            id: format!("{WINDOW_ID_PREFIX}{}", window.window),
            kind: ScreenSourceKind::Window,
            title: window.title,
            application_name: window.application_name,
            width: u32::from(window.width),
            height: u32::from(window.height),
            thumbnail: thumbnail(window.window, area),
        });
    }
    Ok(sources)
}

/// A System V shared memory segment the X server writes pictures into, attached on both sides.
struct SharedSegment {
    segment: u32,
    address: *mut u8,
    size: usize,
}

impl SharedSegment {
    /// A segment of `size` bytes attached to the server, or `None` without MIT-SHM (a remote
    /// display) or on any failure.
    fn new(connection: &RustConnection, size: usize) -> Option<Self> {
        connection.shm_query_version().ok()?.reply().ok()?;
        // SAFETY: plain System V calls; the segment is private to this process and the X
        // server, removed (IPC_RMID) once both attached, so it never outlives them.
        unsafe {
            let id = libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600);
            if id < 0 {
                return None;
            }
            let address = libc::shmat(id, std::ptr::null(), libc::SHM_RDONLY);
            if address.addr() == usize::MAX {
                libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
                return None;
            }
            let attached = connection.generate_id().ok().and_then(|segment| {
                connection
                    .shm_attach(segment, u32::try_from(id).ok()?, false)
                    .ok()?
                    .check()
                    .ok()
                    .map(|()| segment)
            });
            libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
            let Some(segment) = attached else {
                libc::shmdt(address);
                return None;
            };
            Some(Self {
                segment,
                address: address.cast::<u8>(),
                size,
            })
        }
    }

    /// One picture of `area` of `drawable`, packed BGRA into `bytes`.
    fn read_picture(
        &self,
        connection: &RustConnection,
        drawable: Window,
        area: Area,
        bytes: Vec<u8>,
    ) -> Option<CapturedFrame> {
        let length = usize::from(area.width) * usize::from(area.height) * BYTES_PER_PIXEL;
        if length > self.size {
            return None;
        }
        let reply = connection
            .shm_get_image(
                drawable,
                area.x,
                area.y,
                area.width,
                area.height,
                ALL_PLANES,
                ImageFormat::Z_PIXMAP.into(),
                self.segment,
                0,
            )
            .ok()?
            .reply()
            .ok()?;
        if !SUPPORTED_DEPTHS.contains(&reply.depth) {
            return None;
        }
        // SAFETY: the segment maps `size` bytes, at least `length` (checked above), and the
        // server finished writing them when it answered.
        let pixels = unsafe { std::slice::from_raw_parts(self.address, length) };
        pack_bgra_rows(
            bytes,
            u32::from(area.width),
            u32::from(area.height),
            usize::from(area.width) * BYTES_PER_PIXEL,
            pixels,
            false,
        )
    }

    fn detach(self, connection: &RustConnection) {
        let _ = connection.shm_detach(self.segment);
        let _ = connection.flush();
        // SAFETY: attached in `new`, detached once here.
        unsafe {
            libc::shmdt(self.address.cast());
        }
    }
}

/// Whether `window` still exists (an unmapped window keeps its last picture).
fn window_exists(connection: &RustConnection, window: Window) -> bool {
    connection
        .get_window_attributes(window)
        .ok()
        .is_some_and(|cookie| cookie.reply().is_ok())
}

/// The area to read now: a monitor's, or the window's current size.
fn current_area(
    connection: &RustConnection,
    root: Window,
    source: Source,
) -> Option<(Window, Area)> {
    match source {
        Source::Monitor(area) => Some((root, area)),
        Source::Window(window) => {
            let geometry = connection.get_geometry(window).ok()?.reply().ok()?;
            Some((
                window,
                Area {
                    x: 0,
                    y: 0,
                    width: geometry.width,
                    height: geometry.height,
                },
            ))
        }
    }
}

/// Reads pictures at 30 frames per second until the share or the capture stops.
fn capture_loop(
    connection: &RustConnection,
    root: Window,
    source: Source,
    pictures: &VideoInput,
    should_stop: &AtomicBool,
) -> Result<(), ScreenShareEnd> {
    let interval = Duration::from_secs(1) / FRAMES_PER_SECOND;
    let mut segment: Option<SharedSegment> = None;
    let mut has_shared_memory = true;
    let mut next_picture_at = Instant::now();
    let result = loop {
        if should_stop.load(Ordering::Relaxed) || !pictures.is_open() {
            break Ok(());
        }
        let Some((drawable, area)) = current_area(connection, root, source) else {
            break Err(ScreenShareEnd::SourceClosed);
        };
        let length = usize::from(area.width) * usize::from(area.height) * BYTES_PER_PIXEL;
        if has_shared_memory && segment.as_ref().is_none_or(|segment| segment.size < length) {
            if let Some(old) = segment.take() {
                old.detach(connection);
            }
            segment = SharedSegment::new(connection, length);
            has_shared_memory = segment.is_some();
        }
        let bytes = pictures.recycled_bytes();
        let picture = match &segment {
            Some(segment) => segment.read_picture(connection, drawable, area, bytes),
            None => read_picture(connection, drawable, area, bytes),
        };
        match picture {
            Some(picture) => pictures.push(picture),
            None => {
                if let Source::Window(window) = source
                    && !window_exists(connection, window)
                {
                    break Err(ScreenShareEnd::SourceClosed);
                }
                // Minimised or another depth: nothing this time, the last picture repeats.
            }
        }
        next_picture_at += interval;
        let now = Instant::now();
        if next_picture_at < now {
            // A slow read: no catching up with a burst.
            next_picture_at = now;
        }
        std::thread::sleep(next_picture_at.saturating_duration_since(now));
    };
    if let Some(segment) = segment {
        segment.detach(connection);
    }
    result
}

struct X11Capture {
    should_stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _sound: Option<RunningShareSound>,
    sound: ShareSoundState,
}

impl RunningScreenCapture for X11Capture {
    fn sound(&self) -> ShareSoundState {
        self.sound
    }
}

impl Drop for X11Capture {
    fn drop(&mut self) {
        self.should_stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("the X11 screen capture's thread panicked");
        }
    }
}

/// Starts sharing the monitor or window `choice` names.
pub(crate) fn start(
    choice: &ScreenChoice,
    sound: &ShareSound,
    share: ScreenShareContext,
) -> Result<Box<dyn RunningScreenCapture>, ScreenCaptureError> {
    let ScreenChoice::Source(id) = choice else {
        return Err(ScreenCaptureError::Unavailable);
    };
    let source = parse_source(id).ok_or(ScreenCaptureError::SourceGone)?;
    let (connection, root) = connect()?;
    let window_application = match source {
        Source::Monitor(_) => None,
        Source::Window(window) => {
            if !window_exists(&connection, window) {
                return Err(ScreenCaptureError::SourceGone);
            }
            let atoms = Atoms::intern(&connection)?;
            windows(&connection, root, &atoms)?
                .into_iter()
                .find(|described| described.window == window)
                .and_then(|described| described.process)
                .and_then(binary_of_process)
        }
    };
    let pictures = share.start_pictures();
    let is_whole_screen = matches!(source, Source::Monitor(_));
    let (running_sound, sound_state) =
        start_sound_for(sound, is_whole_screen, window_application, &share);
    let should_stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let should_stop = Arc::clone(&should_stop);
        let share = share.clone();
        std::thread::Builder::new()
            .name("mellow-screen-x11".into())
            .spawn(move || {
                if let Err(reason) =
                    capture_loop(&connection, root, source, &pictures, &should_stop)
                {
                    share.end(reason);
                }
            })
            .map_err(|error| {
                share.stop();
                system_error(error)
            })?
    };
    Ok(Box::new(X11Capture {
        should_stop,
        thread: Some(thread),
        _sound: running_sound,
        sound: sound_state,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_ids_name_a_monitor_s_area_or_a_window() {
        let area = Area {
            x: -1_920,
            y: 0,
            width: 1_920,
            height: 1_080,
        };
        assert_eq!(parse_source(&monitor_id(area)), Some(Source::Monitor(area)));
        assert_eq!(
            parse_source("window:4194311"),
            Some(Source::Window(4_194_311))
        );
        assert_eq!(parse_source("monitor:0,0,70000,10"), None);
        assert_eq!(parse_source("display:1"), None);
    }
}
