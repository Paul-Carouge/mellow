import { describe, expect, it } from "vitest";

import { HoverCardController, type HoverCardTimers } from "./hover-card-controller";
import { hoverCardPosition } from "./hover-card-placement";

/** A clock moved by hand: deterministic, no real time. */
function manualTimers(): HoverCardTimers & { advance(milliseconds: number): void; pending(): number } {
  let now = 0;
  let nextHandle = 1;
  const waiting = new Map<number, { at: number; callback: () => void }>();
  return {
    set(callback, milliseconds) {
      const handle = nextHandle++;
      waiting.set(handle, { at: now + milliseconds, callback });
      return handle;
    },
    clear(handle) {
      waiting.delete(handle as number);
    },
    advance(milliseconds) {
      now += milliseconds;
      for (const [handle, wait] of [...waiting].sort(([, left], [, right]) => left.at - right.at)) {
        if (wait.at <= now && waiting.delete(handle)) {
          wait.callback();
        }
      }
    },
    pending: () => waiting.size,
  };
}

const OPEN_DELAY = 450;
const CLOSE_DELAY = 200;

function controller() {
  const timers = manualTimers();
  const changes: (string | null)[] = [];
  const gone = new Set<string>();
  const card = new HoverCardController<string>({
    openDelayMilliseconds: () => OPEN_DELAY,
    closeDelayMilliseconds: () => CLOSE_DELAY,
    isSameTrigger: (left, right) => left === right,
    isStillThere: (target) => !gone.has(target),
    onchange: (shown) => changes.push(shown),
    timers,
  });
  return { card, timers, changes, gone };
}

describe("hover card timing", () => {
  it("opens only after the pointer rests for the whole delay", () => {
    const { card, timers, changes } = controller();
    card.enterTrigger("ana");
    timers.advance(OPEN_DELAY - 1);
    expect(card.shown).toBeNull();
    timers.advance(1);
    expect(card.shown).toBe("ana");
    expect(changes).toEqual(["ana"]);
  });

  it("opens nothing when the pointer only crosses a name", () => {
    const { card, timers, changes } = controller();
    card.enterTrigger("ana");
    timers.advance(100);
    card.leaveTrigger();
    card.enterTrigger("bruno");
    timers.advance(100);
    card.leaveTrigger();
    timers.advance(OPEN_DELAY * 2);
    expect(changes).toEqual([]);
    expect(timers.pending()).toBe(0);
  });

  it("stays open while the pointer travels from the name to the card", () => {
    const { card, timers, changes } = controller();
    card.enterTrigger("ana");
    timers.advance(OPEN_DELAY);
    card.leaveTrigger();
    timers.advance(CLOSE_DELAY - 1);
    card.enterCard();
    timers.advance(CLOSE_DELAY * 5);
    expect(card.shown).toBe("ana");
    card.leaveCard();
    timers.advance(CLOSE_DELAY);
    expect(card.shown).toBeNull();
    expect(changes).toEqual(["ana", null]);
  });

  it("keeps the card when the pointer comes back to its own name", () => {
    const { card, timers, changes } = controller();
    card.enterTrigger("ana");
    timers.advance(OPEN_DELAY);
    card.leaveTrigger();
    card.enterTrigger("ana");
    timers.advance(OPEN_DELAY * 2);
    expect(changes).toEqual(["ana"]);
  });

  it("switches to another name after the grace, without closing in between", () => {
    const { card, timers, changes } = controller();
    card.enterTrigger("ana");
    timers.advance(OPEN_DELAY);
    card.leaveTrigger();
    card.enterTrigger("bruno");
    timers.advance(CLOSE_DELAY);
    expect(changes).toEqual(["ana", "bruno"]);
  });

  it("does not switch when another name is only crossed on the way to the card", () => {
    const { card, timers, changes } = controller();
    card.enterTrigger("ana");
    timers.advance(OPEN_DELAY);
    card.leaveTrigger();
    card.enterTrigger("bruno");
    timers.advance(CLOSE_DELAY / 2);
    card.leaveTrigger();
    card.enterCard();
    timers.advance(CLOSE_DELAY * 5);
    expect(changes).toEqual(["ana"]);
  });

  it("never opens a card for a name that left the page while it waited", () => {
    const { card, timers, changes, gone } = controller();
    card.enterTrigger("ana");
    timers.advance(100);
    gone.add("ana");
    timers.advance(OPEN_DELAY);
    expect(changes).toEqual([]);
  });

  it("closes rather than switching to a name that left the page", () => {
    const { card, timers, changes, gone } = controller();
    card.enterTrigger("ana");
    timers.advance(OPEN_DELAY);
    card.enterTrigger("bruno");
    gone.add("bruno");
    timers.advance(CLOSE_DELAY);
    expect(changes).toEqual(["ana", null]);
  });

  it("drops the wait of a trigger that is removed, and says when its card is the one shown", () => {
    const { card, timers, changes } = controller();
    card.enterTrigger("ana");
    expect(card.dropTrigger("ana")).toBe(false);
    timers.advance(OPEN_DELAY * 2);
    expect(changes).toEqual([]);
    expect(timers.pending()).toBe(0);
    card.enterTrigger("bruno");
    timers.advance(OPEN_DELAY);
    expect(card.dropTrigger("bruno")).toBe(true);
    expect(card.dropTrigger("ana")).toBe(false);
  });

  it("closes at once and forgets any wait", () => {
    const { card, timers, changes } = controller();
    card.enterTrigger("ana");
    timers.advance(OPEN_DELAY);
    card.enterTrigger("bruno");
    card.close();
    timers.advance(OPEN_DELAY * 2);
    expect(changes).toEqual(["ana", null]);
    expect(timers.pending()).toBe(0);
  });
});

