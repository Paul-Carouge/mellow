//! Channel settings (`ChannelSettings.svelte`, Discord's "Edit channel"): the channel's sections
//! under its name, each only with its permission in this channel and once the desktop has its
//! page. See the parent module for how a section is added.
//!
//! shortcut: Permissions and "Delete channel" come with the M3 step 6 settings tracks.

mod overview;

use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, EventEmitter, IntoElement, Render, Window, div};
use mellow_client_core::shared_logic::channel_settings::{
    ChannelSettingsSection, visible_channel_sections,
};
use mellow_client_core::state::ClientState;
use mellow_protocol::{Channel, Snowflake};

use self::overview::ChannelOverviewPage;
use super::{SettingsClosed, SettingsNavigation, ShownPage, is_wide, shown_page};
use crate::components::{SettingsNavEntry, SettingsNavItem, SettingsSheet, SettingsSheetLabels};
use crate::i18n::{Strings, strings};
use crate::session::Session;

/// The sections the desktop has a page for, in no particular order (the rules give the order).
const SECTIONS_WITH_A_PAGE: &[ChannelSettingsSection] = &[ChannelSettingsSection::Overview];

fn channel_of(state: &ClientState, channel_id: Snowflake) -> Option<&Channel> {
    state
        .guilds()
        .iter()
        .flat_map(|guild| &guild.channels)
        .find(|channel| channel.id == channel_id)
}

/// The sections offered to the reader for `channel_id`, in the navigation's order; none for a
/// channel gone or out of sight.
#[must_use]
pub fn offered_sections(state: &ClientState, channel_id: Snowflake) -> Vec<ChannelSettingsSection> {
    let Some(channel) = channel_of(state, channel_id) else {
        return Vec::new();
    };
    visible_channel_sections(channel.kind, state.my_channel_permissions(channel_id))
        .into_iter()
        .filter(|section| SECTIONS_WITH_A_PAGE.contains(section))
        .collect()
}

/// Whether "Edit channel" has a page for the reader (the menus' `can_edit_channel`).
#[must_use]
pub fn can_edit_channel(state: &ClientState, channel_id: Snowflake) -> bool {
    !offered_sections(state, channel_id).is_empty()
}

fn section_label(section: ChannelSettingsSection, catalogue: Strings) -> &'static str {
    let sections = catalogue.channel_settings().sections();
    match section {
        ChannelSettingsSection::Overview => sections.overview(),
        ChannelSettingsSection::Permissions => sections.permissions(),
    }
}

pub struct ChannelSettings {
    session: Entity<Session>,
    channel_id: Snowflake,
    navigation: SettingsNavigation<ChannelSettingsSection>,
    is_closing: bool,
}

impl EventEmitter<SettingsClosed> for ChannelSettings {}

impl std::fmt::Debug for ChannelSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelSettings")
            .field("channel_id", &self.channel_id)
            .field("navigation", &self.navigation)
            .finish_non_exhaustive()
    }
}

impl ChannelSettings {
    /// Opens on the first section offered, its nav item focused.
    pub fn new(
        session: Entity<Session>,
        channel_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut navigation = SettingsNavigation::new(None, is_wide(window), cx);
        let offered = offered_sections(session.read(cx).state(), channel_id);
        if let Some(section) = navigation.section(&offered) {
            let focus = navigation.nav_focus(section, cx);
            window.focus(&focus, cx);
        }
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            channel_id,
            navigation,
            is_closing: false,
        }
    }

    #[must_use]
    pub fn channel_id(&self) -> Snowflake {
        self.channel_id
    }

    fn navigation(&mut self) -> &mut SettingsNavigation<ChannelSettingsSection> {
        &mut self.navigation
    }

    /// One arm per section with a page.
    fn make_page(
        &mut self,
        section: ChannelSettingsSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ShownPage<ChannelSettingsSection>> {
        match section {
            ChannelSettingsSection::Overview => {
                let page = cx.new(|cx| {
                    ChannelOverviewPage::new(self.session.clone(), self.channel_id, window, cx)
                });
                Some(shown_page(section, page, Self::navigation, cx))
            }
            ChannelSettingsSection::Permissions => None,
        }
    }

    fn request_close(&mut self, shown: Option<ChannelSettingsSection>, cx: &mut Context<Self>) {
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

impl Render for ChannelSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalogue = strings(cx);
        let state = self.session.read(cx).state();
        let channel_name = channel_of(state, self.channel_id).map(|channel| channel.name.clone());
        let offered = offered_sections(state, self.channel_id);
        // The channel was deleted or hidden, or the reader lost every section.
        let Some(channel_name) = channel_name.filter(|_| !offered.is_empty()) else {
            cx.defer_in(window, |settings, _, cx| settings.close(cx));
            return div().into_any_element();
        };
        let section = self.navigation.section(&offered);
        if !self.navigation.is_showing(section) {
            let page = section.and_then(|section| self.make_page(section, window, cx));
            self.navigation.replace_page(page);
        }
        let is_wide = is_wide(window);

        let mut entries = vec![SettingsNavEntry::Heading(format!("#{channel_name}").into())];
        for target in &offered {
            entries.push(SettingsNavEntry::Item(SettingsNavItem {
                label: section_label(*target, catalogue).into(),
                focus_handle: self.navigation.nav_focus(*target, cx),
                is_selected: section == Some(*target),
                is_danger: false,
            }));
        }

        let words = catalogue.channel_settings();
        let settings = cx.entity();
        let choose_settings = settings.clone();
        let back_settings = settings.clone();
        let item_sections = Arc::new(offered);
        let mut sheet = SettingsSheet::new(
            "channel-settings",
            SettingsSheetLabels {
                label: words.channel_settings_of(&channel_name).into(),
                title: words.channel_settings().into(),
                back: words.back_to_channel_settings().into(),
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
