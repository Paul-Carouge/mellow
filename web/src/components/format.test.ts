import { describe, expect, it } from "vitest";

import type { PendingMessage } from "../lib/client.svelte";
import type { Guild, Message } from "../lib/protocol";
import {
  avatarInitial,
  messageTimestampLabel,
  serverInitials,
  splitMessageText,
  typingLabel,
  unixMillisecondsOfSnowflake,
} from "./format";
import { AUTHOR_GROUP_MAXIMUM_GAP_MILLISECONDS, buildMessageRows, channelSections } from "./message-rows";
import { parseDurationInMilliseconds, parseRemLength } from "./tokens";

const MELLOW_EPOCH = 1_767_225_600_000;

function snowflakeAt(unixMilliseconds: number, sequence = 0): string {
  return ((BigInt(unixMilliseconds - MELLOW_EPOCH) << 22n) | (BigInt(sequence) << 12n)).toString();
}

function message(id: string, authorId: string, replyToId: string | null = null): Message {
  return {
    id,
    channel_id: "1",
    author: { id: authorId, username: `user${authorId}`, display_name: "" },
    content: "hello",
    reply_to_id: replyToId,
    edited_at_unix_milliseconds: null,
  };
}

describe("unixMillisecondsOfSnowflake", () => {
  it("reads the timestamp bits above the sequence", () => {
    const sentAt = MELLOW_EPOCH + 123_456_789;
    expect(unixMillisecondsOfSnowflake(snowflakeAt(sentAt, 1023))).toBe(sentAt);
  });
});

describe("messageTimestampLabel", () => {
  const now = new Date(2026, 9, 21, 15, 0);

  it("says today and yesterday by local calendar day", () => {
    expect(messageTimestampLabel(new Date(2026, 9, 21, 0, 5).getTime(), now, "en-GB")).toBe("Today at 00:05");
    expect(messageTimestampLabel(new Date(2026, 9, 20, 23, 59).getTime(), now, "en-GB")).toBe(
      "Yesterday at 23:59",
    );
  });

  it("gives the date for older messages", () => {
    expect(messageTimestampLabel(new Date(2026, 9, 19, 8, 30).getTime(), now, "en-GB")).toBe("19/10/2026, 08:30");
  });
});

describe("serverInitials and avatarInitial", () => {
  it("takes the first letter of up to three words", () => {
    expect(serverInitials("  Rust   Lyon ")).toBe("RL");
    expect(serverInitials("a b c d")).toBe("abc");
    expect(serverInitials("Élan")).toBe("É");
  });

  it("falls back to the username when the display name is blank", () => {
    expect(avatarInitial({ display_name: " ", username: "zoé" })).toBe("Z");
  });
});

describe("typingLabel", () => {
  it("follows the DESIGN.md wording from one person to several", () => {
    expect(typingLabel([])).toBe("");
    expect(typingLabel(["Ana"])).toBe("Ana is typing…");
    expect(typingLabel(["Ana", "Léo"])).toBe("Ana and Léo are typing…");
    expect(typingLabel(["Ana", "Léo", "Zoé"])).toBe("Ana, Léo and 1 other are typing…");
    expect(typingLabel(["A", "B", "C", "D", "E"])).toBe("A, B and 3 others are typing…");
    expect(typingLabel(["A", "B", "C", "D", "E", "F"])).toBe("Several people are typing…");
  });
});

describe("splitMessageText", () => {
  it("makes http and https links, leaving sentence punctuation outside", () => {
    expect(splitMessageText("See https://example.org/a?b=1. Thanks")).toEqual([
      { kind: "text", text: "See " },
      { kind: "link", text: "https://example.org/a?b=1", url: "https://example.org/a?b=1" },
      { kind: "text", text: ". Thanks" },
    ]);
  });

  it("never links other schemes", () => {
    const text = "javascript:alert(1) data:text/html,x ftp://example.org";
    expect(splitMessageText(text)).toEqual([{ kind: "text", text }]);
  });

  it("keeps markup as text", () => {
    expect(splitMessageText('<img src=x onerror="y">')).toEqual([{ kind: "text", text: '<img src=x onerror="y">' }]);
  });
});

describe("buildMessageRows", () => {
  const start = MELLOW_EPOCH + 1_000_000;

  it("groups consecutive messages of one author within seven minutes", () => {
    const rows = buildMessageRows(
      [
        message(snowflakeAt(start), "1"),
        message(snowflakeAt(start + 60_000), "1"),
        message(snowflakeAt(start + 60_000 + AUTHOR_GROUP_MAXIMUM_GAP_MILLISECONDS + 1), "1"),
        message(snowflakeAt(start + 9_000_000), "2"),
        message(snowflakeAt(start + 9_000_001), "2", "5"),
      ],
      [],
      "1",
      start,
    );
    expect(rows.map((row) => row.startsAuthorGroup)).toEqual([true, false, true, true, true]);
  });

  it("puts the outbox last, continuing the user's own group", () => {
    const pending: PendingMessage = {
      nonce: "n1",
      channelId: "1",
      content: "hi",
      replyToId: null,
      status: "sending",
    };
    const rows = buildMessageRows([message(snowflakeAt(start), "1")], [pending], "1", start + 1_000);
    expect(rows.map((row) => [row.kind, row.startsAuthorGroup])).toEqual([
      ["sent", true],
      ["pending", false],
    ]);
  });
});

describe("channelSections", () => {
  it("lists uncategorised channels first, then categories by position", () => {
    const channel = (id: string, kind: "text" | "category", position: number, parentId: string | null = null) => ({
      id,
      guild_id: "g",
      kind,
      name: id,
      topic: "",
      parent_id: parentId,
      position,
      permission_overwrites: [],
    });
    const guild: Guild = {
      id: "g",
      name: "G",
      owner_id: "1",
      my_permissions: "0",
      my_role_ids: [],
      roles: [],
      channels: [
        channel("20", "category", 1),
        channel("10", "category", 0),
        channel("21", "text", 0, "20"),
        channel("3", "text", 1),
        channel("2", "text", 0),
      ],
    };
    expect(
      channelSections(guild).map((section) => [section.category?.id ?? null, section.channels.map((c) => c.id)]),
    ).toEqual([
      [null, ["2", "3"]],
      ["10", []],
      ["20", ["21"]],
    ]);
  });
});

describe("token parsing", () => {
  it("reads generated rem lengths and durations", () => {
    expect(parseRemLength("4.5rem")).toBe(4.5);
    expect(parseRemLength(" -1rem")).toBe(-1);
    expect(parseDurationInMilliseconds("2000ms")).toBe(2000);
    expect(parseDurationInMilliseconds("0ms")).toBe(0);
    expect(() => parseRemLength("12px")).toThrow();
  });
});
