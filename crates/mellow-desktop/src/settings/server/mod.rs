//! Server settings (`ServerSettings.svelte`): the server's own sections under its name, then
//! "Moderation", each only with its permission and once the desktop has its page. See the parent
//! module for how a section is added.
//! The owner also gets "Delete server", the danger item under the sections, which asks for the
//! server's name first (`DeleteServerDialog.svelte`).

mod bans;
mod emoji;
mod invites;
mod list_rows;
mod manage_member;
mod members;
mod moderation_log;
mod overview;
mod role_icon_editor;
mod roles;
mod sound_form;
mod soundboard;
#[cfg(test)]
mod test_support;

use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Window, div,
};
use mellow_client_core::shared_logic::server_settings::{
    ServerSettingsSection, is_owner, visible_sections,
};
use mellow_protocol::{Guild, Snowflake};

use self::bans::ServerBansPage;
use self::emoji::ServerEmojiPage;
use self::invites::ServerInvitesPage;
use self::members::MembersPage;
use self::moderation_log::ServerModerationLogPage;
use self::overview::ServerOverviewPage;
use self::roles::RolesPage;
use self::soundboard::ServerSoundboardPage;
// Only the tests against a real server use them; the server refuses Windows (ADR-0006).
#[cfg(all(test, not(windows)))]
pub(crate) use self::{
    bans::ServerBansPage as BansPage, invites::ServerInvitesPage as InvitesPage,
    moderation_log::ServerModerationLogPage as ModerationLogPage,
};
use super::name_confirmation::{
    NameConfirmation, NameConfirmationWords, open_name_confirmation_dialog,
};
use super::{SettingsClosed, SettingsNavigation, ShownPage, is_wide, shown_page};
use crate::components::{
    ModalDialog, ModalDialogEvent, SettingsNavEntry, SettingsNavItem, SettingsSheet,
    SettingsSheetLabels,
};
use crate::i18n::{Strings, strings};
use crate::session::Session;

/// Whether the desktop has a page for `section`: one arm per section, so a new one is decided.
fn has_page(section: ServerSettingsSection) -> bool {
    match section {
        ServerSettingsSection::Overview
        | ServerSettingsSection::Roles
        | ServerSettingsSection::Members
        | ServerSettingsSection::Emoji
        | ServerSettingsSection::Invites
        | ServerSettingsSection::Bans
        | ServerSettingsSection::Soundboard
        | ServerSettingsSection::ModerationLog => true,
    }
}

/// The sections offered to the reader in `guild`, in the navigation's order.
#[must_use]
pub fn offered_sections(guild: &Guild) -> Vec<ServerSettingsSection> {
    visible_sections(guild)
        .into_iter()
        .filter(|section| has_page(*section))
        .collect()
}

/// Whether "Server settings" has a page for the reader (the menus' `can_open_settings`).
#[must_use]
pub fn has_server_settings_page_for_reader(guild: &Guild) -> bool {
    !offered_sections(guild).is_empty()
}

fn section_label(section: ServerSettingsSection, catalogue: Strings) -> &'static str {
    let sections = catalogue.server_settings().sections();
    match section {
        ServerSettingsSection::Overview => sections.overview(),
        ServerSettingsSection::Roles => sections.roles(),
        ServerSettingsSection::Emoji => sections.emoji(),
        ServerSettingsSection::Soundboard => sections.soundboard(),
        ServerSettingsSection::Invites => sections.invites(),
        ServerSettingsSection::Members => sections.members(),
        ServerSettingsSection::Bans => sections.bans(),
        ServerSettingsSection::ModerationLog => sections.moderation_log(),
    }
}

pub struct ServerSettings {
    session: Entity<Session>,
    guild_id: Snowflake,
    navigation: SettingsNavigation<ServerSettingsSection>,
    is_closing: bool,
    delete_focus: FocusHandle,
    delete_dialog: Option<(Entity<ModalDialog>, Entity<NameConfirmation>, Subscription)>,
}

impl EventEmitter<SettingsClosed> for ServerSettings {}

impl std::fmt::Debug for ServerSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerSettings")
            .field("guild_id", &self.guild_id)
            .field("navigation", &self.navigation)
            .finish_non_exhaustive()
    }
}

