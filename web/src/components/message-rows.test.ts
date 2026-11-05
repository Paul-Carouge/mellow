import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

import type { PendingMessage } from "../lib/client.svelte";
import type { Message } from "../lib/protocol";
import {
  arrivalsSinceLeavingPresent,
  buildMessageRows,
  collapseBlockedRuns,
  localDayStart,
  reuseUnchangedRows,
} from "./message-rows";

const MELLOW_EPOCH = 1_767_225_600_000;

/** The id of a message sent at an instant written with its offset, e.g. "2026-11-04T23:58:00+01:00". */
function idSentAt(instant: string, sequence = 0): string {
  return ((BigInt(Date.parse(instant) - MELLOW_EPOCH) << 22n) | BigInt(sequence)).toString();
}

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

describe("collapseBlockedRuns", () => {
  const BLOCKED = "66";
  const OTHER_BLOCKED = "67";
  const by = (authorId: string, id: string): Message => ({
    ...message(id),
    author: { id: authorId, username: `user${authorId}`, display_name: "" },
  });
  const isBlocked = (userId: string) => userId === BLOCKED || userId === OTHER_BLOCKED;
  const shape = (entries: ReturnType<typeof collapseBlockedRuns>) =>
    entries.map((entry) => (entry.kind === "blocked-run" ? `run ${entry.messageIds.length}` : entry.key));
  const runIn = (entries: ReturnType<typeof collapseBlockedRuns>) => {
    const run = entries.find((entry) => entry.kind === "blocked-run");
    if (run?.kind !== "blocked-run") {
      throw new Error("no folded run");
    }
    return run;
  };
  const dividers = (entries: ReturnType<typeof collapseBlockedRuns>) =>
    entries.filter((entry) => entry.kind !== "pending" && entry.isFirstUnread).map((entry) => entry.key);

  it("folds each run of one blocked author into one row, and leaves everyone else alone", () => {
    const rows = buildMessageRows(
      [by("2", "100"), by(BLOCKED, "101"), by(BLOCKED, "102"), by(OTHER_BLOCKED, "103"), by("2", "104"), by(BLOCKED, "105")],
      [],
      "1",
      0,
    );
    expect(shape(collapseBlockedRuns(rows, isBlocked, new Set()))).toEqual(["100", "run 2", "run 1", "104", "run 1"]);
    expect(shape(collapseBlockedRuns(rows, () => false, new Set()))).toEqual(["100", "101", "102", "103", "104", "105"]);
  });

  it("shows a revealed run's messages after its row, and keeps it revealed as the run grows", () => {
    const rows = buildMessageRows([by("2", "100"), by(BLOCKED, "101"), by(BLOCKED, "102")], [], "1", 0);
    const revealed = new Set(runIn(collapseBlockedRuns(rows, isBlocked, new Set())).messageIds);
    const grown = buildMessageRows([by("2", "100"), by(BLOCKED, "101"), by(BLOCKED, "102"), by(BLOCKED, "103")], [], "1", 0);
    expect(shape(collapseBlockedRuns(grown, isBlocked, revealed))).toEqual(["100", "run 3", "101", "102", "103"]);
  });

  it("keeps the scroll anchor and the revealed state when older messages extend the run", () => {
    const newest = buildMessageRows([by(BLOCKED, "102"), by(BLOCKED, "103"), by("2", "104")], [], "1", 0);
    const before = runIn(collapseBlockedRuns(newest, isBlocked, new Set()));
    const revealed = new Set(before.messageIds);
    const withOlderPage = buildMessageRows(
      [by(BLOCKED, "100"), by(BLOCKED, "101"), by(BLOCKED, "102"), by(BLOCKED, "103"), by("2", "104")],
      [],
      "1",
      0,
    );
    const after = collapseBlockedRuns(withOlderPage, isBlocked, revealed);
    expect(runIn(after).anchorKey).toBe(before.anchorKey);
    expect(runIn(after).isRevealed).toBe(true);
    expect(shape(after)).toEqual(["run 4", "100", "101", "102", "103", "104"]);
  });

  it("draws the unread divider once: above the folded run, or above the message once revealed", () => {
    const rows = buildMessageRows([by("2", "100"), by(BLOCKED, "101"), by(BLOCKED, "102")], [], "1", 0, "101");
    const folded = collapseBlockedRuns(rows, isBlocked, new Set());
    expect(dividers(folded)).toEqual([runIn(folded).key]);
    const revealed = collapseBlockedRuns(rows, isBlocked, new Set(["101"]));
    expect(dividers(revealed)).toEqual(["102"]);
  });
});

