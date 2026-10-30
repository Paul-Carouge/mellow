import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../lib/api";
import type { MessageSearchHit, MessageSearchQuery, MessageSearchResults } from "../lib/protocol";
import { MessageSearch, SEARCH_TYPING_PAUSE_MILLISECONDS } from "./message-search.svelte";

type Call = { guildId: string | null; query: Omit<MessageSearchQuery, "utc_offset_minutes"> };

function hit(id: number): MessageSearchHit {
  return {
    message: {
      id: String(id),
      channel_id: "10",
      author: { id: "1", username: "ada", display_name: "Ada" },
      content: `message ${id}`,
      reply_to_id: null,
      edited_at_unix_milliseconds: null,
    },
    highlights: [],
  };
}

function results(ids: number[], total: number, isLowerBound = false): MessageSearchResults {
  return { hits: ids.map(hit), total_count: total, total_count_is_lower_bound: isLowerBound, channel_facets: [], author_facets: [] };
}

/** A fake instance whose answers the test releases one by one. */
function fakeInstance() {
  const calls: Call[] = [];
  const answers: { resolve: (value: MessageSearchResults) => void; reject: (error: unknown) => void }[] = [];
  const searchMessages = (guildId: string | null, query: Call["query"]) => {
    calls.push({ guildId, query });
    return new Promise<MessageSearchResults>((resolve, reject) => answers.push({ resolve, reject }));
  };
  return { calls, answers, searchMessages };
}

const settle = () => vi.advanceTimersByTimeAsync(0);

describe("MessageSearch", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("searches after a pause in typing, never while a filter's value is typed", async () => {
    const instance = fakeInstance();
    const search = new MessageSearch(instance.searchMessages);
    search.setScope("100");
    search.noteTyped("piz", false);
    search.noteTyped("pizza", false);
    await vi.advanceTimersByTimeAsync(SEARCH_TYPING_PAUSE_MILLISECONDS - 1);
    expect(instance.calls).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(instance.calls).toEqual([{ guildId: "100", query: { q: "pizza", sort: "relevance", offset: 0, limit: 25 } }]);
    expect(search.isPanelOpen).toBe(true);
    search.noteTyped("pizza from:a", true);
    await vi.advanceTimersByTimeAsync(SEARCH_TYPING_PAUSE_MILLISECONDS * 2);
    expect(instance.calls).toHaveLength(1);
  });

  it("keeps one search in flight, sends only the latest wish after it and drops the stale answer", async () => {
    const instance = fakeInstance();
    const search = new MessageSearch(instance.searchMessages);
    search.search("one");
    search.search("two");
    search.search("three");
    expect(instance.calls.map((call) => call.query.q)).toEqual(["one"]);
    instance.answers[0]?.resolve(results([1], 1));
    await settle();
    expect(search.status.kind).toBe("loading");
    expect(instance.calls.map((call) => call.query.q)).toEqual(["one", "three"]);
    instance.answers[1]?.resolve(results([3], 1));
    await settle();
    expect(search.status.kind).toBe("loaded");
    expect(search.hits.map((found) => found.message.id)).toEqual(["3"]);
  });

  it("lists filters alone newest first, and asks again when the sort changes", async () => {
    const instance = fakeInstance();
    const search = new MessageSearch(instance.searchMessages);
    search.search("from:ada");
    expect(instance.calls[0]?.query.sort).toBe("newest");
    instance.answers[0]?.resolve(results([1], 1));
    await settle();
    search.search("pizza");
    instance.answers[1]?.resolve(results([1], 1));
    await settle();
    search.setSort("newest");
    expect(instance.calls.map((call) => [call.query.q, call.query.sort])).toEqual([
      ["from:ada", "newest"],
      ["pizza", "relevance"],
      ["pizza", "newest"],
    ]);
  });

  it("pages by what the instance sent, showing a repeated message once", async () => {
    const instance = fakeInstance();
    const search = new MessageSearch(instance.searchMessages);
    search.search("pizza");
    instance.answers[0]?.resolve(results([1, 2], 5));
    await settle();
    expect(search.canLoadMore).toBe(true);
    search.loadMore();
    search.loadMore();
    expect(instance.calls.map((call) => call.query.offset)).toEqual([0, 2]);
    instance.answers[1]?.resolve(results([2, 3], 5));
    await settle();
    expect(search.hits.map((found) => found.message.id)).toEqual(["1", "2", "3"]);
    // Four received, three shown: the next page starts after the four.
    search.loadMore();
    expect(instance.calls.at(-1)?.query.offset).toBe(4);
    instance.answers[2]?.resolve(results([], 5));
    await settle();
    expect(search.canLoadMore).toBe(false);
  });

  it("never asks past the 1 000 results the instance reaches", async () => {
    const instance = fakeInstance();
    const search = new MessageSearch(instance.searchMessages);
    search.search("pizza");
    instance.answers[0]?.resolve(results(Array.from({ length: 990 }, (_, index) => index), 1_000, true));
    await settle();
    search.loadMore();
    expect(instance.calls.at(-1)?.query).toMatchObject({ offset: 990, limit: 10 });
    instance.answers[1]?.resolve(results(Array.from({ length: 10 }, (_, index) => 990 + index), 1_000, true));
    await settle();
    expect(search.hits).toHaveLength(1_000);
    expect(search.canLoadMore).toBe(false);
    expect(search.hasReachedLimit).toBe(true);
  });

  it("tells a rate limit, a query too large and a failed page apart", async () => {
    const instance = fakeInstance();
    const search = new MessageSearch(instance.searchMessages);
    search.search("pizza");
    instance.answers[0]?.reject(new ApiError(429, "RATE_LIMITED"));
    await settle();
    expect(search.status).toEqual({ kind: "rate-limited" });

    search.search("a ".repeat(17));
    expect(search.status).toEqual({ kind: "too-large", problem: "too-many-parts" });
    expect(instance.calls).toHaveLength(1);

    search.search("pizza");
    instance.answers[1]?.resolve(results([1], 30));
    await settle();
    search.loadMore();
    instance.answers[2]?.reject(new ApiError(0, "NETWORK"));
    await settle();
    expect(search.status.kind).toBe("loaded");
    expect(search.loadMoreError).toBeInstanceOf(ApiError);
    search.retry();
    expect(instance.calls.at(-1)?.query.offset).toBe(1);
  });

  it("starts empty in another server, and empties when the field is cleared", async () => {
    const instance = fakeInstance();
    const search = new MessageSearch(instance.searchMessages);
    search.setScope("100");
    search.search("pizza");
    search.setScope(null);
    instance.answers[0]?.resolve(results([1], 1));
    await settle();
    expect(search.status.kind).toBe("idle");
    expect(search.hits).toEqual([]);
    expect(search.isPanelOpen).toBe(false);
    search.search("pizza");
    instance.answers[1]?.resolve(results([1], 1));
    await settle();
    search.noteTyped("", false);
    expect(search.status.kind).toBe("idle");
    expect(search.query).toBe("");
  });
});
