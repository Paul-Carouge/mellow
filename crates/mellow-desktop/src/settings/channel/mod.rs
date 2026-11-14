//! Channel settings (`ChannelSettings.svelte`, Discord's "Edit channel"): the channel's sections
//! under its name, each only with its permission in this channel and once the desktop has its
//! page. See the parent module for how a section is added.
//! "Delete channel", the danger item under the sections, asks for the channel's name first
//! (`DeleteChannelDialog.svelte`), for whoever may manage the channel.

mod add_overwrite_target;
mod overview;
mod permissions;

use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window, div,
};
use mellow_client_core::shared_logic::channel_settings::{
    ChannelSettingsSection, can_manage_channel, visible_channel_sections,
};
use mellow_client_core::state::ClientState;
use mellow_protocol::{ChannelKind, Snowflake};

use self::overview::ChannelOverviewPage;
use self::permissions::ChannelPermissionsPage;
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
fn has_page(section: ChannelSettingsSection) -> bool {
    match section {
        ChannelSettingsSection::Overview | ChannelSettingsSection::Permissions => true,
    }
}

/// The sections offered to the reader for `channel_id`, in the navigation's order; none for a
/// channel gone or out of sight.
#[must_use]
pub fn offered_sections(state: &ClientState, channel_id: Snowflake) -> Vec<ChannelSettingsSection> {
    let Some(channel) = state.guild_channel(channel_id) else {
        return Vec::new();
    };
    visible_channel_sections(channel.kind, state.my_channel_permissions(channel_id))
        .into_iter()
        .filter(|section| has_page(*section))
        .collect()
}

/// Whether "Edit channel" has a page for the reader (the menus' `can_edit_channel`).
#[must_use]
pub fn has_channel_settings_page_for_reader(state: &ClientState, channel_id: Snowflake) -> bool {
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
    delete_focus: FocusHandle,
    delete_dialog: Option<(Entity<ModalDialog>, Entity<NameConfirmation>, Subscription)>,
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
            delete_focus: cx.focus_handle().tab_stop(true),
            delete_dialog: None,
        }
    }

    /// The channel's name, when the reader may delete it (a category goes through its own menu).
    fn deletable_channel_name(&self, cx: &gpui::App) -> Option<String> {
        let state = self.session.read(cx).state();
        let channel = state.guild_channel(self.channel_id)?;
        (channel.kind != ChannelKind::Category
            && can_manage_channel(state.my_channel_permissions(self.channel_id)))
        .then(|| channel.name.clone())
    }

    fn open_delete_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.delete_dialog.is_some() {
            return;
        }
        let Some(name) = self.deletable_channel_name(cx) else {
            return;
        };
        let words = strings(cx).channel_removal();
        let session = self.session.downgrade();
        let channel_id = self.channel_id;
        let (dialog, field) = open_name_confirmation_dialog(
            NameConfirmationWords {
                title: words.channel_title(&name),
                body: words.channel_body().to_owned(),
                field_label: words.confirm_label(&name),
                submit: words.delete_channel(),
                busy_submit: words.deleting(),
            },
            move |cx| {
                let session = session.upgrade()?;
                let name = session
                    .read(cx)
                    .state()
                    .guild_channel(channel_id)?
                    .name
                    .clone();
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
                ModalDialogEvent::Submitted => settings.delete_channel(dialog, window, cx),
            },
        );
        self.delete_dialog = Some((dialog, field, subscription));
        cx.notify();
    }

    /// Once the instance agrees, the gateway takes the channel away and the settings close with
    /// it (see `render`).
    fn delete_channel(
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
        let channel_id = self.channel_id;
        let dialog = dialog.downgrade();
        let window_handle = window.window_handle();
        let failure = strings(cx)
            .channel_removal()
            .channel_delete_failed()
            .to_owned();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                "delete channel",
                failure,
                move |rest| async move { rest.delete_channel(channel_id).await },
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

    fn navigation(&mut self) -> &mut SettingsNavigation<ChannelSettingsSection> {
        &mut self.navigation
    }

    /// One arm per section with a page.
    fn make_page(
        &mut self,
        section: ChannelSettingsSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ShownPage<ChannelSettingsSection> {
        match section {
            ChannelSettingsSection::Overview => {
                let page = cx.new(|cx| {
                    ChannelOverviewPage::new(self.session.clone(), self.channel_id, window, cx)
                });
                shown_page(section, page, Self::navigation, cx)
            }
            ChannelSettingsSection::Permissions => {
                let page = cx.new(|cx| {
                    ChannelPermissionsPage::new(self.session.clone(), self.channel_id, window, cx)
                });
                shown_page(section, page, Self::navigation, cx)
            }
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
        let channel_name = state
            .guild_channel(self.channel_id)
            .map(|channel| channel.name.clone());
        let offered = offered_sections(state, self.channel_id);
        // The channel was deleted or hidden, or the reader lost every section.
        let Some(channel_name) = channel_name.filter(|_| !offered.is_empty()) else {
            cx.defer_in(window, |settings, _, cx| settings.close(cx));
            return div().into_any_element();
        };
        let section = self.navigation.section(&offered);
        if !self.navigation.is_showing(section) {
            let page = section.map(|section| self.make_page(section, window, cx));
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

        if self.deletable_channel_name(cx).is_some() {
            entries.push(SettingsNavEntry::Separator);
            entries.push(SettingsNavEntry::Item(SettingsNavItem {
                label: catalogue.channel_removal().delete_channel().into(),
                focus_handle: self.delete_focus.clone(),
                is_selected: false,
                is_danger: true,
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
            choose_settings.update(cx, |settings, cx| {
                // The item after the sections: "Delete channel".
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
