import { describe, expect, it } from "vitest";

import type { PublicUser } from "../lib/protocol";
import { mentionablePeople, mentionsUser, splitMentions } from "./mentions";

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

describe("mentionsUser", () => {
  it("is true only when the user is mentioned", () => {
    expect(mentionsUser("@paul hello", people, "1")).toBe(true);
    expect(mentionsUser("@zoe hello", people, "1")).toBe(false);
  });
});
