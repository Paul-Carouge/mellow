//! The full-width notice at the top of the window while the gateway connects or reconnects, then
//! "Back online" for a moment (DESIGN.md §5.2 connection dock, §6.37; the web's
//! `ConnectionBanner.svelte` and `connection-notice.ts`).

use std::time::Duration;

use gpui::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, Role,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _, svg,
};
use gpui_base::Button;

use crate::local_widgets::text_style;
use crate::session::{ConnectionStatus, Session};
use crate::strings_pending_i18n::strings;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_BUTTON_HEIGHT_SMALL, COMPONENT_BUTTON_RADIUS, COMPONENT_NOTICE_BAR_ICON_GAP,
    COMPONENT_NOTICE_BAR_ICON_SIZE, COMPONENT_NOTICE_BAR_MIN_HEIGHT,
    COMPONENT_NOTICE_BAR_PADDING_BLOCK, COMPONENT_NOTICE_BAR_PADDING_INLINE, SPACE_2XS, SPACE_SM,
    TYPE_BUTTON,
};
use crate::theme::{color, length, theme};

/// After this long reconnecting, the bar offers "Retry now".
pub const STILL_RECONNECTING_AFTER: Duration = Duration::from_secs(30);
/// "Back online" stays this long, then the bar goes. Not a motion token: reduced motion must not
/// turn the confirmation into a flash.
pub const BACK_ONLINE_SHOWN_FOR: Duration = Duration::from_secs(2);

/// What the bar says, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionTrouble {
    Connecting,
    Reconnecting,
    StillReconnecting,
}

/// shortcut: no "offline" trouble: the desktop has no reliable OS reachability signal yet (the
/// web reads `navigator.onLine`); the gateway's own reconnecting covers it until one is added.
#[must_use]
pub fn connection_trouble(
    status: ConnectionStatus,
    has_been_reconnecting_long: bool,
) -> Option<ConnectionTrouble> {
    match status {
        ConnectionStatus::SignedOut | ConnectionStatus::Connected => None,
        ConnectionStatus::Connecting => Some(ConnectionTrouble::Connecting),
        ConnectionStatus::Reconnecting if has_been_reconnecting_long => {
            Some(ConnectionTrouble::StillReconnecting)
        }
        ConnectionStatus::Reconnecting => Some(ConnectionTrouble::Reconnecting),
    }
}

/// "Back online" follows only a lost connection, never the first connection of the session.
#[must_use]
pub fn is_coming_back_online(
    previous: Option<ConnectionTrouble>,
    current: Option<ConnectionTrouble>,
) -> bool {
    current.is_none() && previous.is_some_and(|previous| previous != ConnectionTrouble::Connecting)
}

pub struct ConnectionBanner {
    session: Entity<Session>,
    has_been_reconnecting_long: bool,
    previous_trouble: Option<ConnectionTrouble>,
    is_back_online_shown: bool,
    /// The status the timers were set for, so a repaint does not restart them.
    observed_status: ConnectionStatus,
    /// Dropping a timer cancels it.
    still_reconnecting_timer: Option<Task<()>>,
    back_online_timer: Option<Task<()>>,
    _session_subscription: Subscription,
}

impl std::fmt::Debug for ConnectionBanner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectionBanner")
            .field("previous_trouble", &self.previous_trouble)
            .field("is_back_online_shown", &self.is_back_online_shown)
            .finish_non_exhaustive()
    }
}

