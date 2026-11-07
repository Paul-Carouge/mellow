//! The conversation view headless: where the list opens, what Esc and the keys do, and how a
//! refused send comes back, over a session fed by hand as the gateway would.

use gpui::{
    AppContext as _, Entity, InputEvent as _, KeyUpEvent, Keystroke, ListOffset, TestAppContext,
    VisualTestContext, WindowAppearance, px, size,
};
use mellow_protocol::{Message, PublicUser, Snowflake};

use super::*;
use crate::components::bind_tab_navigation;
use crate::composer::bind_composer_keys;
use crate::shell_fixtures::{ANA, GUILD_RUST, ME, RUST_GENERAL, RUST_HELP, person, sample_state};
use crate::strings_pending_i18n::{ActiveStrings, ENGLISH};
use crate::theme::ActiveTheme;

fn message(id: u64, author: &PublicUser, content: &str) -> Message {
    Message {
        id: Snowflake::from_raw(id),
        channel_id: RUST_GENERAL,
        author: author.clone(),
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

fn general() -> OpenConversation {
    OpenConversation {
        channel_id: RUST_GENERAL,
        guild_id: Some(GUILD_RUST),
        label: "#general".to_owned(),
        composer_label: "Message #general".to_owned(),
        hero_icon: IconName::Hash,
        hero_title: "Welcome to #general".to_owned(),
        hero_sentence: "This is the start of the #general channel.".to_owned(),
        topic: String::new(),
    }
}

/// `#general` open over `messages` (a whole channel: fewer than a page), read up to `last_read`.
fn open_general(
    cx: &mut TestAppContext,
    messages: Vec<Message>,
    last_read: Option<u64>,
) -> (
    Entity<ConversationView>,
    Entity<Session>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(ActiveStrings(&ENGLISH));
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
        bind_tab_navigation(cx);
        bind_composer_keys(cx);
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| {
        let state = session.state_mut();
        *state = sample_state();
        state.insert_history_page(RUST_GENERAL, messages);
        if let Some(last_read) = last_read {
            state.mark_read(RUST_GENERAL, Snowflake::from_raw(last_read));
        }
    });
    let view_session = session.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| ConversationView::new(view_session, window, cx));
    cx.simulate_resize(size(px(900.0), px(600.0)));
    cx.update(|window, cx| {
        window.activate_window();
        view.update(cx, |view, cx| view.set_place(Some(general()), cx));
    });
    cx.run_until_parked();
    (view, session, cx)
}

fn ana() -> PublicUser {
    person(ANA, "ana", "Ana")
}

fn me() -> PublicUser {
    person(ME, "paul", "Paul")
}

/// A full key press: GPUI activates a focused control on the release, as browsers do.
fn press(cx: &mut VisualTestContext, key: &'static str) {
    cx.simulate_keystrokes(key);
    let keystroke = Keystroke::parse(key).expect("a valid keystroke in a test");
    cx.update(|window, cx| {
        window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
    });
    cx.run_until_parked();
}

fn composer_text(view: &Entity<ConversationView>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        let composer = view.read(cx).composer().clone();
        composer.read(cx).text_in_field(cx)
    })
}

fn focus_composer(view: &Entity<ConversationView>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| view.update(cx, |view, cx| view.focus_composer(window, cx)));
    cx.run_until_parked();
}

fn focus_list(view: &Entity<ConversationView>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        let focus = view.read(cx).list_focus.clone();
        window.focus(&focus, cx);
    });
    cx.run_until_parked();
}

fn last_read(session: &Entity<Session>, cx: &mut VisualTestContext) -> Option<Snowflake> {
    session.read_with(cx, |session, _| {
        session.state().last_read_message_id(RUST_GENERAL)
    })
}

fn many_messages(count: u64) -> Vec<Message> {
    (1..=count)
        .map(|index| {
            message(
                10_000 + index,
                &ana(),
                "A message long enough to take its own line in the list",
            )
        })
        .collect()
}

#[gpui::test]
fn a_short_channel_opens_at_its_latest_message_under_its_history_start_and_is_read(
    cx: &mut TestAppContext,
) {
    let (view, session, cx) = open_general(
        cx,
        vec![
            message(10_001, &ana(), "hello"),
            message(10_002, &ana(), "again"),
        ],
        Some(10_001),
    );
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.entries.first(),
            Some(&rows::Entry::HistoryStart),
            "the hero sits above the first message"
        );
        assert_eq!(view.entries.len(), 3);
        assert!(view.is_at_latest());
    });
    assert_eq!(
        last_read(&session, cx),
        Some(Snowflake::from_raw(10_002)),
        "shown down to its latest message, the channel is read"
    );
}

