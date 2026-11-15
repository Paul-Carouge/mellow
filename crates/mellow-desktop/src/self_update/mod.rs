//! The desktop app's signed self-update (ADR-0057): a check soon after start and then about once
//! a day (with jitter, without the user's Mellow token), the package downloaded and its minisign
//! signature verified by `mellow-update`, unpacked into a staging directory, and swapped
//! in when the user restarts from the interface or quits the app, never during a voice call. On
//! Windows the staged installer does the swap once the app has exited (ADR-0060).

pub mod package;
#[cfg(test)]
mod state_tests;
#[cfg(test)]
mod tests;
pub mod toast;
#[cfg(test)]
mod toast_tests;

use std::hash::BuildHasher as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AppContext as _, Context, Entity, Global, Subscription, Task, WeakEntity};
use mellow_update::{DownloadProgress, Release, UpdateClient};
use semver::Version;

use self::package::{Installation, Relaunch, StagedUpdate};
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
/// How often the toast's progress bar is redrawn while a download it asked for runs: smooth
/// enough to read, and nothing between downloads.
const PROGRESS_REDRAW_INTERVAL: Duration = Duration::from_millis(125);

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
    /// A system or processor the release has no package for.
    UnsupportedSystem,
    /// Windows: Mellow runs as administrator, and an elevated app never updates itself.
    Elevated,
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

/// What the person can do about a copy that cannot update itself once a newer version is known
/// (ADR-0064).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualRemedy {
    /// Install the new version by hand: the latest release's page.
    DownloadPage,
    /// Fix what the sentence says (install FFmpeg), then check again from this copy.
    CheckAgain,
    /// The sentence says what to do outside this copy (move the app to Applications and open it
    /// from there): no button here would help.
    FollowTheSentence,
}

impl UpdateProblem {
    /// The remedy the update toast offers, or `None` when nobody here can fix it (a package
    /// manager's copy, a build from source, an unsupported system): those stay offline and
    /// silent, as ADR-0057 decided.
    #[must_use]
    pub fn manual_remedy(self) -> Option<ManualRemedy> {
        match self {
            Self::InstalledElsewhere | Self::NotWritable => Some(ManualRemedy::DownloadPage),
            Self::NoFfmpeg => Some(ManualRemedy::CheckAgain),
            Self::Translocated => Some(ManualRemedy::FollowTheSentence),
            _ => None,
        }
    }
}

/// Why a check found no update to prepare, and the newer version it saw if it got that far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CheckFailure {
    problem: UpdateProblem,
    newer_version: Option<Version>,
}

