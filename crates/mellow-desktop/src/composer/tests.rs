//! Headless behaviour of the composer: keys, sending, editing, suggestions and drafts.

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, TestAppContext, VisualTestContext, Window, WindowAppearance, div,
};
use std::path::PathBuf;

use mellow_protocol::{PublicUser, Snowflake};

use super::{
    Cancelled, ComposedMessage, Composer, ComposerEvent, MAXIMUM_ATTACHMENTS_PER_MESSAGE,
    ReplyTarget, TrayItemStatus, bind_composer_keys, read_file_within,
};
use crate::components::bind_tab_navigation;
use crate::theme::ActiveTheme;

struct Harness {
    composer: Entity<Composer>,
    events: Vec<ComposerEvent>,
    _subscription: Subscription,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_end()
            .child(self.composer.clone())
    }
}

fn channel(id: u64) -> Snowflake {
    Snowflake::from_raw(id)
}

fn person(id: u64, username: &str) -> PublicUser {
    PublicUser {
        id: Snowflake::from_raw(id),
        username: username.to_owned(),
        display_name: String::new(),
        avatar: None,
        banner: None,
    }
}

/// A composer open on channel 1, focused, as the chat pane leaves it.
fn open_composer(cx: &mut TestAppContext) -> (Entity<Harness>, &mut VisualTestContext) {
    cx.update(|cx| {
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
        gpui_component::init(cx);
        bind_tab_navigation(cx);
        bind_composer_keys(cx);
    });
    let (harness, cx) = cx.add_window_view(|window, cx| {
        let composer = cx.new(|cx| Composer::new(window, cx));
        let subscription = cx.subscribe(&composer, |harness: &mut Harness, _, event, _| {
            harness.events.push(event.clone());
        });
        Harness {
            composer,
            events: Vec::new(),
            _subscription: subscription,
        }
    });
    let composer = composer_of(&harness, cx);
    cx.update(|window, cx| {
        window.activate_window();
        composer.update(cx, |composer, cx| {
            composer.set_channel(Some(channel(1)), "Message #general".into(), window, cx);
            composer.focus(window, cx);
        });
    });
    cx.run_until_parked();
    (harness, cx)
}

fn composer_of(harness: &Entity<Harness>, cx: &mut VisualTestContext) -> Entity<Composer> {
    harness.read_with(cx, |harness, _| harness.composer.clone())
}

fn text(harness: &Entity<Harness>, cx: &mut VisualTestContext) -> String {
    let composer = composer_of(harness, cx);
    composer.read_with(cx, Composer::text)
}

fn take_events(harness: &Entity<Harness>, cx: &mut VisualTestContext) -> Vec<ComposerEvent> {
    harness.update(cx, |harness, _| std::mem::take(&mut harness.events))
}

fn without_typing(events: Vec<ComposerEvent>) -> Vec<ComposerEvent> {
    events
        .into_iter()
        .filter(|event| !matches!(event, ComposerEvent::Typing { .. }))
        .collect()
}

#[gpui::test]
fn an_empty_composer_is_as_tall_as_the_user_card(cx: &mut TestAppContext) {
    // ADR-0022 §4: the field's line is the text input's, so padding it to the message type's
    // line height made the box taller than the dock.
    let (_harness, cx) = open_composer(cx);
    let composer = cx
        .debug_bounds("composer")
        .expect("the composer's box is drawn");
    assert_eq!(
        composer.size.height,
        gpui::px(crate::theme::tokens::SIZE_DOCK_HEIGHT)
    );
}

#[gpui::test]
fn enter_sends_the_trimmed_text_and_shift_enter_writes_a_new_line(cx: &mut TestAppContext) {
    let (harness, cx) = open_composer(cx);
    cx.simulate_input("hi");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("there");
    assert_eq!(text(&harness, cx), "hi\nthere");

    cx.simulate_keystrokes("enter");

    assert_eq!(
        without_typing(take_events(&harness, cx)),
        [ComposerEvent::Send(ComposedMessage {
            channel_id: channel(1),
            text: "hi\nthere".into(),
            reply_to: None,
            attachments: Vec::new(),
        })]
    );
    assert_eq!(text(&harness, cx), "");
    // An empty field sends nothing.
    cx.simulate_keystrokes("enter");
    assert_eq!(without_typing(take_events(&harness, cx)), []);
}

