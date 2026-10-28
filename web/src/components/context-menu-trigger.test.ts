import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createLongPressDetector, isContextMenuKey, shouldKeepNativeContextMenu } from "./context-menu-trigger";

describe("isContextMenuKey", () => {
  const plain = { shiftKey: false, ctrlKey: false, altKey: false, metaKey: false };

  it("opens on Shift+F10 and on the Menu key", () => {
    expect(isContextMenuKey({ ...plain, key: "F10", shiftKey: true })).toBe(true);
    expect(isContextMenuKey({ ...plain, key: "ContextMenu" })).toBe(true);
  });

  it("leaves F10 alone without Shift, and any chord with another modifier", () => {
    expect(isContextMenuKey({ ...plain, key: "F10" })).toBe(false);
    expect(isContextMenuKey({ ...plain, key: "F10", shiftKey: true, ctrlKey: true })).toBe(false);
    expect(isContextMenuKey({ ...plain, key: "ContextMenu", metaKey: true })).toBe(false);
    expect(isContextMenuKey({ ...plain, key: "Enter" })).toBe(false);
  });
});

describe("shouldKeepNativeContextMenu", () => {
  const nowhereSpecial = { isInTextField: false, isOnLink: false, isOnSelectedText: false, isShiftHeld: false };

  it("opens Mellow's menu on an ordinary item", () => {
    expect(shouldKeepNativeContextMenu(nowhereSpecial)).toBe(false);
  });

  it("keeps the browser's menu in text fields, on links, on selected text and with Shift", () => {
    expect(shouldKeepNativeContextMenu({ ...nowhereSpecial, isInTextField: true })).toBe(true);
    expect(shouldKeepNativeContextMenu({ ...nowhereSpecial, isOnLink: true })).toBe(true);
    expect(shouldKeepNativeContextMenu({ ...nowhereSpecial, isOnSelectedText: true })).toBe(true);
    expect(shouldKeepNativeContextMenu({ ...nowhereSpecial, isShiftHeld: true })).toBe(true);
  });
});

describe("createLongPressDetector", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  const press = () => createLongPressDetector({ delayMilliseconds: 500, moveTolerance: 10 });

  it("opens where the finger landed once it rested long enough and lifts", () => {
    const detector = press();
    detector.start(100, 200);
    detector.move(104, 203);
    vi.advanceTimersByTime(500);
    expect(detector.release()).toEqual({ x: 100, y: 200 });
    expect(detector.isPressing()).toBe(false);
  });

  it("is a tap when the finger lifts early", () => {
    const detector = press();
    detector.start(100, 200);
    vi.advanceTimersByTime(499);
    expect(detector.release()).toBeNull();
  });

  it("gives way to scrolling and selecting: moving past the tolerance calls it off", () => {
    const detector = press();
    detector.start(100, 200);
    detector.move(100, 215);
    vi.advanceTimersByTime(1_000);
    expect(detector.release()).toBeNull();
  });

  it("is called off by moving even after it was held long enough", () => {
    const detector = press();
    detector.start(100, 200);
    vi.advanceTimersByTime(600);
    detector.move(130, 200);
    expect(detector.release()).toBeNull();
  });

  it("is held at once when the browser recognised the long press itself", () => {
    const detector = press();
    detector.start(100, 200);
    vi.advanceTimersByTime(300);
    detector.holdNow();
    expect(detector.release()).toEqual({ x: 100, y: 200 });
  });

  it("forgets a press that was cancelled", () => {
    const detector = press();
    detector.start(100, 200);
    vi.advanceTimersByTime(600);
    detector.cancel();
    expect(detector.release()).toBeNull();
  });
});
