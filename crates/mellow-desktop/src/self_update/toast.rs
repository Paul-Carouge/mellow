//! The update toast (ADR-0064, DESIGN.md §6.36): "Mellow X.Y.Z is available" in the main
//! window's bottom trailing corner, "Update and restart", the download's progress, the
//! installation, an error with "Try again", and the copies that cannot update themselves.
//!
//! What it shows is a pure function of the self-update's state ([`update_toast_phase`]); the view
//! only draws it and forwards the buttons to [`SelfUpdate`].

use gpui::{
    App, Context, Entity, FocusHandle, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Window, div,
};
use semver::Version;

use super::{ManualRemedy, SelfUpdate, UpdateProblem, UpdateStatus, is_in_call};
use crate::components::{
    Button, ButtonSize, ButtonVariant, Melli, MelliState, Toast, ToastProgress, ToastTone,
    assertive_live_region, polite_live_region,
};
use crate::i18n::{Strings, strings};
use crate::theme::length;
use crate::theme::tokens::{
    COMPONENT_TOAST_BOTTOM_OFFSET, COMPONENT_TOAST_MARGIN, COMPONENT_TOAST_MELLI_SIZE,
};

/// What the toast shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateToastPhase {
    /// "Mellow X.Y.Z is available", "Update and restart"; the button waits during a call.
    Offered { version: Version, is_in_call: bool },
    /// The download "Update and restart" waits for; `percent` once the host gave the size.
    Downloading {
        version: Version,
        percent: Option<u8>,
    },
    /// Every byte is in: the signature is checked and the package unpacked.
    Verifying { version: Version },
    /// Swapped in; the app restarts in a moment.
    Installing { version: Version },
    /// Something went wrong after "Update and restart": the problem's sentence, "Try again",
    /// which waits during a call.
    Failed {
        version: Version,
        problem: UpdateProblem,
        is_in_call: bool,
    },
    /// This copy cannot update itself, and a newer version is out.
    ManualUpdate {
        version: Version,
        problem: UpdateProblem,
        remedy: ManualRemedy,
    },
}

impl UpdateToastPhase {
    #[must_use]
    pub fn version(&self) -> &Version {
        match self {
            Self::Offered { version, .. }
            | Self::Downloading { version, .. }
            | Self::Verifying { version }
            | Self::Installing { version }
            | Self::Failed { version, .. }
            | Self::ManualUpdate { version, .. } => version,
        }
    }
}

/// Bytes of the package received so far, and its size if the host announced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadedBytes {
    pub received: u64,
    pub total: Option<u64>,
}

/// Everything the toast depends on.
#[derive(Debug, Clone, Copy)]
pub struct UpdateToastInputs<'a> {
    pub status: &'a UpdateStatus,
    /// The newest version the last check found, prepared or not.
    pub newest_version: Option<&'a Version>,
    /// This copy cannot update itself for this reason, and a newer version is out.
    pub manual_update_problem: Option<UpdateProblem>,
    pub is_restart_requested: bool,
    pub progress: Option<DownloadedBytes>,
    pub dismissed_version: Option<&'a Version>,
    pub is_in_call: bool,
}

