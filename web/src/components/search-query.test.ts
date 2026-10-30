import { describe, expect, it } from "vitest";

import type { PublicUser } from "../lib/protocol";
import {
  channelFilterValue,
  hasSearchableContent,
  hasSearchedWords,
  isTypingFilterValue,
  replaceInSearchQuery,
  searchCaretContext,
  searchQueryParts,
  searchQueryProblem,
  searchedWordsOf,
  withSearchFilter,
  withoutSearchQueryPart,
} from "./search-query";
import { searchSuggestions, suggestionInsertion } from "./search-suggestions";

const ADA: PublicUser = { id: "1", username: "ada", display_name: "Ada Lovelace" };
const BOB: PublicUser = { id: "2", username: "bob", display_name: "Bobby" };
const ZOE: PublicUser = { id: "3", username: "zoe", display_name: "Zoé Abadie" };

describe("searchQueryParts", () => {
  it("finds words, phrases, exclusions and filters as the instance does", () => {
    const query = 'from:ada "café au lait" -pizza in:#general has:LINK piz* -"no way" before:2026-02-30 mentions:';
    expect(searchQueryParts(query).map(({ text, isExcluded, isPhrase, filter }) => ({ text, isExcluded, isPhrase, filter }))).toEqual([
      { text: "from:ada", isExcluded: false, isPhrase: false, filter: { key: "from", value: "ada" } },
      { text: "café au lait", isExcluded: false, isPhrase: true, filter: null },
      { text: "pizza", isExcluded: true, isPhrase: false, filter: null },
      { text: "in:#general", isExcluded: false, isPhrase: false, filter: { key: "in", value: "#general" } },
      { text: "has:LINK", isExcluded: false, isPhrase: false, filter: { key: "has", value: "LINK" } },
      { text: "piz*", isExcluded: false, isPhrase: false, filter: null },
      { text: "no way", isExcluded: true, isPhrase: true, filter: null },
      // Not a real day, and an empty value: both searched as words, as on the instance.
      { text: "before:2026-02-30", isExcluded: false, isPhrase: false, filter: null },
      { text: "mentions:", isExcluded: false, isPhrase: false, filter: null },
    ]);
  });

  it("ends a word at a quote and keeps an unclosed phrase to the end", () => {
    expect(searchQueryParts('foo"bar baz').map((part) => [part.text, part.isPhrase])).toEqual([
      ["foo", false],
      ["bar baz", true],
    ]);
  });

  it("knows the instance's limits before sending", () => {
    expect(searchQueryProblem("a ".repeat(16))).toBeNull();
    expect(searchQueryProblem("a ".repeat(17))).toBe("too-many-parts");
    expect(searchQueryProblem("é".repeat(256))).toBeNull();
    // 129 emoji are 258 UTF-16 units but 129 characters: the instance counts characters.
    expect(searchQueryProblem("😀".repeat(129))).toBeNull();
    expect(searchQueryProblem("x".repeat(257))).toBe("too-long");
  });

  it("tells searchable text from punctuation, and words from filters", () => {
    expect(hasSearchableContent("  ?! ")).toBe(false);
    expect(hasSearchableContent("has:file")).toBe(true);
    expect(hasSearchedWords("has:file from:ada")).toBe(false);
    expect(hasSearchedWords("has:file pizza")).toBe(true);
    expect(hasSearchedWords("-pizza")).toBe(false);
    expect(searchedWordsOf('from:ada pizza "au lait" in:general')).toBe('pizza "au lait"');
  });
});

describe("searchCaretContext", () => {
  it("offers filters between parts, a filter's name from its first letters, then its values", () => {
    expect(searchCaretContext("", 0)).toEqual({ kind: "between", start: 0, end: 0 });
    expect(searchCaretContext("pizza ", 6)).toEqual({ kind: "between", start: 6, end: 6 });
    expect(searchCaretContext("pizza fr", 8)).toEqual({ kind: "key", partial: "fr", start: 6, end: 8 });
    expect(searchCaretContext("pizza FROM:ad", 13)).toEqual({ kind: "value", key: "from", partial: "ad", start: 6, end: 13 });
    expect(searchCaretContext('"from:a', 7)).toEqual({ kind: "none" });
    expect(searchCaretContext("-from:a", 7)).toEqual({ kind: "none" });
    expect(searchCaretContext("color:red", 9)).toEqual({ kind: "none" });
  });

  it("holds searches back while a filter's value is typed", () => {
    expect(isTypingFilterValue("pizza from:a", 12)).toBe(true);
    expect(isTypingFilterValue("pizza from:ada ", 15)).toBe(false);
  });
});

