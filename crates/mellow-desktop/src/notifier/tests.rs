use gpui::{
    Entity, SystemNotificationResponse, TestAppContext, VisualTestContext, WindowAppearance, px,
    size,
};
use mellow_client_core::GatewayUpdate;
use mellow_protocol::{GatewayEvent, Message};

use super::*;
use crate::preferences::change_preferences;
use crate::shell_fixtures::{
    ANA, CONVERSATION_ANA, ME, RUST_ANNOUNCEMENTS, RUST_GENERAL, ZOE, person, sample_state,
};
use crate::strings_pending_i18n::{ActiveStrings, ENGLISH};
use crate::theme::ActiveTheme;

fn message(id: u64, channel_id: Snowflake, author: Snowflake, content: &str) -> Message {
    let name = if author == ANA { "Ana" } else { "Zoé" };
    Message {
        id: Snowflake::from_raw(id),
        channel_id,
        author: person(author, &name.to_lowercase(), name),
        content: content.to_owned(),
        reply_to_id: None,
        edited_at_unix_milliseconds: None,
        nonce: None,
        attachments: Vec::new(),
        reactions: Vec::new(),
        pinned: false,
        mention_ids: Vec::new(),
        mention_role_ids: Vec::new(),
        link_previews: Vec::new(),
        poll: None,
        checklist: None,
        progress_bar: None,
        forwarded_from: None,
    }
}

/// The shell over the sample world with its notifier, desktop notifications on and the chime off
/// (no audio device in tests).
fn open_notifier(
    cx: &mut TestAppContext,
) -> (
    Entity<Session>,
    Entity<DuskShell>,
    Entity<Notifier>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(ActiveStrings(&ENGLISH));
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
        cx.set_app_identity("org.mellow.test", "Mellow");
        change_preferences(cx, |preferences| {
            let settings = NotificationSettings {
                are_desktop_notifications_enabled: true,
                is_sound_enabled: false,
                ..NotificationSettings::default()
            };
            preferences.set_notification_settings(ME, settings);
        });
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| *session.state_mut() = sample_state());
    let shell_session = session.clone();
    let (shell, cx) = cx.add_window_view(|window, cx| DuskShell::new(shell_session, window, cx));
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    let notifier_session = session.clone();
    let notifier_shell = shell.clone();
    let notifier = cx.update(|window, cx| {
        let notifier = cx.new(|cx| Notifier::new(notifier_session, cx));
        notifier.update(cx, |notifier, cx| {
            notifier.show_in(&notifier_shell, window, cx);
        });
        notifier
    });
    cx.run_until_parked();
    (session, shell, notifier, cx)
}

fn receive(session: &Entity<Session>, message: Message, cx: &mut VisualTestContext) {
    session.update(cx, |session, cx| {
        session.apply_gateway_updates_of_this_session(
            vec![GatewayUpdate::Event(GatewayEvent::MessageCreate(message))],
            cx,
        );
    });
    cx.run_until_parked();
}

#[gpui::test]
fn a_message_in_the_background_notifies_and_its_click_opens_the_conversation(
    cx: &mut TestAppContext,
) {
    let (session, shell, _notifier, cx) = open_notifier(cx);
    cx.deactivate_window();
    cx.run_until_parked();

    receive(
        &session,
        message(10_001, CONVERSATION_ANA, ANA, "<b>bring</b> tea"),
        cx,
    );
    receive(
        &session,
        message(10_002, CONVERSATION_ANA, ANA, "and <b>cake</b>"),
        cx,
    );

    let delivered = cx.delivered_system_notifications();
    assert_eq!(delivered.len(), 1, "one notification per conversation");
    assert_eq!(delivered[0].tag.as_ref(), "mellow-channel-50");
    // Where a notification cannot be replaced, the first one stays alone (module doc).
    let (expected_title, expected_body) = if CAN_REPLACE_SYSTEM_NOTIFICATIONS {
        ("Ana, 2 new messages", "and <b>cake</b>")
    } else {
        ("Ana", "&lt;b&gt;bring&lt;/b&gt; tea")
    };
    assert_eq!(delivered[0].title.as_ref(), expected_title);
    assert_eq!(delivered[0].body.as_ref(), expected_body);
    assert_eq!(cx.shown_system_notifications().len(), {
        if CAN_REPLACE_SYSTEM_NOTIFICATIONS {
            2
        } else {
            1
        }
    });
    assert_eq!(cx.window_title().as_deref(), Some("(2) Mellow"));

    cx.simulate_system_notification_response(SystemNotificationResponse {
        tag: "mellow-channel-50".into(),
        action_id: None,
    });
    cx.run_until_parked();
    let selected = shell.read_with(cx, |shell, _| shell.navigation().selected_channel_id());
    assert_eq!(selected, Some(CONVERSATION_ANA));
    assert!(
        cx.delivered_system_notifications().is_empty(),
        "seen, it goes"
    );
}

