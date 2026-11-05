import type { Poll } from "./protocol";

/** The poll with one vote added to or taken from `answerId`; `isMine` keeps the reader's mark right. */
export function withPollVote(poll: Poll, answerId: number, change: 1 | -1, isMine: boolean): Poll {
  return {
    ...poll,
    answers: poll.answers.map((answer) =>
      answer.id === answerId
        ? {
            ...answer,
            vote_count: Math.max(0, answer.vote_count + change),
            me_voted: isMine ? change === 1 : answer.me_voted,
          }
        : answer,
    ),
  };
}

/**
 * A poll from an event over the cached one: events carry counts but never the reader's votes
 * (`me_voted` is always false there), so the cached marks stay.
 */
export function withReaderVotesKept(updated: Poll, cached: Poll | undefined): Poll {
  if (!cached) {
    return updated;
  }
  return {
    ...updated,
    answers: updated.answers.map((answer) => ({
      ...answer,
      me_voted: cached.answers.find((known) => known.id === answer.id)?.me_voted ?? false,
    })),
  };
}

/** Whether no more votes are taken at `now`. */
export function isPollEnded(poll: Poll, nowUnixMilliseconds: number): boolean {
  return poll.ended_at_unix_milliseconds !== undefined || poll.expires_at_unix_milliseconds <= nowUnixMilliseconds;
}
