//! Screen sharing on Linux (ADR-0054): the session kind decides the backend at run time, as GPUI
//! decides its own (`WAYLAND_DISPLAY`, then `XDG_SESSION_TYPE`, then `DISPLAY`). Wayland goes
//! through the ScreenCast portal and PipeWire ([`crate::screen_capture_wayland`]), X11 through
//! the X server ([`crate::screen_capture_x11`]). The share's sound comes from PipeWire through
//! Vesper's `vesper-audio`: an application's playback streams, or the default output without
//! Mellow's own streams.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use vesper_audio::{AudioCapture, DuckSettings, Exclusions, Hold, NodeWatch, Options, Source};

use crate::screen_capture::{
    RunningScreenCapture, ScreenCaptureError, ScreenChoice, ScreenPicker, ScreenShareContext,
    ScreenSourceDescription, ScreenSources, ShareSound, ShareSoundState, SoundApplication,
    SystemScreenSources,
};

/// How long PipeWire may take to list the applications playing sound.
const NODE_LISTING_TIMEOUT: Duration = Duration::from_secs(2);

/// The display server this session runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionKind {
    Wayland,
    X11,
    None,
}

fn session_kind() -> SessionKind {
    let is_set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    if is_set("WAYLAND_DISPLAY") {
        return SessionKind::Wayland;
    }
    match std::env::var("XDG_SESSION_TYPE").as_deref() {
        Ok("wayland") => SessionKind::Wayland,
        Ok("x11") => SessionKind::X11,
        _ if is_set("DISPLAY") => SessionKind::X11,
        _ => SessionKind::None,
    }
}

impl ScreenSources for SystemScreenSources {
    fn picker(&self) -> ScreenPicker {
        match session_kind() {
            SessionKind::Wayland => ScreenPicker::System,
            SessionKind::X11 => ScreenPicker::InApp,
            SessionKind::None => ScreenPicker::Unavailable,
        }
    }

    fn sources(
        &self,
        with_thumbnails: bool,
    ) -> Result<Vec<ScreenSourceDescription>, ScreenCaptureError> {
        match session_kind() {
            SessionKind::X11 => crate::screen_capture_x11::sources(with_thumbnails),
            SessionKind::Wayland | SessionKind::None => Ok(Vec::new()),
        }
    }

    fn sound_applications(&self) -> Vec<SoundApplication> {
        sound_applications()
    }

    fn start(
        &self,
        choice: &ScreenChoice,
        sound: &ShareSound,
        share: ScreenShareContext,
    ) -> Result<Box<dyn RunningScreenCapture>, ScreenCaptureError> {
        match session_kind() {
            SessionKind::Wayland => crate::screen_capture_wayland::start(sound, &share),
            SessionKind::X11 => crate::screen_capture_x11::start(choice, sound, &share),
            SessionKind::None => Err(ScreenCaptureError::Unavailable),
        }
    }
}

/// The applications PipeWire sees playing (or able to play) sound now.
fn sound_applications() -> Vec<SoundApplication> {
    let (answer, answered) = mpsc::sync_channel(1);
    let Ok(watch) = NodeWatch::start(move |nodes| {
        let applications: Vec<SoundApplication> = nodes
            .into_iter()
            .filter_map(|node| match node.source {
                Source::App(id) => Some(SoundApplication {
                    id,
                    name: node.label,
                }),
                _ => None,
            })
            .collect();
        // Only the first list is wanted; later ones find the channel full or gone.
        let _ = answer.try_send(applications);
    }) else {
        return Vec::new();
    };
    let applications = answered
        .recv_timeout(NODE_LISTING_TIMEOUT)
        .unwrap_or_default();
    drop(watch);
    // Mellow's own streams are left out by their client's process (`NodeWatch`); its name is
    // the fallback where process ids differ (Flatpak), " (deleted)" after an update included.
    let own = own_binary_name();
    applications
        .into_iter()
        .filter(|application| own.as_deref() != Some(without_deleted_mark(&application.id)))
        .collect()
}

