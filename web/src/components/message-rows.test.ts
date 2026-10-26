import { describe, expect, it } from "vitest";

import type { Message } from "../lib/protocol";
import { buildMessageRows, collapseBlockedRuns, reuseUnchangedRows } from "./message-rows";

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
