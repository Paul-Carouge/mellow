//! Tells the reader about new messages, as the web's `notifier.svelte.ts` does
//! (docs/design/web-notifications.md): a system notification while the window is in the
//! background, the chime while they read elsewhere in Mellow, and the count in the window's title,
//! on the Dock and in the menu bar. Who is told and in what words is decided by client-core's
//! `shared_logic::notifications`; this module gathers the facts and does the telling.
//!
//! Notifications go through GPUI's system notifications: `UNUserNotificationCenter` on macOS,
//! freedesktop notifications over D-Bus on Linux and toasts on Windows. macOS only delivers them
//! to an app bundle: run unbundled (`cargo run -p mellow-desktop`), GPUI logs "system
//! notifications disabled: not running from an app bundle" once and shows nothing, while the
//! chime, the title and the Dock badge still work. macOS asks the reader for permission the first
//! time a notification is shown; when it is refused, nothing shows and nothing fails. GPUI has
//! no API to read that answer, so the settings dialog cannot say "blocked" as the web does; it
//! posts a confirmation when the switch is turned on, so the question comes right after the click.
//!
//! On Linux and BSD, GPUI 0.3.11 ignores the tag and its dismiss does nothing: a notification is
//! never replaced, nor taken back when the channel is read or at sign-out, and each one holds a
//! thread and a D-Bus connection until it closes. So there, a channel notifies once until the
//! reader looks at it, and at most [`MAXIMUM_NOTIFICATIONS_WHILE_AWAY`] show while the window is
//! in the background.
// shortcut: Linux notifications are neither replaced nor dismissed; lift the limit once GPUI keeps
// the server's id, sends `replaces_id` and closes with `CloseNotification`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{
    AnyWindowHandle, App, Context, Entity, SharedString, Subscription, SystemNotification, Task,
    WeakEntity, Window,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::notification_settings::{
    NotificationSettings, next_mute_end, without_ended_mutes,
};
use mellow_client_core::shared_logic::notifications::{
    AlertDelivery, IncomingMessageFacts, MessagePreview, NotificationBody,
    NotificationContentFacts, NotificationPlace, NotificationTitle, PreviewWords, ReaderAttention,
    alert_delivery_for, markdown_as_plain_text, message_preview, notification_content,
    should_alert_about, window_title,
};
use mellow_client_core::state::ClientState;
use mellow_protocol::{Message, Snowflake};

use crate::dock_and_tray;
use crate::home::conversation_display_name;
use crate::i18n::{Strings, strings};
use crate::preferences::{PreferencesStore, change_preferences, preferences};
use crate::session::{IncomingMessage, Session, StateChanged, now_unix_milliseconds};
use crate::shell::DuskShell;
use crate::window_chrome::WINDOW_TITLE;

/// A burst rings once: the chime does not play again within this time.
const MINIMUM_TIME_BETWEEN_SOUNDS: Duration = Duration::from_secs(2);
/// One notification per channel: a newer one replaces it.
const NOTIFICATION_TAG_PREFIX: &str = "mellow-channel-";
/// The settings dialog's confirmation; not a channel's, so a click on it opens nothing.
pub const TURNED_ON_NOTIFICATION_TAG: &str = "mellow-notifications-turned-on";
/// Whether a newer notification with the same tag replaces the older one, and dismiss works.
const CAN_REPLACE_SYSTEM_NOTIFICATIONS: bool = !cfg!(all(unix, not(target_os = "macos")));
/// Where notifications pile up (see the module doc), a flood stops here until the reader is back.
const MAXIMUM_NOTIFICATIONS_WHILE_AWAY: u32 = 20;
/// Platform timers overflow on huge waits; a longer mute wakes up early and waits again (the
/// web's `refreshClock`).
const MAXIMUM_MUTE_END_WAIT: Duration = Duration::from_hours(24);

