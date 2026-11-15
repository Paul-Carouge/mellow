//! What the update toast shows for each state of the self-update (ADR-0064).

use semver::Version;

use super::toast::{
    DownloadedBytes, UpdateToastInputs, UpdateToastPhase, downloaded_percent, update_toast_phase,
    update_toast_words,
};
use super::{ManualRemedy, UpdateProblem, UpdateStatus};
use crate::i18n::{Language, Strings};

fn version(minor: u64) -> Version {
    Version::new(0, minor, 0)
}

/// Inputs with nothing asked, nothing dismissed and no call.
fn inputs(status: &UpdateStatus) -> UpdateToastInputs<'_> {
    UpdateToastInputs {
        status,
        newest_version: None,
        manual_update_problem: None,
        is_restart_requested: false,
        progress: None,
        dismissed_version: None,
        is_in_call: false,
    }
}

#[test]
fn nothing_shows_before_a_newer_version_is_known() {
    for status in [
        UpdateStatus::Idle,
        UpdateStatus::Checking,
        UpdateStatus::UpToDate,
    ] {
        assert_eq!(update_toast_phase(&inputs(&status)), None, "{status:?}");
    }
}

#[test]
fn a_version_downloading_in_the_background_or_ready_is_offered() {
    for status in [
        UpdateStatus::Downloading(version(62)),
        UpdateStatus::Ready(version(62)),
    ] {
        assert_eq!(
            update_toast_phase(&inputs(&status)),
            Some(UpdateToastPhase::Offered {
                version: version(62),
                is_in_call: false
            }),
            "{status:?}"
        );
    }
}

#[test]
fn the_offer_waits_during_a_call() {
    let status = UpdateStatus::Ready(version(62));
    let phase = update_toast_phase(&UpdateToastInputs {
        is_in_call: true,
        ..inputs(&status)
    });
    assert_eq!(
        phase,
        Some(UpdateToastPhase::Offered {
            version: version(62),
            is_in_call: true
        })
    );
    let words = update_toast_words(&phase.unwrap(), Strings::new(Language::French));
    assert_eq!(words.title, "Mellow 0.62.0 est disponible");
    assert_eq!(
        words.sentence,
        "Quitte l’appel pour redémarrer et mettre à jour."
    );
}

#[test]
fn a_requested_download_shows_its_progress_then_the_verification() {
    let status = UpdateStatus::Downloading(version(62));
    let asked = |progress| UpdateToastInputs {
        is_restart_requested: true,
        progress,
        ..inputs(&status)
    };
    assert_eq!(
        update_toast_phase(&asked(None)),
        Some(UpdateToastPhase::Downloading {
            version: version(62),
            percent: None
        }),
        "no byte yet: the bar sweeps"
    );
    let unknown_size = DownloadedBytes {
        received: 4_000,
        total: None,
    };
    assert_eq!(
        update_toast_phase(&asked(Some(unknown_size))),
        Some(UpdateToastPhase::Downloading {
            version: version(62),
            percent: None
        }),
        "a host that announces no size gets the sweeping bar"
    );
    let half = DownloadedBytes {
        received: 42_000_000,
        total: Some(84_000_000),
    };
    assert_eq!(
        update_toast_phase(&asked(Some(half))),
        Some(UpdateToastPhase::Downloading {
            version: version(62),
            percent: Some(50)
        })
    );
    let complete = DownloadedBytes {
        received: 84_000_000,
        total: Some(84_000_000),
    };
    assert_eq!(
        update_toast_phase(&asked(Some(complete))),
        Some(UpdateToastPhase::Verifying {
            version: version(62)
        })
    );
}

#[test]
fn the_percentage_is_floored_and_bounded() {
    let bytes = |received, total| DownloadedBytes { received, total };
    assert_eq!(downloaded_percent(bytes(0, Some(100))), Some(0));
    assert_eq!(downloaded_percent(bytes(999, Some(1_000))), Some(99));
    assert_eq!(downloaded_percent(bytes(5_000, Some(1_000))), Some(100));
    assert_eq!(downloaded_percent(bytes(10, Some(0))), None);
    assert_eq!(
        downloaded_percent(bytes(u64::MAX, Some(u64::MAX))),
        Some(100)
    );
}

#[test]
fn installing_shows_even_after_the_toast_was_closed() {
    let status = UpdateStatus::Installing(version(62));
    let phase = update_toast_phase(&UpdateToastInputs {
        dismissed_version: Some(&version(62)),
        ..inputs(&status)
    });
    assert_eq!(
        phase,
        Some(UpdateToastPhase::Installing {
            version: version(62)
        }),
        "the restart comes from the user card's button: say why the window goes"
    );
}

#[test]
fn a_closed_toast_stays_closed_for_its_version_and_comes_back_for_a_newer_one() {
    let same = UpdateStatus::Ready(version(62));
    let dismissed = version(62);
    let closed = |status| UpdateToastInputs {
        dismissed_version: Some(&dismissed),
        ..inputs(status)
    };
    assert_eq!(update_toast_phase(&closed(&same)), None);
    let older = UpdateStatus::Downloading(version(61));
    assert_eq!(update_toast_phase(&closed(&older)), None);
    let newer = UpdateStatus::Ready(version(63));
    assert_eq!(
        update_toast_phase(&closed(&newer)),
        Some(UpdateToastPhase::Offered {
            version: version(63),
            is_in_call: false
        })
    );
}