impl From<UpdateProblem> for CheckFailure {
    fn from(problem: UpdateProblem) -> Self {
        Self {
            problem,
            newer_version: None,
        }
    }
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
    /// The bytes of the package being downloaded, for the toast's progress bar.
    download_progress: Option<Arc<DownloadProgress>>,
    /// Redraws the toast while a download it asked for runs.
    progress_redraw: Option<Task<()>>,
    /// "Update and restart" was pressed: restart as soon as the update is ready, and say in the
    /// toast what goes wrong on the way.
    is_restart_requested: bool,
    /// The newest version the last check found, whether it could be prepared or not.
    newest_version: Option<Version>,
    /// This copy cannot update itself for this reason, but `newest_version` is out (ADR-0064).
    manual_update_problem: Option<UpdateProblem>,
    /// Where checks ask for the newest release.
    release_lookup: ReleaseLookup,
    /// The toast was closed for this version; it shows again for a newer one, or at the next
    /// launch (nothing is written to disk).
    dismissed_version: Option<Version>,
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
                match package::apply_staged_update(&mut staged, false) {
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
            download_progress: None,
            progress_redraw: None,
            is_restart_requested: false,
            newest_version: None,
            manual_update_problem: None,
            dismissed_version: None,
            release_lookup: ReleaseLookup::ReleaseHost,
            _schedule: schedule,
            _quit_subscription: quit_subscription,
        }
    }

    #[must_use]
    pub fn status(&self) -> &UpdateStatus {
        &self.status
    }

    /// What the update toast shows now, if anything (ADR-0064).
    #[must_use]
    pub fn toast_phase(&self, is_in_call: bool) -> Option<toast::UpdateToastPhase> {
        let progress = self
            .download_progress
            .as_deref()
            .map(|progress| toast::DownloadedBytes {
                received: progress.received_bytes(),
                total: progress.total_bytes(),
            });
        toast::update_toast_phase(&toast::UpdateToastInputs {
            status: &self.status,
            newest_version: self.newest_version.as_ref(),
            manual_update_problem: self.manual_update_problem,
            is_restart_requested: self.is_restart_requested,
            progress,
            dismissed_version: self.dismissed_version.as_ref(),
            is_in_call,
        })
    }

    /// "Update and restart" (and "Try again"): restarts into the ready update, or downloads,
    /// verifies and stages it first, then restarts. Never during a call.
    pub fn update_and_restart(&mut self, cx: &mut Context<Self>) {
        if is_in_call(cx) {
            return;
        }
        match self.status {
            UpdateStatus::Ready(_) => self.restart_to_update(cx),
            UpdateStatus::Installing(_) => {}
            UpdateStatus::Downloading(_) | UpdateStatus::Checking => {
                self.is_restart_requested = true;
                self.redraw_progress_while_downloading(cx);
                cx.notify();
            }
            UpdateStatus::Idle | UpdateStatus::UpToDate | UpdateStatus::Failed(_) => {
                self.check_now(cx);
                // After the check started: `check_now` clears the request of any earlier click.
                self.is_restart_requested = true;
                cx.notify();
            }
        }
    }

    /// "Check again", for a copy that cannot update itself until the person fixes something.
    pub fn check_again(&mut self, cx: &mut Context<Self>) {
        self.check_now(cx);
    }

    /// Closes the toast for the version it shows; a download it asked for goes on quietly and
    /// waits for a restart or the quit, as ADR-0057's do.
    pub fn dismiss_toast(&mut self, cx: &mut Context<Self>) {
        if matches!(self.status, UpdateStatus::Installing(_)) {
            return;
        }
        let shown = match &self.status {
            UpdateStatus::Downloading(version) | UpdateStatus::Ready(version) => {
                Some(version.clone())
            }
            _ => self.newest_version.clone(),
        };
        self.dismissed_version = shown.max(self.dismissed_version.take());
        self.is_restart_requested = false;
        self.progress_redraw = None;
        cx.notify();
    }

    /// Redraws the toast a few times a second while the download it asked for runs; stops by
    /// itself when the download ends.
    fn redraw_progress_while_downloading(&mut self, cx: &mut Context<Self>) {
        if self.progress_redraw.is_some() {
            return;
        }
        self.progress_redraw = Some(cx.spawn(async move |this, cx| {
            let mut shown_bytes = None;
            loop {
                cx.background_executor()
                    .timer(PROGRESS_REDRAW_INTERVAL)
                    .await;
                let is_still_downloading = this.update(cx, |this, cx| {
                    let is_downloading = this.is_restart_requested
                        && matches!(
                            this.status,
                            UpdateStatus::Downloading(_) | UpdateStatus::Checking
                        );
                    let bytes = this
                        .download_progress
                        .as_deref()
                        .map(DownloadProgress::received_bytes);
                    if is_downloading && bytes != shown_bytes {
                        shown_bytes = bytes;
                        cx.notify();
                    }
                    if !is_downloading {
                        this.progress_redraw = None;
                    }
                    is_downloading
                });
                if !matches!(is_still_downloading, Ok(true)) {
                    return;
                }
            }
        }));
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
        // Only "Update and restart" asks to restart, right after starting its check: a scheduled
        // check never restarts the app, and its failures stay silent (ADR-0017).
        self.is_restart_requested = false;
        let newest_known = if let Some(staged) = &self.staged {
            staged.version.clone()
        } else {
            self.status = UpdateStatus::Checking;
            cx.notify();
            mellow_update::current_version()
        };
        let find = self.look_up_release(newest_known, cx);
        self.work = Some(cx.spawn(async move |this, cx| {
            // Only a panic or the runtime shutting down lose the answer.
            let found = find
                .await
                .unwrap_or_else(|_| Err(UpdateProblem::InstallFailed.into()));
            let _ = this.update(cx, |this, cx| match found {
                Ok(None) => {
                    this.manual_update_problem = None;
                    this.finish_unless_ready(UpdateStatus::UpToDate, cx);
                }
                Ok(Some(update)) => {
                    this.manual_update_problem = None;
                    this.download(update, cx);
                }
                Err(failure) => {
                    let is_manual = failure.problem.manual_remedy().is_some()
                        && failure.newer_version.is_some();
                    if this.staged.is_none() {
                        this.manual_update_problem = is_manual.then_some(failure.problem);
                        if failure.newer_version.is_some() {
                            this.newest_version = failure.newer_version;
                        }
                    }
                    this.finish_unless_ready(UpdateStatus::Failed(failure.problem), cx);
                }
            });
        }));
    }

    /// The newest release and what this copy can do with it: from the release host, or the
    /// answers a test scripted.
    fn look_up_release(&mut self, newest_known: Version, cx: &mut Context<Self>) -> CheckTask {
        match &mut self.release_lookup {
            ReleaseLookup::ReleaseHost => {}
            #[cfg(test)]
            ReleaseLookup::Scripted(answers) => {
                let answer = answers.pop_front().unwrap_or(Ok(None));
                return Task::ready(Ok(answer));
            }
        }
        // Detected at each check: installing FFmpeg or fixing the folder's rights takes effect.
        spawn_on_network_runtime(cx, async move {
            let installation = match tokio::task::spawn_blocking(package::detect_installation)
                .await
                .unwrap_or(Err(UpdateProblem::InstallFailed))
            {
                Ok(installation) => installation,
                Err(problem) if problem.manual_remedy().is_some() => {
                    // A development build is nobody's installed copy: it asks nothing of the
                    // release host. A release build compiled from source is a copy put somewhere
                    // by hand, and learns of new versions like one (ADR-0064).
                    let newer_version = if cfg!(debug_assertions) {
                        None
                    } else {
                        newer_version_than(&newest_known).await
                    };
                    return Err(CheckFailure {
                        problem,
                        newer_version,
                    });
                }
                Err(problem) => return Err(problem.into()),
            };
            find_update_for(installation, &newest_known).await
        })
    }

    /// A self-update whose checks get `answers` in turn, then "no newer release": no network, no
    /// installation, no timer that fires unless the test moves the clock.
    #[cfg(test)]
    pub(crate) fn with_scripted_checks(
        answers: Vec<Result<Option<FoundUpdate>, CheckFailure>>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            release_lookup: ReleaseLookup::Scripted(answers.into()),
            ..Self::new(cx)
        }
    }

    /// Puts the self-update where a check or a download would have left it.
    #[cfg(test)]
    pub(crate) fn set_status_for_tests(&mut self, status: UpdateStatus, cx: &mut Context<Self>) {
        if let UpdateStatus::Downloading(version) | UpdateStatus::Ready(version) = &status {
            self.newest_version = Some(version.clone());
        }
        self.status = status;
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn is_restart_requested(&self) -> bool {
        self.is_restart_requested
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
        let progress = Arc::new(DownloadProgress::default());
        if self.staged.is_none() {
            self.newest_version = Some(version.clone());
            self.download_progress = Some(Arc::clone(&progress));
            self.status = UpdateStatus::Downloading(version.clone());
            if self.is_restart_requested {
                self.redraw_progress_while_downloading(cx);
            }
            cx.notify();
        }
        let download = spawn_on_network_runtime(cx, async move {
            let package = package::download_verified_package(
                &update.client,
                &update.release,
                &update.asset_name,
                Some(&progress),
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
            let _ = this.update(cx, |this, cx| {
                this.download_progress = None;
                match staged {
                    Ok(staged) => {
                        let version = staged.version.clone();
                        // Replacing an older ready update removes its staging directory.
                        this.staged = Some(staged);
                        this.work = None;
                        this.newest_version = Some(version.clone());
                        this.finish(UpdateStatus::Ready(version), cx);
                        if this.is_restart_requested {
                            this.restart_to_update(cx);
                        }
                    }
                    Err(problem) => {
                        this.finish_unless_ready(UpdateStatus::Failed(problem), cx);
                    }
                }
            });
        }));
    }

    /// Swaps the ready update in and starts it; does nothing during a call.
    pub fn restart_to_update(&mut self, cx: &mut Context<Self>) {
        if !can_restart_to_update(&self.status, is_in_call(cx)) {
            // A download asked for from the toast finished during a call: the toast offers the
            // restart again rather than cutting the call later.
            self.is_restart_requested = false;
            cx.notify();
            return;
        }
        // From here on, what goes wrong is said in the toast too.
        self.is_restart_requested = true;
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
            let applied = package::apply_staged_update(&mut staged, true);
            (staged, applied)
        });
        self.work = Some(cx.spawn(async move |this, cx| {
            let (staged, applied) = apply.await;
            let relaunch = this
                .update(cx, |this, cx| match applied {
                    Ok(relaunch) => {
                        this.applied = Some(staged);
                        Some(relaunch)
                    }
                    Err(problem) => {
                        this.finish(UpdateStatus::Failed(problem), cx);
                        None
                    }
                })
                .ok()
                .flatten();
            match relaunch {
                Some(Relaunch::ThisApp(restart_path)) => {
                    cx.update(|cx| restart_unless_in_call(&this, restart_path, cx));
                }
                // The installer started and waits for this process to exit, then opens the new
                // version. A call joined in the moments since the button was pressed ends with
                // the quit, as at any quit.
                Some(Relaunch::ByInstaller) => cx.update(|cx| cx.quit()),
                None => {}
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
pub(crate) struct FoundUpdate {
    installation: Installation,
    client: UpdateClient,
    release: Release,
    asset_name: String,
}

/// What a check found: an update to prepare, nothing newer, or why not.
type CheckOutcome = Result<Option<FoundUpdate>, CheckFailure>;
/// A check under way; only a panic or the runtime shutting down lose its answer.
type CheckTask = Task<Result<CheckOutcome, tokio::task::JoinError>>;

/// Where checks ask for the newest release.
enum ReleaseLookup {
    /// The release host, from the network runtime.
    ReleaseHost,
    /// Answers a test scripted, given in turn.
    #[cfg(test)]
    Scripted(std::collections::VecDeque<CheckOutcome>),
}

/// A release newer than `newest_known` (the running version, or the update already ready).
async fn find_update_for(
    installation: Installation,
    newest_known: &Version,
) -> Result<Option<FoundUpdate>, CheckFailure> {
    let client = UpdateClient::new().map_err(|_| UpdateProblem::Network)?;
    let Some(release) = package::find_newer_release(&client, newest_known).await? else {
        return Ok(None);
    };
    let asset_name =
        package::asset_name_for(&installation, &release).map_err(|problem| CheckFailure {
            problem,
            newer_version: Some(release.version.clone()),
        })?;
    Ok(Some(FoundUpdate {
        installation,
        client,
        release,
        asset_name,
    }))
}

/// The newest release's version if it is newer than `newest_known`, for a copy that cannot
/// update itself: the same anonymous request as a check, and no package fetched (ADR-0064).
async fn newer_version_than(newest_known: &Version) -> Option<Version> {
    let client = UpdateClient::new().ok()?;
    package::find_newer_release(&client, newest_known)
        .await
        .ok()
        .flatten()
        .map(|release| release.version)
}
