// The "Create a poll" form's rules (ADR-0043, docs/design/web-polls.md): what the instance would
// refuse, said before sending, counted as the instance counts (Unicode scalar values, trimmed).

import {
  MAXIMUM_POLL_ANSWERS,
  MAXIMUM_POLL_ANSWER_LENGTH,
  MAXIMUM_POLL_QUESTION_LENGTH,
  MINIMUM_POLL_ANSWERS,
  type NewPoll,
} from "../lib/protocol";

/** Discord's default: a day. */
export const DEFAULT_POLL_DURATION_HOURS = 24;

export interface PollDraft {
  question: string;
  /** As typed, blank rows included: blank answers are left out when sent. */
  answers: string[];
  allowsMultipleAnswers: boolean;
  durationHours: number;
}

export function emptyPollDraft(): PollDraft {
  return {
    question: "",
    answers: Array.from({ length: MINIMUM_POLL_ANSWERS }, () => ""),
    allowsMultipleAnswers: false,
    durationHours: DEFAULT_POLL_DURATION_HOURS,
  };
}

export function characterCount(text: string): number {
  return [...text.trim()].length;
}

export type AnswerProblem = "tooLong" | "duplicate" | null;

export interface PollDraftProblems {
  question: "empty" | "tooLong" | null;
  /** One per row; a repeat is marked on its later rows, the first stays clean. */
  answers: AnswerProblem[];
  hasTooFewAnswers: boolean;
}

/** Case ignored, as the instance compares them. */
function foldAnswer(text: string): string {
  return text.trim().toLowerCase();
}

export function pollDraftProblems(draft: PollDraft): PollDraftProblems {
  const questionLength = characterCount(draft.question);
  const seen = new Set<string>();
  const answers = draft.answers.map((text): AnswerProblem => {
    const folded = foldAnswer(text);
    if (folded === "") {
      return null;
    }
    if (characterCount(text) > MAXIMUM_POLL_ANSWER_LENGTH) {
      return "tooLong";
    }
    if (seen.has(folded)) {
      return "duplicate";
    }
    seen.add(folded);
    return null;
  });
  return {
    question: questionLength === 0 ? "empty" : questionLength > MAXIMUM_POLL_QUESTION_LENGTH ? "tooLong" : null,
    answers,
    hasTooFewAnswers: filledAnswers(draft).length < MINIMUM_POLL_ANSWERS,
  };
}

export function hasPollDraftProblems(problems: PollDraftProblems): boolean {
  return problems.question !== null || problems.hasTooFewAnswers || problems.answers.some((problem) => problem !== null);
}

function filledAnswers(draft: PollDraft): string[] {
  return draft.answers.map((text) => text.trim()).filter((text) => text !== "");
}

/** What is sent, once the draft has no problem. */
export function newPollOf(draft: PollDraft): NewPoll {
  return {
    question: draft.question.trim(),
    answers: filledAnswers(draft),
    allows_multiple_answers: draft.allowsMultipleAnswers,
    duration_hours: draft.durationHours,
  };
}

export function canAddAnswer(draft: PollDraft): boolean {
  return draft.answers.length < MAXIMUM_POLL_ANSWERS;
}

export function canRemoveAnswer(draft: PollDraft): boolean {
  return draft.answers.length > MINIMUM_POLL_ANSWERS;
}
