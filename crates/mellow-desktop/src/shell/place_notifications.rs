//! "Mute…" and "Notification settings…" from a server's or a channel's menu
//! (`NotificationSettingsDialog.svelte`, docs/design/web-notifications.md): how long the place
//! stays quiet, and what it tells about. Nothing changes until Save; the choices stay on this
//! device, in the preferences, as the web keeps them in the browser. Focus starts in the group of
//! the menu item that opened it.

use gpui::{
    Context, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div,
};
use mellow_client_core::shared_logic::notification_settings::{
    MuteDuration, MuteEnd, NotificationSettings, NotifyLevel, PlaceNotificationSettings,
    is_mute_active, with_place_settings,
};
use mellow_client_core::shared_logic::notifications::{
    LevelChoice, MuteChoice, NotificationDialogChoices, NotificationTarget, NotificationTargetKind,
    default_notify_level, notification_target_id, place_settings_from_dialog, server_notify_level,
};

use crate::components::{ChoiceGroup, ChoiceLegend, ChoiceOption, with_type};
use crate::i18n::{Strings, strings};
use crate::messages::{local_date_time, upcoming_time_label};
use crate::theme::tokens::{
    COMPONENT_SETTINGS_SECTION_GAP, COMPONENT_TEXT_FIELD_LABEL_GAP, TYPE_BODY_SM, TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};

/// Which group takes focus first: the menu item that opened the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InitialGroup {
    Mute,
    Level,
}

/// The dialog's two choices, drawn inside the `ModalDialog` that owns Save and Cancel.
pub(super) struct PlaceNotificationsBody {
    target: NotificationTarget,
    /// The mute running when the dialog opened: a timed one is a row of its own, so saving does
    /// not restart its clock.
    running_mute_end: Option<MuteEnd>,
    /// What applies without a choice of its own: the server's for a channel, else the default.
    fallback_level: NotifyLevel,
    mute_choices: Vec<MuteChoice>,
    level_choices: Vec<LevelChoice>,
    chosen_mute: MuteChoice,
    chosen_level: LevelChoice,
    opened_at: u64,
    /// Taken at the first frame, which puts focus there.
    initial_group: Option<InitialGroup>,
}

fn target_kind(target: NotificationTarget) -> NotificationTargetKind {
    match target {
        NotificationTarget::Server { .. } => NotificationTargetKind::Server,
        NotificationTarget::Channel { .. } => NotificationTargetKind::Channel,
        NotificationTarget::Conversation { .. } => NotificationTargetKind::Conversation,
    }
}

impl PlaceNotificationsBody {
    pub(super) fn new(
        target: NotificationTarget,
        settings: &NotificationSettings,
        initial_group: InitialGroup,
        now: u64,
    ) -> Self {
        let stored = settings
            .places
            .get(&notification_target_id(target))
            .copied()
            .unwrap_or_default();
        let running_mute_end = stored
            .muted_until
            .filter(|_| is_mute_active(stored.muted_until, now));
        let is_timed_mute_running = matches!(running_mute_end, Some(MuteEnd::At(_)));
        let mute_choices: Vec<MuteChoice> = std::iter::once(MuteChoice::NotMuted)
            .chain(is_timed_mute_running.then_some(MuteChoice::Current))
            .chain(MuteDuration::ALL.into_iter().map(MuteChoice::For))
            .collect();
        let chosen_mute = match running_mute_end {
            None => MuteChoice::NotMuted,
            Some(MuteEnd::Never) => MuteChoice::For(MuteDuration::UntilTurnedBackOn),
            Some(MuteEnd::At(_)) => MuteChoice::Current,
        };
        let is_server_channel = matches!(target, NotificationTarget::Channel { .. });
        let fallback_level = match target {
            NotificationTarget::Channel { guild_id, .. } => server_notify_level(settings, guild_id),
            NotificationTarget::Server { guild_id } => default_notify_level(Some(guild_id)),
            NotificationTarget::Conversation { .. } => default_notify_level(None),
        };
        let level_choices: Vec<LevelChoice> = is_server_channel
            .then_some(LevelChoice::SameAsServer)
            .into_iter()
            .chain(NotifyLevel::ALL.into_iter().map(LevelChoice::Level))
            .collect();
        let chosen_level = match stored.level {
            Some(level) => LevelChoice::Level(level),
            None if is_server_channel => LevelChoice::SameAsServer,
            None => LevelChoice::Level(fallback_level),
        };
        Self {
            target,
            running_mute_end,
            fallback_level,
            mute_choices,
            level_choices,
            chosen_mute,
            chosen_level,
            opened_at: now,
            initial_group: Some(initial_group),
        }
    }

