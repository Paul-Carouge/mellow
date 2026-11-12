//! User settings, Notifications (`NotificationSettings.svelte`): desktop notifications, previews
//! and the chime. The choices are the account's, kept in the preferences file as the web keeps them in
//! the browser. GPUI cannot read the system's permission, so the desktop switch is never locked
//! as the web's is on "blocked"; turning it on posts a confirmation, so macOS asks right then.

use gpui::{
    Context, EventEmitter, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    SystemNotification, Window, div,
};
use mellow_client_core::shared_logic::notification_settings::NotificationSettings;
use mellow_media::SoundPlayer;
use mellow_protocol::Snowflake;

use super::parts::body_text;
use crate::components::Switch;
use crate::i18n::strings;
use crate::notifier::{TURNED_ON_NOTIFICATION_TAG, notification_settings_of, play_chime_with};
use crate::preferences::{PreferencesStore, change_preferences};
use crate::settings::{SettingsPage, SettingsPageEvent};
use crate::theme::length;
use crate::theme::tokens::{
    COMPONENT_SETTINGS_PAGE_TITLE_GAP, COMPONENT_SETTINGS_SECTION_GAP, SPACE_SM,
};
use crate::window_chrome::WINDOW_TITLE;

/// The page's switches; they follow the preferences, so a change elsewhere shows at once.
pub struct NotificationsPage {
    user_id: Snowflake,
    /// Opened when the sound is turned on, to play the chime once.
    sound_player: Option<SoundPlayer>,
    _preferences_subscription: Subscription,
}

impl std::fmt::Debug for NotificationsPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NotificationsPage")
            .field("user_id", &self.user_id)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<SettingsPageEvent> for NotificationsPage {}

impl SettingsPage for NotificationsPage {
    fn set_attention(&mut self, _: u32, _: &mut Context<Self>) {}
}

impl NotificationsPage {
    pub fn new(user_id: Snowflake, cx: &mut Context<Self>) -> Self {
        Self {
            user_id,
            sound_player: None,
            _preferences_subscription: cx.observe_global::<PreferencesStore>(|_, cx| cx.notify()),
        }
    }
}

/// Applies `change` to the choices of `user_id` and saves them.
fn change_notification_settings(
    user_id: Snowflake,
    change: impl FnOnce(&mut NotificationSettings) + 'static,
    cx: &mut gpui::App,
) {
    change_preferences(cx, move |preferences| {
        let mut settings = preferences.notification_settings_of(user_id);
        change(&mut settings);
        preferences.set_notification_settings(user_id, settings);
    });
}

impl Render for NotificationsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let words = strings(cx).notifications();
        let settings = notification_settings_of(self.user_id, cx);
        let user_id = self.user_id;
        let body = cx.entity().downgrade();
        let intro = strings(cx).desktop().notifications_intro();
        let switches = div()
            .mt(length(COMPONENT_SETTINGS_SECTION_GAP))
            .flex()
            .flex_col()
            .gap(length(SPACE_SM))
            .child(
                Switch::new(
                    "notifications-desktop",
                    settings.are_desktop_notifications_enabled,
                    words.desktop(),
                )
                .description(words.desktop_allowed())
                .on_change(move |is_on, _, cx| {
                    change_notification_settings(
                        user_id,
                        move |settings| {
                            settings.are_desktop_notifications_enabled = is_on;
                            settings.has_answered_permission_bar = true;
                        },
                        cx,
                    );
                    if is_on {
                        cx.show_system_notification(SystemNotification {
                            tag: TURNED_ON_NOTIFICATION_TAG.into(),
                            title: WINDOW_TITLE.into(),
                            body: strings(cx).notifications().turned_on().into(),
                            actions: Vec::new(),
                        });
                    }
                }),
            )
            .child(
                Switch::new(
                    "notifications-previews",
                    settings.should_show_previews,
                    words.previews(),
                )
                .description(words.previews_description())
                .on_change(move |is_on, _, cx| {
                    change_notification_settings(
                        user_id,
                        move |settings| settings.should_show_previews = is_on,
                        cx,
                    );
                }),
            )
            .child(
                Switch::new(
                    "notifications-sound",
                    settings.is_sound_enabled,
                    words.sound(),
                )
                .description(words.sound_description())
                .on_change(move |is_on, _, cx| {
                    change_notification_settings(
                        user_id,
                        move |settings| settings.is_sound_enabled = is_on,
                        cx,
                    );
                    // Turning it on plays it once, so the reader knows what they chose.
                    if is_on {
                        let _ = body.update(cx, |body, _| play_chime_with(&mut body.sound_player));
                    }
                }),
            );
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .mt(length(COMPONENT_SETTINGS_PAGE_TITLE_GAP))
                    .child(body_text(intro, cx)),
            )
            .child(switches)
    }
}
