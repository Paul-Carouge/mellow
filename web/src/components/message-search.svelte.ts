// The state of one message search (DESIGN.md 5.12): the text in the field, the query the results
// answer, the pages loaded and their facets. Requests go out one at a time, the latest wanted one
// waiting for the one in flight (the instance answers RATE_LIMITED to overlapping searches), and an
// answer that no longer matches what is asked is dropped.

import { ApiError } from "../lib/api";
import {
  MAXIMUM_REACHABLE_SEARCH_RESULTS,
  MAXIMUM_SEARCH_RESULTS_PER_PAGE,
  type MessageSearchHit,
  type MessageSearchQuery,
  type MessageSearchResults,
  type MessageSearchSort,
  type Snowflake,
} from "../lib/protocol";
import { type SearchQueryProblem, hasSearchableContent, hasSearchedWords, searchQueryProblem } from "./search-query";

/** Typing pauses this long before a search goes out: searches are limited to 30 per 30 s per person. */
export const SEARCH_TYPING_PAUSE_MILLISECONDS = 400;

export type MessageSearchStatus =
  /** Nothing asked yet: the panel explains the filters. */
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "loaded" }
  | { kind: "rate-limited" }
  | { kind: "too-large"; problem: SearchQueryProblem }
  | { kind: "failed"; error: unknown };

export type SearchMessages = (
  guildId: Snowflake | null,
  query: Omit<MessageSearchQuery, "utc_offset_minutes">,
) => Promise<MessageSearchResults>;

interface SearchRequest {
  generation: number;
  scopeGuildId: Snowflake | null;
  query: string;
  sort: MessageSearchSort;
  offset: number;
}

export class MessageSearch {
  /** The server searched, or null for the conversations at Home. */
  scopeGuildId = $state<Snowflake | null>(null);
  /** The field's text, which may run ahead of `query` while typing. */
  draft = $state("");
  /** The query the results answer. */
  query = $state("");
  sort = $state<MessageSearchSort>("relevance");
  status = $state<MessageSearchStatus>({ kind: "idle" });
  hits = $state<MessageSearchHit[]>([]);
  totalCount = $state(0);
  isTotalLowerBound = $state(false);
  channelFacets = $state<MessageSearchResults["channel_facets"]>([]);
  authorFacets = $state<MessageSearchResults["author_facets"]>([]);
  isLoadingMore = $state(false);
  /** Why the last "more" page failed, shown at the end of the list with Try again. */
  loadMoreError = $state<unknown>(null);
  /** The results side sheet shows (results stay when it closes over a phone's conversation). */
  isPanelOpen = $state(false);

  private generation = 0;
  private isRequestInFlight = false;
  private waitingRequest: SearchRequest | null = null;
  private typingTimer: ReturnType<typeof setTimeout> | null = null;
  private fieldElement: HTMLInputElement | null = null;

  constructor(private readonly searchMessages: SearchMessages) {}

  /** Ranked by relevance only when words are searched; filters alone list the newest first. */
  get effectiveSort(): MessageSearchSort {
    return hasSearchedWords(this.query) ? this.sort : "newest";
  }

  get canLoadMore(): boolean {
    return (
      this.status.kind === "loaded" &&
      this.loadMoreError === null &&
      this.hits.length < Math.min(this.totalCount, MAXIMUM_REACHABLE_SEARCH_RESULTS)
    );
  }

  /** Every result reachable is shown, but more matched: refining is the way further. */
  get hasReachedLimit(): boolean {
    return this.status.kind === "loaded" && this.hits.length >= MAXIMUM_REACHABLE_SEARCH_RESULTS && this.totalCount > this.hits.length;
  }

  /** Another server (or Home) starts with an empty search. */
  setScope(guildId: Snowflake | null): void {
    if (guildId !== this.scopeGuildId) {
      this.clear();
      this.scopeGuildId = guildId;
    }
  }

  /** The field registers itself, so Ctrl+F and the panel can give it focus. */
  registerField(element: HTMLInputElement | null): void {
    this.fieldElement = element;
  }

  /** Focuses the field shown now; false when none is (the panel must open with its own). */
  focusField(): boolean {
    if (!this.fieldElement?.isConnected) {
      return false;
    }
    this.fieldElement.focus();
    this.fieldElement.select();
    return true;
  }

  /** The field's text changed: searched after a pause, unless a filter's value is still being typed. */
  noteTyped(draft: string, isTypingFilterValue: boolean): void {
    this.draft = draft;
    this.cancelTypingPause();
    if (isTypingFilterValue) {
      return;
    }
    if (!hasSearchableContent(draft)) {
      if (draft.trim() === "") {
        this.showNothing();
      }
      return;
    }
    this.typingTimer = setTimeout(() => {
      this.typingTimer = null;
      this.search(draft);
    }, SEARCH_TYPING_PAUSE_MILLISECONDS);
  }

