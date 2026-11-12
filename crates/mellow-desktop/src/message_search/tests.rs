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
use mellow_protocol::{Message, MessageSearchHit, MessageSearchResults};

use super::*;
use crate::shell_fixtures::{ANA, GUILD_RUST, RUST_GENERAL, person, sample_state};
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

/// Draws the results as they dock beside the conversation.
struct PanelHost {
    search: Entity<MessageSearchView>,
}

impl Render for PanelHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel: AnyElement = self.search.update(cx, |search, cx| {
            search.render_panel(true, Some(PANEL_WIDTH), window, cx)
        });
        div().size_full().child(panel)
    }
}

const PANEL_WIDTH: f32 = 400.0;

fn open_results(cx: &mut TestAppContext) -> (Entity<MessageSearchView>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(ActiveStrings(&ENGLISH));
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| *session.state_mut() = sample_state());
    let (host, cx) = cx.add_window_view(|window, cx| {
        let search = cx.new(|cx| MessageSearchView::new(session, window, cx));
        PanelHost { search }
    });
    let search = host.read_with(cx, |host, _| host.search.clone());
    search.update_in(cx, |search, window, cx| {
        search.set_scope(Some(GUILD_RUST), window, cx);
    });
    cx.run_until_parked();
    (search, cx)
}

fn hit(id: u64) -> MessageSearchHit {
    MessageSearchHit {
        message: Message {
            id: Snowflake::from_raw(id << 22),
            channel_id: RUST_GENERAL,
            author: person(ANA, "ana", "Ana"),
            content: "pizza tonight".to_owned(),
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
        },
        highlights: Vec::new(),
    }
}

/// The first page of "pizza" arrives with `shown` results of `total`, as the instance answers.
fn show_first_page(
    search: &Entity<MessageSearchView>,
    shown: u64,
    total: u32,
    cx: &mut VisualTestContext,
) {
    search.update(cx, |view, cx| {
        let Some(request) = view.search.search("pizza".into()) else {
            panic!("a new search sends a request");
        };
        cx.notify();
        let results = MessageSearchResults {
            hits: (1..=shown).map(hit).collect(),
            total_count: total,
            total_count_is_lower_bound: false,
            channel_facets: Vec::new(),
            author_facets: Vec::new(),
        };
        assert_eq!(view.search.finish(&request, Ok(results)), None);
    });
    cx.run_until_parked();
}

/// Whether a second page was asked for (it fails here: there is no instance).
fn asked_for_more(search: &Entity<MessageSearchView>, cx: &mut VisualTestContext) -> bool {
    search.read_with(cx, |view, _| {
        view.search().is_loading_more() || view.search().load_more_error().is_some()
    })
}

#[gpui::test]
fn a_page_that_fills_the_panel_asks_for_no_more_until_its_end_comes_near(cx: &mut TestAppContext) {
    let (search, cx) = open_results(cx);
    show_first_page(&search, 50, 200, cx);
    assert_eq!(status(&search, cx), MessageSearchStatus::Loaded);
    // Measured once laid out, not against the skeleton drawn before.
    assert!(!asked_for_more(&search, cx));
}

#[gpui::test]
fn a_page_that_ends_in_view_asks_for_the_next_at_once(cx: &mut TestAppContext) {
    let (search, cx) = open_results(cx);
    show_first_page(&search, 2, 200, cx);
    assert!(asked_for_more(&search, cx));
}

#[gpui::test]
fn jump_opens_its_result_once(cx: &mut TestAppContext) {
    let (search, cx) = open_results(cx);
    let opened = Rc::new(std::cell::Cell::new(0_u32));
    let _subscription = cx.update({
        let opened = Rc::clone(&opened);
        |_, cx| {
            cx.subscribe(&search, move |_, event: &MessageSearchEvent, _| {
                if matches!(event, MessageSearchEvent::OpenResult { .. }) {
                    opened.set(opened.get() + 1);
                }
            })
        }
    });
    show_first_page(&search, 1, 1, cx);
    let jump = cx
        .debug_bounds("search-jump")
        .unwrap_or_else(|| panic!("the result's Jump button is drawn"));
    cx.simulate_mouse_move(jump.center(), None, gpui::Modifiers::default());
    cx.simulate_click(jump.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(opened.get(), 1);
}
