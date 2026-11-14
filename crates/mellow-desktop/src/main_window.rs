//! The main window's root: nothing while the stored session is read, the sign-in card when signed
//! out, the Dusk shell when signed in (a new one per session, so nothing of the last one stays).

use gpui::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};

use crate::components::Tooltip;
use crate::notifier::{end_notifications, notifier_of_session};
use crate::session::{Session, SessionPhase};
use crate::shell::DuskShell;
use crate::sign_in_screen::SignInScreen;
use crate::theme::{color, interface_font, length, theme};
use crate::window_chrome::{self, WINDOW_TITLE};

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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let chrome = window_chrome::client_chrome(window);
        let (screen, ground): (Option<AnyElement>, u32) = match (&self.sign_in_screen, &self.shell)
        {
            (Some(sign_in_screen), _) => (
                Some(sign_in_screen.clone().into_any_element()),
                tokens.component_auth_card_backdrop,
            ),
            (None, Some(shell)) => (
                Some(shell.clone().into_any_element()),
                tokens.color_surface_rail,
            ),
            (None, None) => (None, tokens.component_auth_card_backdrop),
        };
        let content = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .font(interface_font())
            // Lets message text be selected and copied (gpui-base's window text selection).
            .child(gpui_base::TextSelectionLayer)
            // Hides tooltips on a press or a keystroke (DESIGN.md §6.34).
            .child(Tooltip::dismissal_layer())
            .when(chrome.is_title_band_drawn, |content| {
                content.child(div().flex_none().h(length(chrome.title_band_height())))
            })
            // The screen's layers laid out in it (sheets, the settings sheet) stay below the title
            // band; deferred layers (dialogs, scrims, menus) place themselves there with
            // `window_chrome::client_content_bounds`.
            .child(div().relative().flex_1().min_h_0().flex().children(screen))
            // Drawn last: above the screen, and after it in the Tab order.
            .when(chrome.is_title_band_drawn, |content| {
                content.child(window_chrome::title_band(window, cx))
            });
        window_chrome::framed_window_content(content, chrome, color(ground), window, cx)
    }
}