#[gpui::test]
fn with_its_window_closed_mellow_still_notifies(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(ActiveStrings(&ENGLISH));
        cx.set_app_identity("org.mellow.test", "Mellow");
        change_preferences(cx, |preferences| {
            let settings = NotificationSettings {
                are_desktop_notifications_enabled: true,
                is_sound_enabled: false,
                ..NotificationSettings::default()
            };
            preferences.set_notification_settings(ME, settings);
        });
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| *session.state_mut() = sample_state());
    // The app's notifier, as `follow_session_for_notifications` makes it: no window shows it.
    let notifier = cx.update(|cx| notifier_of_session(&session, cx));
    session.update(cx, |session, cx| {
        session.apply_gateway_updates_of_this_session(
            vec![GatewayUpdate::Event(GatewayEvent::MessageCreate(message(
                10_001,
                CONVERSATION_ANA,
                ANA,
                "tea?",
            )))],
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(cx.shown_system_notifications().len(), 1);

    // Signed out, it ends; the next session gets a new one.
    cx.update(end_notifications);
    let next = cx.update(|cx| notifier_of_session(&session, cx));
    assert_ne!(next.entity_id(), notifier.entity_id());
}

#[gpui::test]
fn only_what_the_rules_allow_notifies(cx: &mut TestAppContext) {
    let (session, _shell, _notifier, cx) = open_notifier(cx);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    // Looking at Mellow: nothing reaches the system.
    receive(&session, message(10_001, CONVERSATION_ANA, ANA, "hi"), cx);
    assert_eq!(cx.shown_system_notifications(), []);

    cx.deactivate_window();
    cx.run_until_parked();
    // A server tells about mentions only.
    receive(
        &session,
        message(10_002, RUST_GENERAL, ZOE, "hello all"),
        cx,
    );
    assert_eq!(cx.shown_system_notifications(), []);
    let mut mention = message(10_003, RUST_GENERAL, ZOE, "<@1000> ||secret|| look");
    mention.mention_ids = vec![ME];
    receive(&session, mention, cx);
    let shown = cx.shown_system_notifications();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].title.as_ref(), "#general · Rust Lyon");
    assert!(
        shown[0].body.starts_with("Zoé") && !shown[0].body.contains("secret"),
        "the author, and a spoiler stays hidden: {}",
        shown[0].body
    );

    // Previews off: who wrote, nothing more.
    cx.update(|_, cx| {
        change_preferences(cx, |preferences| {
            let mut settings = preferences.notification_settings_of(ME);
            settings.should_show_previews = false;
            preferences.set_notification_settings(ME, settings);
        });
    });
    let mut mention = message(10_004, RUST_ANNOUNCEMENTS, ZOE, "<@1000> private words");
    mention.mention_ids = vec![ME];
    receive(&session, mention, cx);
    let shown = cx.shown_system_notifications();
    assert_eq!(
        shown.last().map(|shown| shown.body.as_ref()),
        Some("New message from Zoé")
    );
}

