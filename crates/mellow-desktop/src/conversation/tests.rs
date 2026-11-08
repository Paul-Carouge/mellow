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

pub(super) fn message(id: u64, author: &PublicUser, content: &str) -> Message {
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
pub(super) fn open_general(
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

pub(super) fn ana() -> PublicUser {
    person(ANA, "ana", "Ana")
}

pub(super) fn me() -> PublicUser {
    person(ME, "paul", "Paul")
}

/// A full key press: GPUI activates a focused control on the release, as browsers do.
pub(super) fn press(cx: &mut VisualTestContext, key: &'static str) {
    cx.simulate_keystrokes(key);
    let keystroke = Keystroke::parse(key).expect("a valid keystroke in a test");
    cx.update(|window, cx| {
        window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
    });
    cx.run_until_parked();
}

pub(super) fn composer_text(view: &Entity<ConversationView>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        let composer = view.read(cx).composer().clone();
        composer.read(cx).text_in_field(cx)
    })
}

pub(super) fn focus_composer(view: &Entity<ConversationView>, cx: &mut VisualTestContext) {
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
            rows::Entry::HistoryStart | rows::Entry::BlockedRun(_) => None,
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

fn block_ana(session: &Entity<Session>, cx: &mut VisualTestContext) {
    session.update(cx, |session, cx| {
        session
            .state_mut()
            .apply_relationship(mellow_protocol::Relationship {
                user: ana(),
                kind: mellow_protocol::RelationshipKind::Blocked,
                since_unix_milliseconds: 0,
            });
        cx.emit(StateChanged(vec![StateChange::Relationships]));
    });
    cx.run_until_parked();
}

fn blocked_runs(view: &Entity<ConversationView>, cx: &mut VisualTestContext) -> Vec<(usize, bool)> {
    view.read_with(cx, |view, _| {
        view.entries
            .iter()
            .filter_map(|entry| match entry {
                rows::Entry::BlockedRun(run) => Some((run.message_ids.len(), run.is_revealed)),
                _ => None,
            })
            .collect()
    })
}

#[gpui::test]
fn a_blocked_authors_run_folds_into_one_row_that_enter_shows_and_hides(cx: &mut TestAppContext) {
    let (view, session, cx) = open_general(
        cx,
        vec![
            message(10_001, &ana(), "one"),
            message(10_002, &ana(), "two"),
            message(10_003, &me(), "mine"),
        ],
        None,
    );
    block_ana(&session, cx);
    assert_eq!(blocked_runs(&view, cx), [(2, false)]);
    assert_eq!(
        view.read_with(cx, |view, _| view.entries.len()),
        3,
        "the history start, the folded run and the reader's message"
    );

    focus_list(&view, cx);
    press(cx, "up");
    press(cx, "up");
    press(cx, "enter");
    assert_eq!(blocked_runs(&view, cx), [(2, true)]);
    assert_eq!(view.read_with(cx, |view, _| view.entries.len()), 5);

    press(cx, "enter");
    assert_eq!(blocked_runs(&view, cx), [(2, false)]);
}

#[gpui::test]
fn a_new_message_from_someone_else_is_read_out_but_not_the_first_page_or_a_blocked_one(
    cx: &mut TestAppContext,
) {
    let (view, session, cx) = open_general(cx, vec![message(10_001, &ana(), "hello")], None);
    let announcement =
        |cx: &mut VisualTestContext| view.read_with(cx, |view, _| view.announcement.to_string());
    assert_eq!(announcement(cx), "", "the first page is not read out");

    let arrive = |message: Message, cx: &mut VisualTestContext| {
        session.update(cx, |session, cx| {
            let change = session
                .state_mut()
                .apply_event(mellow_protocol::GatewayEvent::MessageCreate(message), 0);
            cx.emit(StateChanged(vec![change]));
        });
        cx.run_until_parked();
    };
    arrive(message(10_002, &ana(), "are you there?"), cx);
    assert_eq!(announcement(cx), "Ana: are you there?");

    arrive(message(10_003, &me(), "yes"), cx);
    assert_eq!(
        announcement(cx),
        "Ana: are you there?",
        "the reader's own is not"
    );

    block_ana(&session, cx);
    arrive(message(10_004, &ana(), "hidden"), cx);
    assert_eq!(
        announcement(cx),
        "Ana: are you there?",
        "a blocked author is not"
    );
}

fn jump_to(view: &Entity<ConversationView>, message_id: u64, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.jump_to_message(Snowflake::from_raw(message_id), window, cx);
        });
    });
    cx.run_until_parked();
}

