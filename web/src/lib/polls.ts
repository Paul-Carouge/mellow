import type { Poll } from "./protocol";

/**
 * The poll with one vote added to or taken from `answerId`; `isMine` keeps the reader's mark right.
 * The reader's own vote counts once: their click already counted it, so the event that follows
 * changes nothing.
 */
export function withPollVote(poll: Poll, answerId: number, change: 1 | -1, isMine: boolean): Poll {
  const target = poll.answers.find((answer) => answer.id === answerId);
  if (!target || (isMine && target.me_voted === (change === 1))) {
    return poll;
  }
  return {
    ...poll,
    answers: poll.answers.map((answer) =>
      answer === target
        ? {
            ...answer,
            vote_count: Math.max(0, answer.vote_count + change),
            me_voted: isMine ? change === 1 : answer.me_voted,
          }
        : answer,
    ),
  };
}

/** Whether no more votes are taken at `now`. */
export function isPollEnded(poll: Poll, nowUnixMilliseconds: number): boolean {
  return poll.ended_at_unix_milliseconds !== undefined || poll.expires_at_unix_milliseconds <= nowUnixMilliseconds;
}

/** One of the reader's own votes to add (`1`) or take back (`-1`). */
export interface OwnPollVoteChange {
  answerId: number;
  change: 1 | -1;
}

/**
 * What choosing `answerId` changes in the reader's votes: a single-choice poll moves the vote
 * there (choosing it again changes nothing), a multiple-choice one toggles that answer.
 */
export function ownVoteChangesForChoice(poll: Poll, answerId: number): OwnPollVoteChange[] {
  const chosen = poll.answers.find((answer) => answer.id === answerId);
  if (!chosen) {
    return [];
  }
  if (poll.allows_multiple_answers) {
    return [{ answerId, change: chosen.me_voted ? -1 : 1 }];
  }
  if (chosen.me_voted) {
    return [];
  }
  return [
    ...poll.answers
      .filter((answer) => answer.me_voted)
      .map((answer): OwnPollVoteChange => ({ answerId: answer.id, change: -1 })),
    { answerId, change: 1 },
  ];
}