/// The notification choices of `user_id`, as the preferences hold them.
#[must_use]
pub fn notification_settings_of(user_id: Snowflake, cx: &App) -> NotificationSettings {
    preferences(cx).map_or_else(NotificationSettings::default, |preferences| {
        preferences.notification_settings_of(user_id)
    })
}

/// Lives as long as the signed-in shell; [`Notifier::end`] clears what it showed.
pub struct Notifier {
    session: Entity<Session>,
    shell: WeakEntity<DuskShell>,
    window: AnyWindowHandle,
    /// Notifications shown per channel since the reader last looked at it.
    alert_count_by_channel: HashMap<Snowflake, u32>,
    last_sound_at: Option<Instant>,
    /// System notifications shown since the window was last active.
    notifications_shown_while_away: u32,
    shown_attention_count: u32,
    /// Wakes at the earliest end of a running mute, which it then drops from the preferences.
    mute_end_timer: Option<(u64, Task<()>)>,
    /// The account the mute clock was last set for, so other gateway events skip it.
    mute_clock_user_id: Option<Snowflake>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for Notifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Notifier")
            .field("alert_count_by_channel", &self.alert_count_by_channel)
            .field("shown_attention_count", &self.shown_attention_count)
            .finish_non_exhaustive()
    }
}

impl Notifier {
    pub fn new(
        session: Entity<Session>,
        shell: &Entity<DuskShell>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.subscribe_in(
                &session,
                window,
                |notifier, _, IncomingMessage(message), window, cx| {
                    notifier.alert_about(message, window, cx);
                },
            ),
            cx.subscribe_in(
                &session,
                window,
                |notifier, _, _: &StateChanged, window, cx| {
                    notifier.show_attention_count(window, cx);
                    // The account is known from the first `Ready` on.
                    let user_id = notifier
                        .session
                        .read(cx)
                        .state()
                        .current_user()
                        .map(|user| user.id);
                    if user_id != notifier.mute_clock_user_id {
                        notifier.mute_clock_user_id = user_id;
                        notifier.wake_at_next_mute_end(window, cx);
                    }
                },
            ),
            // A mute or a choice changed.
            cx.observe_global_in::<PreferencesStore>(window, |notifier, window, cx| {
                notifier.show_attention_count(window, cx);
                notifier.wake_at_next_mute_end(window, cx);
            }),
            cx.observe_in(shell, window, |notifier, _, window, cx| {
                notifier.note_open_channel_seen(window, cx);
            }),
            cx.observe_window_activation(window, Self::note_open_channel_seen),
            // Timers stand still while the Mac sleeps: a mute that ended meanwhile goes now.
            cx.observe_window_activation(window, Self::wake_at_next_mute_end),
        ];
        let notifier = cx.entity().downgrade();
        cx.on_system_notification_response(move |response, cx| {
            let Some(channel_id) = channel_of_tag(&response.tag) else {
                return;
            };
            if let Some(notifier) = notifier.upgrade() {
                notifier.update(cx, |notifier, cx| notifier.open_channel(channel_id, cx));
            }
        });
        let mut notifier = Self {
            session,
            shell: shell.downgrade(),
            window: window.window_handle(),
            alert_count_by_channel: HashMap::new(),
            last_sound_at: None,
            notifications_shown_while_away: 0,
            shown_attention_count: 0,
            mute_end_timer: None,
            mute_clock_user_id: None,
            _subscriptions: subscriptions,
        };
        notifier.show_attention_count(window, cx);
        notifier.wake_at_next_mute_end(window, cx);
        notifier
    }

    /// Signed out: its notifications go (not on Linux, see the module doc), and the title and
    /// badges are cleared.
    pub fn end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for channel_id in self.alert_count_by_channel.keys() {
            cx.dismiss_system_notification(&notification_tag(*channel_id));
        }
        self.alert_count_by_channel.clear();
        self.notifications_shown_while_away = 0;
        self.shown_attention_count = 0;
        window.set_window_title(WINDOW_TITLE);
        dock_and_tray::show_attention_count(0, cx);
    }

    fn alert_about(&mut self, message: &Message, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.session.read(cx).state();
        let Some(user) = state.current_user() else {
            return;
        };
        let Some(place) = place_of(state, message.channel_id) else {
            return;
        };
        let settings = notification_settings_of(user.id, cx);
        let reader_role_ids = place
            .guild_id
            .and_then(|guild_id| state.guild(guild_id))
            .map_or(&[][..], |guild| guild.my_role_ids.as_slice());
        let should_alert = should_alert_about(IncomingMessageFacts {
            message,
            current_user_id: Some(user.id),
            reader_role_ids,
            place,
            is_author_blocked: state.is_blocked(message.author.id),
            chosen_status: Some(user.status),
            settings: &settings,
            now: now_unix_milliseconds(),
        });
        if !should_alert {
            return;
        }
        let open_channel_id = self
            .shell
            .upgrade()
            .and_then(|shell| shell.read(cx).navigation().selected_channel_id());
        let delivery = alert_delivery_for(
            message.channel_id,
            ReaderAttention {
                // GPUI cannot tell a hidden window from one behind others; neither is active.
                is_window_visible: true,
                has_window_focus: window.is_window_active(),
                open_channel_id,
            },
        );
        if delivery == AlertDelivery::None {
            return;
        }
        let should_chime = settings.is_sound_enabled && self.is_chime_due();
        let mut notification = None;
        if delivery == AlertDelivery::System && settings.are_desktop_notifications_enabled {
            let count = self
                .alert_count_by_channel
                .entry(message.channel_id)
                .or_insert(0);
            *count = count.saturating_add(1);
            if should_show_system_notification(
                *count,
                self.notifications_shown_while_away,
                CAN_REPLACE_SYSTEM_NOTIFICATIONS,
            ) {
                self.notifications_shown_while_away =
                    self.notifications_shown_while_away.saturating_add(1);
                notification = Some(notification_for(
                    state,
                    message,
                    place,
                    *count,
                    settings.should_show_previews,
                    strings(cx),
                ));
            }
        }
        if should_chime {
            // On the voice output, through the call during one (`voice`).
            crate::voice::play_notification_chime(cx);
        }
        if let Some(notification) = notification {
            cx.show_system_notification(notification);
        }
    }

    /// Chimes stay apart by `MINIMUM_TIME_BETWEEN_SOUNDS`; true marks this one as played.
    fn is_chime_due(&mut self) -> bool {
        let now = Instant::now();
        if self
            .last_sound_at
            .is_some_and(|last| now.duration_since(last) < MINIMUM_TIME_BETWEEN_SOUNDS)
        {
            return false;
        }
        self.last_sound_at = Some(now);
        true
    }

    /// A notification was clicked: the window comes to the front at its channel.
    fn open_channel(&mut self, channel_id: Snowflake, cx: &mut Context<Self>) {
        // A notification left from an account signed out since (Linux keeps them) opens nothing.
        if place_of(self.session.read(cx).state(), channel_id).is_none() {
            return;
        }
        cx.activate(true);
        let shell = self.shell.clone();
        let opened = self.window.update(cx, |_, window, cx| {
            window.activate_window();
            shell.update(cx, |shell, cx| shell.open_channel(channel_id, window, cx))
        });
        if opened.is_err() {
            tracing::debug!("the notification's window is gone");
        }
    }

    /// The reader looks at a channel: its notification goes and its count starts again.
    fn note_open_channel_seen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !window.is_window_active() {
            return;
        }
        self.notifications_shown_while_away = 0;
        let Some(channel_id) = self
            .shell
            .upgrade()
            .and_then(|shell| shell.read(cx).navigation().selected_channel_id())
        else {
            return;
        };
        if self.alert_count_by_channel.remove(&channel_id).is_some() {
            cx.dismiss_system_notification(&notification_tag(channel_id));
        }
    }

    /// Ends mutes on time, as the web's `refreshClock`: one timer at the earliest end drops the
    /// mutes that ended from the preferences, so the rows, the rail, the count and the badge
    /// follow, and the next end is waited for then. Nothing polls.
    fn wake_at_next_mute_end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(user_id) = self
            .session
            .read(cx)
            .state()
            .current_user()
            .map(|user| user.id)
        else {
            self.mute_end_timer = None;
            return;
        };
        let now = now_unix_milliseconds();
        let settings = notification_settings_of(user_id, cx);
        let cleaned = without_ended_mutes(&settings, now);
        if cleaned != settings {
            // Saving comes back here through the preferences' observer, which waits for the next.
            change_preferences(cx, move |preferences| {
                preferences.set_notification_settings(user_id, cleaned);
            });
            return;
        }
        let Some(end) = next_mute_end(&settings, now) else {
            self.mute_end_timer = None;
            return;
        };
        if self
            .mute_end_timer
            .as_ref()
            .is_some_and(|(waited_end, _)| *waited_end == end)
        {
            return;
        }
        let wait = mute_end_wait(end, now);
        let timer = cx.spawn_in(window, async move |notifier, cx| {
            cx.background_executor().timer(wait).await;
            let _ = notifier.update_in(cx, |notifier, window, cx| {
                notifier.mute_end_timer = None;
                notifier.wake_at_next_mute_end(window, cx);
            });
        });
        self.mute_end_timer = Some((end, timer));
    }

    /// "(3) Mellow" in the title, and the count on the Dock and in the menu bar.
    fn show_attention_count(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.session.read(cx).state();
        let count = state.current_user().map_or(0, |user| {
            let settings = notification_settings_of(user.id, cx);
            state.attention_count(&settings, now_unix_milliseconds())
        });
        if count == self.shown_attention_count {
            return;
        }
        self.shown_attention_count = count;
        window.set_window_title(&window_title(count, WINDOW_TITLE));
        dock_and_tray::show_attention_count(count, cx);
    }
}