#[gpui::test]
fn a_jump_to_a_loaded_pin_focuses_and_lights_it_and_jump_to_present_comes_back(
    cx: &mut TestAppContext,
) {
    let (view, _session, cx) = open_general(cx, many_messages(40), None);
    jump_to(&view, 10_005, cx);
    view.read_with(cx, |view, _| {
        assert_eq!(view.focused_row_key.as_deref(), Some("10005"));
        assert_eq!(view.highlight.map(|(id, _)| id.raw()), Some(10_005));
        assert!(!view.is_at_latest());
    });
    cx.update(|_, cx| view.update(cx, ConversationView::jump_to_present));
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| view.is_at_latest()));
}

#[gpui::test]
fn a_jump_replaced_by_another_drops_its_late_answer(cx: &mut TestAppContext) {
    // A full first page: the first jump has to load older history, which fails without an
    // instance; its answer comes after the second jump started and must not reach the view.
    let (view, _session, cx) = open_general(cx, many_messages(60), None);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.jump_to_message(Snowflake::from_raw(5), window, cx);
            view.jump_to_message(Snowflake::from_raw(10_005), window, cx);
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.action_error, None,
            "the first jump's failure is dropped"
        );
        assert_eq!(view.highlight.map(|(id, _)| id.raw()), Some(10_005));
    });
}

#[gpui::test]
fn an_earlier_jumps_timer_never_puts_out_a_later_highlight(cx: &mut TestAppContext) {
    let (view, _session, cx) = open_general(cx, many_messages(40), None);
    let half = delay(MOTION_DELAY_JUMP_HIGHLIGHT) / 2;
    jump_to(&view, 10_005, cx);
    cx.executor().advance_clock(half);
    let help = OpenConversation {
        channel_id: RUST_HELP,
        label: "#help".to_owned(),
        ..general()
    };
    for place in [help, general()] {
        cx.update(|_, cx| view.update(cx, |view, cx| view.set_place(Some(place), cx)));
        cx.run_until_parked();
    }
    jump_to(&view, 10_006, cx);
    // The first jump's timer ends here; the second one's has half its time left.
    cx.executor().advance_clock(half);
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |view, _| view.highlight.map(|(id, _)| id.raw())),
        Some(10_006)
    );
}

#[gpui::test]
fn an_older_page_extending_a_folded_run_keeps_its_row_and_focus(cx: &mut TestAppContext) {
    // A full page, all from Ana: the channel has older history.
    let page: Vec<Message> = (101..=150)
        .map(|id| message(10_000 + id, &ana(), "blocked"))
        .collect();
    let (view, session, cx) = open_general(cx, page, None);
    block_ana(&session, cx);
    let run_key = view.read_with(cx, |view, _| view.entries[1].key());
    view.update(cx, |view, cx| {
        view.focused_row_key = Some(run_key.clone());
        view.list.scroll_to(ListOffset::default());
        view.visit.has_reader_scrolled = true;
        cx.notify();
    });
    cx.run_until_parked();

    let older: Vec<Message> = (91..=100)
        .map(|id| message(10_000 + id, &ana(), "older"))
        .collect();
    session.update(cx, |session, cx| {
        let change = session.state_mut().insert_history_page(RUST_GENERAL, older);
        cx.emit(StateChanged(vec![change]));
    });
    cx.run_until_parked();

    assert_eq!(blocked_runs(&view, cx), [(60, false)]);
    view.read_with(cx, |view, _| {
        assert_eq!(view.entries[1].key(), run_key, "the run keeps its key");
        assert!(
            view.focused_blocked_run().is_some(),
            "keyboard focus stays on the run"
        );
    });
}

#[gpui::test]
fn a_jump_to_a_message_before_the_channel_start_says_it_is_gone(cx: &mut TestAppContext) {
    let (view, _session, cx) = open_general(cx, many_messages(30), None);
    jump_to(&view, 5, cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.action_error.clone()),
        Some("This message was deleted, or you can no longer read it.".into())
    );
}

#[gpui::test]
fn a_jump_that_cannot_load_older_history_says_so(cx: &mut TestAppContext) {
    // A full first page: older history exists, and this session has no instance to ask.
    let (view, _session, cx) = open_general(cx, many_messages(60), None);
    jump_to(&view, 5, cx);
    assert_eq!(
        view.read_with(cx, |view, _| view.action_error.clone()),
        Some("Couldn't open the message".into())
    );
}

