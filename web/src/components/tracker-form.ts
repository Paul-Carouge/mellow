// The "Create a checklist" and progress bar forms' rules (ADR-0047,
// docs/design/web-checklists-and-progress.md): what the instance would refuse, said before
// sending, counted as it counts (Unicode scalar values, trimmed). Items may repeat.

import {
  MAXIMUM_CHECKLIST_ITEMS,
  MAXIMUM_CHECKLIST_ITEM_LENGTH,
  MAXIMUM_CHECKLIST_TITLE_LENGTH,
  MAXIMUM_PROGRESS_BAR_LABEL_LENGTH,
  MAXIMUM_PROGRESS_BAR_MAXIMUM,
  type NewChecklist,
  type NewProgressBar,
  type ProgressBar,
} from "../lib/protocol";
import { characterCount } from "./poll-form";

/** Rows shown in a new checklist: enough to start, more come with Enter. */
const NEW_CHECKLIST_ROWS = 3;
/** A new progress bar counts in percent. */
const PERCENT = 100;

export interface ChecklistDraft {
  title: string;
  /** As typed, blank rows included: blank items are left out when sent. */
  items: string[];
}

export function emptyChecklistDraft(): ChecklistDraft {
  return { title: "", items: Array.from({ length: NEW_CHECKLIST_ROWS }, () => "") };
}

export interface ChecklistDraftProblems {
  title: "empty" | "tooLong" | null;
  /** One per row. */
  itemsTooLong: boolean[];
  hasNoItem: boolean;
}

function lengthProblem(text: string, maximum: number): "empty" | "tooLong" | null {
  const length = characterCount(text);
  return length === 0 ? "empty" : length > maximum ? "tooLong" : null;
}

export function checklistDraftProblems(draft: ChecklistDraft): ChecklistDraftProblems {
  return {
    title: lengthProblem(draft.title, MAXIMUM_CHECKLIST_TITLE_LENGTH),
    itemsTooLong: draft.items.map((text) => characterCount(text) > MAXIMUM_CHECKLIST_ITEM_LENGTH),
    hasNoItem: draft.items.every((text) => text.trim() === ""),
  };
}

export function hasChecklistDraftProblems(problems: ChecklistDraftProblems): boolean {
  return problems.title !== null || problems.hasNoItem || problems.itemsTooLong.some(Boolean);
}

/** What is sent, once the draft has no problem. */
export function newChecklistOf(draft: ChecklistDraft): NewChecklist {
  return {
    title: draft.title.trim(),
    items: draft.items.map((text) => text.trim()).filter((text) => text !== ""),
  };
}

export function canAddChecklistRow(draft: ChecklistDraft): boolean {
  return draft.items.length < MAXIMUM_CHECKLIST_ITEMS;
}

/** One item typed on a card (added or renamed): `null` when the instance would refuse it. */
export function checklistItemText(text: string): string | null {
  const trimmed = text.trim();
  return lengthProblem(trimmed, MAXIMUM_CHECKLIST_ITEM_LENGTH) === null ? trimmed : null;
}

export interface ProgressBarDraft {
  label: string;
  /** As typed in number fields. */
  value: string;
  maximum: string;
}

export function progressBarDraftOf(bar: ProgressBar | null): ProgressBarDraft {
  return bar
    ? { label: bar.label, value: String(bar.value), maximum: String(bar.maximum) }
    : { label: "", value: "0", maximum: String(PERCENT) };
}

export interface ProgressBarDraftProblems {
  label: "empty" | "tooLong" | null;
  isValueInvalid: boolean;
  isMaximumInvalid: boolean;
}

/** A whole number written in decimal digits only, within the range; `null` otherwise. */
function wholeNumberIn(text: string, minimum: number, maximum: number): number | null {
  const trimmed = text.trim();
  if (!/^\d+$/.test(trimmed)) {
    return null;
  }
  const value = Number(trimmed);
  return value >= minimum && value <= maximum ? value : null;
}

export function progressBarDraftProblems(draft: ProgressBarDraft): ProgressBarDraftProblems {
  const maximum = wholeNumberIn(draft.maximum, 1, MAXIMUM_PROGRESS_BAR_MAXIMUM);
  return {
    label: lengthProblem(draft.label, MAXIMUM_PROGRESS_BAR_LABEL_LENGTH),
    isMaximumInvalid: maximum === null,
    // Judged against a valid maximum only: a wrong maximum is said once, on its own field.
    isValueInvalid: maximum !== null && wholeNumberIn(draft.value, 0, maximum) === null,
  };
}

export function hasProgressBarDraftProblems(problems: ProgressBarDraftProblems): boolean {
  return problems.label !== null || problems.isValueInvalid || problems.isMaximumInvalid;
}

/** What is sent, once the draft has no problem. */
export function newProgressBarOf(draft: ProgressBarDraft): NewProgressBar {
  return { label: draft.label.trim(), value: Number(draft.value.trim()), maximum: Number(draft.maximum.trim()) };
}
