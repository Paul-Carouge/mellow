//! About (`UserSettings.svelte`'s last section): the app's name, its version and self-update
//! (ADR-0057), its licence and where its code lives. The web's "Third-party licences" link opens
//! the notices the instance serves with the web client, which do not describe this app, so the
//! desktop leaves it out.

use gpui::{
    App, Context, EventEmitter, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _,
};

use super::parts::{body_text, error_line, section_label};
use crate::components::{Button, ButtonVariant, Switch, polite_live_region, with_type};
use crate::i18n::{Strings, strings};
use crate::preferences::{PreferencesStore, change_preferences, preferences};
use crate::self_update::{
    SelfUpdate, UpdateProblem, UpdateStatus, can_restart_to_update, is_in_call, self_update,
};
use crate::settings::{SettingsPage, SettingsPageEvent};
use crate::theme::tokens::{
    COMPONENT_SETTINGS_ACTION_GAP, COMPONENT_SETTINGS_PAGE_TITLE_GAP,
    COMPONENT_SETTINGS_SECTION_GAP, COMPONENT_SETTINGS_SECTION_LABEL_GAP, SPACE_SM,
    TYPE_HEADING_MD,
};
use crate::theme::{color, length, theme};

const SOURCE_CODE_URL: &str = "https://github.com/Paul-Carouge/mellow";

/// Follows the self-update, the preferences (the automatic updates switch) and the call (which
/// holds back the restart).
pub struct AboutPage {
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for AboutPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AboutPage").finish_non_exhaustive()
    }
}

impl EventEmitter<SettingsPageEvent> for AboutPage {}

impl SettingsPage for AboutPage {
    fn set_attention(&mut self, _: u32, _: &mut Context<Self>) {}
}

impl AboutPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![cx.observe_global::<PreferencesStore>(|_, cx| cx.notify())];
        if let Some(self_update) = self_update(cx) {
            subscriptions.push(cx.observe(&self_update, |_, _, cx| cx.notify()));
        }
        if let Some(voice) = crate::voice::voice(cx) {
            subscriptions.push(cx.observe(&voice, |_, _, cx| cx.notify()));
        }
        Self {
            _subscriptions: subscriptions,
        }
    }
}

/// The sentence for where the update stands; `None` before any check.
fn status_sentence(status: &UpdateStatus, catalogue: Strings) -> Option<String> {
    let words = catalogue.desktop();
    Some(match status {
        UpdateStatus::Idle => return None,
        UpdateStatus::Checking => words.update_checking().to_owned(),
        UpdateStatus::UpToDate => words.update_up_to_date().to_owned(),
        UpdateStatus::Downloading(version) => words.update_downloading(&version.to_string()),
        UpdateStatus::Ready(version) => words.update_ready(&version.to_string()),
        UpdateStatus::Installing(version) => words.update_installing(&version.to_string()),
        UpdateStatus::Failed(problem) => problem_sentence(*problem, catalogue).to_owned(),
    })
}

fn problem_sentence(problem: UpdateProblem, catalogue: Strings) -> &'static str {
    let words = catalogue.desktop();
    match problem {
        UpdateProblem::NotPackaged => words.update_not_packaged(),
        UpdateProblem::PackageManager => words.update_package_manager(),
        UpdateProblem::InstalledElsewhere => words.update_installed_elsewhere(),
        UpdateProblem::NotWritable => words.update_not_writable(),
        UpdateProblem::UnsupportedSystem => words.update_unsupported_system(),
        UpdateProblem::NoFfmpeg => words.update_no_ffmpeg(),
        UpdateProblem::Network => words.update_network(),
        UpdateProblem::NoPackage => words.update_no_package(),
        UpdateProblem::BadSignature => words.update_bad_signature(),
        UpdateProblem::BadPackage => words.update_bad_package(),
        UpdateProblem::WrongCertificate => words.update_wrong_certificate(),
        UpdateProblem::InstallFailed => words.update_install_failed(),
    }
}