#[gpui::test]
fn files_dropped_over_the_messages_go_to_the_composer_tray_but_not_a_folder(
    cx: &mut TestAppContext,
) {
    let (view, _session, cx) = open_general(cx, Vec::new(), None);
    // A conversation lets the reader attach files; the sample server does not.
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.set_place(
                Some(OpenConversation {
                    channel_id: crate::shell_fixtures::CONVERSATION_ANA,
                    guild_id: None,
                    label: "Ana".to_owned(),
                    composer_label: "Message @Ana".to_owned(),
                    hero_icon: IconName::At,
                    hero_title: "Ana".to_owned(),
                    hero_sentence: String::new(),
                    topic: String::new(),
                }),
                cx,
            );
        });
    });
    cx.run_until_parked();
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("notes.txt");
    std::fs::write(&file, b"notes").unwrap();
    // Over the messages, well above the composer.
    let position = gpui::point(px(450.0), px(100.0));
    cx.simulate_event(gpui::FileDropEvent::Entered {
        position,
        paths: gpui::ExternalPaths(vec![file, directory.path().to_path_buf()].into()),
    });
    cx.run_until_parked();
    cx.simulate_event(gpui::FileDropEvent::Submit { position });
    cx.run_until_parked();
    let tray_len = view.read_with(cx, |view, cx| view.composer().read(cx).tray_len());
    assert_eq!(tray_len, 1, "the file, not the folder");
}

#[gpui::test]
fn the_menu_opens_who_reacted_in_a_dialog_that_esc_closes(cx: &mut TestAppContext) {
    let mut liked = message(10_001, &ana(), "hi");
    liked.reactions = vec![mellow_protocol::ReactionSummary {
        emoji: "👍".into(),
        count: 2,
        me: false,
    }];
    let (view, _session, cx) = open_general(cx, vec![liked], None);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.run_menu_action(
                Snowflake::from_raw(10_001),
                MessageMenuAction::ViewReactions,
                window,
                cx,
            );
        });
    });
    cx.run_until_parked();
    assert!(view.read_with(cx, |view, _| view.confirmation.is_some()));
    press(cx, "escape");
    assert!(view.read_with(cx, |view, _| view.confirmation.is_none()));
}

#[gpui::test]
fn a_conversation_with_someone_blocked_locks_the_composer_until_unblocked(cx: &mut TestAppContext) {
    let (view, session, cx) = open_general(cx, Vec::new(), None);
    block_ana(&session, cx);
    let conversation_with_ana = OpenConversation {
        channel_id: crate::shell_fixtures::CONVERSATION_ANA,
        guild_id: None,
        label: "Ana".to_owned(),
        composer_label: "Message @Ana".to_owned(),
        hero_icon: IconName::ChatCircle,
        hero_title: "Ana".to_owned(),
        hero_sentence: String::new(),
        topic: String::new(),
    };
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.set_place(Some(conversation_with_ana), cx);
        });
    });
    cx.run_until_parked();
    let locking = |view: &Entity<ConversationView>, cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let composer = view.read(cx).composer().clone();
            composer
                .read(cx)
                .locking_recipient()
                .map(|recipient| (recipient.user_id, recipient.name.to_string()))
        })
    };
    assert_eq!(locking(&view, cx), Some((ANA, "Ana".to_owned())));

    // Unblocked (here or in another session): the field comes back.
    session.update(cx, |session, cx| {
        session.state_mut().forget_relationship(ANA);
        cx.emit(StateChanged(vec![StateChange::Relationships]));
    });
    cx.run_until_parked();
    assert_eq!(locking(&view, cx), None);
}

#[gpui::test]
fn shift_f10_or_the_menu_key_on_a_focused_message_opens_its_menu(cx: &mut TestAppContext) {
    let (view, _session, cx) = open_general(cx, vec![message(10_001, &ana(), "hi")], None);
    for key in ["shift-f10", "menu"] {
        focus_list(&view, cx);
        press(cx, "up");
        press(cx, key);
        assert!(
            view.read_with(cx, |view, _| view.menu.is_some()),
            "{key} opens the message's menu"
        );
        press(cx, "escape");
        assert!(view.read_with(cx, |view, _| view.menu.is_none()));
    }
}
