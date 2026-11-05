import { describe, expect, it } from "vitest";

import { MAXIMUM_CHECKLIST_ITEMS, MAXIMUM_CHECKLIST_ITEM_LENGTH } from "../lib/protocol";
import {
  canAddChecklistRow,
  checklistDraftProblems,
  checklistItemText,
  emptyChecklistDraft,
  hasChecklistDraftProblems,
  hasProgressBarDraftProblems,
  newChecklistOf,
  newProgressBarOf,
  progressBarDraftOf,
  progressBarDraftProblems,
} from "./tracker-form";

describe("the checklist form", () => {
  it("needs a title and one item, and sends the filled rows trimmed", () => {
    const draft = emptyChecklistDraft();
    expect(checklistDraftProblems(draft)).toEqual({ title: "empty", itemsTooLong: [false, false, false], hasNoItem: true });
    draft.title = " Release ";
    draft.items = [" Tag", "", "Tag "];
    expect(hasChecklistDraftProblems(checklistDraftProblems(draft))).toBe(false);
    expect(newChecklistOf(draft)).toEqual({ title: "Release", items: ["Tag", "Tag"] });
  });

  it("says which item is too long and stops adding rows at the maximum", () => {
    const draft = { title: "t", items: ["x".repeat(MAXIMUM_CHECKLIST_ITEM_LENGTH + 1), "ok"] };
    expect(checklistDraftProblems(draft).itemsTooLong).toEqual([true, false]);
    expect(canAddChecklistRow({ title: "t", items: Array.from({ length: MAXIMUM_CHECKLIST_ITEMS }, () => "") })).toBe(false);
    expect(checklistItemText("  Deploy ")).toBe("Deploy");
    expect(checklistItemText("   ")).toBeNull();
  });
});

describe("the progress bar form", () => {
  it("starts in percent, or from the bar being updated", () => {
    expect(progressBarDraftOf(null)).toEqual({ label: "", value: "0", maximum: "100" });
    expect(progressBarDraftOf({ label: "Sync", value: 3, maximum: 5, revision: 2 })).toEqual({
      label: "Sync",
      value: "3",
      maximum: "5",
    });
  });

  it("takes whole numbers from 0 to the maximum only", () => {
    const problems = (value: string, maximum: string) => progressBarDraftProblems({ label: "Sync", value, maximum });
    expect(hasProgressBarDraftProblems(problems("5", "5"))).toBe(false);
    expect(problems("6", "5").isValueInvalid).toBe(true);
    expect(problems("2.5", "5").isValueInvalid).toBe(true);
    expect(problems("-1", "5").isValueInvalid).toBe(true);
    expect(problems("1", "0")).toEqual({ label: null, isValueInvalid: false, isMaximumInvalid: true });
    expect(newProgressBarOf({ label: " Sync ", value: " 3", maximum: "5 " })).toEqual({ label: "Sync", value: 3, maximum: 5 });
  });
});
