//! User settings → Notifications (`NotificationSettings.svelte`): desktop notifications, previews
//! and the chime, in a dialog opened from the status menu until the desktop has a settings page
//! (step 6). The choices are the account's, kept in the preferences file as the web keeps them in
//! the browser. GPUI cannot read the system's permission, so the desktop switch is never locked
//! as the web's is on "blocked"; turning it on posts a confirmation, so macOS asks right then.

use gpui::{
    AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    SystemNotification, Window, div,
};
use mellow_client_core::shared_logic::notification_settings::NotificationSettings;
use mellow_media::SoundPlayer;
use mellow_protocol::Snowflake;

use super::DuskShell;
use crate::components::{ModalDialog, ModalDialogEvent, Switch};
use crate::i18n::strings;
use crate::notifier::{TURNED_ON_NOTIFICATION_TAG, notification_settings_of, play_chime_with};
use crate::preferences::{PreferencesStore, change_preferences};
use crate::theme::length;
use crate::theme::tokens::SPACE_SM;
use crate::window_chrome::WINDOW_TITLE;

/// The dialog's switches; they follow the preferences, so a change elsewhere shows at once.
pub(super) struct NotificationSettingsBody {
    user_id: Snowflake,
    /// Opened when the sound is turned on, to play the chime once.
    sound_player: Option<SoundPlayer>,
    _preferences_subscription: Subscription,
}

impl NotificationSettingsBody {
    fn new(user_id: Snowflake, cx: &mut Context<Self>) -> Self {
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

impl Render for NotificationSettingsBody {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let words = strings(cx).notifications();
        let settings = notification_settings_of(self.user_id, cx);
        let user_id = self.user_id;
        let body = cx.entity().downgrade();
        div()
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
            )
    }
}

impl DuskShell {
    pub(super) fn open_notification_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = self
            .session
            .read(cx)
            .state()
            .current_user()
            .map(|user| user.id)
        else {
            return;
        };
        let catalogue = strings(cx);
        let body = cx.new(|cx| NotificationSettingsBody::new(user_id, cx));
        let dialog = cx.new(|cx| {
            ModalDialog::new(
                catalogue.notifications().title(),
                catalogue.common().close(),
                window,
                cx,
            )
            .subtitle(catalogue.desktop().notifications_intro())
            .body(body.into())
        });
        let subscription = cx.subscribe(&dialog, |shell, _, _: &ModalDialogEvent, cx| {
            shell.notification_settings = None;
            cx.notify();
        });
        self.notification_settings = Some((dialog, subscription));
        cx.notify();
    }
}
