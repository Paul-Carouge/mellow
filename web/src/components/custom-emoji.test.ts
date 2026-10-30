import { describe, expect, it } from "vitest";

import { ApiError } from "../lib/api";
import { type CustomEmoji, customEmojiReactionKey } from "../lib/protocol";
import {
  MAXIMUM_EMOJI_SUGGESTIONS,
  composerTextOf,
  emojiCounts,
  emojiFileProblem,
  emojiNameFromFileName,
  emojiRenameProblem,
  emojiSuggestionQuery,
  emojiUploadFailureReason,
  isJumboEmojiMessage,
  reactionAppearance,
  replaceInDraft,
  suggestEmoji,
  withCustomEmojiAsNames,
  withEmojiNamesAsMarkup,
} from "./custom-emoji";
import { parseMessageMarkdown } from "./markdown";

const party: CustomEmoji = { id: "500", name: "party", is_animated: false };
const parrot: CustomEmoji = { id: "501", name: "party_parrot", is_animated: true };
const wave: CustomEmoji = { id: "502", name: "WaveHello", is_animated: false };
const known = new Map([party, parrot, wave].map((emoji) => [emoji.id, emoji]));

describe("the suggestion query", () => {
  it("opens after a colon and two name characters, at the start or after a space", () => {
    expect(emojiSuggestionQuery(":pa", 3)).toEqual({ start: 0, end: 3, query: "pa" });
    expect(emojiSuggestionQuery("hi :party_p", 11)).toEqual({ start: 3, end: 11, query: "party_p" });
    expect(emojiSuggestionQuery("hi\n:Wa", 6)).toEqual({ start: 3, end: 6, query: "Wa" });
    // Only what is before the caret counts.
    expect(emojiSuggestionQuery(":party rest", 4)).toEqual({ start: 0, end: 4, query: "par" });
  });

  it("stays closed for one character, times, URLs and finished names", () => {
    for (const [text, caret] of [
      [":p", 2],
      ["at 12:30", 8],
      ["https://example.org", 6],
      ["a:bc", 4],
      [":party: ", 8],
      [`:${"a".repeat(33)}`, 34],
    ] as const) {
      expect(emojiSuggestionQuery(text, caret), text).toBeNull();
    }
  });
});

describe("suggestions", () => {
  it("put the server's emoji first, those starting with the query before those holding it", () => {
    const suggestions = suggestEmoji("par", [wave, parrot, { id: "9", name: "sparkles", is_animated: false }, party]);
    expect(suggestions.slice(0, 3)).toEqual([
      { kind: "custom", emoji: parrot },
      { kind: "custom", emoji: party },
      { kind: "custom", emoji: { id: "9", name: "sparkles", is_animated: false } },
    ]);
    expect(suggestions.slice(3).every((suggestion) => suggestion.kind === "unicode")).toBe(true);
  });

  it("ignore case, add built-in emoji by their words and stop at ten", () => {
    expect(suggestEmoji("WAVE", [wave])[0]).toEqual({ kind: "custom", emoji: wave });
    expect(suggestEmoji("thumbs_up", [])).toContainEqual({ kind: "unicode", emoji: "👍", name: "thumbs" });
    expect(suggestEmoji("sm", []).length).toBeLessThanOrEqual(MAXIMUM_EMOJI_SUGGESTIONS);
    expect(suggestEmoji("zzzzzz", [party])).toEqual([]);
  });

  it("write a server emoji as its name and a built-in one as itself, then a space", () => {
    expect(composerTextOf(parrot)).toBe(":party_parrot:");
    expect(composerTextOf("👍")).toBe("👍");
    expect(replaceInDraft("hi :par", 3, 7, ":party:")).toEqual({ text: "hi :party: ", caret: 11 });
    expect(replaceInDraft("hi :par there", 3, 7, ":party:")).toEqual({ text: "hi :party: there", caret: 11 });
  });
});

describe("typed names on send", () => {
  it("become the markup of the server's emoji, whatever the case", () => {
    expect(withEmojiNamesAsMarkup("so :party: :PARTY_PARROT:!", [party, parrot])).toBe(
      "so <:party:500> <a:party_parrot:501>!",
    );
    expect(withEmojiNamesAsMarkup(":party::party:", [party])).toBe("<:party:500><:party:500>");
    expect(withEmojiNamesAsMarkup(":nope:party:", [party])).toBe(":nope<:party:500>");
  });

  it("stay as typed in code, escaped, unknown or already written as markup", () => {
    const unchanged = [
      "`:party:`",
      "```\n:party:\n```",
      "``a :party: b``",
      "\\:party:",
      ":unknown:",
      "<:party:500>",
      "at 12:30:45",
    ];
    for (const text of unchanged) {
      expect(withEmojiNamesAsMarkup(text, [party]), text).toBe(text);
    }
    expect(withEmojiNamesAsMarkup("`x` :party: `y`", [party])).toBe("`x` <:party:500> `y`");
    expect(withEmojiNamesAsMarkup("\\\\:party:", [party])).toBe("\\\\<:party:500>");
    expect(withEmojiNamesAsMarkup(":party:", [])).toBe(":party:");
  });

  it("find code as the renderer does, escaped and unclosed backticks being text", () => {
    // Escaped backticks open no code: the message shows the picture between them.
    expect(withEmojiNamesAsMarkup("\\`:party:\\`", [party])).toBe("\\`<:party:500>\\`");
    expect(withEmojiNamesAsMarkup("`unclosed :party:", [party])).toBe("`unclosed <:party:500>");
    // A run of two backticks is closed only by another run of two.
    expect(withEmojiNamesAsMarkup("``a ` :party: ``", [party])).toBe("``a ` :party: ``");
    for (const text of ["\\`:party:\\`", "``a ` :party: ``"]) {
      const shownAsCode = parseMessageMarkdown(text, new Map(), known).some(
        (block) => block.kind === "paragraph" && block.children.some((node) => node.kind === "code"),
      );
      expect(shownAsCode, text).toBe(withEmojiNamesAsMarkup(text, [party]) === text);
    }
  });

  it("read back as names in plain text", () => {
    expect(withCustomEmojiAsNames("hi <:party:500> <a:party_parrot:501>")).toBe("hi :party: :party_parrot:");
  });
});

