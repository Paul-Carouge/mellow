//! Updates the app from signed releases (ADR-0017): checks a little after launch and then
//! periodically, and installs only when the user asks, from User settings → About.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use mellow_update::{Program, Release, UpdateClient, UpdateError};
use slint::{ComponentHandle as _, Weak};
use tokio::runtime::Handle;

use crate::generated_ui::{AppWindow, UpdateStatus};

/// Leaves launch to the network calls that show the user's messages.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(10);
/// Well under GitHub's anonymous API limit, frequent enough to learn about a release the same day.
const CHECK_INTERVAL: Duration = Duration::from_hours(6);

struct Updater {
    window: Weak<AppWindow>,
    runtime: Handle,
    /// The newer release found by the last check, until it is installed.
    available_release: Option<Release>,
    is_busy: bool,
}

type SharedUpdater = Rc<RefCell<Updater>>;

/// Keep the returned timer alive for as long as the app runs: dropping it stops periodic checks.
pub fn start(window: &AppWindow, runtime: Handle) -> slint::Timer {
    let updater = Rc::new(RefCell::new(Updater {
        window: window.as_weak(),
        runtime,
        available_release: None,
        is_busy: false,
    }));
    let timer = slint::Timer::default();
    if cfg!(debug_assertions) {
        // A development build would replace itself with a release build.
        set_status(
            &updater.borrow(),
            UpdateStatus::Unsupported,
            "Updates are turned off in development builds.",
        );
        return timer;
    }
    connect_callbacks(window, &updater);
    let first_check = Rc::clone(&updater);
    slint::Timer::single_shot(FIRST_CHECK_DELAY, move || {
        check_for_updates(&first_check, CheckOrigin::Background);
    });
    let periodic = Rc::clone(&updater);
    timer.start(slint::TimerMode::Repeated, CHECK_INTERVAL, move || {
        check_for_updates(&periodic, CheckOrigin::Background);
    });
    timer
}

