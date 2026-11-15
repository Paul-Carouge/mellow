//! The self-update's state machine behind the toast (ADR-0064), with scripted checks: no network,
//! no download, no installation.

use gpui::{AppContext as _, Entity, TestAppContext};
use semver::Version;

use super::toast::UpdateToastPhase;
use super::{CheckFailure, ManualRemedy, SelfUpdate, UpdateProblem, UpdateStatus};

const NOT_IN_CALL: bool = false;

fn version(minor: u64) -> Version {
    Version::new(0, minor, 0)
}

fn failure(
    problem: UpdateProblem,
    newer_version: Option<Version>,
) -> Result<Option<super::FoundUpdate>, CheckFailure> {
    Err(CheckFailure {
        problem,
        newer_version,
    })
}

fn self_update_answering(
    answers: Vec<Result<Option<super::FoundUpdate>, CheckFailure>>,
    cx: &mut TestAppContext,
) -> Entity<SelfUpdate> {
    cx.new(|cx| SelfUpdate::with_scripted_checks(answers, cx))
}

fn phase(self_update: &Entity<SelfUpdate>, cx: &mut TestAppContext) -> Option<UpdateToastPhase> {
    self_update.read_with(cx, |self_update, _| self_update.toast_phase(NOT_IN_CALL))
}

fn is_restart_requested(self_update: &Entity<SelfUpdate>, cx: &mut TestAppContext) -> bool {
    self_update.read_with(cx, |self_update, _| self_update.is_restart_requested())
}

#[gpui::test]
fn try_again_says_what_failed_then_a_scheduled_check_forgets_the_click(cx: &mut TestAppContext) {
    let self_update = self_update_answering(
        vec![
            failure(UpdateProblem::Network, None),
            failure(UpdateProblem::Network, None),
        ],
        cx,
    );
    self_update.update(cx, |self_update, cx| {
        self_update.set_status_for_tests(UpdateStatus::Downloading(version(62)), cx);
        self_update.set_status_for_tests(UpdateStatus::Failed(UpdateProblem::Network), cx);
    });
    assert_eq!(phase(&self_update, cx), None, "nobody asked: silent");

    self_update.update(cx, SelfUpdate::update_and_restart);
    assert!(is_restart_requested(&self_update, cx));
    assert_eq!(
        phase(&self_update, cx),
        Some(UpdateToastPhase::Downloading {
            version: version(62),
            percent: None
        }),
        "the release is looked up again"
    );
    cx.run_until_parked();
    assert_eq!(
        phase(&self_update, cx),
        Some(UpdateToastPhase::Failed {
            version: version(62),
            problem: UpdateProblem::Network,
            is_in_call: false
        })
    );

    // Hours later, the daily check: it must never restart the app nor speak up.
    self_update.update(cx, SelfUpdate::check_now);
    assert!(!is_restart_requested(&self_update, cx));
    cx.run_until_parked();
    assert_eq!(
        self_update.read_with(cx, |self_update, _| self_update.status().clone()),
        UpdateStatus::Failed(UpdateProblem::Network)
    );
    assert_eq!(phase(&self_update, cx), None);
}

#[gpui::test]
fn a_background_failure_stays_silent(cx: &mut TestAppContext) {
    let self_update =
        self_update_answering(vec![failure(UpdateProblem::Network, Some(version(62)))], cx);
    self_update.update(cx, SelfUpdate::check_now);
    cx.run_until_parked();
    assert!(!is_restart_requested(&self_update, cx));
    assert_eq!(phase(&self_update, cx), None);
}

#[gpui::test]
fn a_click_during_the_download_waits_for_it_and_closing_cancels_the_restart(
    cx: &mut TestAppContext,
) {
    let self_update = self_update_answering(Vec::new(), cx);
    self_update.update(cx, |self_update, cx| {
        self_update.set_status_for_tests(UpdateStatus::Downloading(version(62)), cx);
    });
    assert_eq!(
        phase(&self_update, cx),
        Some(UpdateToastPhase::Offered {
            version: version(62),
            is_in_call: false
        })
    );
    self_update.update(cx, SelfUpdate::update_and_restart);
    assert!(is_restart_requested(&self_update, cx));
    assert_eq!(
        phase(&self_update, cx),
        Some(UpdateToastPhase::Downloading {
            version: version(62),
            percent: None
        })
    );
    self_update.update(cx, SelfUpdate::dismiss_toast);
    assert!(
        !is_restart_requested(&self_update, cx),
        "closed: the download ends quietly"
    );
    assert_eq!(phase(&self_update, cx), None);
}

#[gpui::test]
fn a_closed_toast_comes_back_for_a_newer_version_only(cx: &mut TestAppContext) {
    let self_update = self_update_answering(Vec::new(), cx);
    self_update.update(cx, |self_update, cx| {
        self_update.set_status_for_tests(UpdateStatus::Ready(version(62)), cx);
        self_update.dismiss_toast(cx);
    });
    assert_eq!(phase(&self_update, cx), None);
    self_update.update(cx, |self_update, cx| {
        self_update.set_status_for_tests(UpdateStatus::Ready(version(62)), cx);
    });
    assert_eq!(
        phase(&self_update, cx),
        None,
        "the same version stays closed"
    );
    self_update.update(cx, |self_update, cx| {
        self_update.set_status_for_tests(UpdateStatus::Ready(version(63)), cx);
    });
    assert_eq!(
        phase(&self_update, cx),
        Some(UpdateToastPhase::Offered {
            version: version(63),
            is_in_call: false
        })
    );
}

#[gpui::test]
fn a_copy_that_cannot_update_itself_keeps_its_toast_while_it_checks_again(cx: &mut TestAppContext) {
    let self_update = self_update_answering(
        vec![
            failure(UpdateProblem::NoFfmpeg, Some(version(62))),
            failure(UpdateProblem::NoFfmpeg, Some(version(62))),
        ],
        cx,
    );
    self_update.update(cx, SelfUpdate::check_now);
    cx.run_until_parked();
    let manual = Some(UpdateToastPhase::ManualUpdate {
        version: version(62),
        problem: UpdateProblem::NoFfmpeg,
        remedy: ManualRemedy::CheckAgain,
    });
    assert_eq!(phase(&self_update, cx), manual);
    self_update.update(cx, SelfUpdate::check_again);
    assert_eq!(
        self_update.read_with(cx, |self_update, _| self_update.status().clone()),
        UpdateStatus::Checking
    );
    assert_eq!(phase(&self_update, cx), manual, "no blink while it looks");
    cx.run_until_parked();
    assert_eq!(phase(&self_update, cx), manual);
}
