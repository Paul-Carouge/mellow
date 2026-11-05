import { describe, expect, it } from "vitest";

import { isPollEnded, withPollVote } from "./polls";
import type { Poll } from "./protocol";

const poll: Poll = {
  question: "Lunch?",
  answers: [
    { id: 1, text: "Pizza", vote_count: 1, me_voted: false },
    { id: 2, text: "Soup", vote_count: 0, me_voted: false },
  ],
  allows_multiple_answers: false,
  expires_at_unix_milliseconds: 1_000,
};

describe("polls", () => {
  it("counts votes and marks the reader's own", () => {
    const mine = withPollVote(poll, 2, 1, true);
    expect(mine.answers.map((answer) => [answer.vote_count, answer.me_voted])).toEqual([
      [1, false],
      [1, true],
    ]);
    const theirs = withPollVote(mine, 1, -1, false);
    expect(theirs.answers[0]).toMatchObject({ vote_count: 0, me_voted: false });
    expect(withPollVote(theirs, 1, -1, false).answers[0]?.vote_count).toBe(0);
  });

  it("ends at its expiry or when ended early", () => {
    expect(isPollEnded(poll, 999)).toBe(false);
    expect(isPollEnded(poll, 1_000)).toBe(true);
    expect(isPollEnded({ ...poll, ended_at_unix_milliseconds: 5 }, 0)).toBe(true);
  });
});
