import { describe, expect, it } from "vitest";

import type { GitHubIssueState, GitHubRepositoryEmbed } from "../../lib/protocol";
import { compactCount, issueStateBadge, languageColorOf, wikipediaLanguageName } from "./provider-cards";

const REPOSITORY: GitHubRepositoryEmbed = {
  owner: "rust-lang",
  name: "rust",
  stars: 104_238,
  forks: 13_612,
  topics: [],
  is_archived: false,
};

describe("compactCount", () => {
  it("rounds large counts to three digits in the reader's notation", () => {
    expect(compactCount(104_238, "en")).toBe("104K");
    expect(compactCount(13_612, "en")).toBe("13.6K");
    expect(compactCount(13_612, "fr")).toBe("13,6 k");
    expect(compactCount(1_234, "en")).toBe("1.23K");
    expect(compactCount(42, "en")).toBe("42");
  });
});

describe("languageColorOf", () => {
  it("takes Linguist's colour, then the instance's, and never a prototype's member", () => {
    expect(languageColorOf({ ...REPOSITORY, language: "Rust" })).toBe("#dea584");
    expect(languageColorOf({ ...REPOSITORY, language: "Brainfuck", language_color: 0x2f2530 })).toBe("#2f2530");
    expect(languageColorOf({ ...REPOSITORY, language: "constructor" })).toBeNull();
    expect(languageColorOf(REPOSITORY)).toBeNull();
  });
});

describe("issueStateBadge", () => {
  it("gives every state its own glyph, with draft the only outlined tone", () => {
    const states: GitHubIssueState[] = ["open", "draft", "merged", "completed", "not_planned", "closed"];
    expect(states.map((state) => issueStateBadge(state, true))).toEqual([
      { icon: "git-pull-request", tone: "open" },
      { icon: "git-pull-request", tone: "draft" },
      { icon: "git-merge", tone: "merged" },
      { icon: "check-circle", tone: "completed" },
      { icon: "x-circle", tone: "closed" },
      { icon: "x-circle", tone: "closed" },
    ]);
    expect(issueStateBadge("open", false)).toEqual({ icon: "record", tone: "open" });
  });
});

describe("wikipediaLanguageName", () => {
  it("names another wiki's language in the interface's, and none for its own", () => {
    expect(wikipediaLanguageName("fr", "en")).toBe("French");
    expect(wikipediaLanguageName("en", "fr")).toBe("anglais");
    expect(wikipediaLanguageName("fr", "fr")).toBeNull();
    expect(wikipediaLanguageName("simple", "en")).toBe("simple");
  });
});