#[gpui::test]
fn typing_notices_are_throttled(cx: &mut TestAppContext) {
    let (harness, cx) = open_composer(cx);
    cx.simulate_input("hello");
    let typing: Vec<_> = take_events(&harness, cx)
        .into_iter()
        .filter(|event| matches!(event, ComposerEvent::Typing { .. }))
        .collect();
    assert_eq!(
        typing,
        [ComposerEvent::Typing {
            channel_id: channel(1)
        }]
    );
}

#[gpui::test]
fn up_in_an_empty_field_asks_to_edit_the_last_message_and_moves_the_caret_otherwise(
    cx: &mut TestAppContext,
) {
    let (harness, cx) = open_composer(cx);
    cx.simulate_keystrokes("up");
    assert_eq!(take_events(&harness, cx), [ComposerEvent::EditLast]);

    cx.simulate_input("a");
    take_events(&harness, cx);
    cx.simulate_keystrokes("up");
    assert_eq!(take_events(&harness, cx), []);
}

#[gpui::test]
fn the_reply_goes_with_the_message_and_esc_closes_it(cx: &mut TestAppContext) {
    let (harness, cx) = open_composer(cx);
    let composer = composer_of(&harness, cx);
    let reply = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            composer.update(cx, |composer, cx| {
                composer.set_reply_target(
                    Some(ReplyTarget {
                        message_id: Snowflake::from_raw(77),
                        author_name: "Ana".into(),
                    }),
                    window,
                    cx,
                );
            });
        });
    };
    reply(cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(
        take_events(&harness, cx),
        [ComposerEvent::Cancel(Cancelled::Reply)]
    );
    cx.simulate_keystrokes("escape");
    assert_eq!(
        take_events(&harness, cx),
        [ComposerEvent::Cancel(Cancelled::Nothing)]
    );

    reply(cx);
    cx.simulate_input("yes");
    cx.simulate_keystrokes("enter");
    let sent = without_typing(take_events(&harness, cx));
    assert!(
        matches!(&sent[..], [ComposerEvent::Send(message)] if message.reply_to == Some(Snowflake::from_raw(77))),
        "{sent:?}"
    );
}

#[gpui::test]
fn enter_inserts_the_chosen_emoji_instead_of_sending(cx: &mut TestAppContext) {
    let (harness, cx) = open_composer(cx);
    cx.simulate_input(":thumbs");
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&harness, cx), "👍 ");
    assert_eq!(without_typing(take_events(&harness, cx)), []);
}

#[gpui::test]
fn tab_inserts_a_mention_and_esc_hides_the_list_until_the_word_changes(cx: &mut TestAppContext) {
    let (harness, cx) = open_composer(cx);
    let composer = composer_of(&harness, cx);
    cx.update(|_, cx| {
        composer.update(cx, |composer, cx| {
            composer.set_mention_candidates(
                vec![person(1, "paul"), person(2, "pauline")],
                Some(Snowflake::from_raw(1)),
                cx,
            );
        });
    });
    cx.simulate_input("@pau");
    cx.simulate_keystrokes("escape");
    // The list was there to hide: Esc meant nothing else.
    assert_eq!(without_typing(take_events(&harness, cx)), []);
    assert!(composer.read_with(cx, |composer, _| composer.suggestions.visible().is_empty()));

    cx.simulate_input(" @pau");
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&harness, cx), "@pau @pauline ");
}

#[gpui::test]
fn an_edit_is_saved_with_enter_cancelled_with_esc_and_the_draft_comes_back(
    cx: &mut TestAppContext,
) {
    let (harness, cx) = open_composer(cx);
    let composer = composer_of(&harness, cx);
    let message_id = Snowflake::from_raw(42);
    cx.simulate_input("draft");
    let edit = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            composer.update(cx, |composer, cx| {
                composer.start_editing(message_id, "helo", window, cx);
            });
        });
    };

    edit(cx);
    assert_eq!(text(&harness, cx), "helo");
    cx.simulate_keystrokes("escape");
    assert_eq!(
        without_typing(take_events(&harness, cx)),
        [ComposerEvent::Cancel(Cancelled::Editing { message_id })]
    );
    assert_eq!(text(&harness, cx), "draft");

    edit(cx);
    cx.simulate_input("x");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        take_events(&harness, cx),
        [ComposerEvent::Edit {
            message_id,
            text: "helox".into()
        }]
    );
    assert_eq!(text(&harness, cx), "draft");
}