impl ServerSettings {
    /// Opens on `initial_section` (the first one offered when `None`), its nav item focused.
    pub fn new(
        session: Entity<Session>,
        guild_id: Snowflake,
        initial_section: Option<ServerSettingsSection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut navigation = SettingsNavigation::new(initial_section, is_wide(window), cx);
        let offered = session
            .read(cx)
            .state()
            .guild(guild_id)
            .map(offered_sections)
            .unwrap_or_default();
        if let Some(section) = navigation.section(&offered) {
            let focus = if navigation.is_page_shown_when_narrow() {
                navigation.page_focus.clone()
            } else {
                navigation.nav_focus(section, cx)
            };
            window.focus(&focus, cx);
        }
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            guild_id,
            navigation,
            is_closing: false,
            delete_focus: cx.focus_handle().tab_stop(true),
            delete_dialog: None,
        }
    }

    /// The owner's danger item: the server and everything in it, for everyone.
    fn can_delete_server(&self, cx: &gpui::App) -> bool {
        let state = self.session.read(cx).state();
        let my_id = state.current_user().map(|user| user.id);
        state
            .guild(self.guild_id)
            .is_some_and(|guild| is_owner(guild, my_id))
    }

    fn open_delete_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.delete_dialog.is_some() || !self.can_delete_server(cx) {
            return;
        }
        let Some(name) = self
            .session
            .read(cx)
            .state()
            .guild(self.guild_id)
            .map(|guild| guild.name.clone())
        else {
            return;
        };
        let words = strings(cx).server_settings().delete_server_dialog();
        let session = self.session.downgrade();
        let guild_id = self.guild_id;
        let (dialog, field) = open_name_confirmation_dialog(
            NameConfirmationWords {
                title: words.title(&name),
                body: words.body(&name),
                field_label: words.confirm_label().to_owned(),
                submit: words.submit(),
                busy_submit: words.deleting(),
            },
            move |cx| {
                let session = session.upgrade()?;
                let name = session.read(cx).state().guild(guild_id)?.name.clone();
                Some(name)
            },
            &self.session,
            window,
            cx,
        );
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            |settings, dialog, event: &ModalDialogEvent, window, cx| match event {
                ModalDialogEvent::Closed => {
                    settings.delete_dialog = None;
                    cx.notify();
                }
                ModalDialogEvent::Submitted => settings.delete_server(dialog, window, cx),
            },
        );
        self.delete_dialog = Some((dialog, field, subscription));
        cx.notify();
    }

    /// Once the instance agrees, the gateway takes the server away and the settings close with
    /// it (see `render`); the dialog goes with them.
    fn delete_server(
        &mut self,
        dialog: &Entity<ModalDialog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Renamed since the last keystroke: the typed name no longer confirms.
        if !self
            .delete_dialog
            .as_ref()
            .is_some_and(|(_, field, _)| field.read(cx).is_confirmed(cx))
        {
            dialog.update(cx, |dialog, cx| dialog.set_submit_enabled(false, cx));
            return;
        }
        dialog.update(cx, |dialog, cx| {
            dialog.set_error(None, cx);
            dialog.set_busy(true, cx);
        });
        let guild_id = self.guild_id;
        let dialog = dialog.downgrade();
        let window_handle = window.window_handle();
        let failure = strings(cx)
            .server_settings()
            .delete_server_dialog()
            .failed()
            .to_owned();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                "delete server",
                failure,
                move |rest| async move { rest.delete_guild(guild_id).await },
                cx,
                move |outcome, cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        let _ = dialog.update(cx, |dialog, cx| {
                            dialog.set_busy(false, cx);
                            match outcome {
                                Ok(()) => dialog.request_close(window, cx),
                                Err(message) => dialog.set_error(Some(message.into()), cx),
                            }
                        });
                    });
                },
            );
        });
    }

    fn navigation(&mut self) -> &mut SettingsNavigation<ServerSettingsSection> {
        &mut self.navigation
    }

    /// One arm per section with a page.
    fn make_page(
        &mut self,
        section: ServerSettingsSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ShownPage<ServerSettingsSection> {
        match section {
            ServerSettingsSection::Overview => {
                let page = cx.new(|cx| {
                    ServerOverviewPage::new(self.session.clone(), self.guild_id, window, cx)
                });
                shown_page(section, page, Self::navigation, cx)
            }
            ServerSettingsSection::Emoji => {
                let page = cx.new(|cx| {
                    ServerEmojiPage::new(self.session.clone(), self.guild_id, window, cx)
                });
                shown_page(section, page, Self::navigation, cx)
            }
            ServerSettingsSection::Invites => {
                let page =
                    cx.new(|cx| ServerInvitesPage::new(self.session.clone(), self.guild_id, cx));
                shown_page(section, page, Self::navigation, cx)
            }
            ServerSettingsSection::Bans => {
                let page =
                    cx.new(|cx| ServerBansPage::new(self.session.clone(), self.guild_id, cx));
                shown_page(section, page, Self::navigation, cx)
            }
            ServerSettingsSection::ModerationLog => {
                let page = cx.new(|cx| {
                    ServerModerationLogPage::new(self.session.clone(), self.guild_id, cx)
                });
                shown_page(section, page, Self::navigation, cx)
            }
            ServerSettingsSection::Roles => {
                let page =
                    cx.new(|cx| RolesPage::new(self.session.clone(), self.guild_id, window, cx));
                shown_page(section, page, Self::navigation, cx)
            }
            ServerSettingsSection::Members => {
                let page =
                    cx.new(|cx| MembersPage::new(self.session.clone(), self.guild_id, window, cx));
                shown_page(section, page, Self::navigation, cx)
            }
            ServerSettingsSection::Soundboard => {
                let page =
                    cx.new(|cx| ServerSoundboardPage::new(self.session.clone(), self.guild_id, cx));
                shown_page(section, page, Self::navigation, cx)
            }
        }
    }

    fn request_close(&mut self, shown: Option<ServerSettingsSection>, cx: &mut Context<Self>) {
        if self.navigation.can_leave_page(shown, cx) {
            self.close(cx);
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if !self.is_closing {
            self.is_closing = true;
            cx.emit(SettingsClosed);
        }
    }
}

impl Render for ServerSettings {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalogue = strings(cx);
        let guild = self
            .session
            .read(cx)
            .state()
            .guild(self.guild_id)
            .map(|guild| (guild.name.clone(), offered_sections(guild)));
        // The server was deleted, or the reader left it or lost every section.
        let Some((guild_name, offered)) = guild.filter(|(_, offered)| !offered.is_empty()) else {
            cx.defer_in(window, |settings, _, cx| settings.close(cx));
            return div().into_any_element();
        };
        let section = self.navigation.section(&offered);
        if !self.navigation.is_showing(section) {
            let page = section.map(|section| self.make_page(section, window, cx));
            self.navigation.replace_page(page);
        }
        let is_wide = is_wide(window);

        // The server's own sections, then "Moderation" after a separator, as on the web.
        let (own, moderation): (Vec<_>, Vec<_>) = offered
            .iter()
            .copied()
            .partition(|candidate| !candidate.is_moderation());
        let item_sections: Vec<ServerSettingsSection> =
            own.iter().chain(&moderation).copied().collect();
        let mut entries = Vec::new();
        let headings: [SharedString; 2] = [
            guild_name.clone().into(),
            catalogue.server_settings().moderation().into(),
        ];
        for (heading, group) in headings.into_iter().zip([&own, &moderation]) {
            if group.is_empty() {
                continue;
            }
            if !entries.is_empty() {
                entries.push(SettingsNavEntry::Separator);
            }
            entries.push(SettingsNavEntry::Heading(heading));
            for target in group {
                entries.push(SettingsNavEntry::Item(SettingsNavItem {
                    label: section_label(*target, catalogue).into(),
                    focus_handle: self.navigation.nav_focus(*target, cx),
                    is_selected: section == Some(*target),
                    is_danger: false,
                }));
            }
        }

        let can_delete_server = self.can_delete_server(cx);
        if can_delete_server {
            entries.push(SettingsNavEntry::Separator);
            entries.push(SettingsNavEntry::Item(SettingsNavItem {
                label: catalogue.server_settings().delete_server().into(),
                focus_handle: self.delete_focus.clone(),
                is_selected: false,
                is_danger: true,
            }));
        }

        let words = catalogue.server_settings();
        let settings = cx.entity();
        let choose_settings = settings.clone();
        let back_settings = settings.clone();
        let item_sections = Arc::new(item_sections);
        let mut sheet = SettingsSheet::new(
            "server-settings",
            SettingsSheetLabels {
                label: format!("{}, {guild_name}", words.server_settings()).into(),
                title: words.server_settings().into(),
                back: words.back_to_server_settings().into(),
                close: catalogue.settings().close_settings().into(),
                escape_key: catalogue.settings().escape_key().into(),
            },
            self.navigation.container_focus.clone(),
            self.navigation.page_focus.clone(),
        )
        .wide(is_wide, self.navigation.is_page_shown_when_narrow())
        .entries(entries)
        .on_choose(move |index, window, cx| {
            choose_settings.update(cx, |settings, cx| {
                // The item after the sections: "Delete server".
                match item_sections.get(index).copied() {
                    Some(target) => settings
                        .navigation
                        .choose(target, section, is_wide, window, cx),
                    None => settings.open_delete_dialog(window, cx),
                }
                cx.notify();
            });
        })
        .on_request_close(move |_, cx| {
            settings.update(cx, |settings, cx| settings.request_close(section, cx));
        })
        .on_back(move |window, cx| {
            back_settings.update(cx, |settings, cx| {
                settings.navigation.back(section, window, cx);
                cx.notify();
            });
        });
        if let (Some(section), Some(page)) = (section, self.navigation.page_view()) {
            sheet = sheet.page(section_label(section, catalogue), page.into_any_element());
        }
        div()
            .absolute()
            .inset_0()
            .child(sheet)
            .children(
                self.delete_dialog
                    .as_ref()
                    .map(|(dialog, _, _)| dialog.clone()),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod delete_server_tests {
    use gpui::{Focusable as _, TestAppContext, VisualTestContext};
    use mellow_client_core::shared_logic::server_settings::ServerSettingsSection;
    use mellow_protocol::GatewayEvent;

    use super::ServerSettings;
    use super::test_support::open_server_settings;
    use crate::session::Session;
    use crate::shell_fixtures::{GUILD_RUST, ME};

    /// Gives the server to the reader, or renames it, as the gateway would.
    fn change_guild(
        session: &gpui::Entity<Session>,
        change: impl FnOnce(&mut mellow_protocol::Guild),
        cx: &mut VisualTestContext,
    ) {
        session.update(cx, |session, cx| {
            let mut guild = session.state().guild(GUILD_RUST).cloned().unwrap();
            change(&mut guild);
            session
                .state_mut()
                .apply_event(GatewayEvent::GuildCreate(Box::new(guild)), 0);
            cx.notify();
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn only_the_owner_is_offered_delete_server(cx: &mut TestAppContext) {
        let (session, settings, cx) = open_server_settings(cx, ServerSettingsSection::Overview);
        // An administrator who does not own the server.
        assert!(!settings.read_with(cx, ServerSettings::can_delete_server));
        settings.update_in(cx, ServerSettings::open_delete_dialog);
        assert!(settings.read_with(cx, |settings, _| settings.delete_dialog.is_none()));

        change_guild(&session, |guild| guild.owner_id = ME, cx);
        assert!(settings.read_with(cx, ServerSettings::can_delete_server));
        settings.update_in(cx, ServerSettings::open_delete_dialog);
        assert!(settings.read_with(cx, |settings, _| settings.delete_dialog.is_some()));
    }

    #[gpui::test]
    fn delete_waits_for_the_current_name_and_enter_on_a_partial_one_sends_nothing(
        cx: &mut TestAppContext,
    ) {
        let (session, settings, cx) = open_server_settings(cx, ServerSettingsSection::Overview);
        change_guild(&session, |guild| guild.owner_id = ME, cx);
        settings.update_in(cx, ServerSettings::open_delete_dialog);
        let (dialog, field) = settings.read_with(cx, |settings, _| {
            let (dialog, field, _) = settings.delete_dialog.as_ref().unwrap();
            (dialog.clone(), field.clone())
        });
        let name = session.read_with(cx, |session, _| {
            session.state().guild(GUILD_RUST).unwrap().name.clone()
        });
        cx.update(|window, cx| {
            let focus = field.read(cx).input().focus_handle(cx);
            window.focus(&focus, cx);
        });
        let (partial, rest) = name.split_at(name.len() / 2);
        cx.simulate_input(partial);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(!dialog.read_with(cx, |dialog, _| dialog.is_submit_enabled()));
        assert!(!dialog.read_with(cx, |dialog, _| dialog.is_busy()));
        assert!(settings.read_with(cx, |settings, _| settings.delete_dialog.is_some()));

        cx.simulate_input(rest);
        cx.run_until_parked();
        assert!(dialog.read_with(cx, |dialog, _| dialog.is_submit_enabled()));

        // Renamed meanwhile: the old name no longer wakes the button.
        change_guild(&session, |guild| guild.name = "Renamed".to_owned(), cx);
        assert!(!dialog.read_with(cx, |dialog, _| dialog.is_submit_enabled()));
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        assert!(!dialog.read_with(cx, |dialog, _| dialog.is_busy()));
    }
}