#[gpui::test]
fn reading_the_open_conversation_or_signing_out_clears_what_was_shown(cx: &mut TestAppContext) {
    let (session, shell, notifier, cx) = open_notifier(cx);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_channel(CONVERSATION_ANA, window, cx);
        });
    });
    cx.deactivate_window();
    cx.run_until_parked();
    receive(&session, message(10_001, CONVERSATION_ANA, ANA, "hi"), cx);
    assert_eq!(cx.delivered_system_notifications().len(), 1);

    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert!(
        cx.delivered_system_notifications().is_empty(),
        "back on the open conversation, it goes"
    );

    cx.deactivate_window();
    cx.run_until_parked();
    let mut mention = message(10_002, RUST_GENERAL, ZOE, "<@1000> look");
    mention.mention_ids = vec![ME];
    receive(&session, mention, cx);
    assert_eq!(cx.delivered_system_notifications().len(), 1);
    assert_ne!(cx.window_title().as_deref(), Some("Mellow"));
    notifier.update(cx, Notifier::end);
    cx.run_until_parked();
    assert_eq!(cx.delivered_system_notifications(), []);
    assert_eq!(cx.window_title().as_deref(), Some("Mellow"));
}

#[test]
fn where_notifications_pile_up_a_channel_shows_once_and_a_flood_stops() {
    assert!(should_show_system_notification(5, 500, true));
    assert!(should_show_system_notification(1, 0, false));
    assert!(!should_show_system_notification(2, 0, false));
    assert!(!should_show_system_notification(
        1,
        MAXIMUM_NOTIFICATIONS_WHILE_AWAY,
        false
    ));
}

#[test]
fn text_from_others_reaches_the_system_without_controls_or_markup() {
    assert_eq!(
        plain_text_for_the_system("a\u{0007}b\nc\u{202E}d"),
        "a b c d"
    );
    assert_eq!(
        escape_freedesktop_markup("<a href=\"x\">&'</a>"),
        "&lt;a href=&quot;x&quot;&gt;&amp;&apos;&lt;/a&gt;"
    );
    assert_eq!(
        channel_of_tag(&notification_tag(Snowflake::from_raw(42))),
        Some(Snowflake::from_raw(42))
    );
    assert_eq!(channel_of_tag("someone-else-42"), None);
}

#[test]
fn a_mute_end_is_waited_for_just_past_it_and_never_longer_than_a_day() {
    assert_eq!(mute_end_wait(1_000, 400), Duration::from_millis(601));
    assert_eq!(mute_end_wait(400, 1_000), Duration::from_millis(1));
    assert_eq!(mute_end_wait(u64::MAX, 0), MAXIMUM_MUTE_END_WAIT);
}

#[gpui::test]
fn ended_mutes_are_dropped_and_a_running_one_is_waited_for(cx: &mut TestAppContext) {
    use mellow_client_core::shared_logic::notification_settings::{
        MuteEnd, PlaceNotificationSettings, with_place_settings,
    };
    let (_session, _shell, notifier, cx) = open_notifier(cx);
    let far_future = u64::MAX / 2;
    cx.update(|_, cx| {
        change_preferences(cx, move |preferences| {
            let settings = preferences.notification_settings_of(ME);
            let ended = with_place_settings(
                &settings,
                CONVERSATION_ANA,
                PlaceNotificationSettings {
                    level: None,
                    muted_until: Some(MuteEnd::At(1)),
                },
            );
            let running = with_place_settings(
                &ended,
                RUST_GENERAL,
                PlaceNotificationSettings {
                    level: None,
                    muted_until: Some(MuteEnd::At(far_future)),
                },
            );
            preferences.set_notification_settings(ME, running);
        });
    });
    cx.run_until_parked();
    let places = cx.update(|_, cx| notification_settings_of(ME, cx).places);
    assert!(
        !places.contains_key(&CONVERSATION_ANA),
        "an ended mute goes"
    );
    assert!(places.contains_key(&RUST_GENERAL));
    let waited_end = notifier.read_with(cx, |notifier, _| {
        notifier.mute_end_timer.as_ref().map(|(end, _)| *end)
    });
    assert_eq!(waited_end, Some(far_future));
}
