import { describe, expect, it } from "vitest";

import { MAXIMUM_POLL_ANSWERS, MAXIMUM_POLL_ANSWER_LENGTH, MAXIMUM_POLL_QUESTION_LENGTH } from "../lib/protocol";
import {
  DEFAULT_POLL_DURATION_HOURS,
  canAddAnswer,
  canRemoveAnswer,
  characterCount,
  emptyPollDraft,
  hasPollDraftProblems,
  newPollOf,
  pollDraftProblems,
} from "./poll-form";

const draft = (question: string, answers: string[]) => ({ ...emptyPollDraft(), question, answers });

describe("poll form", () => {
  it("starts with two empty answers for a day, one choice", () => {
    expect(emptyPollDraft()).toEqual({
      question: "",
      answers: ["", ""],
      allowsMultipleAnswers: false,
      durationHours: DEFAULT_POLL_DURATION_HOURS,
    });
    expect(DEFAULT_POLL_DURATION_HOURS).toBe(24);
  });

  it("counts characters as the instance does: trimmed, an emoji is one", () => {
    expect(characterCount("  Pizza ")).toBe(5);
    expect(characterCount("🍕🍕")).toBe(2);
  });

  it("asks for a question and two answers", () => {
    const problems = pollDraftProblems(draft("  ", ["Pizza", " "]));
    expect(problems.question).toBe("empty");
    expect(problems.hasTooFewAnswers).toBe(true);
    expect(hasPollDraftProblems(problems)).toBe(true);
  });

  it("refuses texts past their limits, emoji counted once", () => {
    const longQuestion = "🍕".repeat(MAXIMUM_POLL_QUESTION_LENGTH + 1);
    const longAnswer = "a".repeat(MAXIMUM_POLL_ANSWER_LENGTH + 1);
    const problems = pollDraftProblems(draft(longQuestion, ["Pizza", longAnswer]));
    expect(problems.question).toBe("tooLong");
    expect(problems.answers).toEqual([null, "tooLong"]);
    expect(pollDraftProblems(draft("🍕".repeat(MAXIMUM_POLL_QUESTION_LENGTH), ["a", "b"])).question).toBeNull();
  });

  it("marks a repeated answer on its later rows, case and spaces ignored", () => {
    const problems = pollDraftProblems(draft("Lunch?", ["Pizza", "Soup", " pizza ", "", "PIZZA"]));
    expect(problems.answers).toEqual([null, null, "duplicate", null, "duplicate"]);
    expect(problems.hasTooFewAnswers).toBe(false);
  });

  it("sends trimmed texts without blank rows", () => {
    const filled = { ...draft(" Lunch? ", [" Pizza", "", "Soup "]), allowsMultipleAnswers: true, durationHours: 72 };
    expect(hasPollDraftProblems(pollDraftProblems(filled))).toBe(false);
    expect(newPollOf(filled)).toEqual({
      question: "Lunch?",
      answers: ["Pizza", "Soup"],
      allows_multiple_answers: true,
      duration_hours: 72,
    });
  });

  it("keeps between two and ten rows", () => {
    expect(canRemoveAnswer(emptyPollDraft())).toBe(false);
    expect(canAddAnswer(emptyPollDraft())).toBe(true);
    const full = draft("Q", Array.from({ length: MAXIMUM_POLL_ANSWERS }, (_, index) => `A${index}`));
    expect(canAddAnswer(full)).toBe(false);
    expect(canRemoveAnswer(full)).toBe(true);
  });
});