impl ConnectionBanner {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&session, |banner, session, cx| {
            let status = session.read(cx).status();
            if status != banner.observed_status {
                banner.follow_status(status, cx);
            }
        });
        let status = session.read(cx).status();
        let mut banner = Self {
            session,
            has_been_reconnecting_long: false,
            previous_trouble: None,
            is_back_online_shown: false,
            observed_status: ConnectionStatus::SignedOut,
            still_reconnecting_timer: None,
            back_online_timer: None,
            _session_subscription: subscription,
        };
        banner.follow_status(status, cx);
        banner
    }

    #[must_use]
    pub fn trouble(&self) -> Option<ConnectionTrouble> {
        connection_trouble(self.observed_status, self.has_been_reconnecting_long)
    }

    #[must_use]
    pub fn is_back_online_shown(&self) -> bool {
        self.is_back_online_shown
    }

    fn follow_status(&mut self, status: ConnectionStatus, cx: &mut Context<Self>) {
        self.observed_status = status;
        self.has_been_reconnecting_long = false;
        self.still_reconnecting_timer = (status == ConnectionStatus::Reconnecting).then(|| {
            let timer = cx.background_executor().timer(STILL_RECONNECTING_AFTER);
            cx.spawn(async move |banner, cx| {
                timer.await;
                let _ = banner.update(cx, |banner, cx| {
                    banner.has_been_reconnecting_long = true;
                    banner.previous_trouble = banner.trouble();
                    cx.notify();
                });
            })
        });
        let current = self.trouble();
        self.is_back_online_shown = is_coming_back_online(self.previous_trouble, current);
        self.back_online_timer = self.is_back_online_shown.then(|| {
            let timer = cx.background_executor().timer(BACK_ONLINE_SHOWN_FOR);
            cx.spawn(async move |banner, cx| {
                timer.await;
                let _ = banner.update(cx, |banner, cx| {
                    banner.is_back_online_shown = false;
                    cx.notify();
                });
            })
        });
        self.previous_trouble = current;
        cx.notify();
    }
}

