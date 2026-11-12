//! Message search (DESIGN.md 5.12, `MessageSearchField.svelte`, `MessageSearchPanel.svelte`): one
//! search per server, or across the conversations at Home. The field sits in the chat header where
//! the results dock beside the conversation, at the top of the results otherwise; the results take
//! the side sheet's place, and the members come back when they close.
//!
//! The search itself is client-core's [`MessageSearch`]: this view sends the requests it hands
//! back, one at a time, owns the typing pause and the field, and draws both parts. The shell
//! places them ([`MessageSearchView::render_field`], [`MessageSearchView::render_panel`]) and
//! opens the results it is asked to ([`MessageSearchEvent`]).

mod field;
mod panel;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, Focusable as _, ScrollHandle, SharedString,
    Subscription, Task, Window,
};
use gpui_component::input::{InputEvent, InputState, Position};
use mellow_client_core::shared_logic::message_search::{
    AfterTyping, MessageSearch, MessageSearchStatus, SEARCH_TYPING_PAUSE_MILLISECONDS,
    SearchRequest,
};
use mellow_client_core::shared_logic::search_query::{SearchCaretContext, is_typing_filter_value};
use mellow_protocol::{MessageSearchSort, Snowflake};

use crate::i18n::strings;
use crate::messages::MessageBody;
use crate::session::Session;
use crate::theme::tokens::COMPONENT_MESSAGE_HISTORY_LOAD_DISTANCE;

pub use self::field::FieldPlacement;

/// What the shell does for the search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageSearchEvent {
    /// A result was chosen: its channel opens at the message.
    OpenResult {
        channel_id: Snowflake,
        message_id: Snowflake,
    },
    /// "Go to a channel or person" at the end of the suggestions.
    OpenQuickSwitcher,
    /// The close button ended the search: focus goes back to the conversation.
    Closed,
}