  /** Searches `query` now (Enter, a suggestion, a facet, a chip removed), opening the results. */
  search(query: string = this.draft): void {
    this.cancelTypingPause();
    this.draft = query;
    if (!hasSearchableContent(query)) {
      return;
    }
    this.isPanelOpen = true;
    if (query === this.query && (this.status.kind === "loaded" || this.status.kind === "loading")) {
      return;
    }
    this.query = query;
    const problem = searchQueryProblem(query);
    this.generation += 1;
    this.isLoadingMore = false;
    this.loadMoreError = null;
    if (problem) {
      this.waitingRequest = null;
      this.status = { kind: "too-large", problem };
      this.hits = [];
      return;
    }
    this.status = { kind: "loading" };
    this.send(this.requestFor(0));
  }

  setSort(sort: MessageSearchSort): void {
    if (sort === this.sort) {
      return;
    }
    this.sort = sort;
    if (this.query !== "") {
      const query = this.query;
      this.query = "";
      this.search(query);
    }
  }

  /** The next page, while results remain within the 1 000 reachable. */
  loadMore(): void {
    if (!this.canLoadMore || this.isLoadingMore) {
      return;
    }
    this.isLoadingMore = true;
    this.send(this.requestFor(this.hits.length));
  }

  /** Try again after a failure: the whole search, or only the page that failed. */
  retry(): void {
    if (this.loadMoreError !== null) {
      this.loadMoreError = null;
      this.loadMore();
      return;
    }
    const query = this.query;
    this.query = "";
    this.search(query);
  }

  /** Empties the field and the results, and closes the panel. */
  clear(): void {
    this.cancelTypingPause();
    this.draft = "";
    this.showNothing();
    this.isPanelOpen = false;
  }

  private showNothing(): void {
    this.generation += 1;
    this.waitingRequest = null;
    this.query = "";
    this.status = { kind: "idle" };
    this.hits = [];
    this.totalCount = 0;
    this.isTotalLowerBound = false;
    this.channelFacets = [];
    this.authorFacets = [];
    this.isLoadingMore = false;
    this.loadMoreError = null;
  }

  private requestFor(offset: number): SearchRequest {
    return {
      generation: this.generation,
      scopeGuildId: this.scopeGuildId,
      query: this.query,
      sort: this.effectiveSort,
      offset,
    };
  }

  private cancelTypingPause(): void {
    if (this.typingTimer !== null) {
      clearTimeout(this.typingTimer);
      this.typingTimer = null;
    }
  }

  private send(request: SearchRequest): void {
    if (this.isRequestInFlight) {
      // Only the latest wish waits: an older one would be dropped on arrival anyway.
      this.waitingRequest = request;
      return;
    }
    this.isRequestInFlight = true;
    void this.searchMessages(request.scopeGuildId, {
      q: request.query,
      sort: request.sort,
      offset: request.offset,
      limit: MAXIMUM_SEARCH_RESULTS_PER_PAGE,
    })
      .then(
        (results) => this.applyResults(request, results),
        (error: unknown) => this.applyFailure(request, error),
      )
      .finally(() => {
        this.isRequestInFlight = false;
        const waiting = this.waitingRequest;
        this.waitingRequest = null;
        if (waiting && waiting.generation === this.generation) {
          this.send(waiting);
        }
      });
  }

  private applyResults(request: SearchRequest, results: MessageSearchResults): void {
    if (request.generation !== this.generation) {
      return;
    }
    if (request.offset === 0) {
      this.hits = results.hits;
      this.channelFacets = results.channel_facets;
      this.authorFacets = results.author_facets;
    } else {
      // A message edited between pages could come twice: the first copy stays.
      const known = new Set(this.hits.map((hit) => hit.message.id));
      this.hits = [...this.hits, ...results.hits.filter((hit) => !known.has(hit.message.id))];
      this.isLoadingMore = false;
    }
    this.totalCount = results.total_count;
    this.isTotalLowerBound = results.total_count_is_lower_bound;
    this.status = { kind: "loaded" };
  }

  private applyFailure(request: SearchRequest, error: unknown): void {
    if (request.generation !== this.generation) {
      return;
    }
    if (request.offset > 0) {
      this.isLoadingMore = false;
      this.loadMoreError = error;
      return;
    }
    if (error instanceof ApiError && error.code === "RATE_LIMITED") {
      this.status = { kind: "rate-limited" };
    } else if (error instanceof ApiError && error.code === "VALIDATION_FAILED") {
      this.status = { kind: "too-large", problem: "too-many-parts" };
    } else {
      this.status = { kind: "failed", error };
    }
    this.hits = [];
  }
}