/// The executable's file name: what PipeWire records as `application.process.binary` for
/// Mellow's own streams.
fn own_binary_name() -> Option<String> {
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(executable_file_name)
}

/// Whether `pid` (a window's `_NET_WM_PID`) names one of this user's programs here, whose
/// sound a window share may take: never a kernel thread (no executable), init or another
/// account's process. A Flatpak app's window names its pid inside the sandbox (often 2), which
/// on the host is a kernel thread, the parent of every other one: taken as is, it would bring
/// in every other app's sound.
pub(crate) fn is_this_user_s_program(pid: u32) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    let process = format!("/proc/{pid}");
    let is_program = std::fs::read_link(format!("{process}/exe")).is_ok();
    let owner = std::fs::metadata(&process).map(|metadata| metadata.uid());
    let own = std::fs::metadata("/proc/self").map(|metadata| metadata.uid());
    is_program && matches!((owner, own), (Ok(owner), Ok(own)) if owner == own)
}

/// An executable's file name as PipeWire recorded it when the process connected: Linux adds
/// " (deleted)" to `/proc/<pid>/exe` once a package update replaced the file.
fn executable_file_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy();
    Some(without_deleted_mark(&name).to_owned())
}

/// `name` without the " (deleted)" Linux adds to a replaced binary's name, which PipeWire
/// keeps in the names of a process that connected after the update.
fn without_deleted_mark(name: &str) -> &str {
    name.strip_suffix(" (deleted)").unwrap_or(name)
}

/// Where a share's sound is taken from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinuxShareSound {
    /// The default output, without Mellow's own streams.
    WholeSystem,
    /// One application's playback streams, by its PipeWire application key or binary.
    Application(String),
    /// The playback streams of one process and its children: an X11 window's (`_NET_WM_PID`).
    Process(u32),
}

/// A running capture of a share's sound; dropping it stops it.
pub(crate) struct RunningShareSound {
    _capture: AudioCapture,
    // Keeps Mellow's own streams out of the whole system's sound.
    _hold: Option<Hold>,
}

/// Starts capturing `source` into the share's sound line, Opus-encoded by `vesper-audio`.
///
/// # Errors
/// The share has no picture yet, or PipeWire failed.
pub(crate) fn start_share_sound(
    source: LinuxShareSound,
    share: &ScreenShareContext,
) -> Result<RunningShareSound, ScreenCaptureError> {
    let input = share.sound_input().ok_or(ScreenCaptureError::Unavailable)?;
    let (source, exclusions, hold) = match source {
        LinuxShareSound::WholeSystem => {
            let exclusions = Exclusions::new(None);
            let hold = own_binary_name().map(|own| exclusions.hold(&own));
            (Source::Desktop, Some(exclusions), hold)
        }
        LinuxShareSound::Application(application) => (Source::App(application), None, None),
        LinuxShareSound::Process(pid) => (Source::Process(pid), None, None),
    };
    let options = Options {
        anchor: vesper_audio::monotonic_now(),
        muted: false,
        volume_db: 0.0,
        mix: None,
        process: None,
        on_state: None,
        on_signal: None,
        voice: None,
        speaks: false,
        ducking: DuckSettings::default(),
        exclusions,
        // Never the monitor with every app: it would send the call's own voices back to it.
        exclusions_required: true,
    };
    let capture = AudioCapture::start(source, options, move |packet| {
        input.push_opus(packet.data.to_vec());
    })
    .map_err(|error| ScreenCaptureError::System(error.to_string()))?;
    Ok(RunningShareSound {
        _capture: capture,
        _hold: hold,
    })
}

