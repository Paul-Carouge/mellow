//! The main window's root: nothing while the stored session is read, the sign-in card when signed
//! out, the connection banner above the Dusk shell when signed in.

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _,
};

use crate::connection_banner::ConnectionBanner;
use crate::local_widgets::error_line;
use crate::session::{Session, SessionPhase};
use crate::shell::DuskShell;
use crate::sign_in_screen::SignInScreen;
use crate::theme::tokens::{
    COMPONENT_NOTICE_BAR_ICON_SIZE, COMPONENT_NOTICE_BAR_MIN_HEIGHT,
    COMPONENT_NOTICE_BAR_PADDING_BLOCK, COMPONENT_NOTICE_BAR_PADDING_INLINE,
};
use crate::theme::{SANS_FONT_FAMILY, color, length, theme};

pub struct MainWindow {
    session: Entity<Session>,
    /// Built when the session ends and dropped once signed in, so no typed password outlives it.
    sign_in_screen: Option<Entity<SignInScreen>>,
    banner: Entity<ConnectionBanner>,
    shell: Entity<DuskShell>,
    _session_subscription: Subscription,
}

impl std::fmt::Debug for MainWindow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MainWindow")
            .field("is_sign_in_screen_shown", &self.sign_in_screen.is_some())
            .finish_non_exhaustive()
    }
}

impl MainWindow {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe_in(&session, window, |main_window, session, window, cx| {
            main_window.follow_session_phase(session.read(cx).phase(), window, cx);
        });
        let banner = cx.new(|cx| ConnectionBanner::new(session.clone(), cx));
        let shell = cx.new(|_| DuskShell::default());
        let phase = session.read(cx).phase();
        let mut main_window = Self {
            session,
            sign_in_screen: None,
            banner,
            shell,
            _session_subscription: subscription,
        };
        main_window.follow_session_phase(phase, window, cx);
        main_window
    }

    fn follow_session_phase(
        &mut self,
        phase: SessionPhase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match phase {
            SessionPhase::SignedOut if self.sign_in_screen.is_none() => {
                let session = self.session.clone();
                self.sign_in_screen = Some(cx.new(|cx| SignInScreen::new(session, window, cx)));
            }
            SessionPhase::SignedOut => {}
            SessionPhase::RestoringStoredSession | SessionPhase::SignedIn => {
                self.sign_in_screen = None;
            }
        }
        cx.notify();
    }

    #[must_use]
    pub fn sign_in_screen(&self) -> Option<&Entity<SignInScreen>> {
        self.sign_in_screen.as_ref()
    }
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let root = div()
            .size_full()
            .font_family(SANS_FONT_FAMILY)
            .bg(color(tokens.component_auth_card_backdrop));
        if let Some(sign_in_screen) = &self.sign_in_screen {
            return root.child(sign_in_screen.clone());
        }
        if self.session.read(cx).phase() == SessionPhase::RestoringStoredSession {
            return root;
        }
        let sign_out_error = self.session.read(cx).sign_out_error().map(str::to_owned);
        root.flex()
            .flex_col()
            .bg(color(tokens.color_surface_rail))
            .child(self.banner.clone())
            .when_some(sign_out_error, |root, error| {
                root.child(
                    div()
                        .flex()
                        .justify_center()
                        .min_h(length(COMPONENT_NOTICE_BAR_MIN_HEIGHT))
                        .px(length(COMPONENT_NOTICE_BAR_PADDING_INLINE))
                        .py(length(COMPONENT_NOTICE_BAR_PADDING_BLOCK))
                        .bg(color(tokens.color_danger_wash))
                        .child(error_line(
                            error,
                            tokens.color_text_danger,
                            COMPONENT_NOTICE_BAR_ICON_SIZE,
                        )),
                )
            })
            .child(div().flex_1().min_h_0().child(self.shell.clone()))
    }
}
