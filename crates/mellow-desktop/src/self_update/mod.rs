//! The desktop app's signed self-update (ADR-0057): a check soon after start and then about once
//! a day (with jitter, without the user's Mellow token), the package downloaded and its minisign
//! signature verified by `mellow-update`, unpacked into a private staging directory, and swapped
//! in when the user restarts from the interface or quits the app, never during a voice call.

pub mod package;
#[cfg(test)]
mod tests;

use std::hash::BuildHasher as _;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AppContext as _, Context, Entity, Global, Subscription, Task, WeakEntity};
use mellow_update::{Release, UpdateClient};
use semver::Version;

use self::package::{Installation, StagedUpdate};
use crate::preferences::preferences;
use crate::tokio_bridge::spawn_on_network_runtime;

/// Leaves the start to signing in and loading; the jitter spreads every app's first check.
const FIRST_CHECK_DELAY: Duration = Duration::from_mins(1);
const FIRST_CHECK_JITTER: Duration = Duration::from_mins(5);
/// Releases come every few days at most: one check a day finds them soon enough.
const CHECK_INTERVAL: Duration = Duration::from_hours(24);
const CHECK_JITTER: Duration = Duration::from_hours(2);
/// A call delays a scheduled download (it would take the call's bandwidth).
const IN_CALL_RETRY_DELAY: Duration = Duration::from_mins(15);
/// The preferences file is read within moments of starting.
const PREFERENCES_RETRY_DELAY: Duration = Duration::from_secs(5);

/// Why the app could not update itself, each with its sentence in the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateProblem {
    /// Not a signed release app: built from source, or signed by someone else.
    NotPackaged,
    /// Linux: installed in a system folder, which belongs to the package manager.
    PackageManager,
    /// Linux: not where the package's `install.sh` puts it.
    InstalledElsewhere,
    NotWritable,
    /// macOS: run from the read-only copy macOS makes of an app opened where it was downloaded.
    Translocated,
    /// Windows, or a Linux processor the release has no package for.
    UnsupportedSystem,
    /// Linux: no FFmpeg the packages are built for.
    NoFfmpeg,
    Network,
    /// The release has no package for this system.
    NoPackage,
    BadSignature,
    BadPackage,
    /// macOS: the new app is not signed with this app's certificate.
    WrongCertificate,
    InstallFailed,
}

/// Where the self-update stands, as the interface shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    /// Nothing checked since the start.
    #[default]
    Idle,
    Checking,
    UpToDate,
    Downloading(Version),
    /// Downloaded, verified and unpacked: installed at the restart or the quit.
    Ready(Version),
    Installing(Version),
    Failed(UpdateProblem),
}

/// Whether "Restart to update" may run now: an update is ready and no call would be cut.
#[must_use]
pub fn can_restart_to_update(status: &UpdateStatus, is_in_call: bool) -> bool {
    matches!(status, UpdateStatus::Ready(_)) && !is_in_call
}

/// `base` plus up to `jitter`, picked from `random`.
#[must_use]
pub fn delay_with_jitter(base: Duration, jitter: Duration, random: u64) -> Duration {
    let jitter_milliseconds = u64::try_from(jitter.as_millis()).unwrap_or(u64::MAX);
    base + Duration::from_millis(random % jitter_milliseconds.saturating_add(1))
}

/// A different number at each call: the standard library's per-process hash keys are random.
fn random_number() -> u64 {
    std::collections::hash_map::RandomState::new().hash_one(std::time::Instant::now())
}

/// Whether a call is joined or joining, which an update must not cut.
#[must_use]
pub fn is_in_call(cx: &App) -> bool {
    crate::voice::voice(cx).is_some_and(|voice| voice.read(cx).channel_id().is_some())
}

/// Automatic checks are on unless turned off in Settings, About; `None` while the preferences
/// are read.
fn are_automatic_updates_on(cx: &App) -> Option<bool> {
    preferences(cx).map(|preferences| !preferences.is_automatic_update_off)
}

pub struct SelfUpdate {
    status: UpdateStatus,
    staged: Option<StagedUpdate>,
    /// Swapped in: kept until the process ends, as on macOS it holds the replaced app this
    /// process still runs from.
    applied: Option<StagedUpdate>,
    /// Swapped in while a call started: what "Restart to update" starts once the call ends.
    restart_path_after_call: Option<PathBuf>,
    /// The check, download or installation under way; dropping it cancels it.
    work: Option<Task<()>>,
    _schedule: Task<()>,
    _quit_subscription: Subscription,
}

impl std::fmt::Debug for SelfUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SelfUpdate")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