describe("day dividers", () => {
  // Days are the reader's: the tests read them in Paris, across midnight and a clock change.
  beforeAll(() => {
    vi.stubEnv("TZ", "Europe/Paris");
  });
  afterAll(() => {
    vi.unstubAllEnvs();
  });
  const by = (authorId: string, id: string): Message => ({
    ...message(id),
    author: { id: authorId, username: `user${authorId}`, display_name: "" },
  });
  const days = (rows: readonly { key: string; dayStart: number | null }[]) =>
    rows.filter((row) => row.dayStart !== null).map((row) => [row.key, new Date(row.dayStart ?? 0).toISOString()]);

  it("opens each local day once, and breaks the author group at midnight", () => {
    const beforeMidnight = idSentAt("2026-11-04T23:58:00+01:00");
    const afterMidnight = idSentAt("2026-11-05T00:02:00+01:00");
    // 00:30 UTC: a new day in UTC, still the 5th in Paris.
    const sameLocalDay = idSentAt("2026-11-05T01:30:00+01:00");
    const rows = buildMessageRows([by("2", beforeMidnight), by("2", afterMidnight), by("2", sameLocalDay)], [], "1", 0);
    expect(days(rows)).toEqual([[afterMidnight, "2026-11-04T23:00:00.000Z"]]);
    expect(rows.map((row) => row.startsAuthorGroup)).toEqual([true, true, true]);
  });

  it("follows the clock change: the day after it is 25 hours long", () => {
    const saturday = idSentAt("2026-10-24T23:30:00+02:00");
    const sunday = idSentAt("2026-10-25T23:30:00+01:00");
    const monday = idSentAt("2026-10-26T00:30:00+01:00");
    const rows = buildMessageRows([by("2", saturday), by("2", sunday), by("2", monday)], [], "1", 0);
    expect(days(rows)).toEqual([
      [sunday, "2026-10-24T22:00:00.000Z"],
      [monday, "2026-10-25T23:00:00.000Z"],
    ]);
    expect(localDayStart(Date.parse("2026-10-25T23:30:00+01:00"))).toBe(Date.parse("2026-10-25T00:00:00+02:00"));
  });

  it("dates the oldest loaded message only at the start of the channel", () => {
    const first = idSentAt("2026-11-05T09:00:00+01:00");
    expect(days(buildMessageRows([by("2", first)], [], "1", 0))).toEqual([]);
    expect(days(buildMessageRows([by("2", first)], [], "1", 0, null, true))).toEqual([
      [first, "2026-11-04T23:00:00.000Z"],
    ]);
  });

  it("opens a new day above the outbox when it is sent the next day", () => {
    const yesterday = idSentAt("2026-11-04T22:00:00+01:00");
    const pending: PendingMessage = {
      nonce: "n1",
      channelId: "10",
      content: "Bonjour",
      replyToId: null,
      attachments: [],
      poll: null,
      status: "sending",
    };
    const now = Date.parse("2026-11-05T08:00:00+01:00");
    const rows = buildMessageRows([by("1", yesterday)], [pending], "1", now);
    expect(days(rows)).toEqual([["n1", "2026-11-04T23:00:00.000Z"]]);
    expect(rows.at(-1)?.startsAuthorGroup).toBe(true);
  });

  it("puts the unread and day dividers on one row when new messages start a day", () => {
    const read = idSentAt("2026-11-04T23:50:00+01:00");
    const firstNew = idSentAt("2026-11-05T08:05:00+01:00");
    const [, row] = buildMessageRows([by("2", read), by("2", firstNew)], [], "1", 0, read);
    expect(row?.kind === "sent" && row.isFirstUnread).toBe(true);
    expect(row?.dayStart).toBe(Date.parse("2026-11-05T00:00:00+01:00"));
  });

  it("replaces a row whose day divider appeared when an older page loaded", () => {
    const older = idSentAt("2026-11-04T20:00:00+01:00");
    const newer = idSentAt("2026-11-05T08:00:00+01:00");
    const previous = buildMessageRows([by("2", newer)], [], "1", 0);
    const next = reuseUnchangedRows(
      new Map(previous.map((row) => [row.key, row])),
      buildMessageRows([by("2", older), by("2", newer)], [], "1", 0),
    );
    expect(next[1]).not.toBe(previous[0]);
    expect(next[1]?.dayStart).not.toBeNull();
  });

  it("ends a folded run at a day divider, and hands the divider to the message once revealed", () => {
    const BLOCKED = "66";
    const late = idSentAt("2026-11-04T23:59:00+01:00");
    const early = idSentAt("2026-11-05T00:01:00+01:00");
    const rows = buildMessageRows([by(BLOCKED, late), by(BLOCKED, early)], [], "1", 0);
    const isBlocked = (userId: string) => userId === BLOCKED;
    const folded = collapseBlockedRuns(rows, isBlocked, new Set());
    expect(folded.map((entry) => [entry.kind, entry.dayStart !== null])).toEqual([
      ["blocked-run", false],
      ["blocked-run", true],
    ]);
    const revealed = collapseBlockedRuns(rows, isBlocked, new Set([early]));
    expect(revealed.map((entry) => [entry.key, entry.dayStart !== null])).toEqual([
      [`blocked-${late}`, false],
      [`blocked-${early}`, false],
      [early, true],
    ]);
  });
});

describe("arrivalsSinceLeavingPresent", () => {
  it("counts what others sent after the reader scrolled away, never the reader's own", () => {
    const left = message("4194304010");
    const mine = { ...message("4194304011"), author: { id: "1", username: "me", display_name: "" } };
    const messages = [message("4194304009"), left, mine, message("4194304012"), message("4194304013")];
    expect(arrivalsSinceLeavingPresent(messages, left.id, "1")).toBe(2);
    expect(arrivalsSinceLeavingPresent(messages, "4194304013", "1")).toBe(0);
  });
});
