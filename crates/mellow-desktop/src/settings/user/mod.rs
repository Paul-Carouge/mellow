//! User settings (`UserSettings.svelte`, DESIGN.md §5.8 layout, §5.9 sections): the account's
//! own sections, then the app's, then "Sign out", in the full-window `SettingsSheet`. Esc closes
//! the open form first (My account, Security), then the settings; focus goes back to the button
//! that opened them.
//!
//! shortcut: the web's Accessibility section (animated pictures) is left out: the desktop has no
//! "animate on hover" yet (pictures follow the system's reduced motion). The desktop's own
//! Appearance section holds the theme and the language.

mod about;
mod account;
mod appearance;
mod notifications;
mod parts;
mod profile;
mod security;
mod voice;

use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, IntoElement, Render, SharedString,
    WeakEntity, Window, div,
};
use mellow_client_core::shared_logic::user_settings::UserSettingsSection;

use self::about::AboutPage;
use self::account::AccountPage;
use self::appearance::AppearancePage;
use self::notifications::NotificationsPage;
use self::profile::ProfilePage;
use self::security::SecurityPage;
use self::voice::VoicePage;
use super::{SettingsClosed, SettingsNavigation, ShownPage, is_wide, shown_page};
use crate::components::{SettingsNavEntry, SettingsNavItem, SettingsSheet, SettingsSheetLabels};
use crate::i18n::{Strings, strings};
use crate::session::Session;

/// Every section, in the navigation's order.
fn sections() -> Vec<UserSettingsSection> {
    UserSettingsSection::ACCOUNT_GROUP
        .into_iter()
        .chain(UserSettingsSection::APP_GROUP)
        .collect()
}

fn section_label(section: UserSettingsSection, catalogue: Strings) -> &'static str {
    let settings = catalogue.settings();
    match section {
        UserSettingsSection::MyAccount => settings.my_account(),
        UserSettingsSection::Profile => settings.profile(),
        UserSettingsSection::Security => catalogue.security().title(),
        UserSettingsSection::Appearance => catalogue.desktop().appearance(),
        UserSettingsSection::Voice => settings.voice(),
        UserSettingsSection::Notifications => settings.notifications(),
        UserSettingsSection::About => settings.about(),
    }
}

/// The page shown, when it has a form Esc closes first.
enum FormPage {
    Account(WeakEntity<AccountPage>),
    Security(WeakEntity<SecurityPage>),
}

pub struct UserSettings {
    session: Entity<Session>,
    navigation: SettingsNavigation<UserSettingsSection>,
    form_page: Option<FormPage>,
    sign_out_focus: FocusHandle,
    is_closing: bool,
}

impl EventEmitter<SettingsClosed> for UserSettings {}

impl std::fmt::Debug for UserSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserSettings")
            .field("navigation", &self.navigation)
            .finish_non_exhaustive()
    }
}