struct SelfUpdateGlobal(Entity<SelfUpdate>);

impl Global for SelfUpdateGlobal {}

/// Starts the scheduled checks and installs a ready update when the app quits.
pub fn install_self_update(cx: &mut App) -> Entity<SelfUpdate> {
    // Once per start, automatic updates on or off: after a restart to update, the replaced app
    // waits there (on macOS, beside the installed one).
    cx.background_spawn(async {
        if let Some(folder) = package::staging_folder_of_this_copy() {
            package::remove_stale_staging_directories(&folder);
        }
    })
    .detach();
    let self_update = cx.new(SelfUpdate::new);
    cx.set_global(SelfUpdateGlobal(self_update.clone()));
    self_update
}

/// The app's self-update; `None` before [`install_self_update`] (tests of other views).
#[must_use]
pub fn self_update(cx: &App) -> Option<Entity<SelfUpdate>> {
    cx.try_global::<SelfUpdateGlobal>()
        .map(|global| global.0.clone())
}

impl SelfUpdate {
    fn new(cx: &mut Context<Self>) -> Self {
        let schedule = cx.spawn(async move |this, cx| {
            let mut delay =
                delay_with_jitter(FIRST_CHECK_DELAY, FIRST_CHECK_JITTER, random_number());
            loop {
                cx.background_executor().timer(delay).await;
                match this.update(cx, Self::run_scheduled_check) {
                    Ok(next_delay) => delay = next_delay,
                    Err(_) => return,
                }
            }
        });
        // At a quit the call ends anyway; a ready update is swapped in, and the next start runs
        // it. Blocking, but the window is already closing and the swap is a rename or two.
        let quit_subscription = cx.on_app_quit(|this, _| {
            if let Some(mut staged) = this.staged.take() {
                match package::apply_staged_update(&mut staged) {
                    Ok(_) => this.applied = Some(staged),
                    Err(problem) => {
                        tracing::warn!(?problem, "the update could not be installed at quit");
                    }
                }
            }
            async {}
        });
        Self {
            status: UpdateStatus::Idle,
            staged: None,
            applied: None,
            restart_path_after_call: None,
            work: None,
            _schedule: schedule,
            _quit_subscription: quit_subscription,
        }
    }

    #[must_use]
    pub fn status(&self) -> &UpdateStatus {
        &self.status
    }

    /// Returns when to look again.
    fn run_scheduled_check(&mut self, cx: &mut Context<Self>) -> Duration {
        match are_automatic_updates_on(cx) {
            None => return PREFERENCES_RETRY_DELAY,
            Some(false) => return CHECK_INTERVAL,
            Some(true) => {}
        }
        if is_in_call(cx) {
            return IN_CALL_RETRY_DELAY;
        }
        self.check_now(cx);
        delay_with_jitter(CHECK_INTERVAL, CHECK_JITTER, random_number())
    }

