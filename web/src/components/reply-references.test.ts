import { describe, expect, it } from "vitest";

import { strings } from "../i18n";
import type { Message, PublicUser } from "../lib/protocol";
import { createReplyReferenceLookup } from "./reply-references";

const ana: PublicUser = { id: "1", username: "ana", display_name: "Ana" };
const bob: PublicUser = { id: "2", username: "bob", display_name: "Bob" };

function message(id: string, author: PublicUser, content: string, replyToId: string | null = null): Message {
  return { id, channel_id: "1", author, content, reply_to_id: replyToId, edited_at_unix_milliseconds: null };
}

const byId = (messages: Message[]) => new Map(messages.map((each) => [each.id, each]));
const nobodyBlocked = () => false;

describe("createReplyReferenceLookup", () => {
  const original = message("10", ana, "hello");
  const reply = message("11", bob, "hi", "10");

  it("quotes the original's author and text", () => {
    const lookup = createReplyReferenceLookup();
    expect(lookup(reply, byId([original, reply]), nobodyBlocked)).toEqual({ author: ana, text: "hello" });
    expect(lookup(original, byId([original, reply]), nobodyBlocked)).toBeNull();
  });

  it("returns the same object while the original stays the same", () => {
    const lookup = createReplyReferenceLookup();
    const first = lookup(reply, byId([original, reply]), nobodyBlocked);
    // A new message elsewhere rebuilds the map but keeps the original.
    const later = lookup(reply, byId([original, reply, message("12", ana, "more")]), nobodyBlocked);
    expect(later).toBe(first);
  });

  it("quotes the new text once the original is edited", () => {
    const lookup = createReplyReferenceLookup();
    const first = lookup(reply, byId([original, reply]), nobodyBlocked);
    const edited = { ...original, content: "hello again" };
    const afterEdit = lookup(reply, byId([edited, reply]), nobodyBlocked);
    expect(afterEdit).not.toBe(first);
    expect(afterEdit?.text).toBe("hello again");
  });

  it("hides a blocked author, and quotes them again once unblocked", () => {
    const lookup = createReplyReferenceLookup();
    const messages = byId([original, reply]);
    const quoted = lookup(reply, messages, nobodyBlocked);
    expect(lookup(reply, messages, (userId) => userId === ana.id)).toEqual({ author: null, text: strings.messages.blockedReply });
    expect(lookup(reply, messages, nobodyBlocked)).toBe(quoted);
  });

  it("says the original is unavailable when it is not loaded", () => {
    const lookup = createReplyReferenceLookup();
    expect(lookup(reply, byId([reply]), nobodyBlocked)).toEqual({ author: null, text: strings.messages.originalUnavailable });
  });
});