/// What the toast shows, or `None` when it is hidden: no newer version known, closed for this
/// version, or a background failure nobody asked about (those stay silent, ADR-0017).
#[must_use]
pub fn update_toast_phase(inputs: &UpdateToastInputs<'_>) -> Option<UpdateToastPhase> {
    let version = match inputs.status {
        UpdateStatus::Downloading(version)
        | UpdateStatus::Ready(version)
        | UpdateStatus::Installing(version) => version.clone(),
        UpdateStatus::Checking | UpdateStatus::Failed(_) => inputs.newest_version?.clone(),
        UpdateStatus::Idle | UpdateStatus::UpToDate => return None,
    };
    let is_installing = matches!(inputs.status, UpdateStatus::Installing(_));
    if !is_installing
        && inputs
            .dismissed_version
            .is_some_and(|dismissed| *dismissed >= version)
    {
        return None;
    }
    // Kept while "Check again" looks again, so the toast does not blink away and back.
    if let (UpdateStatus::Checking | UpdateStatus::Failed(_), Some(problem)) =
        (inputs.status, inputs.manual_update_problem)
        && let Some(remedy) = problem.manual_remedy()
    {
        return Some(UpdateToastPhase::ManualUpdate {
            version,
            problem,
            remedy,
        });
    }
    Some(match inputs.status {
        UpdateStatus::Downloading(_) if inputs.is_restart_requested => match inputs.progress {
            Some(DownloadedBytes {
                received,
                total: Some(total),
            }) if received >= total => UpdateToastPhase::Verifying { version },
            progress => UpdateToastPhase::Downloading {
                version,
                percent: progress.and_then(downloaded_percent),
            },
        },
        UpdateStatus::Downloading(_) | UpdateStatus::Ready(_) => UpdateToastPhase::Offered {
            version,
            is_in_call: inputs.is_in_call,
        },
        // "Try again" looks for the release before downloading it again.
        UpdateStatus::Checking if inputs.is_restart_requested => UpdateToastPhase::Downloading {
            version,
            percent: None,
        },
        UpdateStatus::Installing(_) => UpdateToastPhase::Installing { version },
        UpdateStatus::Failed(problem) if inputs.is_restart_requested => UpdateToastPhase::Failed {
            version,
            problem: *problem,
            is_in_call: inputs.is_in_call,
        },
        UpdateStatus::Failed(_)
        | UpdateStatus::Checking
        | UpdateStatus::Idle
        | UpdateStatus::UpToDate => return None,
    })
}

/// The share of the package received, 0 to 100, once its size is known.
#[must_use]
pub fn downloaded_percent(bytes: DownloadedBytes) -> Option<u8> {
    let total = bytes.total.filter(|&total| total > 0)?;
    let percent = u128::from(bytes.received.min(total)) * 100 / u128::from(total);
    u8::try_from(percent).ok()
}

/// The sentence for a problem, as Settings, About says it.
#[must_use]
pub fn problem_sentence(problem: UpdateProblem, catalogue: Strings) -> &'static str {
    let words = catalogue.desktop();
    match problem {
        UpdateProblem::NotPackaged => words.update_not_packaged(),
        UpdateProblem::PackageManager => words.update_package_manager(),
        UpdateProblem::InstalledElsewhere => words.update_installed_elsewhere(),
        UpdateProblem::NotWritable => words.update_not_writable(),
        UpdateProblem::Translocated => words.update_translocated(),
        UpdateProblem::UnsupportedSystem => words.update_unsupported_system(),
        UpdateProblem::Elevated => words.update_elevated(),
        UpdateProblem::NoFfmpeg => words.update_no_ffmpeg(),
        UpdateProblem::Network => words.update_network(),
        UpdateProblem::NoPackage => words.update_no_package(),
        UpdateProblem::BadSignature => words.update_bad_signature(),
        UpdateProblem::BadPackage => words.update_bad_package(),
        UpdateProblem::WrongCertificate => words.update_wrong_certificate(),
        UpdateProblem::InstallFailed => words.update_install_failed(),
    }
}

/// The toast's words for `phase`: title, sentence and what a screen reader hears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateToastWords {
    pub title: String,
    pub sentence: String,
    pub spoken: String,
}

