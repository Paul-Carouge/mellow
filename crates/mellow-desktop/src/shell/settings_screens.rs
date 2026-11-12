//! The server, channel and user settings over the shell (`Shell.svelte`'s `openDialog`): one at a time,
//! opened from the place menus and the chat header, closed by Esc, its close button, or when what
//! it shows goes away; focus goes back where it came from.

use gpui::{AppContext as _, Context, Entity, FocusHandle, IntoElement, Subscription, Window};
use mellow_client_core::shared_logic::server_settings::ServerSettingsSection;
use mellow_protocol::Snowflake;

use super::DuskShell;
use crate::settings::SettingsClosed;
use crate::settings::channel::ChannelSettings;
use crate::settings::server::ServerSettings;
use crate::settings::user::UserSettings;

/// The settings screen open over the shell.
pub(super) enum SettingsScreen {
    Server(Entity<ServerSettings>),
    Channel(Entity<ChannelSettings>),
    User(Entity<UserSettings>),
}

/// The open screen, what closes it, and what had focus before it opened.
pub(super) struct OpenSettings {
    screen: SettingsScreen,
    _subscription: Subscription,
    opener: Option<FocusHandle>,
}

impl DuskShell {
    /// "Server settings": opens on `section`, else on the first section offered.
    pub(super) fn open_server_settings(
        &mut self,
        guild_id: Snowflake,
        section: Option<ServerSettingsSection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let opener = window.focused(cx);
        let session = self.session.clone();
        let screen = cx.new(|cx| ServerSettings::new(session, guild_id, section, window, cx));
        let subscription = cx.subscribe_in(&screen, window, Self::close_settings);
        self.open_settings = Some(OpenSettings {
            screen: SettingsScreen::Server(screen),
            _subscription: subscription,
            opener,
        });
        cx.notify();
    }

    /// "Edit channel", "Channel settings".
    pub(super) fn open_channel_settings(
        &mut self,
        channel_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let opener = window.focused(cx);
        let session = self.session.clone();
        let screen = cx.new(|cx| ChannelSettings::new(session, channel_id, window, cx));
        let subscription = cx.subscribe_in(&screen, window, Self::close_settings);
        self.open_settings = Some(OpenSettings {
            screen: SettingsScreen::Channel(screen),
            _subscription: subscription,
            opener,
        });
        cx.notify();
    }

    /// The user card's gear and Ctrl+, (Cmd+, on macOS); does nothing while a screen is open.
    pub(super) fn open_user_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_settings.is_some() {
            return;
        }
        let opener = window.focused(cx);
        let session = self.session.clone();
        let screen = cx.new(|cx| UserSettings::new(session, window, cx));
        let subscription = cx.subscribe_in(&screen, window, Self::close_settings);
        self.open_settings = Some(OpenSettings {
            screen: SettingsScreen::User(screen),
            _subscription: subscription,
            opener,
        });
        cx.notify();
    }

    #[allow(clippy::trivially_copy_pass_by_ref)] // The shape `subscribe_in` calls.
    fn close_settings<Screen>(
        &mut self,
        _: &Entity<Screen>,
        _: &SettingsClosed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(opener) = self.open_settings.take().and_then(|open| open.opener) {
            window.focus(&opener, cx);
        } else {
            window.focus(&self.root_focus, cx);
        }
        cx.notify();
    }

    #[must_use]
    pub fn is_settings_screen_open(&self) -> bool {
        self.open_settings.is_some()
    }

    /// The open screen, drawn over everything but dialogs and menus.
    pub(super) fn render_settings_screen(&self) -> Option<gpui::AnyElement> {
        self.open_settings.as_ref().map(|open| match &open.screen {
            SettingsScreen::Server(screen) => screen.clone().into_any_element(),
            SettingsScreen::Channel(screen) => screen.clone().into_any_element(),
            SettingsScreen::User(screen) => screen.clone().into_any_element(),
        })
    }
}

#[cfg(test)]
#[path = "settings_screens_tests.rs"]
mod tests;
