//! The state of one message search (`web/src/components/message-search.svelte.ts`, DESIGN.md
//! 5.12): the text in the field, the query the results answer, the pages loaded and their facets.
//! Requests go out one at a time, the latest wanted one waiting for the one in flight (the instance
//! answers `RATE_LIMITED` to overlapping searches), and an answer that no longer matches what is
//! asked is dropped (a generation check).
//!
//! No clock and no I/O here: methods hand back the request to send, the client sends it and gives
//! the answer to [`MessageSearch::finish`]; the typing pause is the client's timer.

use mellow_protocol::{
    ApiErrorCode, AuthorSearchFacet, ChannelSearchFacet, MAXIMUM_REACHABLE_SEARCH_RESULTS,
    MAXIMUM_SEARCH_RESULTS_PER_PAGE, MessageSearchHit, MessageSearchQuery, MessageSearchResults,
    MessageSearchSort, Snowflake,
};

use super::search_query::{
    SearchQueryProblem, has_searchable_content, has_searched_words, search_query_problem,
};
use crate::ClientError;

/// Typing pauses this long before a search goes out: searches are limited to 30 per 30 s per person.
pub const SEARCH_TYPING_PAUSE_MILLISECONDS: u64 = 400;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MessageSearchStatus {
    /// Nothing asked yet: the panel explains the filters.
    #[default]
    Idle,
    Loading,
    Loaded,
    RateLimited,
    TooLarge(SearchQueryProblem),
    /// [`MessageSearch::failure`] says why.
    Failed,
}

/// One page to ask the instance for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    generation: u64,
    /// The server searched, or `None` for the conversations.
    pub scope_guild_id: Option<Snowflake>,
    pub query: String,
    pub sort: MessageSearchSort,
    pub offset: u32,
    pub limit: u32,
}

impl SearchRequest {
    /// The body of the search routes, the searcher's days being theirs (`utc_offset_minutes`).
    #[must_use]
    pub fn body(&self, utc_offset_minutes: i16) -> MessageSearchQuery {
        MessageSearchQuery {
            q: self.query.clone(),
            sort: Some(self.sort),
            offset: Some(self.offset),
            limit: Some(self.limit),
            utc_offset_minutes: Some(utc_offset_minutes),
        }
    }
}

/// What typing asks of the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterTyping {
    /// Search the draft once typing pauses for [`SEARCH_TYPING_PAUSE_MILLISECONDS`].
    SearchAfterPause,
    Nothing,
}

#[derive(Debug, Default)]
#[allow(clippy::struct_excessive_bools)] // Independent flags, each read on its own (as the web's).
pub struct MessageSearch {
    scope_guild_id: Option<Snowflake>,
    /// The field's text, which may run ahead of `query` while typing.
    pub draft: String,
    query: String,
    sort: MessageSearchSort,
    status: MessageSearchStatus,
    failure: Option<ClientError>,
    hits: Vec<MessageSearchHit>,
    total_count: u32,
    is_total_lower_bound: bool,
    channel_facets: Vec<ChannelSearchFacet>,
    author_facets: Vec<AuthorSearchFacet>,
    is_loading_more: bool,
    /// Why the last "more" page failed, shown at the end of the list with Try again.
    load_more_error: Option<ClientError>,
    /// The results side sheet shows (results stay when the members take the sheet back).
    pub is_panel_open: bool,
    generation: u64,
    /// Where the next page starts on the instance: every hit received, repeats included, so a
    /// message shown twice (edited between pages) never shifts the pages.
    next_offset: u32,
    /// An empty page came back: the instance has nothing more, whatever the total said.
    has_no_more_pages: bool,
    is_request_in_flight: bool,
    waiting_request: Option<SearchRequest>,
}

impl MessageSearch {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn scope_guild_id(&self) -> Option<Snowflake> {
        self.scope_guild_id
    }

    /// The query the results answer.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    #[must_use]
    pub fn status(&self) -> MessageSearchStatus {
        self.status
    }

