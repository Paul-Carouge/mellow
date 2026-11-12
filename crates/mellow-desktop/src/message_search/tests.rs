//! The search field and its state headless, over a session without an instance: every request
//! fails as "not signed in", which shows whether and when one went out.

use std::time::Duration;

use gpui::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, VisualTestContext, Window, WindowAppearance, div,
};
use mellow_client_core::shared_logic::message_search::{
    MessageSearchStatus, SEARCH_TYPING_PAUSE_MILLISECONDS,
};

use super::*;
use crate::shell_fixtures::{GUILD_RUST, sample_state};
use crate::strings_pending_i18n::{ActiveStrings, ENGLISH};
use crate::theme::ActiveTheme;

/// Draws the field as the chat header does.
struct FieldHost {
    search: Entity<MessageSearchView>,
}

impl Render for FieldHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let field: AnyElement = self.search.update(cx, |search, cx| {
            search.render_field(FieldPlacement::Header, window, cx)
        });
        div().size_full().child(field)
    }
}

fn open_search(cx: &mut TestAppContext) -> (Entity<MessageSearchView>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(ActiveStrings(&ENGLISH));
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| *session.state_mut() = sample_state());
    let (host, cx) = cx.add_window_view(|window, cx| {
        let search = cx.new(|cx| MessageSearchView::new(session, window, cx));
        FieldHost { search }
    });
    let search = host.read_with(cx, |host, _| host.search.clone());
    search.update_in(cx, |search, window, cx| {
        search.set_scope(Some(GUILD_RUST), window, cx);
        search.focus_field(window, cx);
    });
    cx.run_until_parked();
    (search, cx)
}

fn status(search: &Entity<MessageSearchView>, cx: &mut VisualTestContext) -> MessageSearchStatus {
    search.read_with(cx, |search, _| search.search().status())
}

fn field_text(search: &Entity<MessageSearchView>, cx: &mut VisualTestContext) -> String {
    search.read_with(cx, |search, cx| search.input.read(cx).value().to_string())
}

#[gpui::test]
fn typing_searches_after_a_pause_never_while_a_filters_value_is_typed(cx: &mut TestAppContext) {
    let (search, cx) = open_search(cx);
    search.update(cx, |search, cx| search.note_typed("pizza".into(), 5, cx));
    cx.executor()
        .advance_clock(Duration::from_millis(SEARCH_TYPING_PAUSE_MILLISECONDS - 1));
    cx.run_until_parked();
    assert_eq!(status(&search, cx), MessageSearchStatus::Idle);
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    // The request went out (and failed: no instance here); the results opened.
    assert_eq!(status(&search, cx), MessageSearchStatus::Failed);
    assert!(search.read_with(cx, |search, _| search.is_panel_open()));

    search.update(cx, |search, cx| {
        search.note_typed("pizza from:a".into(), 12, cx);
    });
    cx.executor()
        .advance_clock(Duration::from_millis(SEARCH_TYPING_PAUSE_MILLISECONDS * 2));
    cx.run_until_parked();
    assert_eq!(
        search.read_with(cx, |search, _| search.search().query().to_owned()),
        "pizza"
    );
}

#[gpui::test]
fn enter_searches_at_once_and_esc_empties_the_headers_field(cx: &mut TestAppContext) {
    let (search, cx) = open_search(cx);
    search.update_in(cx, |search, window, cx| {
        search.set_field_text("pizza", None, window, cx);
        search.note_typed("pizza".into(), 5, cx);
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(status(&search, cx), MessageSearchStatus::Failed);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(field_text(&search, cx), "");
    assert_eq!(status(&search, cx), MessageSearchStatus::Idle);
    assert!(!search.read_with(cx, |search, _| search.is_panel_open()));
}

#[gpui::test]
fn a_chosen_filter_goes_in_at_the_caret_and_its_values_follow(cx: &mut TestAppContext) {
    let (search, cx) = open_search(cx);
    // An empty field lists every filter, "from:" first, then "Go to".
    search.update_in(cx, |search, window, cx| search.choose(0, window, cx));
    cx.run_until_parked();
    assert_eq!(field_text(&search, cx), "from:");
    // A filter's value is still being typed: nothing is searched.
    assert_eq!(status(&search, cx), MessageSearchStatus::Idle);
    // Its people are listed, the first one active: Enter picks it and searches.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let text = field_text(&search, cx);
    assert!(
        text.starts_with("from:") && text.len() > "from:".len(),
        "{text}"
    );
    assert_eq!(status(&search, cx), MessageSearchStatus::Failed);
}

#[gpui::test]
fn esc_hides_the_suggestions_before_it_empties_anything(cx: &mut TestAppContext) {
    let (search, cx) = open_search(cx);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(search.read_with(cx, |search, _| search.dismissed_at.is_some()));
    // Typing brings the list back.
    search.update(cx, |search, cx| search.note_typed("p".into(), 1, cx));
    assert!(search.read_with(cx, |search, _| search.dismissed_at.is_none()));
}

#[gpui::test]
fn another_server_starts_with_an_empty_search(cx: &mut TestAppContext) {
    let (search, cx) = open_search(cx);
    search.update_in(cx, |search, window, cx| {
        search.set_field_text("pizza", None, window, cx);
        search.search_now("pizza".into(), cx);
    });
    cx.run_until_parked();
    search.update_in(cx, |search, window, cx| search.set_scope(None, window, cx));
    cx.run_until_parked();
    assert_eq!(field_text(&search, cx), "");
    assert_eq!(status(&search, cx), MessageSearchStatus::Idle);
    assert!(!search.read_with(cx, |search, _| search.is_panel_open()));
}
