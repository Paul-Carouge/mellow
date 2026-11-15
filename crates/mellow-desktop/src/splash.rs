//! The launch screen (ADR-0064, DESIGN.md §5.0): from the window's first frame until the app is
//! ready, Melli waits in front of a low sun on the dusk ground, the app icon come alive. It covers
//! the secret store read and the first connection, never lengthens them, and fades into the screen
//! already drawn underneath: the sign-in card, the shell once the first `Ready` came, or the shell
//! with its connection bar when the instance cannot be reached.

use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationExt as _, BoxShadow, Context, EventEmitter, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Task, Window, div, linear_color_stop, linear_gradient, point,
    prelude::FluentBuilder as _, px,
};

use crate::components::{LiveRegion as _, Melli, MelliState, easing, with_type};
use crate::i18n::strings;
use crate::session::{ConnectionStatus, SessionPhase};
use crate::theme::tokens::{
    COMPONENT_SPLASH_EXIT_DURATION, COMPONENT_SPLASH_GAP, COMPONENT_SPLASH_LONGEST_WAIT,
    COMPONENT_SPLASH_MELLI_SIZE, COMPONENT_SPLASH_STATUS_DELAY, COMPONENT_SPLASH_SUN_BLUR,
    COMPONENT_SPLASH_SUN_OFFSET, COMPONENT_SPLASH_SUN_RISE_DURATION,
    COMPONENT_SPLASH_SUN_RISE_OFFSET, COMPONENT_SPLASH_SUN_SIZE, MOTION_EASING_ENTER,
    MOTION_EASING_EXIT, MOTION_EASING_STANDARD, SPACE_XS, TYPE_BODY, TYPE_DISPLAY,
};
use crate::theme::{animation_duration, color, delay, length, theme};

/// The wordmark under Melli: the product's name, never translated.
const WORDMARK: &str = "Mellow";
/// Where the sky's plum has faded into the ground, as a share of the window's height.
const SKY_FADE_END: f32 = 0.6;

/// What the launch is waiting for, said under the wordmark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchStage {
    /// The session kept in the secret store is being read.
    OpeningSession,
    /// Signed in: the first connection to this instance until its `Ready`.
    Connecting { instance: String },
}

/// What the launch waits for, or `None` once the screen underneath is ready: signed out (the
/// sign-in card), connected, or reconnecting (the shell's connection bar says it better).
#[must_use]
pub fn launch_stage(
    phase: SessionPhase,
    status: ConnectionStatus,
    instance: Option<&str>,
) -> Option<LaunchStage> {
    match (phase, status) {
        (SessionPhase::RestoringStoredSession, _) => Some(LaunchStage::OpeningSession),
        (SessionPhase::SignedIn, ConnectionStatus::Connecting) => Some(LaunchStage::Connecting {
            instance: instance.unwrap_or_default().to_owned(),
        }),
        _ => None,
    }
}

/// The splash has faded out: its host drops it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplashLeft;

pub struct Splash {
    stage: LaunchStage,
    opened_at: Instant,
    /// Set when the fade-out starts.
    leaving_since: Option<Instant>,
    /// The status line shows only after `splash.status-delay`, so a fast start never flashes it.
    is_status_shown: bool,
    _status_timer: Task<()>,
    _longest_wait_timer: Task<()>,
    exit_timer: Option<Task<()>>,
}

impl std::fmt::Debug for Splash {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Splash")
            .field("stage", &self.stage)
            .field("leaving_since", &self.leaving_since)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<SplashLeft> for Splash {}

impl Splash {
    pub fn new(stage: LaunchStage, cx: &mut Context<Self>) -> Self {
        let status_timer = cx.spawn(async move |splash, cx| {
            cx.background_executor()
                .timer(delay(COMPONENT_SPLASH_STATUS_DELAY))
                .await;
            let _ = splash.update(cx, |splash, cx| {
                splash.is_status_shown = true;
                cx.notify();
            });
        });
        // A connection that hangs gives way to the shell, whose bar says "Connecting" and
        // offers nothing the splash could not; the splash never holds the app back.
        let longest_wait_timer = cx.spawn(async move |splash, cx| {
            cx.background_executor()
                .timer(delay(COMPONENT_SPLASH_LONGEST_WAIT))
                .await;
            let _ = splash.update(cx, |splash, cx| splash.leave("still connecting", cx));
        });
        Self {
            stage,
            opened_at: Instant::now(),
            leaving_since: None,
            is_status_shown: false,
            _status_timer: status_timer,
            _longest_wait_timer: longest_wait_timer,
            exit_timer: None,
        }
    }

