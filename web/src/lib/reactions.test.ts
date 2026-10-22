import { describe, expect, it } from "vitest";

import { EMOJI_CATALOGUE, searchEmoji } from "./emoji-catalogue";
import {
  MAXIMUM_RECENT_EMOJI,
  displayEmoji,
  quickReactions,
  reactionKey,
  readRecentEmoji,
  rememberEmoji,
  withReactionAdded,
  withReactionRemoved,
  withoutEmoji,
} from "./reactions";

const THUMBS = { emoji: "👍", count: 2, me: false };

describe("reaction counts", () => {
  it("add a new emoji at the end and count others' reactions each time", () => {
    expect(withReactionAdded([THUMBS], "❤", false)).toEqual([THUMBS, { emoji: "❤", count: 1, me: false }]);
    expect(withReactionAdded([THUMBS], "👍", false)).toEqual([{ emoji: "👍", count: 3, me: false }]);
  });

  it("count the user's own reaction once, so the event after their click changes nothing", () => {
    const clicked = withReactionAdded([THUMBS], "👍", true);
    expect(clicked).toEqual([{ emoji: "👍", count: 3, me: true }]);
    expect(withReactionAdded(clicked, "👍", true)).toEqual(clicked);
    const removed = withReactionRemoved(clicked, "👍", true);
    expect(removed).toEqual([THUMBS]);
    expect(withReactionRemoved(removed, "👍", true)).toEqual(removed);
  });

  it("drop an emoji at zero, and when a moderator removes it", () => {
    expect(withReactionRemoved([{ emoji: "👍", count: 1, me: false }], "👍", false)).toEqual([]);
    expect(withoutEmoji([THUMBS, { emoji: "❤", count: 1, me: true }], "👍")).toEqual([
      { emoji: "❤", count: 1, me: true },
    ]);
  });
});

describe("emoji keys", () => {
  it("drop variation selectors, as the instance stores them", () => {
    expect(reactionKey("❤️")).toBe("❤");
    expect(reactionKey("1️⃣")).toBe("1⃣");
    expect(reactionKey("👍")).toBe("👍");
  });

  it("show with the picker's spelling, or ask for the emoji look of a lone symbol", () => {
    expect(displayEmoji("❤")).toBe("❤️");
    expect(displayEmoji("1⃣")).toBe("1️⃣");
    expect(displayEmoji("☺")).toBe("☺️");
    expect(displayEmoji("👨‍💻")).toBe("👨‍💻");
  });
});

describe("the emoji catalogue", () => {
  it("holds about two hundred distinct emoji, each with words in both languages", () => {
    const keys = new Set(EMOJI_CATALOGUE.map((entry) => reactionKey(entry.emoji)));
    expect(keys.size).toBe(EMOJI_CATALOGUE.length);
    expect(EMOJI_CATALOGUE.length).toBeGreaterThanOrEqual(200);
    for (const entry of EMOJI_CATALOGUE) {
      expect(entry.keywords.length, entry.emoji).toBeGreaterThanOrEqual(2);
      // Every one is a single grapheme, as the instance requires.
      expect([...new Intl.Segmenter().segment(entry.emoji)], entry.emoji).toHaveLength(1);
    }
  });

  it("is searched in English and French, accents and case aside, exact words first", () => {
    expect(searchEmoji("heart").map((entry) => entry.emoji)).toContain("❤️");
    expect(searchEmoji("Coeur").map((entry) => entry.emoji)).toContain("❤️");
    expect(searchEmoji("cœur rouge").map((entry) => entry.emoji)).toEqual(["❤️"]);
    expect(searchEmoji("pouce")[0]?.emoji).toBe("👍");
    expect(searchEmoji("cat")[0]?.emoji).toBe("🐱");
    expect(searchEmoji("   ")).toEqual([]);
    expect(searchEmoji("zzzzqx")).toEqual([]);
  });
});

function memoryStorage(initial: Record<string, string> = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => void values.set(key, value),
  };
}

describe("recently used emoji", () => {
  it("put the latest first without duplicates, and keep a bounded list", () => {
    const storage = memoryStorage();
    rememberEmoji(storage, "👍");
    rememberEmoji(storage, "❤️");
    expect(rememberEmoji(storage, "❤")).toEqual(["❤", "👍"]);
    for (const entry of EMOJI_CATALOGUE.slice(0, 30)) {
      rememberEmoji(storage, entry.emoji);
    }
    expect(readRecentEmoji(storage)).toHaveLength(MAXIMUM_RECENT_EMOJI);
  });

  it("ignore what the browser holds if it is not a list of known emoji, or cannot be read", () => {
    expect(readRecentEmoji(memoryStorage({ "mellow.recent-emoji": "not json" }))).toEqual([]);
    expect(readRecentEmoji(memoryStorage({ "mellow.recent-emoji": '{"a":1}' }))).toEqual([]);
    expect(readRecentEmoji(memoryStorage({ "mellow.recent-emoji": '["👍", 3, "<b>", "lol"]' }))).toEqual(["👍"]);
    expect(readRecentEmoji(null)).toEqual([]);
    const blocked = {
      getItem: () => null,
      setItem: () => {
        throw new Error("quota");
      },
    };
    expect(rememberEmoji(blocked, "👍")).toEqual(["👍"]);
  });

  it("offer the latest three as quick reactions, then the defaults", () => {
    expect(quickReactions([])).toEqual(["👍", "❤️", "😂"]);
    expect(quickReactions(["🔥"])).toEqual(["🔥", "👍", "❤️"]);
    expect(quickReactions(["❤", "🎉", "🔥", "👀"])).toEqual(["❤", "🎉", "🔥"]);
  });
});