    /// Why the search failed, while the status is [`MessageSearchStatus::Failed`].
    #[must_use]
    pub fn failure(&self) -> Option<&ClientError> {
        self.failure.as_ref()
    }

    #[must_use]
    pub fn hits(&self) -> &[MessageSearchHit] {
        &self.hits
    }

    #[must_use]
    pub fn total_count(&self) -> u32 {
        self.total_count
    }

    #[must_use]
    pub fn is_total_lower_bound(&self) -> bool {
        self.is_total_lower_bound
    }

    #[must_use]
    pub fn channel_facets(&self) -> &[ChannelSearchFacet] {
        &self.channel_facets
    }

    #[must_use]
    pub fn author_facets(&self) -> &[AuthorSearchFacet] {
        &self.author_facets
    }

    #[must_use]
    pub fn is_loading_more(&self) -> bool {
        self.is_loading_more
    }

    #[must_use]
    pub fn load_more_error(&self) -> Option<&ClientError> {
        self.load_more_error.as_ref()
    }

    /// Ranked by relevance only when words are searched; filters alone list the newest first.
    #[must_use]
    pub fn effective_sort(&self) -> MessageSearchSort {
        if has_searched_words(&self.query) {
            self.sort
        } else {
            MessageSearchSort::Newest
        }
    }

    #[must_use]
    pub fn can_load_more(&self) -> bool {
        self.status == MessageSearchStatus::Loaded
            && self.load_more_error.is_none()
            && !self.has_no_more_pages
            && self.next_offset < self.total_count.min(MAXIMUM_REACHABLE_SEARCH_RESULTS)
    }

    /// Every result reachable is shown, but more matched: refining is the way further.
    #[must_use]
    pub fn has_reached_limit(&self) -> bool {
        self.status == MessageSearchStatus::Loaded
            && self.next_offset >= MAXIMUM_REACHABLE_SEARCH_RESULTS
            && self.is_total_lower_bound
    }

    /// Another server (or Home) starts with an empty search.
    pub fn set_scope(&mut self, guild_id: Option<Snowflake>) {
        if guild_id != self.scope_guild_id {
            self.clear();
            self.scope_guild_id = guild_id;
        }
    }

    /// The field's text changed: searched after a pause, unless a filter's value is still typed.
    pub fn note_typed(&mut self, draft: String, is_typing_filter_value: bool) -> AfterTyping {
        self.draft = draft;
        if is_typing_filter_value {
            return AfterTyping::Nothing;
        }
        if !has_searchable_content(&self.draft) {
            if self.draft.trim().is_empty() {
                self.show_nothing();
            }
            return AfterTyping::Nothing;
        }
        AfterTyping::SearchAfterPause
    }

    /// Searches `query` now (Enter, a suggestion, a facet, a chip removed, the typing pause),
    /// opening the results; the request to send, if one must go out now.
    pub fn search(&mut self, query: String) -> Option<SearchRequest> {
        self.draft.clone_from(&query);
        if !has_searchable_content(&query) {
            return None;
        }
        self.is_panel_open = true;
        if query == self.query
            && matches!(
                self.status,
                MessageSearchStatus::Loaded | MessageSearchStatus::Loading
            )
        {
            return None;
        }
        self.query = query;
        self.generation += 1;
        self.is_loading_more = false;
        self.load_more_error = None;
        self.failure = None;
        if let Some(problem) = search_query_problem(&self.query) {
            self.waiting_request = None;
            self.status = MessageSearchStatus::TooLarge(problem);
            self.hits.clear();
            return None;
        }
        self.status = MessageSearchStatus::Loading;
        let request = self.request_for(0);
        self.send(request)
    }

    /// Searches the field's text now (Enter).
    pub fn search_draft(&mut self) -> Option<SearchRequest> {
        self.search(self.draft.clone())
    }

