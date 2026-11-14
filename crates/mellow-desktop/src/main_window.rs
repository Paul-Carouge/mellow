//! The main window's root: nothing while the stored session is read, the sign-in card when signed
//! out, the Dusk shell when signed in (a new one per session, so nothing of the last one stays).

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div,
};

use crate::components::Tooltip;
use crate::notifier::{end_notifications, notifier_of_session};
use crate::session::{Session, SessionPhase};
use crate::shell::DuskShell;
use crate::sign_in_screen::SignInScreen;
use crate::theme::{color, interface_font, theme};
use crate::window_chrome::WINDOW_TITLE;

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
                // Signed out, nothing more is told; the notifier cannot reach this window's
                // title while it is being updated, so the window resets it.
                end_notifications(cx);
                window.set_window_title(WINDOW_TITLE);
                self.shell = None;
                if self.sign_in_screen.is_none() {
                    let session = self.session.clone();
                    let screen = cx.new(|cx| SignInScreen::new(session, window, cx));
                    // "Connect to another instance": straight to the address, filled with the
                    // one just left.
                    if self
                        .session
                        .update(cx, |session, _| session.take_leaving_for_another_instance())
                    {
                        screen.update(cx, |screen, cx| screen.use_another_instance(window, cx));
                    }
                    self.sign_in_screen = Some(screen);
                }
            }
            SessionPhase::RestoringStoredSession => self.sign_in_screen = None,
            SessionPhase::SignedIn => {
                self.sign_in_screen = None;
                if self.shell.is_none() {
                    let session = self.session.clone();
                    let shell = cx.new(|cx| DuskShell::new(session.clone(), window, cx));
                    // The app's notifier outlives this window: Mellow keeps notifying in the
                    // background once it closes.
                    notifier_of_session(&session, cx)
                        .update(cx, |notifier, cx| notifier.show_in(&shell, window, cx));
                    self.shell = Some(shell);
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
            .font(interface_font())
            .bg(color(tokens.component_auth_card_backdrop))
            // Lets message text be selected and copied (gpui-base's window text selection).
            .child(gpui_base::TextSelectionLayer)
            // Hides tooltips on a press or a keystroke (DESIGN.md §6.34).
            .child(Tooltip::dismissal_layer());
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