/// How long to wait for a mute ending at `end`: just past it, at most [`MAXIMUM_MUTE_END_WAIT`].
fn mute_end_wait(end: u64, now: u64) -> Duration {
    Duration::from_millis(end.saturating_sub(now).saturating_add(1)).min(MAXIMUM_MUTE_END_WAIT)
}

/// Where notifications cannot be replaced, a channel shows only its first one until it is seen,
/// and a flood stops at [`MAXIMUM_NOTIFICATIONS_WHILE_AWAY`].
fn should_show_system_notification(
    count_in_channel: u32,
    shown_while_away: u32,
    can_replace: bool,
) -> bool {
    can_replace || (count_in_channel == 1 && shown_while_away < MAXIMUM_NOTIFICATIONS_WHILE_AWAY)
}

fn notification_tag(channel_id: Snowflake) -> String {
    format!("{NOTIFICATION_TAG_PREFIX}{channel_id}")
}

fn channel_of_tag(tag: &str) -> Option<Snowflake> {
    tag.strip_prefix(NOTIFICATION_TAG_PREFIX)?.parse().ok()
}

/// A server's channel, or a conversation; `None` for a channel this session does not know.
fn place_of(state: &ClientState, channel_id: Snowflake) -> Option<NotificationPlace> {
    if state.direct_message_channel(channel_id).is_some() {
        return Some(NotificationPlace {
            guild_id: None,
            channel_id,
        });
    }
    state
        .guild_of_channel(channel_id)
        .map(|guild| NotificationPlace {
            guild_id: Some(guild.id),
            channel_id,
        })
}

