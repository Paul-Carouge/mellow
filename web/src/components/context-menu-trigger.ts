// Opens Mellow's own menu instead of the browser's (DESIGN.md §6.33, ROADMAP item 9): a
// right-click, a long press on a touch screen, or Shift+F10 and the Menu key on the focused item.
// The browser's menu stays where it does something Mellow's cannot: inside text fields, on links
// (attachments included) and on selected text, and with Shift held while right-clicking.

import type { Attachment } from "svelte/attachments";

import type { MenuOrigin } from "./menu-placement";
import { durationTokenInMilliseconds, lengthTokenInPixels } from "./tokens";

/** Shift+F10, or the Menu key, as on every desktop (WAI-ARIA, Discord). */
export function isContextMenuKey(event: {
  readonly key: string;
  readonly shiftKey: boolean;
  readonly ctrlKey: boolean;
  readonly altKey: boolean;
  readonly metaKey: boolean;
}): boolean {
  if (event.ctrlKey || event.altKey || event.metaKey) {
    return false;
  }
  return event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey);
}

export interface NativeMenuFacts {
  /** The target is a text field or inside one: Paste, spelling and the like live there. */
  readonly isInTextField: boolean;
  /** A link (a file card too): Open in a new tab, Copy link address, Save link as. */
  readonly isOnLink: boolean;
  /** The pointer is on text the reader selected: Copy, Search. */
  readonly isOnSelectedText: boolean;
  /** Shift + right-click: the browser's menu on purpose, wherever it is. */
  readonly isShiftHeld: boolean;
}

export function shouldKeepNativeContextMenu(facts: NativeMenuFacts): boolean {
  return facts.isInTextField || facts.isOnLink || facts.isOnSelectedText || facts.isShiftHeld;
}

/**
 * A long press (touch only). The press is held long enough after `delayMilliseconds` (or as soon
 * as the browser says so with its own contextmenu) without the finger leaving `moveTolerance` of
 * where it landed; moving further means scrolling or selecting and calls it off, even after.
 *
 * The menu opens when the finger lifts (`release`), not while it rests: the lift would otherwise
 * light-dismiss a popover opened during the press (a primary pointerup outside any open popover
 * closes them all). The timer functions are passed in for tests.
 */
export function createLongPressDetector(options: {
  readonly delayMilliseconds: number;
  readonly moveTolerance: number;
  readonly schedule?: (callback: () => void, delayMilliseconds: number) => unknown;
  readonly cancelSchedule?: (handle: unknown) => void;
}): {
  start: (x: number, y: number) => void;
  move: (x: number, y: number) => void;
  /** The browser recognised a long press itself (Android's contextmenu): no need to wait. */
  holdNow: () => void;
  /** The finger lifted, or the browser took the touch over: where to open, or `null` for a tap. */
  release: () => { x: number; y: number } | null;
  cancel: () => void;
  /** Whether the press is still going on (started, not yet released or cancelled). */
  isPressing: () => boolean;
} {
  const schedule = options.schedule ?? ((callback, delay) => setTimeout(callback, delay));
  const cancelSchedule = options.cancelSchedule ?? ((handle) => clearTimeout(handle as ReturnType<typeof setTimeout>));
  let state: "idle" | "pressing" | "held" = "idle";
  let pending: unknown = null;
  let origin = { x: 0, y: 0 };

  function cancel() {
    if (pending !== null) {
      cancelSchedule(pending);
      pending = null;
    }
    state = "idle";
  }

  function hold() {
    if (pending !== null) {
      cancelSchedule(pending);
      pending = null;
    }
    if (state === "pressing") {
      state = "held";
    }
  }

  return {
    start(x, y) {
      cancel();
      state = "pressing";
      origin = { x, y };
      pending = schedule(hold, options.delayMilliseconds);
    },
    move(x, y) {
      if (state !== "idle" && Math.hypot(x - origin.x, y - origin.y) > options.moveTolerance) {
        cancel();
      }
    },
    holdNow: hold,
    release() {
      const wasHeld = state === "held";
      cancel();
      return wasHeld ? origin : null;
    },
    cancel,
    isPressing: () => state !== "idle",
  };
}

function nativeMenuFactsAt(target: EventTarget | null, isShiftHeld: boolean): NativeMenuFacts {
  const element = target instanceof Element ? target : null;
  const selection = window.getSelection();
  return {
    isInTextField: element?.closest("input, textarea, select, [contenteditable]:not([contenteditable=false])") != null,
    isOnLink: element?.closest("a[href]") != null,
    isOnSelectedText:
      element !== null && selection !== null && !selection.isCollapsed && selection.containsNode(element, true),
    isShiftHeld,
  };
}