    pub fn set_sort(&mut self, sort: MessageSearchSort) -> Option<SearchRequest> {
        if sort == self.sort {
            return None;
        }
        self.sort = sort;
        if self.query.is_empty() {
            return None;
        }
        let query = std::mem::take(&mut self.query);
        self.search(query)
    }

    /// The next page, while results remain within the 1 000 reachable.
    pub fn load_more(&mut self) -> Option<SearchRequest> {
        if !self.can_load_more() || self.is_loading_more {
            return None;
        }
        self.is_loading_more = true;
        let request = self.request_for(self.next_offset);
        self.send(request)
    }

    /// Try again after a failure: the whole search, or only the page that failed.
    pub fn retry(&mut self) -> Option<SearchRequest> {
        if self.load_more_error.take().is_some() {
            return self.load_more();
        }
        let query = std::mem::take(&mut self.query);
        self.search(query)
    }

    /// Empties the field and the results, and closes the panel.
    pub fn clear(&mut self) {
        self.draft.clear();
        self.show_nothing();
        self.is_panel_open = false;
    }

    /// The instance answered `request`: applied unless the search moved on; then the request
    /// that waited for this one, if it is still wanted.
    pub fn finish(
        &mut self,
        request: &SearchRequest,
        answer: Result<MessageSearchResults, ClientError>,
    ) -> Option<SearchRequest> {
        self.is_request_in_flight = false;
        if request.generation == self.generation {
            match answer {
                Ok(results) => self.apply_results(request, results),
                Err(error) => self.apply_failure(request, error),
            }
        }
        let waiting = self.waiting_request.take()?;
        if waiting.generation == self.generation {
            self.send(waiting)
        } else {
            None
        }
    }

    fn show_nothing(&mut self) {
        self.generation += 1;
        self.waiting_request = None;
        self.query.clear();
        self.status = MessageSearchStatus::Idle;
        self.failure = None;
        self.hits.clear();
        self.total_count = 0;
        self.is_total_lower_bound = false;
        self.channel_facets.clear();
        self.author_facets.clear();
        self.is_loading_more = false;
        self.load_more_error = None;
        self.next_offset = 0;
        self.has_no_more_pages = false;
    }

    fn request_for(&self, offset: u32) -> SearchRequest {
        SearchRequest {
            generation: self.generation,
            scope_guild_id: self.scope_guild_id,
            query: self.query.clone(),
            sort: self.effective_sort(),
            offset,
            // Never past the 1 000 the instance reaches (it refuses offset + limit beyond).
            limit: MAXIMUM_SEARCH_RESULTS_PER_PAGE
                .min(MAXIMUM_REACHABLE_SEARCH_RESULTS.saturating_sub(offset)),
        }
    }

    fn send(&mut self, request: SearchRequest) -> Option<SearchRequest> {
        if self.is_request_in_flight {
            // Only the latest wish waits: an older one would be dropped on arrival anyway.
            self.waiting_request = Some(request);
            return None;
        }
        self.is_request_in_flight = true;
        Some(request)
    }

    fn apply_results(&mut self, request: &SearchRequest, results: MessageSearchResults) {
        let received = u32::try_from(results.hits.len()).unwrap_or(u32::MAX);
        self.next_offset = request.offset.saturating_add(received);
        self.has_no_more_pages = results.hits.is_empty();
        if request.offset == 0 {
            self.hits = results.hits;
            self.channel_facets = results.channel_facets;
            self.author_facets = results.author_facets;
        } else {
            // A message edited between pages could come twice: the first copy stays.
            let known: std::collections::HashSet<Snowflake> =
                self.hits.iter().map(|hit| hit.message.id).collect();
            self.hits.extend(
                results
                    .hits
                    .into_iter()
                    .filter(|hit| !known.contains(&hit.message.id)),
            );
            self.is_loading_more = false;
        }
        self.total_count = results.total_count;
        self.is_total_lower_bound = results.total_count_is_lower_bound;
        self.status = MessageSearchStatus::Loaded;
    }