/// The sound to capture for `sound`, given what the capture knows of its source: whether it is
/// a whole screen, and the process of a window when the system names it.
pub(crate) fn share_sound_source(
    sound: &ShareSound,
    is_whole_screen: bool,
    window_process: Option<u32>,
) -> (Option<LinuxShareSound>, ShareSoundState) {
    match sound {
        ShareSound::Off => (None, ShareSoundState::Off),
        ShareSound::Application(application) => (
            Some(LinuxShareSound::Application(application.clone())),
            ShareSoundState::Captured,
        ),
        ShareSound::FromSource if is_whole_screen => (
            Some(LinuxShareSound::WholeSystem),
            ShareSoundState::Captured,
        ),
        ShareSound::FromSource => match window_process {
            Some(pid) => (
                Some(LinuxShareSound::Process(pid)),
                ShareSoundState::Captured,
            ),
            // The Wayland portal names no application for a window.
            None => (None, ShareSoundState::NotAvailable),
        },
    }
}

/// Starts the sound `sound` says, if any; a sound that fails to start leaves the picture going
/// and says so in the returned state.
pub(crate) fn start_sound_for(
    sound: &ShareSound,
    is_whole_screen: bool,
    window_process: Option<u32>,
    share: &ScreenShareContext,
) -> (Option<RunningShareSound>, ShareSoundState) {
    let (source, state) = share_sound_source(sound, is_whole_screen, window_process);
    let Some(source) = source else {
        return (None, state);
    };
    match start_share_sound(source, share) {
        Ok(running) => (Some(running), state),
        Err(error) => {
            tracing::warn!(%error, "the shared screen's sound could not start");
            (None, ShareSoundState::NotAvailable)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_without_its_application_has_no_sound_and_says_so() {
        assert_eq!(
            share_sound_source(&ShareSound::FromSource, false, None),
            (None, ShareSoundState::NotAvailable)
        );
        assert_eq!(
            share_sound_source(&ShareSound::FromSource, true, None),
            (
                Some(LinuxShareSound::WholeSystem),
                ShareSoundState::Captured
            )
        );
        assert_eq!(
            share_sound_source(&ShareSound::FromSource, false, Some(4_242)).0,
            Some(LinuxShareSound::Process(4_242))
        );
        assert_eq!(
            share_sound_source(&ShareSound::Application("vlc".into()), false, None).0,
            Some(LinuxShareSound::Application("vlc".into()))
        );
        assert_eq!(
            share_sound_source(&ShareSound::Off, true, None),
            (None, ShareSoundState::Off)
        );
    }

    #[test]
    fn a_binary_replaced_by_an_update_keeps_its_name() {
        assert_eq!(
            executable_file_name(Path::new("/usr/bin/mellow (deleted)")).as_deref(),
            Some("mellow")
        );
        assert_eq!(
            executable_file_name(Path::new("/usr/lib/firefox/firefox")).as_deref(),
            Some("firefox")
        );
        assert_eq!(without_deleted_mark("mellow (deleted)"), "mellow");
    }

    #[test]
    fn only_this_user_s_programs_give_a_window_its_sound() {
        use std::os::unix::fs::MetadataExt as _;
        assert!(is_this_user_s_program(std::process::id()));
        assert!(!is_this_user_s_program(u32::MAX), "no such process");
        // Where pid 2 is the kernel's thread daemon (any Linux host), a sandboxed window's pid.
        if std::fs::read_to_string("/proc/2/comm").is_ok_and(|name| name.trim() == "kthreadd") {
            assert!(!is_this_user_s_program(2));
        }
        let own = std::fs::metadata("/proc/self").unwrap().uid();
        if std::fs::metadata("/proc/1").is_ok_and(|init| init.uid() != own) {
            assert!(!is_this_user_s_program(1), "another account's process");
        }
    }

    #[test]
    fn the_own_process_binary_is_found() {
        assert!(own_binary_name().is_some());
        let from_proc = std::fs::read_link("/proc/self/exe").unwrap();
        assert_eq!(
            executable_file_name(&from_proc),
            own_binary_name(),
            "PipeWire names Mellow's streams by this binary"
        );
    }
}