/// The version, where the update stands, its button and the automatic updates switch.
fn updates_section(cx: &App) -> gpui::Div {
    let catalogue = strings(cx);
    let words = catalogue.desktop();
    let status = self_update(cx).map_or(UpdateStatus::Idle, |update| {
        update.read(cx).status().clone()
    });
    let is_in_call = is_in_call(cx);
    let is_ready = matches!(status, UpdateStatus::Ready(_));
    let is_busy = matches!(
        status,
        UpdateStatus::Checking | UpdateStatus::Downloading(_) | UpdateStatus::Installing(_)
    );
    let (problem, progress) = match &status {
        UpdateStatus::Failed(_) => (status_sentence(&status, catalogue), None),
        _ => (None, status_sentence(&status, catalogue)),
    };
    let is_automatic =
        preferences(cx).is_none_or(|preferences| !preferences.is_automatic_update_off);
    let action = if is_ready {
        Button::new("about-restart-to-update", words.restart_to_update())
            .variant(ButtonVariant::Primary)
            .disabled(!can_restart_to_update(&status, is_in_call))
            .on_click(|_, cx| {
                if let Some(self_update) = self_update(cx) {
                    self_update.update(cx, SelfUpdate::restart_to_update);
                }
            })
    } else {
        Button::new("about-check-for-updates", words.check_for_updates())
            .variant(ButtonVariant::Secondary)
            .disabled(is_busy || self_update(cx).is_none())
            .on_click(|_, cx| {
                if let Some(self_update) = self_update(cx) {
                    self_update.update(cx, SelfUpdate::check_now);
                }
            })
    };
    let progress: SharedString = progress.unwrap_or_default().into();
    div()
        .mt(length(COMPONENT_SETTINGS_SECTION_GAP))
        .flex()
        .flex_col()
        .items_start()
        .child(section_label("about-updates", words.updates(), cx))
        .child(
            body_text(words.app_version(env!("CARGO_PKG_VERSION")), cx)
                .mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP)),
        )
        // Always drawn, empty before a check: screen readers miss a region inserted with its text.
        .child(polite_live_region(
            "about-update-status-announcement",
            progress.clone(),
        ))
        .when(!progress.is_empty(), |section| {
            section.child(body_text(progress, cx))
        })
        .when(is_ready && is_in_call, |section| {
            section.child(body_text(words.update_after_call(), cx))
        })
        .child(error_line(
            "about-update-problem",
            problem.map(Into::into),
            cx,
        ))
        .child(
            div()
                .mt(length(COMPONENT_SETTINGS_ACTION_GAP))
                .child(action),
        )
        .child(
            div().mt(length(SPACE_SM)).child(
                Switch::new(
                    "about-automatic-updates",
                    is_automatic,
                    words.automatic_updates(),
                )
                .description(words.automatic_updates_description())
                .on_change(|is_on, _, cx| {
                    change_preferences(cx, move |preferences| {
                        preferences.is_automatic_update_off = !is_on;
                    });
                }),
            ),
        )
}

impl Render for AboutPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalogue = strings(cx);
        let words = catalogue.settings();
        div()
            .flex()
            .flex_col()
            .items_start()
            .child(
                with_type(div(), TYPE_HEADING_MD)
                    .mt(length(COMPONENT_SETTINGS_PAGE_TITLE_GAP))
                    .text_color(color(theme(cx).component_settings_app_name_text))
                    .child(catalogue.desktop().desktop_app()),
            )
            .child(updates_section(cx))
            .child(
                div()
                    .mt(length(COMPONENT_SETTINGS_SECTION_GAP))
                    .child(section_label("about-source-code", words.source_code(), cx)),
            )
            .child(body_text(words.licence(), cx).mt(length(COMPONENT_SETTINGS_SECTION_LABEL_GAP)))
            .child(
                div().mt(length(COMPONENT_SETTINGS_ACTION_GAP)).child(
                    Button::new("about-source-code-link", words.source_code_on_git_hub())
                        .variant(ButtonVariant::Ghost)
                        .on_click(|_, cx| cx.open_url(SOURCE_CODE_URL)),
                ),
            )
    }
}
