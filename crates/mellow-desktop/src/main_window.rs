//! The main window's root: nothing while the stored session is read, the sign-in card when signed
//! out, the Dusk shell when signed in (a new one per session, so nothing of the last one stays).

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div,
};

use crate::session::{Session, SessionPhase};
use crate::shell::DuskShell;
use crate::sign_in_screen::SignInScreen;
use crate::theme::{SANS_FONT_FAMILY, color, theme};

pub struct MainWindow {
    session: Entity<Session>,
    /// Built when the session ends and dropped once signed in, so no typed password outlives it.
    sign_in_screen: Option<Entity<SignInScreen>>,
    shell: Option<Entity<DuskShell>>,
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
        let phase = session.read(cx).phase();
        let mut main_window = Self {
            session,
            sign_in_screen: None,
            shell: None,
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
            SessionPhase::SignedOut => {
                self.shell = None;
                if self.sign_in_screen.is_none() {
                    let session = self.session.clone();
                    self.sign_in_screen = Some(cx.new(|cx| SignInScreen::new(session, window, cx)));
                }
            }
            SessionPhase::RestoringStoredSession => self.sign_in_screen = None,
            SessionPhase::SignedIn => {
                self.sign_in_screen = None;
                if self.shell.is_none() {
                    let session = self.session.clone();
                    self.shell = Some(cx.new(|cx| DuskShell::new(session, window, cx)));
                }
            }
        }
        cx.notify();
    }

    #[must_use]
    pub fn sign_in_screen(&self) -> Option<&Entity<SignInScreen>> {
        self.sign_in_screen.as_ref()
    }

    #[must_use]
    pub fn shell(&self) -> Option<&Entity<DuskShell>> {
        self.shell.as_ref()
    }
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let root = div()
            .size_full()
            .font_family(SANS_FONT_FAMILY)
            .bg(color(tokens.component_auth_card_backdrop))
            // Lets message text be selected and copied (gpui-base's window text selection).
            .child(gpui_base::TextSelectionLayer);
        if let Some(sign_in_screen) = &self.sign_in_screen {
            return root.child(sign_in_screen.clone());
        }
        match &self.shell {
            Some(shell) => root
                .bg(color(tokens.color_surface_rail))
                .child(shell.clone()),
            None => root,
        }
    }
}