impl Render for ConnectionBanner {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let strings = strings(cx);
        let bar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_center()
            .min_h(length(COMPONENT_NOTICE_BAR_MIN_HEIGHT))
            .px(length(COMPONENT_NOTICE_BAR_PADDING_INLINE))
            .py(length(COMPONENT_NOTICE_BAR_PADDING_BLOCK))
            .gap_x(length(COMPONENT_NOTICE_BAR_ICON_GAP))
            .gap_y(length(SPACE_2XS));
        let icon_and_text = |icon: IconName, text: &'static str, ink: u32| {
            text_style(div(), TYPE_BUTTON)
                .flex()
                .items_center()
                .gap(length(COMPONENT_NOTICE_BAR_ICON_GAP))
                .text_color(color(ink))
                .child(
                    svg()
                        .path(icon.asset_path())
                        .flex_none()
                        .size(length(COMPONENT_NOTICE_BAR_ICON_SIZE))
                        .text_color(color(ink)),
                )
                .child(text)
        };
        // A status region, always present, so screen readers announce its changes.
        let region = div().id("connection-banner").role(Role::Status);
        let session = self.session.clone();
        match self.trouble() {
            Some(trouble) => {
                let text = match trouble {
                    ConnectionTrouble::Connecting => strings.connection_connecting,
                    ConnectionTrouble::Reconnecting => strings.connection_reconnecting,
                    ConnectionTrouble::StillReconnecting => strings.connection_still_reconnecting,
                };
                let ink = tokens.component_notice_bar_warning_text;
                region.child(
                    bar.bg(color(tokens.component_notice_bar_warning_background))
                        .child(icon_and_text(IconName::WifiSlash, text, ink))
                        .when(trouble == ConnectionTrouble::StillReconnecting, |bar| {
                            // The bar's own ink as the fill: a second colour on the amber would
                            // compete with it.
                            bar.child(
                                text_style(Button::new("retry-now"), TYPE_BUTTON)
                                    .flex_none()
                                    .h(length(COMPONENT_BUTTON_HEIGHT_SMALL))
                                    .px(length(SPACE_SM))
                                    .flex()
                                    .items_center()
                                    .rounded(length(COMPONENT_BUTTON_RADIUS))
                                    .bg(color(ink))
                                    .text_color(color(
                                        tokens.component_notice_bar_warning_background,
                                    ))
                                    .child(strings.connection_retry_now)
                                    .on_click(move |_, _, cx| {
                                        session.read(cx).reconnect_now();
                                    }),
                            )
                        }),
                )
            }
            None if self.is_back_online_shown => region.child(
                bar.bg(color(tokens.color_positive_wash))
                    .child(icon_and_text(
                        IconName::Check,
                        strings.connection_back_online,
                        tokens.color_text_positive,
                    )),
            ),
            None => region,
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, TestAppContext};
    use mellow_client_core::GatewayUpdate;

    use super::*;

    fn drive(session: &Entity<Session>, update: GatewayUpdate, cx: &mut TestAppContext) {
        session.update(cx, |session, cx| {
            session.apply_gateway_updates_of_this_session(vec![update], cx);
        });
        cx.run_until_parked();
    }

    fn read(
        banner: &Entity<ConnectionBanner>,
        cx: &mut TestAppContext,
    ) -> (Option<ConnectionTrouble>, bool) {
        banner.read_with(cx, |banner, _| {
            (banner.trouble(), banner.is_back_online_shown())
        })
    }

    #[gpui::test]
    fn the_bar_offers_retry_after_a_while_then_says_back_online(cx: &mut TestAppContext) {
        let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
        let banner = cx.new(|cx| ConnectionBanner::new(session.clone(), cx));
        let one_second = Duration::from_secs(1);

        drive(&session, GatewayUpdate::Disconnected, cx);
        assert_eq!(
            read(&banner, cx),
            (Some(ConnectionTrouble::Reconnecting), false)
        );
        cx.executor()
            .advance_clock(STILL_RECONNECTING_AFTER.checked_sub(one_second).unwrap());
        assert_eq!(
            read(&banner, cx),
            (Some(ConnectionTrouble::Reconnecting), false)
        );
        cx.executor().advance_clock(one_second);
        assert_eq!(
            read(&banner, cx),
            (Some(ConnectionTrouble::StillReconnecting), false)
        );

        drive(&session, GatewayUpdate::Resumed, cx);
        assert_eq!(read(&banner, cx), (None, true));
        cx.executor().advance_clock(BACK_ONLINE_SHOWN_FOR);
        assert_eq!(read(&banner, cx), (None, false));

        // A new loss starts its own wait: the earlier one does not carry over.
        drive(&session, GatewayUpdate::Disconnected, cx);
        cx.executor()
            .advance_clock(STILL_RECONNECTING_AFTER.checked_sub(one_second).unwrap());
        assert_eq!(
            read(&banner, cx),
            (Some(ConnectionTrouble::Reconnecting), false)
        );
    }

    #[test]
    fn the_bar_follows_the_connection_status() {
        assert_eq!(connection_trouble(ConnectionStatus::SignedOut, false), None);
        assert_eq!(connection_trouble(ConnectionStatus::Connected, true), None);
        assert_eq!(
            connection_trouble(ConnectionStatus::Connecting, false),
            Some(ConnectionTrouble::Connecting)
        );
        assert_eq!(
            connection_trouble(ConnectionStatus::Reconnecting, false),
            Some(ConnectionTrouble::Reconnecting)
        );
        assert_eq!(
            connection_trouble(ConnectionStatus::Reconnecting, true),
            Some(ConnectionTrouble::StillReconnecting)
        );
    }

    #[test]
    fn back_online_follows_only_a_lost_connection() {
        assert!(!is_coming_back_online(None, None));
        assert!(!is_coming_back_online(
            Some(ConnectionTrouble::Connecting),
            None
        ));
        assert!(is_coming_back_online(
            Some(ConnectionTrouble::Reconnecting),
            None
        ));
        assert!(is_coming_back_online(
            Some(ConnectionTrouble::StillReconnecting),
            None
        ));
        assert!(!is_coming_back_online(
            Some(ConnectionTrouble::Reconnecting),
            Some(ConnectionTrouble::Reconnecting)
        ));
    }
}
