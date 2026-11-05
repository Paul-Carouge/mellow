import { describe, expect, it } from "vitest";

import { checklistProgress, newestTracker, progressPercent, withItemTick } from "./checklists";
import type { Checklist } from "./protocol";

const checklist: Checklist = {
  title: "Release",
  items: [
    { id: 1, text: "Tag", checked: { user_id: "2", at_unix_milliseconds: 5 } },
    { id: 3, text: "Deploy" },
  ],
  revision: 4,
};

describe("checklists", () => {
  it("keep the copy of the highest revision, and the cached one on a tie", () => {
    const older = { ...checklist, revision: 3 };
    const same = { ...checklist };
    const newer = { ...checklist, revision: 5 };
    expect(newestTracker(checklist, older)).toBe(checklist);
    expect(newestTracker(checklist, same)).toBe(checklist);
    expect(newestTracker(checklist, newer)).toBe(newer);
    expect(newestTracker(undefined, newer)).toBe(newer);
    expect(newestTracker(checklist, undefined)).toBe(checklist);
  });

  it("tick and untick one item, keeping the object when nothing changes", () => {
    const tick = { user_id: "1", at_unix_milliseconds: 9 };
    const ticked = withItemTick(checklist, 3, tick);
    expect(ticked.items[1]?.checked).toEqual(tick);
    expect(ticked.items[0]).toBe(checklist.items[0]);
    expect(withItemTick(checklist, 1, tick)).toBe(checklist);
    expect(withItemTick(checklist, 9, tick)).toBe(checklist);
    expect(withItemTick(checklist, 1, undefined).items[0]).toEqual({ id: 1, text: "Tag" });
  });

  it("count ticked items and round percentages down", () => {
    expect(checklistProgress(checklist)).toEqual({ done: 1, total: 2 });
    expect(progressPercent(3, 5)).toBe(60);
    expect(progressPercent(995, 1000)).toBe(99);
    expect(progressPercent(7, 5)).toBe(100);
    expect(progressPercent(1, 0)).toBe(0);
  });
});