    /// `settings` with the choices made here, as Save stores them.
    pub(super) fn saved_into(
        &self,
        settings: &NotificationSettings,
        now: u64,
    ) -> NotificationSettings {
        let place: PlaceNotificationSettings =
            place_settings_from_dialog(NotificationDialogChoices {
                target_kind: target_kind(self.target),
                chosen_mute: self.chosen_mute,
                chosen_level: self.chosen_level,
                running_mute_end: self.running_mute_end,
                fallback_level: self.fallback_level,
                now,
            });
        with_place_settings(settings, notification_target_id(self.target), place)
    }

    fn mute_label(&self, choice: MuteChoice, words: Strings) -> ChoiceOption {
        let notifications = words.notifications();
        match choice {
            MuteChoice::NotMuted => ChoiceOption::new(notifications.dont_mute()),
            MuteChoice::Current => {
                let end = match self.running_mute_end {
                    Some(MuteEnd::At(end)) => end,
                    _ => self.opened_at,
                };
                let time = upcoming_time_label(
                    local_date_time(end),
                    local_date_time(self.opened_at),
                    words,
                );
                ChoiceOption::new(notifications.muted_until_time(&time))
                    .description(notifications.muted_now())
            }
            MuteChoice::For(duration) => {
                let lengths = notifications.mute_for();
                ChoiceOption::new(match duration {
                    MuteDuration::FifteenMinutes => lengths.key_15_minutes(),
                    MuteDuration::OneHour => lengths.key_1_hour(),
                    MuteDuration::EightHours => lengths.key_8_hours(),
                    MuteDuration::TwentyFourHours => lengths.key_24_hours(),
                    MuteDuration::UntilTurnedBackOn => lengths.until_turned_back_on(),
                })
            }
        }
    }

    fn level_label(&self, choice: LevelChoice, words: Strings) -> ChoiceOption {
        let notifications = words.notifications();
        let worded = |level: NotifyLevel| {
            let levels = notifications.levels();
            match level {
                NotifyLevel::All => (levels.all().label(), levels.all().description()),
                NotifyLevel::Mentions => {
                    (levels.mentions().label(), levels.mentions().description())
                }
                NotifyLevel::Nothing => (levels.nothing().label(), levels.nothing().description()),
            }
        };
        match choice {
            LevelChoice::SameAsServer => ChoiceOption::new(notifications.same_as_server())
                .description(notifications.same_as_server_now(worded(self.fallback_level).0)),
            LevelChoice::Level(level) => {
                let (label, description) = worded(level);
                ChoiceOption::new(label).description(description)
            }
        }
    }
}

/// A group's visible name, and the sentence under it.
fn legend(text: &'static str, cx: &gpui::App) -> impl IntoElement + use<> {
    with_type(div(), TYPE_CATEGORY)
        .mb(length(COMPONENT_TEXT_FIELD_LABEL_GAP))
        .text_color(color(theme(cx).component_text_field_label_text))
        .child(text)
}

