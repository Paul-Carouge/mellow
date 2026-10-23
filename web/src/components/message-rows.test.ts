import { describe, expect, it } from "vitest";

import type { Message } from "../lib/protocol";
import { buildMessageRows, reuseUnchangedRows } from "./message-rows";

function message(id: string, content = `message ${id}`): Message {
  return {
    id,
    channel_id: "10",
    author: { id: "2", username: "other", display_name: "Other" },
    content,
    reply_to_id: null,
    edited_at_unix_milliseconds: null,
  };
}

describe("reuseUnchangedRows", () => {
  it("keeps unchanged rows and replaces edited ones and rows whose grouping changed", () => {
    const first = message("4194304000");
    const second = message("4194304001");
    const previous = buildMessageRows([first, second], [], "1", 0);
    const previousByKey = new Map(previous.map((row) => [row.key, row]));

    const edited = { ...second, content: "edited" };
    const next = reuseUnchangedRows(previousByKey, buildMessageRows([first, edited], [], "1", 0));
    expect(next[0]).toBe(previous[0]);
    expect(next[1]).not.toBe(previous[1]);

    // The unread divider above the second row starts a new author group there.
    const withDivider = reuseUnchangedRows(previousByKey, buildMessageRows([first, second], [], "1", 0, first.id));
    expect(withDivider[0]).toBe(previous[0]);
    expect(withDivider[1]).not.toBe(previous[1]);
  });
});
