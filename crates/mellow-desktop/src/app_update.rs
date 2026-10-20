//! Updates the app from signed releases (ADR-0017): checks a little after launch and then
//! periodically, and installs only when the user asks, from User settings → About.

use std::cell::RefCell;
use std::path::PathBuf;
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
const RESTART_FAILED_TEXT: &str =
    "Mellow couldn't restart. Quit it and open it again to finish updating.";
const NO_UPDATE_FOR_THIS_COMPUTER_TEXT: &str = "No update is published for this computer yet. \
     Download Mellow from github.com/Paul-Carouge/mellow/releases.";

/// Where the updater is; the visible `UpdateStatus` follows from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    /// A check is running; `is_user_waiting` once the user asked, so its outcome is shown.
    Checking {
        is_user_waiting: bool,
    },
    Downloading,
    /// The new executable is in place: only a restart is left, checks stop.
    Installed,
    /// This copy cannot update itself (location, platform); checks stop until the next launch.
    CannotUpdate,
}

/// What a finished check found.
// Slint's generated enums are not `Eq`.
#[derive(Debug, Clone, PartialEq)]
enum CheckOutcome {
    NewerVersion {
        has_this_platform: bool,
    },
    UpToDate,
    Failed {
        status: UpdateStatus,
        text: &'static str,
    },
}

/// The phase and what to show after a check: a failure nobody asked about changes nothing.
fn after_check(
    outcome: &CheckOutcome,
    is_user_waiting: bool,
) -> Option<(Phase, UpdateStatus, &'static str)> {
    match outcome {
        CheckOutcome::NewerVersion {
            has_this_platform: true,
        } => Some((Phase::Idle, UpdateStatus::Available, "")),
        CheckOutcome::NewerVersion {
            has_this_platform: false,
        } => Some((
            Phase::CannotUpdate,
            UpdateStatus::Unsupported,
            NO_UPDATE_FOR_THIS_COMPUTER_TEXT,
        )),
        CheckOutcome::UpToDate => Some((Phase::Idle, UpdateStatus::UpToDate, "")),
        CheckOutcome::Failed { status, text } if is_user_waiting => {
            Some((phase_after_failure(*status), *status, text))
        }
        CheckOutcome::Failed { .. } => None,
    }
}

/// An unsupported setup stays unsupported: offering the same failing update every six hours helps
/// nobody.
fn phase_after_failure(status: UpdateStatus) -> Phase {
    if status == UpdateStatus::Unsupported {
        Phase::CannotUpdate
    } else {
        Phase::Idle
    }
}

struct Updater {
    window: Weak<AppWindow>,
    runtime: Handle,
    /// Resolved at launch: once replaced, Linux reports the running executable as "(deleted)".
    executable: PathBuf,
    phase: Phase,
    /// The newer release found by the last check, until it is installed.
    available_release: Option<Release>,
}

type SharedUpdater = Rc<RefCell<Updater>>;

/// Keep the returned timer alive for as long as the app runs: dropping it stops periodic checks.
pub fn start(window: &AppWindow, runtime: Handle) -> slint::Timer {
    let timer = slint::Timer::default();
    let show_unsupported = |text: &str| {
        window.set_update_status(UpdateStatus::Unsupported);
        window.set_update_error_text(text.into());
    };
    if cfg!(debug_assertions) {
        // A development build would replace itself with a release build.
        show_unsupported("Updates are turned off in development builds.");
        return timer;
    }
    let Ok(executable) = std::env::current_exe() else {
        show_unsupported("Mellow can't find where it is installed, so it can't update itself.");
        return timer;
    };
    let updater = Rc::new(RefCell::new(Updater {
        window: window.as_weak(),
        runtime,
        executable,
        phase: Phase::Idle,
        available_release: None,
    }));
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
    let for_restart = Rc::clone(updater);
    window.on_restart_to_update(move || restart_to_update(&for_restart));
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
        let is_user = origin == CheckOrigin::User;
        match updater.phase {
            Phase::Downloading | Phase::CannotUpdate => return,
            // "Try again" after a failed restart offers the restart again, never a reinstall.
            Phase::Installed => {
                if is_user {
                    set_status(&updater, UpdateStatus::ReadyToRestart, "");
                }
                return;
            }
            // A click during a background check waits for that check and sees its outcome.
            Phase::Checking { is_user_waiting } => {
                if is_user && !is_user_waiting {
                    updater.phase = Phase::Checking {
                        is_user_waiting: true,
                    };
                    set_status(&updater, UpdateStatus::Checking, "");
                }
                return;
            }
            Phase::Idle => {}
        }
        updater.phase = Phase::Checking {
            is_user_waiting: is_user,
        };
        if is_user {
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
        let Phase::Checking { is_user_waiting } = updater.phase else {
            return;
        };
        let outcome = match &result {
            Ok(release) if mellow_update::is_newer_than_current(release) => {
                CheckOutcome::NewerVersion {
                    has_this_platform: release.has_executable_for(Program::Desktop),
                }
            }
            Ok(_) | Err(UpdateError::NoReleasePublished) => CheckOutcome::UpToDate,
            Err(error) => {
                tracing::warn!(error = %error, "update check failed");
                let (status, text) = failure_presentation(error);
                CheckOutcome::Failed { status, text }
            }
        };
        let Some((phase, status, text)) = after_check(&outcome, is_user_waiting) else {
            updater.phase = Phase::Idle;
            return;
        };
        updater.phase = phase;
        updater.available_release = result.ok().filter(|_| status == UpdateStatus::Available);
        if let (Some(release), Some(window)) =
            (&updater.available_release, updater.window.upgrade())
        {
            window.set_available_version(release.version.to_string().into());
        }
        set_status(&updater, status, text);
    });
}

