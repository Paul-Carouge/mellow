import { describe, expect, it } from "vitest";

import { strings } from "../i18n";
import type { Poll } from "../lib/protocol";
import { pendingPollPreview, pollResults, pollSummary, pollTimeLeft, pollTimeLeftLabel } from "./poll-view";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const NOW = 1_000_000_000;

function poll(counts: number[], overrides: Partial<Poll> = {}): Poll {
  return {
    question: "Lunch?",
    answers: counts.map((count, index) => ({ id: index + 1, text: `A${index}`, vote_count: count, me_voted: false })),
    allows_multiple_answers: false,
    expires_at_unix_milliseconds: NOW + HOUR,
    ...overrides,
  };
}

describe("pollResults", () => {
  it("gives each answer its share of every vote, rounded, and finds the leaders", () => {
    const results = pollResults(poll([1, 2, 0]));
    expect(results.totalVotes).toBe(3);
    expect(results.answers.map((answer) => [answer.percent, answer.isLeading])).toEqual([
      [33, false],
      [67, true],
      [0, false],
    ]);
    expect(results.hasReaderVoted).toBe(false);
  });

  it("leads with every tied answer, and with none before anyone voted", () => {
    expect(pollResults(poll([2, 2, 1])).answers.map((answer) => answer.isLeading)).toEqual([true, true, false]);
    const empty = pollResults(poll([0, 0]));
    expect(empty.answers.map((answer) => [answer.percent, answer.isLeading])).toEqual([
      [0, false],
      [0, false],
    ]);
  });

  it("knows whether the reader voted", () => {
    const voted = poll([1, 0]);
    voted.answers[0] = { id: 1, text: "A0", vote_count: 1, me_voted: true };
    expect(pollResults(voted).hasReaderVoted).toBe(true);
  });
});

describe("pollTimeLeft", () => {
  const endingIn = (milliseconds: number) => pollTimeLeft(poll([0, 0], { expires_at_unix_milliseconds: NOW + milliseconds }), NOW);

  it("rounds down to minutes, then hours, then days from two days", () => {
    expect(endingIn(30_000)).toEqual({ kind: "lessThanAMinute" });
    expect(endingIn(MINUTE)).toEqual({ kind: "left", unit: "minutes", amount: 1 });
    expect(endingIn(59 * MINUTE + 59_000)).toEqual({ kind: "left", unit: "minutes", amount: 59 });
    expect(endingIn(HOUR)).toEqual({ kind: "left", unit: "hours", amount: 1 });
    expect(endingIn(47 * HOUR + 59 * MINUTE)).toEqual({ kind: "left", unit: "hours", amount: 47 });
    expect(endingIn(48 * HOUR)).toEqual({ kind: "left", unit: "days", amount: 2 });
    expect(endingIn(336 * HOUR)).toEqual({ kind: "left", unit: "days", amount: 14 });
  });

  it("ends at its expiry or when its author ended it", () => {
    expect(endingIn(0)).toEqual({ kind: "ended" });
    expect(pollTimeLeft(poll([0, 0], { ended_at_unix_milliseconds: NOW - 1 }), NOW)).toEqual({ kind: "ended" });
  });

  it("reads in the interface's language", () => {
    expect(pollTimeLeftLabel({ kind: "left", unit: "hours", amount: 3 })).toBe(strings.polls.endsIn("hours", 3));
    expect(pollTimeLeftLabel({ kind: "ended" })).toBe(strings.polls.ended);
  });
});

describe("pollSummary and pendingPollPreview", () => {
  it("stands for a poll by its question", () => {
    expect(pollSummary(poll([0, 0]))).toBe(strings.polls.summary("Lunch?"));
  });

  it("shows a poll still in the outbox with no votes, for its whole duration", () => {
    const preview = pendingPollPreview(
      { question: "Lunch?", answers: ["Pizza", "Soup"], allows_multiple_answers: true, duration_hours: 4 },
      NOW,
    );
    expect(preview.answers).toEqual([
      { id: 1, text: "Pizza", vote_count: 0, me_voted: false },
      { id: 2, text: "Soup", vote_count: 0, me_voted: false },
    ]);
    expect(preview.allows_multiple_answers).toBe(true);
    expect(preview.expires_at_unix_milliseconds).toBe(NOW + 4 * HOUR);
  });
});