#[test]
fn a_failure_after_the_click_offers_to_try_again() {
    let status = UpdateStatus::Failed(UpdateProblem::Network);
    let phase = update_toast_phase(&UpdateToastInputs {
        newest_version: Some(&version(62)),
        is_restart_requested: true,
        ..inputs(&status)
    });
    assert_eq!(
        phase,
        Some(UpdateToastPhase::Failed {
            version: version(62),
            problem: UpdateProblem::Network,
            is_in_call: false
        })
    );
    let words = update_toast_words(&phase.unwrap(), Strings::new(Language::English));
    assert_eq!(words.title, "The update didn't finish");
    assert_eq!(
        words.sentence,
        "Couldn't download the update. Check your connection and try again."
    );
}

#[test]
fn trying_again_shows_the_download_while_the_release_is_looked_up() {
    let status = UpdateStatus::Checking;
    assert_eq!(
        update_toast_phase(&UpdateToastInputs {
            newest_version: Some(&version(62)),
            is_restart_requested: true,
            ..inputs(&status)
        }),
        Some(UpdateToastPhase::Downloading {
            version: version(62),
            percent: None
        })
    );
}

#[test]
fn a_background_failure_nobody_asked_about_stays_silent() {
    for problem in [
        UpdateProblem::Network,
        UpdateProblem::BadSignature,
        UpdateProblem::NoPackage,
    ] {
        let status = UpdateStatus::Failed(problem);
        assert_eq!(
            update_toast_phase(&UpdateToastInputs {
                newest_version: Some(&version(62)),
                ..inputs(&status)
            }),
            None,
            "{problem:?}"
        );
    }
}

#[test]
fn a_copy_that_cannot_update_itself_points_to_its_remedy() {
    let cases = [
        (
            UpdateProblem::InstalledElsewhere,
            ManualRemedy::DownloadPage,
        ),
        (UpdateProblem::NotWritable, ManualRemedy::DownloadPage),
        (UpdateProblem::Translocated, ManualRemedy::FollowTheSentence),
        (UpdateProblem::NoFfmpeg, ManualRemedy::CheckAgain),
    ];
    for (problem, remedy) in cases {
        let status = UpdateStatus::Failed(problem);
        assert_eq!(
            update_toast_phase(&UpdateToastInputs {
                newest_version: Some(&version(62)),
                manual_update_problem: Some(problem),
                ..inputs(&status)
            }),
            Some(UpdateToastPhase::ManualUpdate {
                version: version(62),
                problem,
                remedy
            }),
            "{problem:?}"
        );
    }
}

#[test]
fn package_manager_and_source_copies_never_get_a_toast() {
    for problem in [
        UpdateProblem::PackageManager,
        UpdateProblem::NotPackaged,
        UpdateProblem::Elevated,
        UpdateProblem::UnsupportedSystem,
    ] {
        assert_eq!(problem.manual_remedy(), None, "{problem:?}");
        let status = UpdateStatus::Failed(problem);
        assert_eq!(
            update_toast_phase(&UpdateToastInputs {
                newest_version: Some(&version(62)),
                manual_update_problem: Some(problem),
                ..inputs(&status)
            }),
            None,
            "{problem:?}"
        );
    }
}

#[test]
fn the_toast_speaks_french_with_its_typography() {
    let phase = UpdateToastPhase::Downloading {
        version: version(62),
        percent: Some(42),
    };
    let catalogue = Strings::new(Language::French);
    let words = update_toast_words(&phase, catalogue);
    assert_eq!(words.title, "Mellow 0.62.0 est disponible");
    assert_eq!(words.sentence, "Téléchargement de Mellow 0.62.0…");
    assert_eq!(
        catalogue.desktop().update_toast_progress(42),
        "42\u{202F}%",
        "a narrow no-break space before the percent sign"
    );
}

#[test]
fn a_manual_update_stays_while_the_copy_checks_again() {
    let status = UpdateStatus::Checking;
    assert_eq!(
        update_toast_phase(&UpdateToastInputs {
            newest_version: Some(&version(62)),
            manual_update_problem: Some(UpdateProblem::NoFfmpeg),
            ..inputs(&status)
        }),
        Some(UpdateToastPhase::ManualUpdate {
            version: version(62),
            problem: UpdateProblem::NoFfmpeg,
            remedy: ManualRemedy::CheckAgain
        })
    );
}

#[test]
fn trying_again_waits_for_the_end_of_the_call() {
    let status = UpdateStatus::Failed(UpdateProblem::Network);
    let phase = update_toast_phase(&UpdateToastInputs {
        newest_version: Some(&version(62)),
        is_restart_requested: true,
        is_in_call: true,
        ..inputs(&status)
    });
    assert_eq!(
        phase,
        Some(UpdateToastPhase::Failed {
            version: version(62),
            problem: UpdateProblem::Network,
            is_in_call: true
        })
    );
    let words = update_toast_words(&phase.unwrap(), Strings::new(Language::French));
    assert_eq!(
        words.sentence,
        "Impossible de télécharger la mise à jour. Vérifie ta connexion et réessaie. \
         Quitte l’appel pour redémarrer et mettre à jour."
    );
}