#[gpui::test]
fn each_channel_keeps_its_own_draft(cx: &mut TestAppContext) {
    let (harness, cx) = open_composer(cx);
    let composer = composer_of(&harness, cx);
    let open = |id: u64, cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            composer.update(cx, |composer, cx| {
                composer.set_channel(Some(channel(id)), "Message".into(), window, cx);
            });
        });
    };
    cx.simulate_input("first");
    open(2, cx);
    assert_eq!(text(&harness, cx), "");
    open(1, cx);
    assert_eq!(text(&harness, cx), "first");
}

#[gpui::test]
fn a_message_over_the_limit_is_not_sent(cx: &mut TestAppContext) {
    let (harness, cx) = open_composer(cx);
    let composer = composer_of(&harness, cx);
    cx.update(|window, cx| {
        composer.update(cx, |composer, cx| {
            composer.set_text("a".repeat(4_001), window, cx);
        });
    });
    cx.simulate_keystrokes("enter");
    assert_eq!(without_typing(take_events(&harness, cx)), []);
    assert!(composer.read_with(cx, |composer, _| composer.has_tried_sending_too_long));
}

#[gpui::test]
fn an_unchanged_edit_closes_without_saving_even_with_spaces_added(cx: &mut TestAppContext) {
    let (harness, cx) = open_composer(cx);
    let composer = composer_of(&harness, cx);
    let message_id = Snowflake::from_raw(42);
    cx.update(|window, cx| {
        composer.update(cx, |composer, cx| {
            composer.start_editing(message_id, "hello", window, cx);
        });
    });
    cx.simulate_input("  ");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        take_events(&harness, cx),
        [ComposerEvent::Cancel(Cancelled::Editing { message_id })]
    );
}

#[gpui::test]
fn a_refused_edit_waits_instead_of_taking_over_a_message_being_written(cx: &mut TestAppContext) {
    let (harness, cx) = open_composer(cx);
    let composer = composer_of(&harness, cx);
    let message_id = Snowflake::from_raw(42);
    cx.simulate_input("next message");
    cx.update(|window, cx| {
        composer.update(cx, |composer, cx| {
            composer.give_back_refused_edit(message_id, "edited".into(), true, window, cx);
        });
    });
    assert_eq!(text(&harness, cx), "next message");
    assert!(composer.read_with(cx, |composer, _| composer.edit_session.is_none()));

    // The typed edit comes back with the message's next edit.
    cx.update(|window, cx| {
        composer.update(cx, |composer, cx| {
            composer.start_editing(message_id, "original", window, cx);
        });
    });
    assert_eq!(text(&harness, cx), "edited");
}

fn temporary_file(directory: &tempfile::TempDir, name: &str, byte_count: usize) -> PathBuf {
    let path = directory.path().join(name);
    std::fs::write(&path, vec![b'x'; byte_count]).unwrap();
    path
}

fn tray_statuses(composer: &Entity<Composer>, cx: &mut VisualTestContext) -> Vec<TrayItemStatus> {
    composer.read_with(cx, |composer, _| {
        composer
            .drafts
            .tray()
            .iter()
            .map(|item| item.status.clone())
            .collect()
    })
}

fn attach(composer: &Entity<Composer>, paths: Vec<PathBuf>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        composer.update(cx, |composer, cx| composer.attach_files(paths, window, cx));
    });
}