    /// A splash that stays until its window closes: the launch preview's.
    pub fn held_for_preview(stage: LaunchStage, cx: &mut Context<Self>) -> Self {
        Self {
            _longest_wait_timer: Task::ready(()),
            ..Self::new(stage, cx)
        }
    }

    /// Follows the launch: a new stage changes the status line; `None` fades the splash out.
    pub fn follow(&mut self, stage: Option<LaunchStage>, cx: &mut Context<Self>) {
        match stage {
            Some(stage) if self.leaving_since.is_none() => {
                if stage != self.stage {
                    self.stage = stage;
                    cx.notify();
                }
            }
            Some(_) => {}
            None => self.leave("ready", cx),
        }
    }

    /// Starts the fade-out; once it has played, [`SplashLeft`] tells the host to drop the splash.
    fn leave(&mut self, reason: &'static str, cx: &mut Context<Self>) {
        if self.leaving_since.is_some() {
            return;
        }
        let now = Instant::now();
        let shown_for = now.duration_since(self.opened_at);
        // How long the launch took, without the person's instance or account.
        tracing::info!(
            milliseconds = u64::try_from(shown_for.as_millis()).unwrap_or(u64::MAX),
            reason,
            "the launch screen gives way"
        );
        self.leaving_since = Some(now);
        let exit = animation_duration(COMPONENT_SPLASH_EXIT_DURATION, cx);
        self.exit_timer = Some(cx.spawn(async move |splash, cx| {
            if !exit.is_zero() {
                cx.background_executor().timer(exit).await;
            }
            let _ = splash.update(cx, |_, cx| cx.emit(SplashLeft));
        }));
        cx.notify();
    }

    #[cfg(test)]
    fn is_leaving(&self) -> bool {
        self.leaving_since.is_some()
    }

    fn status_sentence(&self, cx: &Context<Self>) -> String {
        let words = strings(cx).desktop();
        match &self.stage {
            LaunchStage::OpeningSession => words.splash_opening_session().to_owned(),
            LaunchStage::Connecting { instance } if instance.is_empty() => {
                words.splash_opening_session().to_owned()
            }
            LaunchStage::Connecting { instance } => words.splash_connecting(instance),
        }
    }
}

impl Render for Splash {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let sky = color(tokens.component_splash_sky);
        let sun = color(tokens.component_splash_sun);
        let status = self.status_sentence(cx);

        let hero = Melli::new(
            "splash-melli",
            MelliState::Connecting,
            COMPONENT_SPLASH_MELLI_SIZE,
        );

        // Its height is kept from the start, so nothing moves when the line appears.
        let status_line = with_type(div(), TYPE_BODY)
            .id("splash-status")
            .live_polite(if self.is_status_shown {
                status.clone()
            } else {
                String::new()
            })
            .h(length(TYPE_BODY.font_size * TYPE_BODY.line_height))
            .text_color(color(tokens.component_splash_status_text))
            .when(self.is_status_shown, |line| {
                line.child(
                    div().child(status.clone()).with_animation(
                        "splash-status-in",
                        Animation::new(animation_duration(COMPONENT_SPLASH_EXIT_DURATION, cx))
                            .with_easing(|progress| {
                                easing(MOTION_EASING_STANDARD).sample(progress)
                            }),
                        gpui::Styled::opacity,
                    ),
                )
            });

        let content = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(length(COMPONENT_SPLASH_GAP))
            .child(hero)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(length(SPACE_XS))
                    .child(
                        with_type(div(), TYPE_DISPLAY)
                            .text_color(color(tokens.component_splash_wordmark_text))
                            .child(WORDMARK),
                    )
                    .child(status_line),
            );

        let screen = div()
            .id("launch-screen")
            .aria_label(SharedString::from(strings(cx).desktop().splash_label()))
            // Nothing under the splash takes a click or the pointer while it shows.
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(color(tokens.component_splash_ground))
            .overflow_hidden()
            .child(div().absolute().inset_0().bg(linear_gradient(
                180.0,
                linear_color_stop(sky, 0.0),
                linear_color_stop(sky.opacity(0.0), SKY_FADE_END),
            )))
            .child(sunrise_glow(sun))
            .child(content);
        match self.leaving_since {
            None => screen.into_any_element(),
            Some(_) => screen
                .with_animation(
                    "splash-exit",
                    Animation::new(animation_duration(COMPONENT_SPLASH_EXIT_DURATION, cx))
                        .with_easing(|progress| easing(MOTION_EASING_EXIT).sample(progress)),
                    |screen, progress| screen.opacity(1.0 - progress.clamp(0.0, 1.0)),
                )
                .into_any_element(),
        }
    }
}