pub struct MessageSearchView {
    session: Entity<Session>,
    search: MessageSearch,
    input: Entity<InputState>,
    /// Esc hid the suggestions for this text and caret; typing or moving brings them back.
    dismissed_at: Option<(String, usize)>,
    /// The placeholder last given to the field, which depends on its place, focus and scope.
    shown_placeholder: SharedString,
    /// The suggestion the arrows point at; `None` until they move (a value's first is active).
    active_index: Option<usize>,
    /// The caret context the suggestions were last listed for, to start a new list at its top.
    shown_context: Option<SearchCaretContext>,
    typing_pause: Option<Task<()>>,
    /// Why the last result chosen could not open, said in the results.
    notice: Option<SharedString>,
    /// Results by someone the reader blocked that they chose to show (ADR-0025).
    revealed_blocked_ids: HashSet<Snowflake>,
    /// Each result's text with its matched words, parsed once per search.
    bodies: HashMap<Snowflake, Rc<MessageBody>>,
    /// The query and sort the results were scrolled for: a new search starts at the top.
    scrolled_for: (String, MessageSearchSort),
    /// The results' count and state last drawn: a new page is measured once it is laid out.
    drawn_page: (usize, MessageSearchStatus),
    results_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for MessageSearchView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MessageSearchView")
            .field("status", &self.search.status())
            .field("is_panel_open", &self.search.is_panel_open)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<MessageSearchEvent> for MessageSearchView {}

impl MessageSearchView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = strings(cx).search().placeholder();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subscriptions = vec![cx.subscribe(&input, |view, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let (text, caret) = {
                    let input = input.read(cx);
                    (input.value().to_string(), input.cursor())
                };
                view.note_typed(text, caret, cx);
            }
            cx.notify();
        })];
        Self {
            session,
            search: MessageSearch::new(),
            input,
            shown_placeholder: SharedString::from(placeholder),
            dismissed_at: None,
            active_index: None,
            shown_context: None,
            typing_pause: None,
            notice: None,
            revealed_blocked_ids: HashSet::new(),
            bodies: HashMap::new(),
            scrolled_for: (String::new(), MessageSearchSort::Relevance),
            drawn_page: (0, MessageSearchStatus::Idle),
            results_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    #[must_use]
    pub fn search(&self) -> &MessageSearch {
        &self.search
    }

    #[must_use]
    pub fn is_panel_open(&self) -> bool {
        self.search.is_panel_open
    }

    /// The results give the side sheet back (the members, Esc, a narrow window's result opened);
    /// they stay for the next opening.
    pub fn hide_panel(&mut self, cx: &mut Context<Self>) {
        if self.search.is_panel_open {
            self.search.is_panel_open = false;
            cx.notify();
        }
    }

    /// Opens the results, empty or as they were (the narrow window's magnifier, Ctrl+F there).
    pub fn show_panel(&mut self, cx: &mut Context<Self>) {
        if !self.search.is_panel_open {
            self.search.is_panel_open = true;
            cx.notify();
        }
    }

    /// Another server, or Home: the search starts empty.
    pub fn set_scope(
        &mut self,
        guild_id: Option<Snowflake>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if guild_id != self.search.scope_guild_id() {
            self.search.set_scope(guild_id);
            self.after_cleared(window, cx);
        }
    }

    /// Ctrl+F: the field takes focus with its text selected.
    pub fn focus_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
    }

    #[must_use]
    pub fn is_field_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.input.read(cx).focus_handle(cx).is_focused(window)
    }

    /// The close button: the search ends and its field empties.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.clear();
        self.after_cleared(window, cx);
        cx.emit(MessageSearchEvent::Closed);
    }

    /// Why the result chosen could not open (its conversation left the list, its channel was
    /// hidden since): said in the results, which stay open.
    pub fn show_notice(&mut self, notice: SharedString, cx: &mut Context<Self>) {
        self.notice = Some(notice);
        cx.notify();
    }

    fn after_cleared(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.typing_pause = None;
        self.notice = None;
        self.dismissed_at = None;
        self.revealed_blocked_ids.clear();
        self.bodies.clear();
        self.set_field_text("", None, window, cx);
        cx.notify();
    }

    /// The field's text changed by typing: searched after a pause, unless a filter's value is
    /// still being typed.
    fn note_typed(&mut self, text: String, caret: usize, cx: &mut Context<Self>) {
        self.dismissed_at = None;
        self.typing_pause = None;
        let is_typing_value = is_typing_filter_value(&text, caret);
        let was_searching = !self.search.query().is_empty();
        if self.search.note_typed(text, is_typing_value) == AfterTyping::SearchAfterPause {
            self.typing_pause = Some(cx.spawn(async move |view, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(SEARCH_TYPING_PAUSE_MILLISECONDS))
                    .await;
                let _ = view.update(cx, |view, cx| {
                    view.typing_pause = None;
                    let request = view.search.search_draft();
                    view.after_search(request, cx);
                });
            }));
        } else if was_searching && self.search.query().is_empty() {
            self.forget_results();
        }
    }

    /// Searches `query` now (Enter, a suggestion, a facet, a chip removed).
    pub(crate) fn search_now(&mut self, query: String, cx: &mut Context<Self>) {
        self.typing_pause = None;
        let request = self.search.search(query);
        self.after_search(request, cx);
    }

    fn after_search(&mut self, request: Option<SearchRequest>, cx: &mut Context<Self>) {
        // A new search, or none, says nothing about an older result.
        self.notice = None;
        self.send(request, cx);
        cx.notify();
    }

    fn set_sort(&mut self, sort: MessageSearchSort, cx: &mut Context<Self>) {
        let request = self.search.set_sort(sort);
        self.after_search(request, cx);
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        let request = self.search.load_more();
        self.send(request, cx);
        cx.notify();
    }

    /// The next page, once the end of the laid-out list is near (or in view).
    fn load_more_when_near_end(&mut self, cx: &mut Context<Self>) {
        if !self.search.can_load_more() || self.search.is_loading_more() {
            return;
        }
        // The handle describes the last frame laid out; the offset is negative going down.
        let distance_to_end =
            f32::from(self.results_scroll.max_offset().y + self.results_scroll.offset().y);
        if distance_to_end <= COMPONENT_MESSAGE_HISTORY_LOAD_DISTANCE {
            self.load_more(cx);
        }
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        let request = self.search.retry();
        self.send(request, cx);
        cx.notify();
    }

    fn forget_results(&mut self) {
        self.bodies.clear();
        self.revealed_blocked_ids.clear();
    }

    /// Sends `request` (client-core keeps one in flight); its answer goes back to client-core,
    /// which drops it when the search moved on and hands back the request that waited.
    fn send(&mut self, request: Option<SearchRequest>, cx: &mut Context<Self>) {
        let Some(request) = request else {
            return;
        };
        if request.offset == 0 {
            self.forget_results();
        }
        let view = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            let sent = request.clone();
            session.search_messages(&request, cx, move |answer, cx| {
                let _ = view.update(cx, |view, cx| {
                    let next = view.search.finish(&sent, answer);
                    view.send(next, cx);
                    cx.notify();
                });
            });
        });
    }

    /// Puts `text` in the field, the caret at byte `caret` (or the end), with focus.
    fn set_field_text(
        &mut self,
        text: &str,
        caret: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let characters = text
            .get(..caret.unwrap_or(text.len()))
            .unwrap_or(text)
            .chars()
            .count();
        let text = text.to_owned();
        self.input.update(cx, |input, cx| {
            // `set_value` sends no `Change`: this is not typing.
            input.set_value(text, window, cx);
            if caret.is_some() {
                input.set_cursor_position(
                    Position::new(0, u32::try_from(characters).unwrap_or(u32::MAX)),
                    window,
                    cx,
                );
            }
        });
    }

    /// A result was chosen: the shell opens it.
    pub(crate) fn open_result(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        cx: &mut Context<Self>,
    ) {
        self.notice = None;
        cx.emit(MessageSearchEvent::OpenResult {
            channel_id,
            message_id,
        });
    }
}