#[gpui::test]
fn the_new_divider_sits_above_the_first_unread_message_until_esc_in_the_composer(
    cx: &mut TestAppContext,
) {
    let (view, _session, cx) = open_general(
        cx,
        vec![
            message(10_001, &ana(), "read"),
            message(10_002, &me(), "mine"),
            message(10_003, &ana(), "new"),
        ],
        Some(10_001),
    );
    let first_unread = |view: &ConversationView| {
        view.entries.iter().find_map(|entry| match entry {
            rows::Entry::Row(row) => match row.kind {
                mellow_client_core::shared_logic::message_rows::MessageRowKind::Sent {
                    message_id,
                    is_first_unread: true,
                    ..
                } => Some(message_id),
                _ => None,
            },
            rows::Entry::HistoryStart => None,
        })
    };
    view.read_with(cx, |view, cx| {
        assert_eq!(
            first_unread(view),
            Some(Snowflake::from_raw(10_003)),
            "the reader's own message is never new"
        );
        assert!(view.unread_bar_sentence(cx).is_some());
    });

    focus_composer(&view, cx);
    press(cx, "escape");

    view.read_with(cx, |view, cx| {
        assert_eq!(first_unread(view), None);
        assert!(view.unread_bar_sentence(cx).is_none());
    });
}

#[gpui::test]
fn a_long_unread_run_opens_on_its_divider_and_stays_unread(cx: &mut TestAppContext) {
    let (view, session, cx) = open_general(cx, many_messages(40), Some(10_005));
    cx.run_until_parked();
    assert!(!view.read_with(cx, |view, _| view.is_at_latest()));
    assert_eq!(
        last_read(&session, cx),
        Some(Snowflake::from_raw(10_005)),
        "the messages below the divider are not read yet"
    );
}

#[gpui::test]
fn esc_in_the_list_jumps_to_present_and_typing_there_goes_to_the_composer(cx: &mut TestAppContext) {
    let (view, _session, cx) = open_general(cx, many_messages(40), None);
    view.update(cx, |view, cx| {
        view.list.scroll_to(ListOffset::default());
        view.visit.has_reader_scrolled = true;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(!view.read_with(cx, |view, _| view.is_at_latest()));

    focus_list(&view, cx);
    press(cx, "escape");
    assert!(view.read_with(cx, |view, _| view.is_at_latest()));

    focus_list(&view, cx);
    press(cx, "h");
    assert_eq!(composer_text(&view, cx), "h");
}

#[gpui::test]
fn up_in_the_list_walks_the_messages_and_page_up_leaves_the_present(cx: &mut TestAppContext) {
    let (view, _session, cx) = open_general(cx, many_messages(40), None);
    focus_list(&view, cx);
    press(cx, "up");
    let newest = (10_000 + 40_u64).to_string();
    assert_eq!(
        view.read_with(cx, |view, _| view.focused_row_key.clone()),
        Some(newest),
        "the first Up lands on the newest message"
    );
    press(cx, "up");
    assert_eq!(
        view.read_with(cx, |view, _| view.focused_row_key.clone()),
        Some((10_000 + 39_u64).to_string())
    );
    press(cx, "pageup");
    press(cx, "pageup");
    assert!(!view.read_with(cx, |view, _| view.is_at_latest()));
}

#[gpui::test]
fn up_in_an_empty_composer_edits_the_reader_s_last_message(cx: &mut TestAppContext) {
    let (view, _session, cx) = open_general(
        cx,
        vec![
            message(10_001, &me(), "my first"),
            message(10_002, &me(), "my last"),
            message(10_003, &ana(), "theirs"),
        ],
        None,
    );
    focus_composer(&view, cx);
    press(cx, "up");
    assert_eq!(composer_text(&view, cx), "my last");
}

#[gpui::test]
fn a_refused_send_gives_its_text_back_to_an_empty_composer_with_the_reason(
    cx: &mut TestAppContext,
) {
    let (view, session, cx) = open_general(cx, vec![message(10_001, &ana(), "hi")], None);
    session.update(cx, |session, cx| {
        let state = session.state_mut();
        state.queue_message(RUST_GENERAL, "too rude".into(), None, "nonce-1".into());
        state.mark_send_failed("nonce-1");
        cx.emit(SendRefused {
            channel_id: RUST_GENERAL,
            nonce: "nonce-1".to_owned(),
            reason: "Couldn't send the message: the instance refused it.".to_owned(),
        });
    });
    cx.run_until_parked();
    assert_eq!(composer_text(&view, cx), "too rude");
    assert!(view.read_with(cx, |view, _| view.action_error.is_some()));
    let is_still_queued = session.read_with(cx, |session, _| {
        session
            .state()
            .pending_messages(RUST_GENERAL)
            .next()
            .is_some()
    });
    assert!(
        !is_still_queued,
        "the text left the outbox for the composer"
    );
}

#[gpui::test]
fn a_refused_send_stays_failed_when_the_composer_holds_another_message(cx: &mut TestAppContext) {
    let (view, session, cx) = open_general(cx, vec![message(10_001, &ana(), "hi")], None);
    focus_composer(&view, cx);
    press(cx, "w");
    session.update(cx, |session, cx| {
        let state = session.state_mut();
        state.queue_message(RUST_GENERAL, "too rude".into(), None, "nonce-1".into());
        state.mark_send_failed("nonce-1");
        cx.emit(SendRefused {
            channel_id: RUST_GENERAL,
            nonce: "nonce-1".to_owned(),
            reason: "Couldn't send the message.".to_owned(),
        });
    });
    cx.run_until_parked();
    assert_eq!(composer_text(&view, cx), "w");
    let is_still_queued = session.read_with(cx, |session, _| {
        session
            .state()
            .pending_messages(RUST_GENERAL)
            .next()
            .is_some()
    });
    assert!(
        is_still_queued,
        "nothing typed is lost: Retry and Delete stay"
    );
}

#[gpui::test]
fn copy_message_id_puts_the_id_on_the_clipboard_and_says_so(cx: &mut TestAppContext) {
    let (view, _session, cx) = open_general(cx, vec![message(10_001, &ana(), "hi")], None);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.run_menu_action(
                Snowflake::from_raw(10_001),
                MessageMenuAction::CopyMessageId,
                window,
                cx,
            );
        });
    });
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    assert_eq!(copied, "10001");
    assert_eq!(
        view.read_with(cx, |view, _| view.announcement.to_string()),
        "Message ID copied."
    );
}

