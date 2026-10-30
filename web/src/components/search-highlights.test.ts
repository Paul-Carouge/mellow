import { describe, expect, it } from "vitest";

import { type BlockNode, parseMessageMarkdown } from "./markdown";
import { mentionablePeople } from "./mentions";
import { normalizedHighlightRanges, splitByHighlights, withSearchHighlights } from "./search-highlights";
import { queryNarrowedToAuthor, queryNarrowedToChannel, searchResultRows } from "./search-results";

const NOBODY = mentionablePeople([]);

function marked(content: string, ranges: { start: number; end: number }[]): BlockNode[] {
  return withSearchHighlights(parseMessageMarkdown(content, NOBODY), content, ranges);
}

describe("highlight ranges", () => {
  it("slices by UTF-16 units and never cuts an emoji in half", () => {
    const content = "a😀b pizza";
    // 😀 is two units (1 and 2): a range starting or ending inside it takes all of it.
    expect(normalizedHighlightRanges(content, [{ start: 2, end: 3 }])).toEqual([{ start: 1, end: 3 }]);
    expect(normalizedHighlightRanges(content, [{ start: 0, end: 2 }])).toEqual([{ start: 0, end: 3 }]);
    expect(splitByHighlights(content, 0, normalizedHighlightRanges(content, [{ start: 5, end: 10 }]))).toEqual([
      { text: "a😀b ", isMatch: false },
      { text: "pizza", isMatch: true },
    ]);
  });

  it("clamps, sorts and merges ranges, and drops empty ones", () => {
    expect(
      normalizedHighlightRanges("hello world", [
        { start: 6, end: 99 },
        { start: 0, end: 2 },
        { start: 2, end: 5 },
        { start: 4, end: 4 },
      ]),
    ).toEqual([
      { start: 0, end: 5 },
      { start: 6, end: 11 },
    ]);
  });
});

describe("withSearchHighlights", () => {
  it("marks matched words inside formatting, across the parser's text nodes", () => {
    const content = "the **pizza** is *ready*";
    expect(marked(content, [{ start: 6, end: 11 }, { start: 18, end: 23 }])).toEqual([
      {
        kind: "paragraph",
        children: [
          { kind: "text", text: "the " },
          { kind: "strong", children: [{ kind: "match", text: "pizza" }] },
          { kind: "text", text: " is " },
          { kind: "emphasis", children: [{ kind: "match", text: "ready" }] },
        ],
      },
    ]);
  });

  it("finds each text after the previous one, so a repeated word is marked where it matched", () => {
    const content = "pizza `pizza` pizza";
    const [paragraph] = marked(content, [{ start: 14, end: 19 }]);
    expect(paragraph).toEqual({
      kind: "paragraph",
      children: [
        { kind: "text", text: "pizza " },
        { kind: "code", text: "pizza" },
        { kind: "text", text: " " },
        { kind: "match", text: "pizza" },
      ],
    });
  });

  it("marks a word written after an emoji at the right place", () => {
    const content = "🍕🍕 pizza";
    const [paragraph] = marked(content, [{ start: 5, end: 10 }]);
    expect(paragraph).toEqual({
      kind: "paragraph",
      children: [
        { kind: "text", text: "🍕🍕 " },
        { kind: "match", text: "pizza" },
      ],
    });
  });

  it("leaves the message as parsed without ranges", () => {
    const content = "> quoted\nplain";
    expect(marked(content, [])).toEqual(parseMessageMarkdown(content, NOBODY));
  });
});

describe("search results", () => {
  const hit = (id: string, channelId: string) => ({
    message: {
      id,
      channel_id: channelId,
      author: { id: "1", username: "ada", display_name: "Ada" },
      content: "",
      reply_to_id: null,
      edited_at_unix_milliseconds: null,
    },
    highlights: [],
  });

  it("labels the first result of each run from one channel", () => {
    expect(searchResultRows([hit("1", "a"), hit("2", "a"), hit("3", "b"), hit("4", "a")]).map((row) => row.startsChannelRun)).toEqual([
      true,
      false,
      true,
      true,
    ]);
  });

  it("narrows the query to a facet's channel or author", () => {
    expect(queryNarrowedToChannel("pizza", { id: "9", name: "général" })).toBe("pizza in:général");
    expect(queryNarrowedToChannel("pizza", { id: "9", name: "Game night" })).toBe("pizza in:9");
    expect(queryNarrowedToChannel("pizza", { id: "9", name: null })).toBe("pizza in:9");
    expect(queryNarrowedToAuthor("pizza in:9", { username: "ada" })).toBe("pizza in:9 from:ada");
  });
});