    /// Looks for a newer release and, if there is one, downloads, verifies and stages it. With an
    /// update already ready (the app can run for weeks), only a newer one replaces it, quietly.
    pub fn check_now(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.status,
            UpdateStatus::Checking | UpdateStatus::Downloading(_) | UpdateStatus::Installing(_)
        ) || self.applied.is_some()
            || self.work.is_some() && self.staged.is_some()
        {
            return;
        }
        let newest_known = if let Some(staged) = &self.staged {
            staged.version.clone()
        } else {
            self.status = UpdateStatus::Checking;
            cx.notify();
            mellow_update::current_version()
        };
        // Detected at each check: installing FFmpeg or fixing the folder's rights takes effect.
        let find = spawn_on_network_runtime(cx, async move {
            let installation = tokio::task::spawn_blocking(package::detect_installation)
                .await
                .unwrap_or(Err(UpdateProblem::InstallFailed))?;
            find_update_for(installation, &newest_known).await
        });
        self.work = Some(cx.spawn(async move |this, cx| {
            // Only a panic or the runtime shutting down lose the answer.
            let found = find.await.unwrap_or(Err(UpdateProblem::InstallFailed));
            let _ = this.update(cx, |this, cx| match found {
                Ok(None) => this.finish_unless_ready(UpdateStatus::UpToDate, cx),
                Ok(Some(update)) => this.download(update, cx),
                Err(problem) => this.finish_unless_ready(UpdateStatus::Failed(problem), cx),
            });
        }));
    }

    /// Ends a check or a download; with an update still ready, its status stays.
    fn finish_unless_ready(&mut self, status: UpdateStatus, cx: &mut Context<Self>) {
        self.work = None;
        if self.staged.is_none() {
            self.finish(status, cx);
        } else if let UpdateStatus::Failed(problem) = status {
            tracing::info!(?problem, "a newer update could not be prepared");
        }
    }

    fn finish(&mut self, status: UpdateStatus, cx: &mut Context<Self>) {
        if let UpdateStatus::Failed(problem) = &status {
            tracing::info!(?problem, "the app could not update itself");
        }
        self.status = status;
        cx.notify();
    }

    fn download(&mut self, update: FoundUpdate, cx: &mut Context<Self>) {
        let version = update.release.version.clone();
        if self.staged.is_none() {
            self.status = UpdateStatus::Downloading(version.clone());
            cx.notify();
        }
        let download = spawn_on_network_runtime(cx, async move {
            let package = package::download_verified_package(
                &update.client,
                &update.release,
                &update.asset_name,
            )
            .await?;
            // Unpacking a large archive and running `codesign` block: off the network workers.
            tokio::task::spawn_blocking(move || {
                package::stage_package(&update.installation, version, &update.asset_name, &package)
            })
            .await
            .unwrap_or(Err(UpdateProblem::InstallFailed))
        });
        self.work = Some(cx.spawn(async move |this, cx| {
            let staged = download.await.unwrap_or(Err(UpdateProblem::InstallFailed));
            let _ = this.update(cx, |this, cx| match staged {
                Ok(staged) => {
                    let version = staged.version.clone();
                    // Replacing an older ready update removes its staging directory.
                    this.staged = Some(staged);
                    this.work = None;
                    this.finish(UpdateStatus::Ready(version), cx);
                }
                Err(problem) => this.finish_unless_ready(UpdateStatus::Failed(problem), cx),
            });
        }));
    }

    /// Swaps the ready update in and starts it; does nothing during a call.
    pub fn restart_to_update(&mut self, cx: &mut Context<Self>) {
        if !can_restart_to_update(&self.status, is_in_call(cx)) {
            return;
        }
        if let Some(restart_path) = self.restart_path_after_call.take() {
            self.work = Some(cx.spawn(async move |this, cx| {
                cx.update(|cx| restart_unless_in_call(&this, restart_path, cx));
            }));
            return;
        }
        // A newer update being prepared quietly is dropped: this one is what the user saw.
        let Some(mut staged) = self.staged.take() else {
            return;
        };
        self.status = UpdateStatus::Installing(staged.version.clone());
        cx.notify();
        let apply = cx.background_spawn(async move {
            let applied = package::apply_staged_update(&mut staged);
            (staged, applied)
        });
        self.work = Some(cx.spawn(async move |this, cx| {
            let (staged, applied) = apply.await;
            let restart_path = this
                .update(cx, |this, cx| match applied {
                    Ok(restart_path) => {
                        this.applied = Some(staged);
                        Some(restart_path)
                    }
                    Err(problem) => {
                        this.finish(UpdateStatus::Failed(problem), cx);
                        None
                    }
                })
                .ok()
                .flatten();
            if let Some(restart_path) = restart_path {
                cx.update(|cx| restart_unless_in_call(&this, restart_path, cx));
            }
        }));
    }
}

/// Starts `restart_path` instead of this process, unless a call began while the update was
/// swapped in: then the swapped files wait for "Restart to update" after the call, or the next
/// start. Outside the entity's update: quitting runs this entity's quit hook.
fn restart_unless_in_call(this: &WeakEntity<SelfUpdate>, restart_path: PathBuf, cx: &mut App) {
    if is_in_call(cx) {
        let _ = this.update(cx, |this, cx| {
            let version = this
                .applied
                .as_ref()
                .map_or_else(mellow_update::current_version, |applied| {
                    applied.version.clone()
                });
            this.restart_path_after_call = Some(restart_path);
            this.finish(UpdateStatus::Ready(version), cx);
        });
        return;
    }
    cx.set_restart_path(restart_path);
    cx.restart();
}

/// A newer release, the client that found it and the asset for this installation.
struct FoundUpdate {
    installation: Installation,
    client: UpdateClient,
    release: Release,
    asset_name: String,
}

/// A release newer than `newest_known` (the running version, or the update already ready).
async fn find_update_for(
    installation: Installation,
    newest_known: &Version,
) -> Result<Option<FoundUpdate>, UpdateProblem> {
    let client = UpdateClient::new().map_err(|_| UpdateProblem::Network)?;
    let Some(release) = package::find_newer_release(&client, newest_known).await? else {
        return Ok(None);
    };
    let asset_name = package::asset_name_for(&installation, &release)?;
    Ok(Some(FoundUpdate {
        installation,
        client,
        release,
        asset_name,
    }))
}