/// The low sun: a soft apricot glow whose centre sits under the window's bottom edge, as on the
/// app icon's tile. It rises once, as the screen opens. A shadow without a box: GPUI has no radial
/// gradient, and a blurred disc is one.
fn sunrise_glow(sun: Hsla) -> impl IntoElement {
    let rise = Duration::from_millis(COMPONENT_SPLASH_SUN_RISE_DURATION.0);
    let below_edge = COMPONENT_SPLASH_SUN_SIZE / 2.0 + COMPONENT_SPLASH_SUN_OFFSET;
    let glow = div()
        .flex_none()
        .size(length(COMPONENT_SPLASH_SUN_SIZE))
        .rounded_full()
        .shadow(vec![BoxShadow {
            color: sun,
            offset: point(px(0.0), px(0.0)),
            blur_radius: px(COMPONENT_SPLASH_SUN_BLUR),
            spread_radius: px(0.0),
            inset: false,
        }]);
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom(-length(below_edge))
        .h(length(COMPONENT_SPLASH_SUN_SIZE))
        .flex()
        .justify_center()
        .child(glow)
        .with_animation(
            "splash-sunrise",
            Animation::new(rise)
                .with_easing(|progress| easing(MOTION_EASING_ENTER).sample(progress)),
            move |horizon, progress| {
                horizon
                    .bottom(-length(
                        below_edge + COMPONENT_SPLASH_SUN_RISE_OFFSET * (1.0 - progress),
                    ))
                    .opacity(progress.clamp(0.0, 1.0))
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_splash_covers_the_secret_store_read_and_the_first_connection() {
        assert_eq!(
            launch_stage(
                SessionPhase::RestoringStoredSession,
                ConnectionStatus::SignedOut,
                None
            ),
            Some(LaunchStage::OpeningSession)
        );
        assert_eq!(
            launch_stage(
                SessionPhase::SignedIn,
                ConnectionStatus::Connecting,
                Some("chat.example.org")
            ),
            Some(LaunchStage::Connecting {
                instance: "chat.example.org".to_owned()
            })
        );
    }

    #[test]
    fn the_splash_gives_way_as_soon_as_the_screen_underneath_is_ready() {
        for (phase, status) in [
            (SessionPhase::SignedOut, ConnectionStatus::SignedOut),
            (SessionPhase::SignedIn, ConnectionStatus::Connected),
            // The shell's connection bar says it better, with "Retry now".
            (SessionPhase::SignedIn, ConnectionStatus::Reconnecting),
        ] {
            assert_eq!(
                launch_stage(phase, status, Some("chat.example.org")),
                None,
                "{phase:?} {status:?}"
            );
        }
    }

    #[gpui::test]
    fn the_splash_fades_out_once_and_tells_its_host(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext as _;
        let left = std::rc::Rc::new(std::cell::Cell::new(0));
        let splash = cx.new(|cx| Splash::new(LaunchStage::OpeningSession, cx));
        let counted = std::rc::Rc::clone(&left);
        cx.update(|cx| {
            cx.subscribe(&splash, move |_, _: &SplashLeft, _| {
                counted.set(counted.get() + 1);
            })
            .detach();
        });
        splash.update(cx, |splash, cx| {
            splash.follow(
                Some(LaunchStage::Connecting {
                    instance: "chat.example.org".to_owned(),
                }),
                cx,
            );
            assert!(!splash.is_leaving());
            splash.follow(None, cx);
            assert!(splash.is_leaving());
            // A late status change does not bring it back.
            splash.follow(Some(LaunchStage::OpeningSession), cx);
            splash.follow(None, cx);
        });
        cx.executor()
            .advance_clock(animation_duration_for_tests() + Duration::from_millis(1));
        cx.run_until_parked();
        assert_eq!(left.get(), 1);
    }

    #[gpui::test]
    fn a_connection_that_hangs_gives_way_to_the_shell(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext as _;
        let splash = cx.new(|cx| {
            Splash::new(
                LaunchStage::Connecting {
                    instance: "chat.example.org".to_owned(),
                },
                cx,
            )
        });
        cx.executor().advance_clock(
            delay(COMPONENT_SPLASH_LONGEST_WAIT).saturating_sub(Duration::from_millis(1)),
        );
        cx.run_until_parked();
        assert!(!splash.read_with(cx, |splash, _| splash.is_leaving()));
        cx.executor().advance_clock(Duration::from_millis(2));
        cx.run_until_parked();
        assert!(splash.read_with(cx, |splash, _| splash.is_leaving()));
    }

    fn animation_duration_for_tests() -> Duration {
        Duration::from_millis(COMPONENT_SPLASH_EXIT_DURATION.0)
    }
}