describe("hover card placement", () => {
  const viewport = { width: 1440, height: 900 };
  const card = { width: 300, height: 260 };
  const spacing = { offset: 8, margin: 8 };
  // The docked member list: a 240 px column at the window's trailing edge.
  const memberColumn = { top: 56, bottom: 892, left: 1192, right: 1432 };

  it("puts a member's card on the leading side of the member list, level with the row", () => {
    const row = { top: 300, bottom: 342, left: 1200, right: 1424 };
    expect(hoverCardPosition("beside", row, memberColumn, card, viewport, spacing)).toEqual({
      top: 300,
      left: 1192 - 8 - 300,
      side: "left",
    });
  });

  it("puts a voice participant's card on the trailing side of the navigator", () => {
    const navigator = { top: 8, bottom: 892, left: 8, right: 320 };
    const row = { top: 400, bottom: 432, left: 80, right: 312 };
    expect(hoverCardPosition("beside", row, navigator, card, viewport, spacing)).toEqual({ top: 400, left: 328, side: "right" });
  });

  it("keeps a row's card inside the window near its bottom", () => {
    const row = { top: 860, bottom: 892, left: 1200, right: 1424 };
    expect(hoverCardPosition("beside", row, memberColumn, card, viewport, spacing).top).toBe(900 - 8 - 260);
  });

  it("goes under the row when neither side of a full-window column has room", () => {
    const phone = { width: 600, height: 800 };
    const column = { top: 0, bottom: 800, left: 0, right: 600 };
    const row = { top: 100, bottom: 142, left: 8, right: 592 };
    expect(hoverCardPosition("beside", row, column, card, phone, spacing)).toEqual({ top: 150, left: 8, side: "below" });
  });

  it("opens under an author's name, start-aligned, and above it near the window's foot", () => {
    const name = { top: 200, bottom: 220, left: 400, right: 480 };
    expect(hoverCardPosition("below", name, null, card, viewport, spacing)).toEqual({ top: 228, left: 400, side: "below" });
    const low = { top: 800, bottom: 820, left: 400, right: 480 };
    expect(hoverCardPosition("below", low, null, card, viewport, spacing)).toEqual({ top: 800 - 8 - 260, left: 400, side: "above" });
  });

  it("never leaves the window on the trailing side", () => {
    const name = { top: 200, bottom: 220, left: 1300, right: 1380 };
    expect(hoverCardPosition("below", name, null, card, viewport, spacing).left).toBe(1440 - 8 - 300);
  });
});
