import { describe, expect, it } from "vitest";

import type { PublicUser } from "../lib/protocol";
import { mentionablePeople, splitMentions } from "./mentions";

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
  it("stays linear on a long run of dots and ignores names longer than any account's", () => {
    const dots = `@${".".repeat(4_000)}`;
    const started = performance.now();
    for (let run = 0; run < 20; run += 1) {
      expect(splitMentions(dots, people)).toEqual([{ kind: "text", text: dots }]);
    }
    expect(performance.now() - started).toBeLessThan(20);
    expect(splitMentions(`@paul${"x".repeat(40)}`, people)).toEqual([{ kind: "text", text: `@paul${"x".repeat(40)}` }]);
  });
});
