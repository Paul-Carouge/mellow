import { describe, expect, it } from "vitest";

import type { PendingMessage } from "../lib/client.svelte";
import type { Guild, Message } from "../lib/protocol";
import {
  attachmentCountLabel,
  avatarInitial,
  avatarTintToken,
  fileNameStartAndEnd,
  humanFileSize,
  mentionBadgeText,
  messageTimestampLabel,
  middleTruncate,
  nameWithUnreadState,
  serverInitials,
  timeOfDayLabel,
  typingLabel,
  unixMillisecondsOfSnowflake,
} from "./format";
import {
  AUTHOR_GROUP_MAXIMUM_GAP_MILLISECONDS,
  buildMessageRows,
  channelSections,
  unreadBarLabel,
  unreadSummary,
} from "./message-rows";
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

describe("avatarTintToken", () => {
  it("keeps one tint per person and spreads neighbouring ids over the palette", () => {
    expect(avatarTintToken("106689215642730496")).toBe(avatarTintToken("106689215642730496"));
    const ids = Array.from({ length: 64 }, (_, index) => (106689215642730496n + BigInt(index) * 4096n).toString());
    const tints = new Set(ids.map(avatarTintToken));
    expect(tints.size).toBe(8);
    for (const tint of tints) {
      expect(tint).toMatch(/^--mw-color-avatar-tint-[1-8]$/u);
    }
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

describe("humanFileSize", () => {
  it("counts bytes, then binary kilobytes and megabytes with one decimal at most", () => {
    expect(humanFileSize(0, "en-US")).toBe("0 bytes");
    expect(humanFileSize(1, "en-US")).toBe("1 byte");
    expect(humanFileSize(1023, "en-US")).toBe("1023 bytes");
    expect(humanFileSize(1024, "en-US")).toBe("1 KB");
    expect(humanFileSize(2_516_582, "en-US")).toBe("2.4 MB");
    expect(humanFileSize(25 * 1024 * 1024, "en-US")).toBe("25 MB");
    expect(humanFileSize(3 * 1024 ** 3, "en-US")).toBe("3 GB");
  });

  it("moves to the next unit instead of showing 1024 of the previous one", () => {
    expect(humanFileSize(1024 * 1024 - 1, "en-US")).toBe("1 MB");
  });

  it("uses the reader's decimal separator", () => {
    expect(humanFileSize(2_516_582, "fr-FR")).toBe("2,4 MB");
  });
});

describe("fileNameStartAndEnd", () => {
  it("keeps the extension and a few characters before it as the end", () => {
    expect(fileNameStartAndEnd("compte-rendu-version-finale.txt")).toEqual({
      start: "compte-rendu-version-fi",
      end: "nale.txt",
    });
  });

  it("keeps the last characters of a name without a recognisable extension", () => {
    expect(fileNameStartAndEnd("archive.backup-2026")).toEqual({ start: "archive.backup-", end: "2026" });
    expect(fileNameStartAndEnd("README")).toEqual({ start: "RE", end: "ADME" });
  });

  it("always leaves something to cut and never splits a character", () => {
    expect(fileNameStartAndEnd("a.txt")).toEqual({ start: "a", end: ".txt" });
    expect(fileNameStartAndEnd("😀😀😀😀😀😀.png")).toEqual({ start: "😀😀", end: "😀😀😀😀.png" });
  });
});

describe("middleTruncate", () => {
  it("leaves names that fit unchanged", () => {
    expect(middleTruncate("report.pdf", 20)).toBe("report.pdf");
    expect(middleTruncate("exactly-twenty-chars", 20)).toBe("exactly-twenty-chars");
  });

  it("cuts the middle and keeps the extension readable", () => {
    const shortened = middleTruncate("holiday-photos-from-summer.jpg", 20);
    expect(shortened).toBe("holiday-photo…er.jpg");
    expect(Array.from(shortened)).toHaveLength(20);
  });

  it("does not split characters outside the basic plane", () => {
    expect(middleTruncate("😀😀😀😀😀😀😀😀😀😀😀😀.png", 10)).toBe("😀😀😀…😀😀.png");
  });

  it("treats a long ending as part of the name", () => {
    expect(middleTruncate("archive.backup-of-everything", 12)).toBe("archive.…ing");
  });
});

describe("attachmentCountLabel", () => {
  it("names the count, singular and plural, and nothing for none", () => {
    expect(attachmentCountLabel(0)).toBe("");
    expect(attachmentCountLabel(1)).toBe("1 attachment");
    expect(attachmentCountLabel(2)).toBe("2 attachments");
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

  it("marks the first message from others after the unread marker, which starts a group", () => {
    const ids = [0, 60_000, 120_000, 180_000].map((offset) => snowflakeAt(start + offset));
    const rows = buildMessageRows(
      [message(ids[0] ?? "", "2"), message(ids[1] ?? "", "1"), message(ids[2] ?? "", "1"), message(ids[3] ?? "", "1")],
      [],
      "2",
      start,
      ids[0] ?? null,
    );
    expect(rows.map((row) => row.kind === "sent" && row.isFirstUnread)).toEqual([false, true, false, false]);
    expect(rows.map((row) => row.startsAuthorGroup)).toEqual([true, true, false, false]);
  });

  it("puts the outbox last, continuing the user's own group", () => {
    const pending: PendingMessage = {
      nonce: "n1",
      channelId: "1",
      content: "hi",
      replyToId: null,
      attachments: [],
      status: "sending",
    };
    const rows = buildMessageRows([message(snowflakeAt(start), "1")], [pending], "1", start + 1_000);
    expect(rows.map((row) => [row.kind, row.startsAuthorGroup])).toEqual([
      ["sent", true],
      ["pending", false],
    ]);
  });
});

describe("unreadSummary and unreadBarLabel", () => {
  const start = MELLOW_EPOCH + 1_000_000;
  const ids = [0, 1, 2, 3, 4].map((minute) => snowflakeAt(start + minute * 60_000));
  const id = (index: number) => ids[index] ?? "";

  it("counts only what others wrote between the last read message and the visit's opening", () => {
    const messages = [message(id(0), "2"), message(id(1), "2"), message(id(2), "1"), message(id(3), "2"), message(id(4), "2")];
    // Read up to 0; 1 to 3 were there at open (2 is the user's own); 4 arrived live.
    const summary = unreadSummary(messages, { lastReadMessageId: id(0), newestAtOpenMessageId: id(3) }, "1", true);
    expect(summary).toEqual({ count: 2, isPartial: false });
  });

  it("says 50+ when older unread messages are not loaded, and dates the label from the marker", () => {
    const loaded = [message(id(2), "2"), message(id(3), "2")];
    const marker = { lastReadMessageId: id(0), newestAtOpenMessageId: id(3) };
    const summary = unreadSummary(loaded, marker, "1", false);
    expect(summary).toEqual({ count: 2, isPartial: true });
    const now = new Date(start + 10 * 60_000);
    expect(unreadBarLabel(summary, start, now)).toBe(`2+ new messages since today at ${timeOfDayLabel(start)}`);
    expect(unreadBarLabel({ count: 1, isPartial: false }, start, now)).toBe(
      `1 new message since today at ${timeOfDayLabel(start)}`,
    );
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

  it("lists voice channels among the text ones, by position", () => {
    const channel = (id: string, kind: "text" | "voice" | "category", position: number, parentId: string | null = null) => ({
      id,
      guild_id: "g",
      kind,
      name: id,
      topic: "",
      parent_id: parentId,
      position,
      permission_overwrites: [],
    });
    const guild = {
      channels: [channel("10", "category", 0), channel("12", "text", 1, "10"), channel("11", "voice", 0, "10"), channel("13", "voice", 2, "10")],
    };
    expect(channelSections(guild).map((section) => section.channels.map((c) => c.id))).toEqual([["11", "12", "13"]]);
  });

  it("lists a channel of a hidden category with the uncategorised ones, keeping its parent", () => {
    const hiddenParent = {
      id: "30",
      guild_id: "g",
      kind: "text" as const,
      name: "30",
      topic: "",
      parent_id: "99",
      position: 2,
      permission_overwrites: [],
    };
    const [section] = channelSections({ channels: [hiddenParent] });
    expect(section?.category).toBeNull();
    expect(section?.channels[0]?.parent_id).toBe("99");
  });
});

describe("token parsing", () => {
  it("reads generated rem lengths and durations", () => {
    expect(parseRemLength("4.5rem")).toBe(4.5);
    expect(parseRemLength(" -1rem")).toBe(-1);
    expect(parseDurationInMilliseconds("2000ms")).toBe(2000);
    expect(parseDurationInMilliseconds("0ms")).toBe(0);
    // As Chrome serialises computed custom properties.
    expect(parseDurationInMilliseconds(".3s")).toBe(300);
    expect(parseRemLength(".5rem")).toBe(0.5);
    expect(() => parseRemLength("12px")).toThrow();
  });
});

describe("mention badges", () => {
  it("count up to 99, then say 99+ as the server stops counting", () => {
    expect(mentionBadgeText(1)).toBe("1");
    expect(mentionBadgeText(99)).toBe("99");
    expect(mentionBadgeText(100)).toBe("99+");
  });

  it("name a row with its unread state and its mentions", () => {
    expect(nameWithUnreadState("general", false, 0)).toBe("general");
    expect(nameWithUnreadState("general", true, 0)).toBe("general, unread");
    expect(nameWithUnreadState("general", true, 2)).toBe("general, 2 mentions, unread");
  });
});
