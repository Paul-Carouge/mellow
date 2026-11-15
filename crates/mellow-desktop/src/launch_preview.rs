//! The hidden preview of the launch screen and the update toast (`MELLOW_PREVIEW=splash` or
//! `=toasts`, with `-dark` or `-light` to force a theme and `-still` for reduced motion): the screens a running app shows only
//! for a moment or after a release, held still for screenshots and design review. No network and
//! no installation: the app opens the preview before it installs its self-update, and the toasts
//! are drawn from fixed states with buttons that do nothing. Developer-facing, like the component
//! gallery, which it opens beside.

use gpui::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Window, WindowAppearance, div,
};
use semver::Version;

use crate::components::gallery::gallery_window_options;
use crate::components::with_type;
use crate::self_update::toast::{UpdateToastPhase, toast_for, update_toast_words};
use crate::self_update::{ManualRemedy, UpdateProblem};
use crate::splash::{LaunchStage, Splash};
use crate::theme::tokens::{SPACE_LG, SPACE_MD, TYPE_CATEGORY};
use crate::theme::{ActiveTheme, apply_theme, color, length, theme};
use crate::window_chrome::TITLE_BAR_BAND_HEIGHT;

/// The environment variable that opens the preview instead of the app.
pub const LAUNCH_PREVIEW_VARIABLE: &str = "MELLOW_PREVIEW";

/// What the preview shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewSubject {
    Splash,
    Toasts,
}

/// What the variable asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewRequest {
    pub subject: PreviewSubject,
    /// A forced theme, or the system's.
    pub appearance: Option<WindowAppearance>,
    /// Reduced motion, whatever the system says.
    pub is_still: bool,
}

/// The preview the variable names, if it names one.
#[must_use]
pub fn preview_requested() -> Option<PreviewRequest> {
    parse_preview_request(&std::env::var(LAUNCH_PREVIEW_VARIABLE).ok()?)
}

fn parse_preview_request(value: &str) -> Option<PreviewRequest> {
    let mut words = value.split('-');
    let subject = match words.next()? {
        "splash" => PreviewSubject::Splash,
        "toasts" => PreviewSubject::Toasts,
        _ => return None,
    };
    let mut request = PreviewRequest {
        subject,
        appearance: None,
        is_still: false,
    };
    for word in words {
        match word {
            "dark" => request.appearance = Some(WindowAppearance::Dark),
            "light" => request.appearance = Some(WindowAppearance::Light),
            "still" => request.is_still = true,
            _ => return None,
        }
    }
    Some(request)
}

/// Opens the preview window; the app quits when it closes.
///
/// # Errors
/// When the window cannot open.
pub fn open_launch_preview(request: PreviewRequest, cx: &mut App) -> anyhow::Result<()> {
    cx.on_window_closed(|cx, _| cx.quit()).detach();
    if request.is_still {
        cx.set_reduce_motion(true);
    }
    cx.open_window(gallery_window_options(cx), |window, cx| {
        match request.appearance {
            Some(appearance) => apply_theme(ActiveTheme::for_appearance(appearance), cx),
            None => crate::theme::follow_window_appearance(window, cx),
        }
        cx.new(|cx| LaunchPreview::new(request.subject, cx))
    })?;
    cx.activate(true);
    Ok(())
}

struct LaunchPreview {
    subject: PreviewSubject,
    splash: Option<Entity<Splash>>,
}

impl LaunchPreview {
    fn new(subject: PreviewSubject, cx: &mut Context<Self>) -> Self {
        let splash = (subject == PreviewSubject::Splash).then(|| {
            cx.new(|cx| {
                Splash::held_for_preview(
                    LaunchStage::Connecting {
                        instance: "chat.example.org".to_owned(),
                    },
                    cx,
                )
            })
        });
        Self { subject, splash }
    }
}

/// One toast per state, as `update_toast_phase` gives them.
fn sample_phases() -> Vec<(&'static str, UpdateToastPhase)> {
    let version = Version::new(0, 62, 0);
    vec![
        (
            "Offered",
            UpdateToastPhase::Offered {
                version: version.clone(),
                is_in_call: false,
            },
        ),
        (
            "Offered during a call",
            UpdateToastPhase::Offered {
                version: version.clone(),
                is_in_call: true,
            },
        ),
        (
            "Downloading",
            UpdateToastPhase::Downloading {
                version: version.clone(),
                percent: Some(42),
            },
        ),
        (
            "Downloading, size unknown",
            UpdateToastPhase::Downloading {
                version: version.clone(),
                percent: None,
            },
        ),
        (
            "Verifying",
            UpdateToastPhase::Verifying {
                version: version.clone(),
            },
        ),
        (
            "Installing",
            UpdateToastPhase::Installing {
                version: version.clone(),
            },
        ),
        (
            "Failed",
            UpdateToastPhase::Failed {
                version: version.clone(),
                problem: UpdateProblem::Network,
                is_in_call: false,
            },
        ),
        (
            "Failed, during a call",
            UpdateToastPhase::Failed {
                version: version.clone(),
                problem: UpdateProblem::Network,
                is_in_call: true,
            },
        ),
        (
            "Cannot update itself",
            UpdateToastPhase::ManualUpdate {
                version: version.clone(),
                problem: UpdateProblem::InstalledElsewhere,
                remedy: ManualRemedy::DownloadPage,
            },
        ),
        (
            "Opened from App Translocation",
            UpdateToastPhase::ManualUpdate {
                version,
                problem: UpdateProblem::Translocated,
                remedy: ManualRemedy::FollowTheSentence,
            },
        ),
    ]
}