fn install_update(updater: &SharedUpdater) {
    let (runtime, release, executable) = {
        let mut updater = updater.borrow_mut();
        if updater.phase != Phase::Idle {
            return;
        }
        let Some(release) = updater.available_release.clone() else {
            return;
        };
        updater.phase = Phase::Downloading;
        set_status(&updater, UpdateStatus::Downloading, "");
        (updater.runtime.clone(), release, updater.executable.clone())
    };
    let task = runtime.spawn(async move {
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
        match result {
            Ok(()) => {
                updater.phase = Phase::Installed;
                updater.available_release = None;
                set_status(&updater, UpdateStatus::ReadyToRestart, "");
            }
            Err(error) => {
                tracing::warn!(error = %error, "update install failed");
                let (status, text) = failure_presentation(&error);
                updater.phase = phase_after_failure(status);
                set_status(&updater, status, text);
            }
        }
    });
}

/// Starts the freshly installed executable, then quits so the usual exit cleanup runs.
fn restart_to_update(updater: &SharedUpdater) {
    let updater = updater.borrow();
    if updater.phase != Phase::Installed {
        return;
    }
    let started = std::process::Command::new(&updater.executable)
        .args(std::env::args_os().skip(1))
        .spawn();
    match started {
        Ok(_) => {
            let _ = slint::quit_event_loop();
        }
        Err(error) => {
            tracing::warn!(error = %error, "cannot start the updated app");
            set_status(&updater, UpdateStatus::Failed, RESTART_FAILED_TEXT);
        }
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
        UpdateError::MissingAsset { .. } => {
            (UpdateStatus::Unsupported, NO_UPDATE_FOR_THIS_COMPUTER_TEXT)
        }
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

    #[test]
    fn a_check_shows_failures_only_to_a_waiting_user_and_stops_where_it_cannot_install() {
        let failed = CheckOutcome::Failed {
            status: UpdateStatus::Failed,
            text: "offline",
        };
        assert_eq!(
            after_check(&failed, false),
            None,
            "silent in the background"
        );
        assert_eq!(
            after_check(&failed, true),
            Some((Phase::Idle, UpdateStatus::Failed, "offline"))
        );
        assert_eq!(
            after_check(
                &CheckOutcome::NewerVersion {
                    has_this_platform: true
                },
                false
            ),
            Some((Phase::Idle, UpdateStatus::Available, ""))
        );
        assert_eq!(
            after_check(
                &CheckOutcome::NewerVersion {
                    has_this_platform: false
                },
                false
            ),
            Some((
                Phase::CannotUpdate,
                UpdateStatus::Unsupported,
                NO_UPDATE_FOR_THIS_COMPUTER_TEXT
            )),
            "never offered again every six hours"
        );
        assert_eq!(
            after_check(&CheckOutcome::UpToDate, false),
            Some((Phase::Idle, UpdateStatus::UpToDate, ""))
        );
        assert_eq!(
            phase_after_failure(UpdateStatus::Unsupported),
            Phase::CannotUpdate
        );
    }
}
