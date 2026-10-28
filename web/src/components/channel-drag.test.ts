import { describe, expect, it } from "vitest";

import { createDragGesture } from "./channel-drag";

function gestureWithManualTimer() {
  let pendingCallback: (() => void) | null = null;
  const gesture = createDragGesture({
    startDistance: 4,
    holdDelayMilliseconds: 500,
    moveTolerance: 10,
    schedule: (callback) => {
      pendingCallback = callback;
      return 1;
    },
    cancelSchedule: () => {
      pendingCallback = null;
    },
  });
  return { gesture, elapse: () => pendingCallback?.() };
}

describe("createDragGesture", () => {
  it("starts a mouse drag past the start distance, then follows and drops", () => {
    const { gesture } = gestureWithManualTimer();
    gesture.press("mouse", 0, 0);
    expect(gesture.move(2, 2)).toBe("none");
    expect(gesture.move(0, 5)).toBe("start");
    expect(gesture.move(0, 40)).toBe("move");
    expect(gesture.release()).toBe(true);
    expect(gesture.move(0, 50)).toBe("none");
  });

  it("treats a mouse press that never moved as a click", () => {
    const { gesture } = gestureWithManualTimer();
    gesture.press("mouse", 0, 0);
    expect(gesture.release()).toBe(false);
  });

  it("lets a finger that moves at once scroll the list", () => {
    const { gesture, elapse } = gestureWithManualTimer();
    gesture.press("touch", 0, 0);
    expect(gesture.isHoldingOrDragging()).toBe(false);
    expect(gesture.move(0, 20)).toBe("cancel");
    elapse();
    expect(gesture.move(0, 40)).toBe("none");
    expect(gesture.release()).toBe(false);
  });

  it("starts a touch drag only after the hold, once the finger moves", () => {
    const { gesture, elapse } = gestureWithManualTimer();
    gesture.press("touch", 0, 0);
    expect(gesture.move(0, 5)).toBe("none");
    elapse();
    expect(gesture.isHoldingOrDragging()).toBe(true);
    expect(gesture.move(0, 8)).toBe("none");
    expect(gesture.move(0, 12)).toBe("start");
    expect(gesture.isDragging()).toBe(true);
  });

  it("leaves a held finger that lifts without moving to the row's menu", () => {
    const { gesture, elapse } = gestureWithManualTimer();
    gesture.press("touch", 0, 0);
    elapse();
    expect(gesture.release()).toBe(false);
  });

  it("forgets everything on cancel (Esc, or the browser taking the pointer)", () => {
    const { gesture } = gestureWithManualTimer();
    gesture.press("pen", 0, 0);
    gesture.move(0, 10);
    gesture.cancel();
    expect(gesture.isDragging()).toBe(false);
    expect(gesture.release()).toBe(false);
  });
});