#[gpui::test]
fn a_file_over_the_limit_fails_before_upload_and_retry_refuses_it_again(cx: &mut TestAppContext) {
    let (harness, cx) = open_composer(cx);
    let composer = composer_of(&harness, cx);
    let directory = tempfile::tempdir().unwrap();
    cx.update(|_, cx| composer.update(cx, |composer, cx| composer.set_uploads(None, Some(4), cx)));

    attach(
        &composer,
        vec![temporary_file(&directory, "big.bin", 5)],
        cx,
    );
    cx.run_until_parked();
    let statuses = tray_statuses(&composer, cx);
    assert!(
        matches!(&statuses[..], [TrayItemStatus::Failed(sentence)] if sentence.contains("big.bin") && sentence.contains("5 bytes, more than")),
        "{statuses:?}"
    );

    let key = composer.read_with(cx, |composer, _| composer.drafts.tray()[0].key);
    cx.update(|_, cx| composer.update(cx, |composer, cx| composer.retry_upload(key, cx)));
    cx.run_until_parked();
    assert_eq!(tray_statuses(&composer, cx), statuses);

    // A failed file is never sent and does not block the text.
    cx.simulate_input("hi");
    cx.simulate_keystrokes("enter");
    let sent = without_typing(take_events(&harness, cx));
    assert!(
        matches!(&sent[..], [ComposerEvent::Send(message)] if message.attachments.is_empty()),
        "{sent:?}"
    );
}

#[gpui::test]
fn send_waits_for_uploads_and_removing_a_file_stops_its_upload(cx: &mut TestAppContext) {
    let (harness, cx) = open_composer(cx);
    let composer = composer_of(&harness, cx);
    let directory = tempfile::tempdir().unwrap();
    cx.update(|_, cx| {
        composer.update(cx, |composer, cx| composer.set_uploads(None, Some(100), cx));
    });
    attach(&composer, vec![temporary_file(&directory, "a.txt", 1)], cx);
    cx.run_until_parked();
    let key = composer.read_with(cx, |composer, _| composer.drafts.tray()[0].key);
    // Holds the file as uploading, as a slow upload would.
    cx.update(|_, cx| {
        composer.update(cx, |composer, _| {
            composer.drafts.restart_upload(key);
        });
    });
    cx.simulate_input("hi");
    cx.simulate_keystrokes("enter");
    assert_eq!(without_typing(take_events(&harness, cx)), []);
    assert!(composer.read_with(cx, |composer, _| composer.notice
        == Some(super::ComposerNotice::UploadsRunning)));

    cx.update(|window, cx| {
        composer.update(cx, |composer, cx| {
            composer.remove_tray_item(key, window, cx);
        });
    });
    assert_eq!(tray_statuses(&composer, cx), []);
    cx.simulate_keystrokes("enter");
    assert!(matches!(
        &without_typing(take_events(&harness, cx))[..],
        [ComposerEvent::Send(_)]
    ));
}

#[gpui::test]
fn the_tray_never_holds_more_than_ten_files_even_from_two_quick_drops(cx: &mut TestAppContext) {
    let (harness, cx) = open_composer(cx);
    let composer = composer_of(&harness, cx);
    let directory = tempfile::tempdir().unwrap();
    let paths: Vec<PathBuf> = (0..6)
        .map(|index| temporary_file(&directory, &format!("{index}.txt"), 1))
        .collect();

    attach(&composer, paths.clone(), cx);
    attach(&composer, paths, cx);
    cx.run_until_parked();

    assert_eq!(
        tray_statuses(&composer, cx).len(),
        MAXIMUM_ATTACHMENTS_PER_MESSAGE
    );
    assert!(composer.read_with(cx, |composer, _| {
        composer.notice == Some(super::ComposerNotice::TooManyAttachments)
    }));
}

#[gpui::test]
fn files_chosen_for_a_channel_never_land_in_the_next_one(cx: &mut TestAppContext) {
    let (harness, cx) = open_composer(cx);
    let composer = composer_of(&harness, cx);
    let directory = tempfile::tempdir().unwrap();
    attach(&composer, vec![temporary_file(&directory, "a.txt", 1)], cx);
    cx.update(|window, cx| {
        composer.update(cx, |composer, cx| {
            composer.set_channel(Some(channel(2)), "Message".into(), window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(tray_statuses(&composer, cx), []);
}

#[test]
fn reads_a_file_only_up_to_the_limit() {
    let directory = tempfile::tempdir().unwrap();
    let path = temporary_file(&directory, "five.bin", 5);
    assert_eq!(read_file_within(&path, 5).unwrap(), Some(vec![b'x'; 5]));
    assert_eq!(read_file_within(&path, 4).unwrap(), None);
}