describe("jumbo emoji", () => {
  const isJumbo = (text: string) => isJumboEmojiMessage(parseMessageMarkdown(text, new Map(), known));

  it("are messages made only of emoji and spaces", () => {
    expect(isJumbo("<:party:500>")).toBe(true);
    expect(isJumbo("<:party:500> <a:party_parrot:501>")).toBe(true);
    expect(isJumbo("👍 ❤️ <:party:500>")).toBe(true);
    expect(isJumbo("👨‍👩‍👧 🇫🇷 1️⃣")).toBe(true);
    expect(isJumbo("<:party:500>".repeat(30))).toBe(true);
  });

  it("are not with text, formatting, an unknown emoji, more than 30 or several blocks", () => {
    for (const text of [
      "hi <:party:500>",
      "**<:party:500>**",
      "<:gone:999>",
      "<:party:500>".repeat(31),
      "<:party:500>\n\n> <:party:500>",
      "123",
      "©",
      "",
    ]) {
      expect(isJumbo(text), text).toBe(false);
    }
  });
});

describe("uploads", () => {
  it("name a picture after its file, unique whatever the case", () => {
    expect(emojiNameFromFileName("party parrot.gif", [])).toBe("party_parrot");
    expect(emojiNameFromFileName("Café-crème!.png", [])).toBe("Cafe_creme");
    expect(emojiNameFromFileName("a.png", [])).toBe("a_");
    expect(emojiNameFromFileName("😀.png", [])).toBe("emoji");
    expect(emojiNameFromFileName(`${"x".repeat(40)}.png`, [])).toBe("x".repeat(32));
    expect(emojiNameFromFileName("party.png", ["PARTY", "party_2"])).toBe("party_3");
    expect(emojiNameFromFileName(`${"x".repeat(40)}.png`, ["x".repeat(32)])).toBe(`${"x".repeat(30)}_2`);
  });

  it("count still and animated emoji apart", () => {
    expect(emojiCounts([party, parrot, wave])).toEqual({ still: 2, animated: 1 });
  });

  it("refuse other files, files over 256 KB, and a kind with no room left", () => {
    const empty = { still: 0, animated: 0 };
    expect(emojiFileProblem({ type: "image/png", size: 1000 }, empty)).toBeNull();
    expect(emojiFileProblem({ type: "image/svg+xml", size: 1000 }, empty)).toBe("not-a-picture");
    expect(emojiFileProblem({ type: "", size: 1000 }, empty)).toBe("not-a-picture");
    expect(emojiFileProblem({ type: "image/gif", size: 256 * 1024 + 1 }, empty)).toBe("too-large");
    const stillFull = { still: 50, animated: 3 };
    expect(emojiFileProblem({ type: "image/jpeg", size: 1000 }, stillFull)).toBe("no-room");
    // A GIF may be animated: the instance decides.
    expect(emojiFileProblem({ type: "image/gif", size: 1000 }, stillFull)).toBeNull();
    expect(emojiFileProblem({ type: "image/gif", size: 1000 }, { still: 50, animated: 50 })).toBe("no-room");
  });

  it("say why the instance refused a picture", () => {
    const file = { size: 300 * 1024 };
    expect(emojiUploadFailureReason(new ApiError(413, "FILE_TOO_LARGE"), file)).toBe(
      "it weighs 300 KB, and emoji are at most 256 KB.",
    );
    expect(emojiUploadFailureReason(new ApiError(409, "LIMIT_REACHED"), file)).toMatch(/no room left/u);
    expect(emojiUploadFailureReason(new ApiError(400, "VALIDATION_FAILED"), file)).toMatch(/couldn't use this picture/u);
    expect(emojiUploadFailureReason(new ApiError(429, "RATE_LIMITED"), file)).toMatch(/Wait a few minutes/u);
    expect(emojiUploadFailureReason(new ApiError(403, "MISSING_PERMISSIONS"), file)).toBe(
      "you don't have the permission to do that here.",
    );
  });
});

describe("renaming", () => {
  it("needs a valid name no other emoji of the server has, whatever the case", () => {
    expect(emojiRenameProblem("fiesta", party.id, [party, parrot])).toBeNull();
    expect(emojiRenameProblem("Party", party.id, [party, parrot])).toBeNull();
    expect(emojiRenameProblem("PARTY_parrot", party.id, [party, parrot])).toBe("taken");
    expect(emojiRenameProblem("a", party.id, [party])).toBe("invalid");
    expect(emojiRenameProblem("two words", party.id, [party])).toBe("invalid");
  });
});

describe("reactions", () => {
  it("show a server emoji by id under its current name, a missing one as deleted", () => {
    expect(reactionAppearance("👍", known)).toEqual({ kind: "unicode", emoji: "👍", label: "👍" });
    expect(reactionAppearance(customEmojiReactionKey(wave), known)).toEqual({
      kind: "custom",
      emoji: wave,
      label: ":WaveHello:",
    });
    expect(reactionAppearance(":999", known)).toEqual({ kind: "deleted", label: "a deleted emoji" });
  });
});