/// The person or group for a conversation, "#general · Rust Lyon" for a channel; and whether the
/// conversation is one-to-one.
fn place_title(state: &ClientState, place: NotificationPlace, strings: Strings) -> (String, bool) {
    let Some(guild_id) = place.guild_id else {
        return state.direct_message_channel(place.channel_id).map_or_else(
            || (WINDOW_TITLE.to_owned(), false),
            |conversation| {
                let current_user_id = state.current_user().map(|user| user.id);
                (
                    conversation_display_name(conversation, current_user_id, strings),
                    conversation.owner_id.is_none(),
                )
            },
        );
    };
    let guild = state.guild(guild_id);
    let channel = guild.and_then(|guild| {
        guild
            .channels
            .iter()
            .find(|channel| channel.id == place.channel_id)
    });
    match (guild, channel) {
        (Some(guild), Some(channel)) => (
            strings
                .notifications()
                .channel_in_server(&channel.name, &guild.name),
            false,
        ),
        _ => (WINDOW_TITLE.to_owned(), false),
    }
}

/// The message as one line: a poll's or tracker's summary, else its text without Markdown.
fn preview_of(state: &ClientState, message: &Message, strings: Strings) -> MessagePreview {
    let text = if let Some(poll) = &message.poll {
        strings.polls().summary(&poll.question)
    } else if let Some(checklist) = &message.checklist {
        strings.checklists().summary(&checklist.title)
    } else if let Some(progress_bar) = &message.progress_bar {
        strings.progress_bars().summary(&progress_bar.label)
    } else {
        let roles = state
            .guild_of_channel(message.channel_id)
            .map(|guild| {
                guild
                    .roles
                    .iter()
                    .map(|role| (role.id, role.clone()))
                    .collect()
            })
            .unwrap_or_default();
        markdown_as_plain_text(
            &message.content,
            &roles,
            PreviewWords {
                spoiler: strings.notifications().spoiler(),
                deleted_role_name: strings.messages().deleted_role_name(),
            },
        )
    };
    message_preview(&text, message.attachments.len())
}

