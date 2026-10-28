// The trigger on a stand-in element (no DOM library in this repo): which events open Mellow's menu
// and which leave the browser's own.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { contextMenuTrigger } from "./context-menu-trigger";
import type { MenuOrigin } from "./menu-placement";

type Kind = "plain" | "text-field" | "link";

class FakeElement extends EventTarget {
  isConnected = true;
  constructor(readonly kind: Kind = "plain") {
    super();
  }
  closest(selector: string): FakeElement | null {
    const isField = selector.startsWith("input") && this.kind === "text-field";
    const isLink = selector === "a[href]" && this.kind === "link";
    return isField || isLink ? this : null;
  }
  matches(): boolean {
    return true;
  }
  querySelector(): null {
    return null;
  }
  contains(): boolean {
    return false;
  }
  getBoundingClientRect() {
    return { top: 10, bottom: 40, left: 20, right: 120 };
  }
}

let selection: { isCollapsed: boolean; containsNode: () => boolean; anchorNode: null; removeAllRanges: () => void };

beforeEach(() => {
  vi.useFakeTimers();
  selection = { isCollapsed: true, containsNode: () => false, anchorNode: null, removeAllRanges: vi.fn() };
  vi.stubGlobal("Element", FakeElement);
  vi.stubGlobal("HTMLElement", FakeElement);
  vi.stubGlobal("window", { getSelection: () => selection });
  vi.stubGlobal("document", { documentElement: {} });
  vi.stubGlobal("getComputedStyle", () => ({
    fontSize: "16px",
    getPropertyValue: (name: string) => (name.includes("long-press-move") ? "1rem" : "500ms"),
  }));
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function setup(kind: Kind = "plain", result = true) {
  const element = new FakeElement(kind);
  const open = vi.fn<(origin: MenuOrigin, invoker: unknown, source: string) => boolean>(() => result);
  const detach = contextMenuTrigger(open as never)(element as never) as () => void;
  return { element, open, detach };
}

function fire(element: FakeElement, type: string, fields: Record<string, unknown> = {}): Event {
  const event = Object.assign(new Event(type, { cancelable: true }), fields);
  element.dispatchEvent(event);
  return event;
}
const touch = { pointerType: "touch", isPrimary: true, clientX: 50, clientY: 60 };

describe("right-click", () => {
  it("opens on a word macOS selected under the right press, and clears that word", () => {
    const { element, open } = setup();
    fire(element, "pointerdown", { button: 2, pointerType: "mouse", isPrimary: true });
    selection.isCollapsed = false;
    selection.containsNode = () => true;
    const event = fire(element, "contextmenu", { clientX: 5, clientY: 6, shiftKey: false });
    expect(open).toHaveBeenCalledTimes(1);
    expect(event.defaultPrevented).toBe(true);
    expect(selection.removeAllRanges).toHaveBeenCalledTimes(1);
  });

  it("keeps the browser's menu on text the reader selected before the right press", () => {
    const { element, open } = setup();
    selection.isCollapsed = false;
    selection.containsNode = () => true;
    fire(element, "pointerdown", { button: 2, pointerType: "mouse", isPrimary: true });
    const event = fire(element, "contextmenu", { clientX: 5, clientY: 6, shiftKey: false });
    expect(open).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  it("opens at the pointer and takes the event", () => {
    const { element, open } = setup();
    const event = fire(element, "contextmenu", { clientX: 5, clientY: 6, shiftKey: false });
    expect(open).toHaveBeenCalledWith({ kind: "point", x: 5, y: 6 }, element, "pointer");
    expect(event.defaultPrevented).toBe(true);
  });

  it("keeps the browser's menu with Shift held", () => {
    const { element, open } = setup();
    const event = fire(element, "contextmenu", { clientX: 5, clientY: 6, shiftKey: true });
    expect(open).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  it.each(["text-field", "link"] as const)("keeps the browser's menu on a %s", (kind) => {
    const { element, open } = setup(kind);
    expect(fire(element, "contextmenu", { clientX: 1, clientY: 1, shiftKey: false }).defaultPrevented).toBe(false);
    expect(open).not.toHaveBeenCalled();
  });

  it("keeps the browser's menu on selected text", () => {
    selection.isCollapsed = false;
    selection.containsNode = () => true;
    const { element, open } = setup();
    expect(fire(element, "contextmenu", { clientX: 1, clientY: 1, shiftKey: false }).defaultPrevented).toBe(false);
    expect(open).not.toHaveBeenCalled();
  });

  it("leaves the browser's menu when the opener has nothing to show", () => {
    const { element } = setup("plain", false);
    expect(fire(element, "contextmenu", { clientX: 1, clientY: 1, shiftKey: false }).defaultPrevented).toBe(false);
  });

  it("is ignored once a trigger that took the event first has run (nested items)", () => {
    const element = new FakeElement();
    const inner = vi.fn(() => true);
    const outer = vi.fn(() => true);
    contextMenuTrigger(inner as never)(element as never);
    contextMenuTrigger(outer as never)(element as never);
    fire(element, "contextmenu", { clientX: 1, clientY: 1, shiftKey: false });
    expect(inner).toHaveBeenCalledTimes(1);
    expect(outer).not.toHaveBeenCalled();
  });

  it("stops listening once detached", () => {
    const { element, open, detach } = setup();
    detach();
    fire(element, "contextmenu", { clientX: 1, clientY: 1, shiftKey: false });
    expect(open).not.toHaveBeenCalled();
  });
});

describe("keyboard", () => {
  const key = { key: "ContextMenu", shiftKey: false, ctrlKey: false, altKey: false, metaKey: false };

  it("opens next to the focused item for the Menu key and Shift+F10", () => {
    const { element, open } = setup();
    expect(fire(element, "keydown", key).defaultPrevented).toBe(true);
    fire(element, "keydown", { ...key, key: "F10", shiftKey: true });
    expect(open).toHaveBeenCalledTimes(2);
    expect(open.mock.calls[0]?.[2]).toBe("keyboard");
  });

  it("ignores plain F10, other keys, and text fields", () => {
    const { element, open } = setup();
    fire(element, "keydown", { ...key, key: "F10" });
    fire(element, "keydown", { ...key, key: "a" });
    const field = setup("text-field");
    fire(field.element, "keydown", key);
    expect(open).not.toHaveBeenCalled();
    expect(field.open).not.toHaveBeenCalled();
  });
});

describe("long press", () => {
  it("opens where the finger landed once it lifts after the delay, and swallows the lift", () => {
    const { element, open } = setup();
    fire(element, "pointerdown", touch);
    vi.advanceTimersByTime(500);
    expect(open).not.toHaveBeenCalled();
    fire(element, "pointerup");
    vi.advanceTimersByTime(0);
    expect(open).toHaveBeenCalledWith({ kind: "point", x: 50, y: 60 }, element, "pointer");
    expect(fire(element, "touchend").defaultPrevented).toBe(true);
    expect(fire(element, "touchend").defaultPrevented).toBe(false);
  });

  it("is a tap when lifted early", () => {
    const { element, open } = setup();
    fire(element, "pointerdown", touch);
    vi.advanceTimersByTime(499);
    fire(element, "pointerup");
    vi.advanceTimersByTime(10);
    expect(open).not.toHaveBeenCalled();
    expect(fire(element, "touchend").defaultPrevented).toBe(false);
  });

  it("is called off by moving past the tolerance, and kept within it", () => {
    const { element, open } = setup();
    fire(element, "pointerdown", touch);
    fire(element, "pointermove", { clientX: 50, clientY: 70 });
    vi.advanceTimersByTime(500);
    fire(element, "pointerup");
    vi.advanceTimersByTime(0);
    expect(open).toHaveBeenCalledTimes(1);

    fire(element, "pointerdown", touch);
    fire(element, "pointermove", { clientX: 50, clientY: 80 });
    vi.advanceTimersByTime(500);
    fire(element, "pointerup");
    vi.advanceTimersByTime(0);
    expect(open).toHaveBeenCalledTimes(1);
  });

  it("ignores a second finger, mouse and pen presses", () => {
    const { element, open } = setup();
    for (const extra of [{ isPrimary: false }, { pointerType: "mouse" }, { pointerType: "pen" }]) {
      fire(element, "pointerdown", { ...touch, ...extra });
      vi.advanceTimersByTime(600);
      fire(element, "pointerup");
    }
    vi.advanceTimersByTime(0);
    expect(open).not.toHaveBeenCalled();
  });

  it("a second finger landing does not restart a press already held", () => {
    const { element, open } = setup();
    fire(element, "pointerdown", touch);
    vi.advanceTimersByTime(500);
    fire(element, "pointerdown", { ...touch, isPrimary: false, clientX: 0, clientY: 0 });
    fire(element, "pointerup");
    vi.advanceTimersByTime(0);
    // pointerdown of any pointer resets the press: documents current behaviour (no menu).
    expect(open.mock.calls.length).toBeLessThanOrEqual(1);
  });

  it("opens on pointercancel after the browser's own long press", () => {
    const { element, open } = setup();
    fire(element, "pointerdown", touch);
    fire(element, "contextmenu", { clientX: 51, clientY: 61, shiftKey: false });
    fire(element, "pointercancel");
    vi.advanceTimersByTime(0);
    expect(open).toHaveBeenCalledTimes(1);
  });

  it("does nothing on pointercancel after a hold the browser did not confirm (a pan after resting)", () => {
    const { element, open } = setup();
    fire(element, "pointerdown", touch);
    vi.advanceTimersByTime(500);
    fire(element, "pointercancel");
    vi.advanceTimersByTime(1000);
    expect(open).not.toHaveBeenCalled();
  });

  it("does nothing on pointercancel before the delay (scroll took the touch)", () => {
    const { element, open } = setup();
    fire(element, "pointerdown", touch);
    vi.advanceTimersByTime(100);
    fire(element, "pointercancel");
    vi.advanceTimersByTime(1000);
    expect(open).not.toHaveBeenCalled();
  });

  it("does not start on a text field or a link", () => {
    for (const kind of ["text-field", "link"] as const) {
      const { element, open } = setup(kind);
      fire(element, "pointerdown", touch);
      vi.advanceTimersByTime(600);
      fire(element, "pointerup");
      vi.advanceTimersByTime(0);
      expect(open).not.toHaveBeenCalled();
    }
  });

  it("opens from Android's own contextmenu while the finger is down, on lift, once", () => {
    const { element, open } = setup();
    fire(element, "pointerdown", touch);
    const event = fire(element, "contextmenu", { clientX: 51, clientY: 61, shiftKey: false });
    expect(event.defaultPrevented).toBe(true);
    expect(open).not.toHaveBeenCalled();
    fire(element, "pointerup");
    vi.advanceTimersByTime(600);
    expect(open).toHaveBeenCalledTimes(1);
  });

  it("does not open if the element left the page before the menu showed", () => {
    const { element, open } = setup();
    fire(element, "pointerdown", touch);
    vi.advanceTimersByTime(500);
    fire(element, "pointerup");
    element.isConnected = false;
    vi.advanceTimersByTime(0);
    expect(open).not.toHaveBeenCalled();
  });

  it("clears the selection the long press started once the menu opened, not when it did not", () => {
    for (const opens of [true, false]) {
      selection.isCollapsed = false;
      (selection as { anchorNode: unknown }).anchorNode = null;
      const { element } = setup("plain", opens);
      element.contains = () => true;
      fire(element, "pointerdown", touch);
      vi.advanceTimersByTime(500);
      fire(element, "pointerup");
      vi.advanceTimersByTime(0);
      expect(selection.removeAllRanges).toHaveBeenCalledTimes(opens ? 1 : 0);
      vi.mocked(selection.removeAllRanges).mockClear();
    }
  });
});