impl UserSettings {
    /// Opens on My account, its nav item focused, so screen readers start in the navigation.
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut navigation = SettingsNavigation::new(None, is_wide(window), cx);
        if let Some(section) = navigation.section(&sections()) {
            let focus = navigation.nav_focus(section, cx);
            window.focus(&focus, cx);
        }
        cx.observe(&session, |_, _, cx| cx.notify()).detach();
        Self {
            session,
            navigation,
            form_page: None,
            sign_out_focus: cx.focus_handle(),
            is_closing: false,
        }
    }

    fn navigation(&mut self) -> &mut SettingsNavigation<UserSettingsSection> {
        &mut self.navigation
    }

    fn make_page(
        &mut self,
        section: UserSettingsSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ShownPage<UserSettingsSection>> {
        let session = self.session.clone();
        self.form_page = None;
        Some(match section {
            UserSettingsSection::MyAccount => {
                let page = cx.new(|cx| AccountPage::new(session, cx));
                self.form_page = Some(FormPage::Account(page.downgrade()));
                shown_page(section, page, Self::navigation, cx)
            }
            UserSettingsSection::Profile => {
                let page = cx.new(|cx| ProfilePage::new(session, cx));
                shown_page(section, page, Self::navigation, cx)
            }
            UserSettingsSection::Security => {
                let page = cx.new(|cx| SecurityPage::new(session, cx));
                self.form_page = Some(FormPage::Security(page.downgrade()));
                shown_page(section, page, Self::navigation, cx)
            }
            UserSettingsSection::Appearance => {
                let page = cx.new(AppearancePage::new);
                shown_page(section, page, Self::navigation, cx)
            }
            UserSettingsSection::Voice => {
                let page = cx.new(|cx| VoicePage::new(window, cx));
                shown_page(section, page, Self::navigation, cx)
            }
            UserSettingsSection::Notifications => {
                let user_id = session.read(cx).state().current_user()?.id;
                let page = cx.new(|cx| NotificationsPage::new(user_id, cx));
                shown_page(section, page, Self::navigation, cx)
            }
            UserSettingsSection::About => {
                let page = cx.new(|_| AboutPage);
                shown_page(section, page, Self::navigation, cx)
            }
        })
    }

    /// Esc: the open form closes first, then the settings; the close button closes outright.
    fn request_close(&mut self, cx: &mut Context<Self>) {
        let has_closed_a_form = match &self.form_page {
            Some(FormPage::Account(page)) => page
                .update(cx, AccountPage::close_open_form)
                .unwrap_or(false),
            Some(FormPage::Security(page)) => page
                .update(cx, SecurityPage::close_open_form)
                .unwrap_or(false),
            None => false,
        };
        if !has_closed_a_form {
            self.close(cx);
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if !self.is_closing {
            self.is_closing = true;
            cx.emit(SettingsClosed);
        }
    }

    /// The settings close first: a sign-out the instance refuses is said by the shell's notice
    /// bar, which the sheet would cover; a successful one ends the shell anyway.
    fn sign_out(&mut self, cx: &mut Context<Self>) {
        self.close(cx);
        self.session.update(cx, Session::sign_out);
    }
}

impl Render for UserSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalogue = strings(cx);
        // Signed out elsewhere, or the session ended: nothing left to set.
        if self.session.read(cx).state().current_user().is_none() {
            cx.defer_in(window, |settings, _, cx| settings.close(cx));
            return div().into_any_element();
        }
        let offered = sections();
        let section = self.navigation.section(&offered);
        if !self.navigation.is_showing(section) {
            let page = section.and_then(|section| self.make_page(section, window, cx));
            self.navigation.replace_page(page);
        }
        let is_wide = is_wide(window);
        let words = catalogue.settings();

        let mut entries = Vec::new();
        let groups: [(SharedString, &[UserSettingsSection]); 2] = [
            (
                words.user_settings().into(),
                &UserSettingsSection::ACCOUNT_GROUP,
            ),
            (words.app_settings().into(), &UserSettingsSection::APP_GROUP),
        ];
        for (heading, group) in groups {
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
        entries.push(SettingsNavEntry::Separator);
        entries.push(SettingsNavEntry::Item(SettingsNavItem {
            label: words.sign_out().into(),
            focus_handle: self.sign_out_focus.clone(),
            is_selected: false,
            is_danger: true,
        }));

        let settings = cx.entity();
        let choose_settings = settings.clone();
        let back_settings = settings.clone();
        let close_settings = settings.clone();
        let item_sections = Arc::new(offered);
        let mut sheet = SettingsSheet::new(
            "user-settings",
            SettingsSheetLabels {
                label: words.user_settings().into(),
                title: words.user_settings().into(),
                back: words.back_to_user_settings().into(),
                close: words.close_settings().into(),
                escape_key: words.escape_key().into(),
            },
            self.navigation.container_focus.clone(),
            self.navigation.page_focus.clone(),
        )
        .wide(is_wide, self.navigation.is_page_shown_when_narrow())
        .entries(entries)
        .on_choose(move |index, window, cx| {
            choose_settings.update(cx, |settings, cx| {
                // The items after the sections: "Sign out".
                match item_sections.get(index).copied() {
                    Some(target) => settings
                        .navigation
                        .choose(target, section, is_wide, window, cx),
                    None => settings.sign_out(cx),
                }
                cx.notify();
            });
        })
        .on_request_close(move |_, cx| {
            settings.update(cx, Self::request_close);
        })
        .on_close_button(move |_, cx| {
            close_settings.update(cx, Self::close);
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
