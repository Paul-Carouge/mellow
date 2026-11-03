import { describe, expect, it } from "vitest";

import type { PublicUser } from "../lib/protocol";
import {
  MAXIMUM_MENTION_SUGGESTIONS,
  memoizedMentionablePeople,
  mentionablePeople,
  mentionSuggestionQuery,
  splitMentions,
  suggestMentions,
} from "./mentions";

const paul: PublicUser = { id: "1", username: "paul", display_name: "Paul Carouge" };
const zoe: PublicUser = { id: "2", username: "zoe", display_name: "Zoé" };
const dotted: PublicUser = { id: "3", username: "j.doe", display_name: "" };
const people = mentionablePeople([paul, zoe, dotted]);

describe("splitMentions", () => {
  it("finds usernames and one-word display names, in any case", () => {
    expect(splitMentions("@Paul can you review, @Zoé?", people)).toEqual([
      { kind: "mention", text: "@Paul", user: paul },
      { kind: "text", text: " can you review, " },
      { kind: "mention", text: "@Zoé", user: zoe },
      { kind: "text", text: "?" },
    ]);
  });

  it("gives a display name two people share to no one, but keeps usernames", () => {
    const eve: PublicUser = { id: "5", username: "eve", display_name: "Twin" };
    const mallory: PublicUser = { id: "6", username: "mallory", display_name: "twin" };
    const twins = mentionablePeople([eve, mallory]);
    expect(splitMentions("@twin @eve", twins)).toEqual([
      { kind: "text", text: "@twin " },
      { kind: "mention", text: "@eve", user: eve },
    ]);
  });

  it("leaves trailing dots, unknown names and addresses as text", () => {
    expect(splitMentions("thanks @j.doe.", people)).toEqual([
      { kind: "text", text: "thanks " },
      { kind: "mention", text: "@j.doe", user: dotted },
      { kind: "text", text: "." },
    ]);
    expect(splitMentions("write to paul@example.org or @nobody", people)).toEqual([
      { kind: "text", text: "write to paul@example.org or @nobody" },
    ]);
  });
});

describe("splitMentions on hostile text", () => {
  // The candidate pattern is bounded and must not be followed by a name character, so these runs
  // cost constant work by construction; a clock cannot check that deterministically.
  it("reads neither a long run of dots nor a name longer than any account's as a mention", () => {
    const dots = `@${".".repeat(4_000)}`;
    expect(splitMentions(dots, people)).toEqual([{ kind: "text", text: dots }]);
    expect(splitMentions(`@paul${"x".repeat(40)}`, people)).toEqual([{ kind: "text", text: `@paul${"x".repeat(40)}` }]);
  });
});

describe("memoizedMentionablePeople", () => {
  it("keeps the same map when a role change rebuilds the members around the same users", () => {
    const mentionable = memoizedMentionablePeople();
    const members = [{ user: paul, role_ids: [] as string[] }, { user: zoe, role_ids: [] as string[] }];
    const first = mentionable(members.map((member) => member.user));
    const afterRoleChange = members.map((member) => (member.user === zoe ? { ...member, role_ids: ["9"] } : member));
    expect(mentionable(afterRoleChange.map((member) => member.user))).toBe(first);
  });

  it("builds a new map when someone joins, leaves or changes their name", () => {
    const mentionable = memoizedMentionablePeople();
    const first = mentionable([paul, zoe]);
    const joined = mentionable([paul, zoe, dotted]);
    expect(joined).not.toBe(first);
    expect(joined.get("j.doe")).toBe(dotted);
    const left = mentionable([paul, zoe]);
    expect(left).not.toBe(joined);
    expect(left.has("j.doe")).toBe(false);
    const renamed = { ...zoe, username: "zoe2" };
    expect(mentionable([paul, renamed]).get("zoe2")).toBe(renamed);
  });
});

describe("mentionSuggestionQuery", () => {
  it("reads the name typed after an @ at the start or after a space", () => {
    expect(mentionSuggestionQuery("@pa", 3)).toEqual({ start: 0, end: 3, query: "pa" });
    expect(mentionSuggestionQuery("hi @Zo", 6)).toEqual({ start: 3, end: 6, query: "Zo" });
    expect(mentionSuggestionQuery("hi @", 4)).toEqual({ start: 3, end: 4, query: "" });
    expect(mentionSuggestionQuery("@j.do and more", 5)).toEqual({ start: 0, end: 5, query: "j.do" });
  });

  it("ignores an @ inside a word, after the caret moved past a space, or after a long name", () => {
    expect(mentionSuggestionQuery("mail@pa", 7)).toBeNull();
    expect(mentionSuggestionQuery("@pa ", 4)).toBeNull();
    expect(mentionSuggestionQuery(`@${"a".repeat(33)}`, 34)).toBeNull();
    expect(mentionSuggestionQuery(`@${"a".repeat(32)}`, 33)?.query).toHaveLength(32);
  });
});

describe("suggestMentions", () => {
  const ana: PublicUser = { id: "10", username: "ana", display_name: "Ana Paula" };
  const pablo: PublicUser = { id: "11", username: "pablo", display_name: "" };
  const everyone = [paul, zoe, dotted, ana, pablo];

  it("ranks names starting with the query, then display name words, then names holding it", () => {
    expect(suggestMentions("pa", everyone, "0").map((person) => person.id)).toEqual(["11", "1", "10"]);
    expect(suggestMentions("ul", everyone, "0").map((person) => person.id)).toEqual(["10", "1"]);
  });

  it("ignores case and accents, leaves out the writer and repeats no one", () => {
    expect(suggestMentions("ZOE", everyone, "0")).toEqual([zoe]);
    expect(suggestMentions("zoé", [zoe, zoe], "0")).toEqual([zoe]);
    expect(suggestMentions("pau", everyone, "1")).toEqual([ana]);
  });

  it("offers everyone for a lone @, by name, and at most a bounded number", () => {
    expect(suggestMentions("", [zoe, paul], "0")).toEqual([paul, zoe]);
    const crowd = Array.from({ length: 20 }, (_, index): PublicUser => ({
      id: String(100 + index),
      username: `user${index}`,
      display_name: "",
    }));
    expect(suggestMentions("user", crowd, "0")).toHaveLength(MAXIMUM_MENTION_SUGGESTIONS);
  });
});