impl Render for LaunchPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        if let Some(splash) = &self.splash {
            return div()
                .relative()
                .size_full()
                .bg(color(tokens.component_splash_ground))
                .child(splash.clone())
                .into_any_element();
        }
        debug_assert_eq!(self.subject, PreviewSubject::Toasts);
        let catalogue = crate::i18n::strings(cx);
        let toasts = sample_phases().into_iter().map(|(name, phase)| {
            let words = update_toast_words(&phase, catalogue);
            // No self-update: the preview opens before it is installed, and its buttons do nothing.
            let toast = toast_for(&phase, words, None, catalogue);
            div()
                .flex()
                .flex_col()
                .gap(length(SPACE_MD / 2.0))
                .child(
                    with_type(div(), TYPE_CATEGORY)
                        .text_color(color(tokens.color_text_muted))
                        .child(name),
                )
                .child(toast.entrance(name))
        });
        div()
            .size_full()
            .bg(color(tokens.color_surface_chat))
            .p(length(SPACE_LG))
            .pt(length(TITLE_BAR_BAND_HEIGHT))
            .flex()
            .flex_wrap()
            .content_start()
            .gap(length(SPACE_LG))
            .children(toasts)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use gpui::{
        FocusHandle, InputEvent as _, KeyUpEvent, Keystroke, TestAppContext, VisualTestContext,
    };
    use semver::Version;

    use super::*;
    use crate::components::{bind_tab_navigation, with_tab_navigation};
    use crate::self_update::self_update;

    /// The app opens the preview and the gallery before it installs its self-update, so neither
    /// can check, download or install a release.
    #[test]
    fn the_preview_and_the_gallery_open_before_the_self_update_is_installed() {
        let start = include_str!("lib.rs");
        let installed_at = start
            .find("self_update::install_self_update(cx)")
            .expect("start installs the self-update");
        for exit in [
            "launch_preview::preview_requested()",
            "components::gallery::gallery_requested()",
        ] {
            let exit_at = start
                .find(exit)
                .expect("start opens the preview and the gallery");
            assert!(exit_at < installed_at, "{exit} comes after the self-update");
        }
    }

    struct PreviewToastHarness {
        root: FocusHandle,
    }

    impl Render for PreviewToastHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let phase = UpdateToastPhase::Offered {
                version: Version::new(0, 62, 0),
                is_in_call: false,
            };
            let catalogue = crate::i18n::strings(cx);
            let words = update_toast_words(&phase, catalogue);
            with_tab_navigation(div().size_full(), &self.root)
                .child(toast_for(&phase, words, None, catalogue))
        }
    }

    fn press(cx: &mut VisualTestContext, key: &'static str) {
        cx.simulate_keystrokes(key);
        let keystroke = Keystroke::parse(key).expect("a valid keystroke in a test");
        cx.update(|window, cx| {
            window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn a_preview_toast_s_buttons_do_nothing(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
            bind_tab_navigation(cx);
        });
        let (harness, cx) = cx.add_window_view(|_, cx| PreviewToastHarness {
            root: cx.focus_handle(),
        });
        let root = harness.read_with(cx, |harness, _| harness.root.clone());
        cx.update(|window, cx| {
            window.activate_window();
            window.focus(&root, cx);
        });
        cx.run_until_parked();
        // "Update and restart", then the close button.
        for _ in 0..2 {
            press(cx, "tab");
            press(cx, "enter");
        }
        assert!(cx.update(|_, cx| self_update(cx).is_none()));
    }

    #[test]
    fn reads_the_subject_and_the_forced_theme() {
        let request = |subject, appearance, is_still| {
            Some(PreviewRequest {
                subject,
                appearance,
                is_still,
            })
        };
        assert_eq!(
            parse_preview_request("splash"),
            request(PreviewSubject::Splash, None, false)
        );
        assert_eq!(
            parse_preview_request("toasts-light"),
            request(PreviewSubject::Toasts, Some(WindowAppearance::Light), false)
        );
        assert_eq!(
            parse_preview_request("splash-dark-still"),
            request(PreviewSubject::Splash, Some(WindowAppearance::Dark), true)
        );
        assert_eq!(parse_preview_request("splash-blue"), None);
        assert_eq!(parse_preview_request("gallery"), None);
        assert_eq!(parse_preview_request(""), None);
    }
}