    fn apply_failure(&mut self, request: &SearchRequest, error: ClientError) {
        if request.offset > 0 {
            self.is_loading_more = false;
            self.load_more_error = Some(error);
            return;
        }
        self.status = match &error {
            ClientError::Api {
                code: ApiErrorCode::RateLimited,
                ..
            } => MessageSearchStatus::RateLimited,
            ClientError::Api {
                code: ApiErrorCode::ValidationFailed,
                ..
            } => MessageSearchStatus::TooLarge(SearchQueryProblem::TooManyParts),
            _ => MessageSearchStatus::Failed,
        };
        self.failure = Some(error);
        self.hits.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_logic::search_results::tests::hit;

    fn results(
        ids: impl IntoIterator<Item = u64>,
        total: u32,
        is_lower_bound: bool,
    ) -> MessageSearchResults {
        MessageSearchResults {
            hits: ids.into_iter().map(|id| hit(id, 10)).collect(),
            total_count: total,
            total_count_is_lower_bound: is_lower_bound,
            channel_facets: Vec::new(),
            author_facets: Vec::new(),
        }
    }

    fn api_error(status: u16, code: ApiErrorCode) -> ClientError {
        ClientError::Api {
            status,
            code,
            message: String::new(),
        }
    }

    fn shown_ids(search: &MessageSearch) -> Vec<u64> {
        search
            .hits()
            .iter()
            .map(|hit| hit.message.id.raw())
            .collect()
    }

    #[test]
    fn searches_after_a_pause_in_typing_never_while_a_filters_value_is_typed() {
        let mut search = MessageSearch::new();
        search.set_scope(Some(Snowflake::from_raw(100)));
        assert_eq!(
            search.note_typed("pizza".into(), false),
            AfterTyping::SearchAfterPause
        );
        let request = search.search_draft().expect("the first search goes out");
        assert_eq!(request.scope_guild_id, Some(Snowflake::from_raw(100)));
        assert_eq!(
            request.body(60),
            MessageSearchQuery {
                q: "pizza".into(),
                sort: Some(MessageSearchSort::Relevance),
                offset: Some(0),
                limit: Some(25),
                utc_offset_minutes: Some(60),
            }
        );
        assert!(search.is_panel_open);
        assert_eq!(
            search.note_typed("pizza from:a".into(), true),
            AfterTyping::Nothing
        );
        assert_eq!(search.note_typed("!!".into(), false), AfterTyping::Nothing);
    }

    #[test]
    fn keeps_one_search_in_flight_sends_only_the_latest_wish_after_it_and_drops_the_stale_answer() {
        let mut search = MessageSearch::new();
        let one = search.search("one".into()).expect("sent");
        assert_eq!(search.search("two".into()), None);
        assert_eq!(search.search("three".into()), None);
        let three = search
            .finish(&one, Ok(results([1], 1, false)))
            .expect("the latest wish goes out after the first answer");
        assert_eq!(three.query, "three");
        assert_eq!(search.status(), MessageSearchStatus::Loading);
        assert_eq!(search.finish(&three, Ok(results([3], 1, false))), None);
        assert_eq!(search.status(), MessageSearchStatus::Loaded);
        assert_eq!(shown_ids(&search), vec![3]);
    }

    #[test]
    fn lists_filters_alone_newest_first_and_asks_again_when_the_sort_changes() {
        let mut search = MessageSearch::new();
        let filters = search.search("from:ada".into()).expect("sent");
        assert_eq!(filters.sort, MessageSearchSort::Newest);
        search.finish(&filters, Ok(results([1], 1, false)));
        let words = search.search("pizza".into()).expect("sent");
        assert_eq!(words.sort, MessageSearchSort::Relevance);
        search.finish(&words, Ok(results([1], 1, false)));
        let newest = search
            .set_sort(MessageSearchSort::Newest)
            .expect("asked again");
        assert_eq!(
            (newest.query.as_str(), newest.sort),
            ("pizza", MessageSearchSort::Newest)
        );
    }

    #[test]
    fn pages_by_what_the_instance_sent_showing_a_repeated_message_once() {
        let mut search = MessageSearch::new();
        let first = search.search("pizza".into()).expect("sent");
        search.finish(&first, Ok(results([1, 2], 5, false)));
        assert!(search.can_load_more());
        let second = search.load_more().expect("sent");
        assert_eq!(search.load_more(), None);
        assert_eq!(second.offset, 2);
        search.finish(&second, Ok(results([2, 3], 5, false)));
        assert_eq!(shown_ids(&search), vec![1, 2, 3]);
        // Four received, three shown: the next page starts after the four.
        let third = search.load_more().expect("sent");
        assert_eq!(third.offset, 4);
        search.finish(&third, Ok(results([], 5, false)));
        assert!(!search.can_load_more());
    }

    #[test]
    fn never_asks_past_the_1000_results_the_instance_reaches() {
        let mut search = MessageSearch::new();
        let first = search.search("pizza".into()).expect("sent");
        search.finish(&first, Ok(results(0..990, 1_000, true)));
        let last = search.load_more().expect("sent");
        assert_eq!((last.offset, last.limit), (990, 10));
        search.finish(&last, Ok(results(990..1_000, 1_000, true)));
        assert_eq!(search.hits().len(), 1_000);
        assert!(!search.can_load_more());
        assert!(search.has_reached_limit());
    }

    #[test]
    fn tells_a_rate_limit_a_query_too_large_and_a_failed_page_apart() {
        let mut search = MessageSearch::new();
        let limited = search.search("pizza".into()).expect("sent");
        search.finish(&limited, Err(api_error(429, ApiErrorCode::RateLimited)));
        assert_eq!(search.status(), MessageSearchStatus::RateLimited);

        assert_eq!(search.search("a ".repeat(17)), None);
        assert_eq!(
            search.status(),
            MessageSearchStatus::TooLarge(SearchQueryProblem::TooManyParts)
        );

        let first = search.search("pizza".into()).expect("sent");
        search.finish(&first, Ok(results([1], 30, false)));
        let page = search.load_more().expect("sent");
        search.finish(&page, Err(ClientError::TransferStalled));
        assert_eq!(search.status(), MessageSearchStatus::Loaded);
        assert!(search.load_more_error().is_some());
        let again = search.retry().expect("the page is asked again");
        assert_eq!(again.offset, 1);
    }

    #[test]
    fn starts_empty_in_another_server_and_empties_when_the_field_is_cleared() {
        let mut search = MessageSearch::new();
        search.set_scope(Some(Snowflake::from_raw(100)));
        let stale = search.search("pizza".into()).expect("sent");
        search.set_scope(None);
        // The stale answer is dropped; no request waited for it.
        assert_eq!(search.finish(&stale, Ok(results([1], 1, false))), None);
        assert_eq!(search.status(), MessageSearchStatus::Idle);
        assert_eq!(search.hits(), &[]);
        assert!(!search.is_panel_open);
        let fresh = search.search("pizza".into()).expect("sent");
        search.finish(&fresh, Ok(results([1], 1, false)));
        assert_eq!(
            search.note_typed(String::new(), false),
            AfterTyping::Nothing
        );
        assert_eq!(search.status(), MessageSearchStatus::Idle);
        assert_eq!(search.query(), "");
    }

    #[test]
    fn a_waiting_request_of_a_cleared_search_never_goes_out() {
        let mut search = MessageSearch::new();
        let first = search.search("one".into()).expect("sent");
        assert_eq!(search.search("two".into()), None);
        search.clear();
        assert_eq!(search.finish(&first, Ok(results([1], 1, false))), None);
        assert_eq!(search.status(), MessageSearchStatus::Idle);
        // Nothing is in flight any more: the next search goes out at once.
        assert!(search.search("three".into()).is_some());
    }
}