fn connect_callbacks(window: &AppWindow, updater: &SharedUpdater) {
    let for_check = Rc::clone(updater);
    window.on_check_for_updates(move || check_for_updates(&for_check, CheckOrigin::User));
    let for_install = Rc::clone(updater);
    window.on_install_update(move || install_update(&for_install));
    window.on_restart_to_update(restart_to_update);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckOrigin {
    /// A failure stays silent: the user did not ask, and the next check may succeed.
    Background,
    User,
}

fn check_for_updates(updater: &SharedUpdater, origin: CheckOrigin) {
    let runtime = {
        let mut updater = updater.borrow_mut();
        // Never interrupt a download, nor undo "ready to restart".
        if updater.is_busy || current_status(&updater) == Some(UpdateStatus::ReadyToRestart) {
            return;
        }
        updater.is_busy = true;
        if origin == CheckOrigin::User {
            set_status(&updater, UpdateStatus::Checking, "");
        }
        updater.runtime.clone()
    };
    let task = runtime.spawn(async { UpdateClient::new()?.fetch_latest_release().await });
    let updater = Rc::clone(updater);
    let _ = slint::spawn_local(async move {
        let result = task
            .await
            .unwrap_or_else(|_| Err(UpdateError::UnexpectedResponse("check stopped".into())));
        let mut updater = updater.borrow_mut();
        updater.is_busy = false;
        match result {
            Ok(release) if mellow_update::is_newer_than_current(&release) => {
                if let Some(window) = updater.window.upgrade() {
                    window.set_available_version(release.version.to_string().into());
                }
                updater.available_release = Some(release);
                set_status(&updater, UpdateStatus::Available, "");
            }
            Ok(_) | Err(UpdateError::NoReleasePublished) => {
                updater.available_release = None;
                set_status(&updater, UpdateStatus::UpToDate, "");
            }
            Err(error) => {
                tracing::warn!(error = %error, "update check failed");
                if origin == CheckOrigin::User {
                    let (status, text) = failure_presentation(&error);
                    set_status(&updater, status, text);
                }
            }
        }
    });
}

fn install_update(updater: &SharedUpdater) {
    let (runtime, release) = {
        let mut updater = updater.borrow_mut();
        let Some(release) = updater.available_release.clone() else {
            return;
        };
        if updater.is_busy {
            return;
        }
        updater.is_busy = true;
        set_status(&updater, UpdateStatus::Downloading, "");
        (updater.runtime.clone(), release)
    };
    let task = runtime.spawn(async move {
        let executable = std::env::current_exe().map_err(UpdateError::ReplaceExecutable)?;
        let contents = UpdateClient::new()?
            .download_verified_executable(&release, Program::Desktop)
            .await?;
        tokio::task::spawn_blocking(move || {
            mellow_update::install_executable(&contents, &executable)
        })
        .await
        .map_err(|_| {
            UpdateError::ReplaceExecutable(std::io::Error::other("install task stopped"))
        })?
    });
    let updater = Rc::clone(updater);
    let _ = slint::spawn_local(async move {
        let result = task
            .await
            .unwrap_or_else(|_| Err(UpdateError::UnexpectedResponse("download stopped".into())));
        let mut updater = updater.borrow_mut();
        updater.is_busy = false;
        match result {
            Ok(()) => {
                updater.available_release = None;
                set_status(&updater, UpdateStatus::ReadyToRestart, "");
            }
            Err(error) => {
                tracing::warn!(error = %error, "update install failed");
                let (status, text) = failure_presentation(&error);
                set_status(&updater, status, text);
            }
        }
    });
}

/// Starts the freshly installed executable, then quits so the usual exit cleanup runs.
fn restart_to_update() {
    let started = std::env::current_exe().and_then(|executable| {
        std::process::Command::new(executable)
            .args(std::env::args_os().skip(1))
            .spawn()
    });
    match started {
        Ok(_) => {
            let _ = slint::quit_event_loop();
        }
        Err(error) => tracing::warn!(error = %error, "cannot start the updated app"),
    }
}

/// What the user sees for a failed check or install; never the error's own text, which can carry
/// details from the network or the release host.
fn failure_presentation(error: &UpdateError) -> (UpdateStatus, &'static str) {
    match error {
        UpdateError::Network(_) | UpdateError::UnexpectedResponse(_) => (
            UpdateStatus::Failed,
            "Couldn't reach the update server. Check your connection and try again.",
        ),
        UpdateError::InvalidSignature | UpdateError::WrongSignedFile { .. } => (
            UpdateStatus::Failed,
            "The download didn't pass Mellow's signature check, so nothing was installed.",
        ),
        UpdateError::MissingAsset { .. } => (
            UpdateStatus::Unsupported,
            "No update is published for this computer yet.",
        ),
        UpdateError::ReplaceExecutable(_) | UpdateError::UnsafeInstallLocation { .. } => (
            UpdateStatus::Unsupported,
            "Mellow can't replace itself in this folder. Download the new version from \
             github.com/Paul-Carouge/mellow/releases.",
        ),
        UpdateError::ExecutableLeftAside { .. } => (
            UpdateStatus::Failed,
            "The update failed and Mellow's files need repairing. Download Mellow again from \
             github.com/Paul-Carouge/mellow/releases.",
        ),
        // Unreachable from a check or an install; kept explicit so a new variant is a decision.
        UpdateError::NoReleasePublished | UpdateError::NoPreviousExecutable => {
            (UpdateStatus::UpToDate, "")
        }
    }
}

fn current_status(updater: &Updater) -> Option<UpdateStatus> {
    updater
        .window
        .upgrade()
        .map(|window| window.get_update_status())
}

fn set_status(updater: &Updater, status: UpdateStatus, error_text: &str) {
    if let Some(window) = updater.window.upgrade() {
        window.set_update_status(status);
        window.set_update_error_text(error_text.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_never_show_the_error_text() {
        let presented = [
            failure_presentation(&UpdateError::UnexpectedResponse("<secret detail>".into())),
            failure_presentation(&UpdateError::InvalidSignature),
            failure_presentation(&UpdateError::ReplaceExecutable(std::io::Error::other(
                "/Users/me",
            ))),
        ];
        assert_eq!(presented[0].0, UpdateStatus::Failed);
        assert_eq!(presented[1].0, UpdateStatus::Failed);
        assert_eq!(presented[2].0, UpdateStatus::Unsupported);
        assert!(
            presented
                .iter()
                .all(|(_, text)| !text.contains("secret") && !text.contains("/Users"))
        );
        assert!(
            presented[2]
                .1
                .contains("github.com/Paul-Carouge/mellow/releases")
        );
    }
}