impl Render for PlaceNotificationsBody {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let words = strings(cx);
        let notifications = words.notifications();
        let initial_group = self.initial_group.take();
        let entity = cx.entity();
        let mute_index = self
            .mute_choices
            .iter()
            .position(|choice| *choice == self.chosen_mute)
            .unwrap_or(0);
        let level_index = self
            .level_choices
            .iter()
            .position(|choice| *choice == self.chosen_level)
            .unwrap_or(0);
        let mute_options = self
            .mute_choices
            .iter()
            .map(|choice| self.mute_label(*choice, words))
            .collect();
        let level_options = self
            .level_choices
            .iter()
            .map(|choice| self.level_label(*choice, words))
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_SETTINGS_SECTION_GAP))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(legend(notifications.mute_legend(), cx))
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .mb(length(COMPONENT_TEXT_FIELD_LABEL_GAP))
                            .text_color(color(theme(cx).color_text_muted))
                            .child(SharedString::from(notifications.mute_description())),
                    )
                    .child(
                        ChoiceGroup::new(
                            "place-mute",
                            notifications.mute_legend(),
                            mute_options,
                            mute_index,
                        )
                        // The dialog draws the legend and its sentence itself.
                        .legend(ChoiceLegend::Hidden)
                        .description(notifications.mute_description())
                        .focus_checked(initial_group == Some(InitialGroup::Mute))
                        .on_select({
                            let entity = entity.clone();
                            move |index, _, cx| {
                                entity.update(cx, |body, cx| {
                                    if let Some(choice) = body.mute_choices.get(index) {
                                        body.chosen_mute = *choice;
                                        cx.notify();
                                    }
                                });
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(legend(notifications.level_legend(), cx))
                    .child(
                        ChoiceGroup::new(
                            "place-level",
                            notifications.level_legend(),
                            level_options,
                            level_index,
                        )
                        .legend(ChoiceLegend::Hidden)
                        .focus_checked(initial_group == Some(InitialGroup::Level))
                        .on_select(move |index, _, cx| {
                            entity.update(cx, |body, cx| {
                                if let Some(choice) = body.level_choices.get(index) {
                                    body.chosen_level = *choice;
                                    cx.notify();
                                }
                            });
                        }),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::Snowflake;

    use super::*;

    const SERVER: Snowflake = Snowflake::from_raw(10);
    const CHANNEL: Snowflake = Snowflake::from_raw(11);
    const NOW: u64 = 1_000_000;

    fn channel() -> NotificationTarget {
        NotificationTarget::Channel {
            guild_id: SERVER,
            channel_id: CHANNEL,
        }
    }

    fn with_place(id: Snowflake, place: PlaceNotificationSettings) -> NotificationSettings {
        with_place_settings(&NotificationSettings::default(), id, place)
    }

    #[test]
    fn a_channel_follows_its_server_until_told_otherwise() {
        let body = PlaceNotificationsBody::new(
            channel(),
            &NotificationSettings::default(),
            InitialGroup::Level,
            NOW,
        );
        assert_eq!(body.chosen_level, LevelChoice::SameAsServer);
        assert_eq!(body.chosen_mute, MuteChoice::NotMuted);
        assert_eq!(body.level_choices.len(), 4);
        assert!(!body.mute_choices.contains(&MuteChoice::Current));
    }

    #[test]
    fn a_running_timed_mute_is_kept_as_it_is_when_saved() {
        let end = NOW + 60_000;
        let settings = with_place(
            CHANNEL,
            PlaceNotificationSettings {
                level: None,
                muted_until: Some(MuteEnd::At(end)),
            },
        );
        let body = PlaceNotificationsBody::new(channel(), &settings, InitialGroup::Mute, NOW);
        assert_eq!(body.chosen_mute, MuteChoice::Current);
        let saved = body.saved_into(&settings, NOW + 5_000);
        assert_eq!(
            saved
                .places
                .get(&CHANNEL)
                .and_then(|place| place.muted_until),
            Some(MuteEnd::At(end))
        );
    }

    #[test]
    fn choosing_a_length_mutes_from_now_and_dont_mute_forgets_the_place() {
        let mut body = PlaceNotificationsBody::new(
            NotificationTarget::Server { guild_id: SERVER },
            &NotificationSettings::default(),
            InitialGroup::Mute,
            NOW,
        );
        body.chosen_mute = MuteChoice::For(MuteDuration::OneHour);
        let muted = body.saved_into(&NotificationSettings::default(), NOW);
        assert_eq!(
            muted
                .places
                .get(&SERVER)
                .and_then(|place| place.muted_until),
            Some(MuteEnd::At(NOW + 3_600_000))
        );
        body.chosen_mute = MuteChoice::NotMuted;
        assert!(body.saved_into(&muted, NOW).places.is_empty());
    }

    #[test]
    fn a_mute_until_turned_back_on_shows_as_that_choice() {
        let settings = with_place(
            SERVER,
            PlaceNotificationSettings {
                level: None,
                muted_until: Some(MuteEnd::Never),
            },
        );
        let body = PlaceNotificationsBody::new(
            NotificationTarget::Server { guild_id: SERVER },
            &settings,
            InitialGroup::Mute,
            NOW,
        );
        assert_eq!(
            body.chosen_mute,
            MuteChoice::For(MuteDuration::UntilTurnedBackOn)
        );
    }
}
