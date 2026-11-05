import { describe, expect, it } from "vitest";

import { strings } from "../i18n";
import type { Message, PublicUser, Role } from "../lib/protocol";
import { createReplyReferenceLookup } from "./reply-references";

const ana: PublicUser = { id: "1", username: "ana", display_name: "Ana" };
const bob: PublicUser = { id: "2", username: "bob", display_name: "Bob" };

function message(id: string, author: PublicUser, content: string, replyToId: string | null = null): Message {
  return { id, channel_id: "1", author, content, reply_to_id: replyToId, edited_at_unix_milliseconds: null };
}

const byId = (messages: Message[]) => new Map(messages.map((each) => [each.id, each]));
const nobodyBlocked = () => false;
const NO_ROLES = new Map<string, Role>();

describe("createReplyReferenceLookup", () => {
  const original = message("10", ana, "hello");
  const reply = message("11", bob, "hi", "10");

  it("quotes the original's author and text", () => {
    const lookup = createReplyReferenceLookup();
    expect(lookup(reply, byId([original, reply]), nobodyBlocked, NO_ROLES)).toEqual({ author: ana, text: "hello" });
    expect(lookup(original, byId([original, reply]), nobodyBlocked, NO_ROLES)).toBeNull();
  });

  it("quotes a poll by its question (ADR-0043)", () => {
    const poll = { question: "Lunch?", answers: [], allows_multiple_answers: false, expires_at_unix_milliseconds: 0 };
    const pollMessage = { ...message("10", ana, ""), poll };
    const lookup = createReplyReferenceLookup();
    expect(lookup(reply, byId([pollMessage, reply]), nobodyBlocked, NO_ROLES)?.text).toBe(strings.polls.summary("Lunch?"));
  });

  it("returns the same object while the original stays the same", () => {
    const lookup = createReplyReferenceLookup();
    const first = lookup(reply, byId([original, reply]), nobodyBlocked, NO_ROLES);
    // A new message elsewhere rebuilds the map but keeps the original.
    const later = lookup(reply, byId([original, reply, message("12", ana, "more")]), nobodyBlocked, NO_ROLES);
    expect(later).toBe(first);
  });

  it("quotes the new text once the original is edited", () => {
    const lookup = createReplyReferenceLookup();
    const first = lookup(reply, byId([original, reply]), nobodyBlocked, NO_ROLES);
    const edited = { ...original, content: "hello again" };
    const afterEdit = lookup(reply, byId([edited, reply]), nobodyBlocked, NO_ROLES);
    expect(afterEdit).not.toBe(first);
    expect(afterEdit?.text).toBe("hello again");
  });

  it("hides a blocked author, and quotes them again once unblocked", () => {
    const lookup = createReplyReferenceLookup();
    const messages = byId([original, reply]);
    const quoted = lookup(reply, messages, nobodyBlocked, NO_ROLES);
    expect(lookup(reply, messages, (userId) => userId === ana.id, NO_ROLES)).toEqual({ author: null, text: strings.messages.blockedReply });
    expect(lookup(reply, messages, nobodyBlocked, NO_ROLES)).toBe(quoted);
  });

  it("says the original is unavailable when it is not loaded", () => {
    const lookup = createReplyReferenceLookup();
    expect(lookup(reply, byId([reply]), nobodyBlocked, NO_ROLES)).toEqual({ author: null, text: strings.messages.originalUnavailable });
  });

  it("names a mentioned role, and words the quote again once the role is renamed", () => {
    const moderators: Role = { id: "300", name: "Moderators", color: 0, position: 1, permissions: "0", is_displayed_separately: false };
    const mentioning = message("12", ana, "ping <@&300>");
    const answer = message("13", bob, "ok", "12");
    const messages = byId([mentioning, answer]);
    const lookup = createReplyReferenceLookup();
    expect(lookup(answer, messages, nobodyBlocked, new Map([[moderators.id, moderators]]))?.text).toBe("ping @Moderators");
    const renamed = { ...moderators, name: "Mods" };
    expect(lookup(answer, messages, nobodyBlocked, new Map([[renamed.id, renamed]]))?.text).toBe("ping @Mods");
  });
});