fn notification_for(
    state: &ClientState,
    message: &Message,
    place: NotificationPlace,
    message_count: u32,
    should_show_preview: bool,
    strings: Strings,
) -> SystemNotification {
    let (place_title, is_one_to_one_conversation) = place_title(state, place, strings);
    let preview = match preview_of(state, message, strings) {
        MessagePreview::Text(text) => Some(text),
        MessagePreview::SentFiles(count) => Some(
            strings
                .notifications()
                .sent_files(i64::try_from(count).unwrap_or(i64::MAX)),
        ),
        MessagePreview::Empty => None,
    };
    let (title, body) = notification_content(NotificationContentFacts {
        is_one_to_one_conversation,
        has_preview: preview.is_some(),
        message_count,
        should_show_preview,
    });
    let author = display_name(&message.author);
    let preview = preview.unwrap_or_default();
    let title = match title {
        NotificationTitle::Place => place_title,
        NotificationTitle::PlaceWithCount(count) => strings
            .notifications()
            .title_with_count(&place_title, i64::from(count)),
    };
    let body = match body {
        NotificationBody::NewMessageFrom => strings.notifications().new_message_from(author),
        NotificationBody::Preview => preview,
        NotificationBody::AuthorSaid => strings.messages().author_said(author, &preview),
    };
    SystemNotification {
        tag: notification_tag(message.channel_id).into(),
        title: plain_text_for_the_system(&title).into(),
        body: body_for_the_system(&body),
        actions: Vec::new(),
    }
}

/// Names and messages come from other people: no control characters, and no bidirectional
/// overrides that would make the text read as something else.
fn plain_text_for_the_system(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() || is_bidirectional_control(character) {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn is_bidirectional_control(character: char) -> bool {
    matches!(character, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// The body as plain text; freedesktop notification servers read `<b>`, `<a href>` and `<img>`
/// in it, so its markup characters are escaped there.
fn body_for_the_system(text: &str) -> SharedString {
    let plain = plain_text_for_the_system(text);
    if cfg!(all(unix, not(target_os = "macos"))) {
        escape_freedesktop_markup(&plain).into()
    } else {
        plain.into()
    }
}

fn escape_freedesktop_markup(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests;