describe("editing the query", () => {
  it("writes a suggestion in place of the part, with a space after a finished filter", () => {
    expect(replaceInSearchQuery("pizza fr", 6, 8, "from:", false)).toEqual({ query: "pizza from:", caret: 11 });
    expect(replaceInSearchQuery("pizza from:a", 6, 12, "from:ada", true)).toEqual({ query: "pizza from:ada ", caret: 15 });
    expect(replaceInSearchQuery("from:a pizza", 0, 6, "from:ada", true)).toEqual({ query: "from:ada pizza", caret: 9 });
    expect(replaceInSearchQuery("pizza", 5, 5, "has:", false)).toEqual({ query: "pizza has:", caret: 10 });
  });

  it("adds a facet's filter once, and removes a chip's part with its space", () => {
    expect(withSearchFilter("pizza ", "from", "ada")).toBe("pizza from:ada");
    expect(withSearchFilter("pizza from:Ada", "from", "ada")).toBe("pizza from:Ada");
    expect(withSearchFilter("", "in", "general")).toBe("in:general");
    const query = "pizza from:ada  has:link";
    const [, from] = searchQueryParts(query);
    expect(from && withoutSearchQueryPart(query, from)).toBe("pizza has:link");
  });

  it("names a channel by id when its name would not stay one part", () => {
    expect(channelFilterValue({ id: "7", name: "général" })).toBe("général");
    expect(channelFilterValue({ id: "7", name: "Game night" })).toBe("7");
    expect(channelFilterValue({ id: "7", name: 'say "hi"' })).toBe("7");
    expect(channelFilterValue({ id: "7", name: "#hash" })).toBe("7");
  });
});

describe("searchSuggestions", () => {
  const today = new Date(2026, 9, 30, 15, 0);
  const sources = {
    people: [ADA, BOB, ZOE],
    channels: [
      { id: "10", name: "général", isConversation: false, value: "général" },
      { id: "11", name: "Game night", isConversation: false, value: "11" },
    ],
    today,
  };

  it("lists every filter between parts, and the filters a word could start", () => {
    expect(searchSuggestions({ kind: "between", start: 0, end: 0 }, sources)).toHaveLength(7);
    expect(searchSuggestions({ kind: "key", partial: "M", start: 0, end: 1 }, sources)).toEqual([{ kind: "key", key: "mentions" }]);
    expect(searchSuggestions({ kind: "key", partial: "pizza", start: 0, end: 5 }, sources)).toEqual([]);
  });

  it("suggests people by username or name, ignoring accents, starting matches first", () => {
    const people = searchSuggestions({ kind: "value", key: "from", partial: "@a", start: 0, end: 7 }, sources);
    expect(people.map((suggestion) => (suggestion.kind === "person" ? suggestion.user.username : ""))).toEqual(["ada", "zoe"]);
    const [first] = people;
    expect(first && suggestionInsertion(first)).toEqual({ text: "from:ada", endsPart: true });
  });

  it("suggests channels by name, writing the value that stays one part", () => {
    const channels = searchSuggestions({ kind: "value", key: "in", partial: "#ga", start: 0, end: 6 }, sources);
    expect(channels.map((suggestion) => suggestion.kind === "channel" && suggestionInsertion(suggestion).text)).toEqual(["in:11"]);
    expect(searchSuggestions({ kind: "value", key: "in", partial: "gene", start: 0, end: 7 }, sources)).toHaveLength(1);
  });

  it("suggests kinds and days, the typed day first when it is a real one", () => {
    expect(searchSuggestions({ kind: "value", key: "has", partial: "i", start: 0, end: 5 }, sources)).toEqual([{ kind: "has", value: "image" }]);
    const days = searchSuggestions({ kind: "value", key: "before", partial: "", start: 0, end: 7 }, sources);
    expect(days.map((suggestion) => (suggestion.kind === "day" ? [suggestion.day, suggestion.relativeDay] : null))).toEqual([
      ["2026-10-30", "today"],
      ["2026-10-29", "yesterday"],
      ["2026-10-23", "a-week-ago"],
      ["2026-09-30", "a-month-ago"],
    ]);
    const typed = searchSuggestions({ kind: "value", key: "during", partial: "2024-02-29", start: 0, end: 17 }, sources);
    expect(typed).toEqual([{ kind: "day", key: "during", day: "2024-02-29", relativeDay: "typed" }]);
    expect(searchSuggestions({ kind: "value", key: "during", partial: "2023-02-29", start: 0, end: 17 }, sources)).toEqual([]);
  });
});