#[gpui::test]
fn scrolling_and_unrelated_changes_do_not_rebuild_the_rows(cx: &mut TestAppContext) {
    let (view, _session, cx) = open_general(cx, many_messages(40), None);
    view.update(cx, |view, cx| {
        view.note_reader_scrolled(false, cx);
        assert!(!view.are_rows_stale, "a scroll leaves the rows as they are");
        view.follow_state(
            &[
                StateChange::Typing {
                    channel_id: RUST_GENERAL,
                },
                StateChange::Messages {
                    channel_id: RUST_HELP,
                },
            ],
            cx,
        );
        assert!(!view.are_rows_stale, "nor does typing or another channel");
        view.follow_state(
            &[StateChange::Messages {
                channel_id: RUST_GENERAL,
            }],
            cx,
        );
        assert!(view.are_rows_stale, "the open channel's messages do");
    });
}

#[gpui::test]
fn space_on_a_control_inside_the_list_activates_it_instead_of_typing(cx: &mut TestAppContext) {
    let mut reacted = message(10_001, &ana(), "hello");
    reacted.reactions = vec![mellow_protocol::ReactionSummary {
        emoji: "👍".to_owned(),
        count: 1,
        me: false,
    }];
    let (view, _session, cx) = open_general(cx, vec![reacted], None);
    focus_list(&view, cx);
    // The test window has no root to turn Tab into this.
    cx.update(Window::focus_next);
    cx.run_until_parked();
    let is_list_itself_focused = cx.update(|window, cx| {
        let list_focus = view.read(cx).list_focus.clone();
        list_focus.is_focused(window)
    });
    assert!(!is_list_itself_focused, "Tab reaches the reaction pill");

    press(cx, "space");

    assert_eq!(
        composer_text(&view, cx),
        "",
        "Space is not typed into the composer"
    );
}

#[gpui::test]
fn an_older_page_landing_at_the_history_start_keeps_the_rows_read_in_place(
    cx: &mut TestAppContext,
) {
    // A full page: the channel has older history.
    let page: Vec<Message> = (101..=150)
        .map(|id| {
            message(
                10_000 + id,
                &ana(),
                "A message long enough to take its own line",
            )
        })
        .collect();
    let (view, session, cx) = open_general(cx, page, None);
    view.update(cx, |view, cx| {
        view.list.scroll_to(ListOffset::default());
        view.visit.has_reader_scrolled = true;
        cx.notify();
    });
    cx.run_until_parked();

    let older: Vec<Message> = (91..=100)
        .map(|id| message(10_000 + id, &ana(), "An older message"))
        .collect();
    session.update(cx, |session, cx| {
        let change = session.state_mut().insert_history_page(RUST_GENERAL, older);
        cx.emit(StateChanged(vec![change]));
    });
    cx.run_until_parked();

    view.read_with(cx, |view, _| {
        let top = view.list.logical_scroll_top();
        assert_eq!(
            top.item_ix,
            1 + 10,
            "the first row read stays at the top, below the ten new ones"
        );
    });
}