#[must_use]
pub fn update_toast_words(phase: &UpdateToastPhase, catalogue: Strings) -> UpdateToastWords {
    let words = catalogue.desktop();
    let version = phase.version().to_string();
    let available = words.update_toast_title(&version);
    let (title, sentence) = match phase {
        UpdateToastPhase::Offered { is_in_call, .. } => (
            available,
            if *is_in_call {
                words.update_after_call().to_owned()
            } else {
                words.update_toast_body().to_owned()
            },
        ),
        UpdateToastPhase::Downloading { .. } => (available, words.update_downloading(&version)),
        UpdateToastPhase::Verifying { .. } => {
            (available, words.update_toast_verifying().to_owned())
        }
        UpdateToastPhase::Installing { .. } => (
            words.update_installing(&version),
            words.update_toast_restarting().to_owned(),
        ),
        UpdateToastPhase::Failed {
            problem,
            is_in_call,
            ..
        } => (
            words.update_toast_failed_title().to_owned(),
            if *is_in_call {
                format!(
                    "{} {}",
                    problem_sentence(*problem, catalogue),
                    words.update_after_call()
                )
            } else {
                problem_sentence(*problem, catalogue).to_owned()
            },
        ),
        UpdateToastPhase::ManualUpdate { problem, .. } => {
            (available, problem_sentence(*problem, catalogue).to_owned())
        }
    };
    // The percentage is not read out at each step: the bar's own value carries it.
    let spoken = format!("{title}. {sentence}");
    UpdateToastWords {
        title,
        sentence,
        spoken,
    }
}

/// The toast's view in the main window: hidden until there is something to say, and its live
/// regions present from the start so what it says is read out.
pub struct UpdateToast {
    self_update: Option<Entity<SelfUpdate>>,
    focus_handle: FocusHandle,
    /// Whether focus was in the toast at the last frame: when the focused button goes (the
    /// download starts), focus moves to the toast instead of being lost.
    was_focus_inside: bool,
    shown_phase: Option<std::mem::Discriminant<UpdateToastPhase>>,
    was_in_call: bool,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for UpdateToast {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UpdateToast")
            .field("shown_phase", &self.shown_phase)
            .finish_non_exhaustive()
    }
}

impl UpdateToast {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let self_update = super::self_update(cx);
        let mut subscriptions = Vec::new();
        if let Some(self_update) = &self_update {
            subscriptions.push(cx.observe(self_update, |_, _, cx| cx.notify()));
        }
        // The call holds the restart back; only its start and end matter here.
        if let Some(voice) = crate::voice::voice(cx) {
            subscriptions.push(cx.observe(&voice, |toast, _, cx| {
                let is_in_call_now = is_in_call(cx);
                if is_in_call_now != toast.was_in_call {
                    toast.was_in_call = is_in_call_now;
                    cx.notify();
                }
            }));
        }
        Self {
            self_update,
            focus_handle: cx.focus_handle(),
            was_focus_inside: false,
            shown_phase: None,
            was_in_call: is_in_call(cx),
            _subscriptions: subscriptions,
        }
    }
}

impl Render for UpdateToast {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let phase = self
            .self_update
            .as_ref()
            .and_then(|self_update| self_update.read(cx).toast_phase(is_in_call(cx)));
        let catalogue = strings(cx);
        let words = phase
            .as_ref()
            .map(|phase| update_toast_words(phase, catalogue));
        let is_failure = matches!(phase, Some(UpdateToastPhase::Failed { .. }));
        let polite = words
            .as_ref()
            .filter(|_| !is_failure)
            .map(|words| words.spoken.clone())
            .unwrap_or_default();
        let assertive = words
            .as_ref()
            .filter(|_| is_failure)
            .map(|words| words.spoken.clone())
            .unwrap_or_default();

        let discriminant = phase.as_ref().map(std::mem::discriminant);
        let is_focus_inside = self.focus_handle.contains_focused(window, cx);
        if self.was_focus_inside && !is_focus_inside && discriminant != self.shown_phase {
            if phase.is_some() {
                window.focus(&self.focus_handle, cx);
            } else {
                // The toast went (the update restarts, or it was closed): focus goes back to
                // the window, where Tab starts again.
                window.blur(cx);
            }
        }
        self.was_focus_inside = self.focus_handle.contains_focused(window, cx);
        self.shown_phase = discriminant;

        let regions = div()
            .absolute()
            .child(polite_live_region("update-toast-status", polite.into()))
            .child(assertive_live_region(
                "update-toast-alert",
                assertive.into(),
            ));
        let (Some(phase), Some(words), Some(self_update)) =
            (phase, words, self.self_update.clone())
        else {
            return div().child(regions).into_any_element();
        };
        let toast = toast_for(&phase, words, Some(&self_update), catalogue)
            .track_focus(&self.focus_handle)
            .entrance(SharedString::from(phase.version().to_string()));
        div()
            .absolute()
            .right(length(COMPONENT_TOAST_MARGIN))
            .left(length(COMPONENT_TOAST_MARGIN))
            .bottom(length(COMPONENT_TOAST_BOTTOM_OFFSET))
            .flex()
            .justify_end()
            .child(regions)
            .child(toast)
            .into_any_element()
    }
}

