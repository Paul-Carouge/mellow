//! Server settings (`ServerSettings.svelte`): the server's own sections under its name, then
//! "Moderation", each only with its permission and once the desktop has its page. See the parent
//! module for how a section is added.
//!
//! shortcut: "Delete server" (the owner's danger item) comes with a later M3 step.

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
    AppContext as _, Context, Entity, EventEmitter, IntoElement, Render, SharedString, Window, div,
};
use mellow_client_core::shared_logic::server_settings::{ServerSettingsSection, visible_sections};
use mellow_protocol::{Guild, Snowflake};

use self::bans::ServerBansPage;
use self::emoji::ServerEmojiPage;
use self::invites::ServerInvitesPage;
use self::members::MembersPage;
use self::moderation_log::ServerModerationLogPage;
use self::overview::ServerOverviewPage;
use self::roles::RolesPage;
use self::soundboard::ServerSoundboardPage;
#[cfg(test)]
pub(crate) use self::{
    bans::ServerBansPage as BansPage, invites::ServerInvitesPage as InvitesPage,
    moderation_log::ServerModerationLogPage as ModerationLogPage,
};
use super::{SettingsClosed, SettingsNavigation, ShownPage, is_wide, shown_page};
use crate::components::{SettingsNavEntry, SettingsNavItem, SettingsSheet, SettingsSheetLabels};
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
        }
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
            let Some(target) = item_sections.get(index).copied() else {
                return;
            };
            choose_settings.update(cx, |settings, cx| {
                settings
                    .navigation
                    .choose(target, section, is_wide, window, cx);
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
        sheet.into_any_element()
    }
}
