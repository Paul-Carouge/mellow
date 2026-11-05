// How a poll reads (ADR-0043, DESIGN.md §5.4, docs/design/web-polls.md): its results, the time it
// has left, and the one line that stands for it where a message is quoted.

import { strings } from "../i18n";
import { isPollEnded } from "../lib/polls";
import type { NewPoll, Poll, PollAnswer } from "../lib/protocol";

const MILLISECONDS_PER_MINUTE = 60_000;
const MINUTES_PER_HOUR = 60;
const HOURS_PER_DAY = 24;
/** Below two days, hours read better than "1 day" for anything between 24 and 47 hours. */
const HOURS_SHOWN_AS_DAYS_FROM = 2 * HOURS_PER_DAY;

export interface PollAnswerResult {
  answer: PollAnswer;
  /** Of every vote cast in the poll, rounded; 0 when nobody voted. */
  percent: number;
  /** Among the most voted, once anyone voted; only shown when the poll has ended. */
  isLeading: boolean;
}

export interface PollResults {
  totalVotes: number;
  hasReaderVoted: boolean;
  answers: PollAnswerResult[];
}

export function pollResults(poll: Poll): PollResults {
  const totalVotes = poll.answers.reduce((sum, answer) => sum + answer.vote_count, 0);
  const mostVotes = Math.max(0, ...poll.answers.map((answer) => answer.vote_count));
  return {
    totalVotes,
    hasReaderVoted: poll.answers.some((answer) => answer.me_voted),
    answers: poll.answers.map((answer) => ({
      answer,
      percent: totalVotes === 0 ? 0 : Math.round((answer.vote_count / totalVotes) * 100),
      isLeading: mostVotes > 0 && answer.vote_count === mostVotes,
    })),
  };
}

export type PollTimeLeft =
  | { kind: "ended" }
  | { kind: "lessThanAMinute" }
  | { kind: "left"; unit: "minutes" | "hours" | "days"; amount: number };

/** Rounded down, so a poll never seems to last longer than it does. */
export function pollTimeLeft(poll: Poll, nowUnixMilliseconds: number): PollTimeLeft {
  if (isPollEnded(poll, nowUnixMilliseconds)) {
    return { kind: "ended" };
  }
  const minutes = Math.floor((poll.expires_at_unix_milliseconds - nowUnixMilliseconds) / MILLISECONDS_PER_MINUTE);
  const hours = Math.floor(minutes / MINUTES_PER_HOUR);
  if (hours >= HOURS_SHOWN_AS_DAYS_FROM) {
    return { kind: "left", unit: "days", amount: Math.floor(hours / HOURS_PER_DAY) };
  }
  if (hours >= 1) {
    return { kind: "left", unit: "hours", amount: hours };
  }
  return minutes >= 1 ? { kind: "left", unit: "minutes", amount: minutes } : { kind: "lessThanAMinute" };
}

export function pollTimeLeftLabel(timeLeft: PollTimeLeft): string {
  switch (timeLeft.kind) {
    case "ended":
      return strings.polls.ended;
    case "lessThanAMinute":
      return strings.polls.endsInLessThanAMinute;
    case "left":
      return strings.polls.endsIn(timeLeft.unit, timeLeft.amount);
  }
}

/** When the poll stopped taking votes, or will. */
export function pollEndUnixMilliseconds(poll: Poll): number {
  return poll.ended_at_unix_milliseconds ?? poll.expires_at_unix_milliseconds;
}

/** "Poll: Lunch?": what stands for a poll where a message is quoted on one line. */
export function pollSummary(poll: Poll | NewPoll): string {
  return strings.polls.summary(poll.question);
}

/** A poll still in the outbox, shown as it will arrive: no votes yet, its clock not started. */
export function pendingPollPreview(poll: NewPoll, nowUnixMilliseconds: number): Poll {
  return {
    question: poll.question,
    answers: poll.answers.map((text, index) => ({ id: index + 1, text, vote_count: 0, me_voted: false })),
    allows_multiple_answers: poll.allows_multiple_answers,
    expires_at_unix_milliseconds: nowUnixMilliseconds + poll.duration_hours * MINUTES_PER_HOUR * MILLISECONDS_PER_MINUTE,
  };
}
