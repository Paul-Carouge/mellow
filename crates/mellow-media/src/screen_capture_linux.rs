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
            SessionKind::Wayland => crate::screen_capture_wayland::start(sound, share),
            SessionKind::X11 => crate::screen_capture_x11::start(choice, sound, share),
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
    let own = own_binary_name();
    applications
        .into_iter()
        .filter(|application| own.as_deref() != Some(application.id.as_str()))
        .collect()
}

/// The executable's file name: what PipeWire records as `application.process.binary` for
/// Mellow's own streams.
fn own_binary_name() -> Option<String> {
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
}

/// The binary of process `pid`, for a window's application (X11's `_NET_WM_PID`).
pub(crate) fn binary_of_process(pid: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .as_deref()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
}

/// Where a share's sound is taken from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinuxShareSound {
    /// The default output, without Mellow's own streams.
    WholeSystem,
    /// One application's playback streams, by its PipeWire application key or binary.
    Application(String),
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
/// a whole screen, and the application of a window when the system names it.
pub(crate) fn share_sound_source(
    sound: &ShareSound,
    is_whole_screen: bool,
    window_application: Option<String>,
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
        ShareSound::FromSource => match window_application {
            Some(application) => (
                Some(LinuxShareSound::Application(application)),
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
    window_application: Option<String>,
    share: &ScreenShareContext,
) -> (Option<RunningShareSound>, ShareSoundState) {
    let (source, state) = share_sound_source(sound, is_whole_screen, window_application);
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
            share_sound_source(&ShareSound::FromSource, false, Some("firefox".into())).0,
            Some(LinuxShareSound::Application("firefox".into()))
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
    fn the_own_process_binary_is_found() {
        assert!(own_binary_name().is_some());
        assert_eq!(
            binary_of_process(std::process::id()),
            own_binary_name(),
            "PipeWire names Mellow's streams by this binary"
        );
    }
}