/// `button` running `action` on the self-update when pressed; without a self-update (the launch
/// preview), a button that does nothing, so a preview can never check, download or install.
fn wired(
    button: Button,
    self_update: Option<&Entity<SelfUpdate>>,
    action: fn(&mut SelfUpdate, &mut Context<SelfUpdate>),
) -> Button {
    match self_update {
        Some(self_update) => {
            let self_update = self_update.clone();
            button.on_click(move |_, cx| self_update.update(cx, action))
        }
        None => button,
    }
}

/// The toast for `phase`, its buttons wired to the self-update when there is one.
pub(crate) fn toast_for(
    phase: &UpdateToastPhase,
    words: UpdateToastWords,
    self_update: Option<&Entity<SelfUpdate>>,
    catalogue: Strings,
) -> Toast {
    let desktop = catalogue.desktop();
    let mut toast = Toast::new("update-toast", words.title)
        .mark(
            div().flex_none().child(
                Melli::new(
                    "update-toast-melli",
                    MelliState::Idle,
                    COMPONENT_TOAST_MELLI_SIZE,
                )
                .still(true),
            ),
        )
        .sentence(words.sentence.clone());
    let primary = |id: &'static str, label: &'static str| {
        Button::new(id, label)
            .variant(ButtonVariant::Primary)
            .size(ButtonSize::Small)
    };
    match phase {
        UpdateToastPhase::Offered { is_in_call, .. } => {
            toast = toast.action(wired(
                primary("update-toast-update", desktop.update_and_restart()).disabled(*is_in_call),
                self_update,
                SelfUpdate::update_and_restart,
            ));
        }
        UpdateToastPhase::Downloading { percent, .. } => {
            toast = toast.progress(ToastProgress {
                share: percent.map(|percent| f32::from(percent) / 100.0),
                figure: percent
                    .map(|percent| desktop.update_toast_progress(i64::from(percent)).into()),
                label: words.sentence.into(),
            });
        }
        UpdateToastPhase::Verifying { .. } | UpdateToastPhase::Installing { .. } => {
            toast = toast.progress(ToastProgress {
                share: None,
                figure: None,
                label: words.sentence.into(),
            });
        }
        UpdateToastPhase::Failed { is_in_call, .. } => {
            toast = toast.tone(ToastTone::Danger).action(wired(
                primary("update-toast-try-again", catalogue.common().try_again())
                    .disabled(*is_in_call),
                self_update,
                SelfUpdate::update_and_restart,
            ));
        }
        UpdateToastPhase::ManualUpdate { remedy, .. } => match remedy {
            ManualRemedy::DownloadPage => {
                let button = primary("update-toast-download-page", desktop.open_download_page());
                toast = toast.action(match self_update {
                    Some(_) => {
                        button.on_click(|_, cx| cx.open_url(mellow_update::LATEST_RELEASE_PAGE_URL))
                    }
                    None => button,
                });
            }
            ManualRemedy::CheckAgain => {
                toast = toast.action(wired(
                    Button::new("update-toast-check-again", desktop.check_again())
                        .size(ButtonSize::Small),
                    self_update,
                    SelfUpdate::check_again,
                ));
            }
            ManualRemedy::FollowTheSentence => {}
        },
    }
    // Installing cannot be stopped any more: no close button then.
    if !matches!(phase, UpdateToastPhase::Installing { .. }) {
        let self_update = self_update.cloned();
        toast = toast.on_dismiss(desktop.update_toast_dismiss(), move |_, cx: &mut App| {
            if let Some(self_update) = &self_update {
                self_update.update(cx, SelfUpdate::dismiss_toast);
            }
        });
    }
    toast
}