/** Where focus goes back to: the element itself when it can take focus, else its first button. */
function invokerOf(element: HTMLElement): HTMLElement {
  return element.matches("button, a[href], [tabindex]")
    ? element
    : (element.querySelector<HTMLElement>("button, [tabindex]") ?? element);
}

/** Events a trigger inside another one already took: the outer one leaves them alone. */
const claimedEvents = new WeakSet<Event>();

/** The menu's opener: returns `false` when it has nothing to show, so the browser's menu does. */
export type OpenContextMenu = (origin: MenuOrigin, invoker: HTMLElement, source: "pointer" | "keyboard") => boolean;

/**
 * `{@attach contextMenuTrigger(open)}` on an item. A trigger inside another one (a message's
 * author inside the message) wins: the outer one ignores the events the inner one took.
 */
export function contextMenuTrigger(open: OpenContextMenu): Attachment<HTMLElement> {
  return (element) => {
    let isLiftAfterLongPress = false;
    const longPress = createLongPressDetector({
      delayMilliseconds: durationTokenInMilliseconds("motion-delay-long-press"),
      moveTolerance: lengthTokenInPixels("component-context-menu-long-press-move-tolerance"),
    });

    function openAfterLongPress() {
      const origin = longPress.release();
      if (origin === null) {
        return;
      }
      isLiftAfterLongPress = true;
      // After the lift's own light dismiss has run.
      setTimeout(() => {
        if (!element.isConnected || !open({ kind: "point", x: origin.x, y: origin.y }, invokerOf(element), "pointer")) {
          return;
        }
        // A long press on text may have started a selection: the menu replaces it.
        const selection = window.getSelection();
        if (selection && !selection.isCollapsed && element.contains(selection.anchorNode)) {
          selection.removeAllRanges();
        }
      }, 0);
    }

    function handleContextMenu(event: MouseEvent) {
      if (claimedEvents.has(event)) {
        return;
      }
      claimedEvents.add(event);
      if (shouldKeepNativeContextMenu(nativeMenuFactsAt(event.target, event.shiftKey))) {
        longPress.cancel();
        return;
      }
      if (longPress.isPressing()) {
        // Android Chrome's own long press, finger still down: the menu opens when it lifts.
        event.preventDefault();
        longPress.holdNow();
        return;
      }
      if (open({ kind: "point", x: event.clientX, y: event.clientY }, invokerOf(element), "pointer")) {
        event.preventDefault();
      }
    }

    function handleKeydown(event: KeyboardEvent) {
      if (!isContextMenuKey(event) || claimedEvents.has(event)) {
        return;
      }
      claimedEvents.add(event);
      if (shouldKeepNativeContextMenu(nativeMenuFactsAt(event.target, false))) {
        return;
      }
      const focused = event.target instanceof HTMLElement ? event.target : invokerOf(element);
      const origin: MenuOrigin = { kind: "anchor", bounds: focused.getBoundingClientRect(), align: "start" };
      if (open(origin, focused, "keyboard")) {
        event.preventDefault();
      }
    }

    function handlePointerDown(event: PointerEvent) {
      isLiftAfterLongPress = false;
      longPress.cancel();
      if (event.pointerType !== "touch" || !event.isPrimary || claimedEvents.has(event)) {
        return;
      }
      claimedEvents.add(event);
      if (shouldKeepNativeContextMenu(nativeMenuFactsAt(event.target, false))) {
        return;
      }
      longPress.start(event.clientX, event.clientY);
    }

    function handleTouchEnd(event: TouchEvent) {
      // The lift after a long press is not a tap: no click on the row, and none of the mouse
      // events a tap brings, whose release would light-dismiss the menu at once.
      if (isLiftAfterLongPress) {
        isLiftAfterLongPress = false;
        event.preventDefault();
      }
    }

    const handleMove = (event: PointerEvent) => longPress.move(event.clientX, event.clientY);
    const listeners: [string, EventListener, boolean][] = [
      ["contextmenu", handleContextMenu as EventListener, false],
      ["keydown", handleKeydown as EventListener, false],
      ["pointerdown", handlePointerDown as EventListener, false],
      ["pointermove", handleMove as EventListener, false],
      ["pointerup", openAfterLongPress, false],
      // Scrolling takes the touch over too, but it moved first, which called the press off.
      ["pointercancel", openAfterLongPress, false],
      ["touchend", handleTouchEnd as EventListener, false],
    ];
    for (const [type, listener, capture] of listeners) {
      element.addEventListener(type, listener, capture);
    }
    return () => {
      longPress.cancel();
      for (const [type, listener, capture] of listeners) {
        element.removeEventListener(type, listener, capture);
      }
    };
  };
}
