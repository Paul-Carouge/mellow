// Carrying a channel row or a category header with a pointer (DESIGN.md §5.3). A mouse or pen
// press becomes a drag once it moves past `component.channel-drag.start-distance`; a finger must
// first hold still for `motion.delay.long-press`, so a swipe still scrolls the list and a press
// that lifts without moving still opens the row's menu (`context-menu-trigger.ts`). The menu's
// Move items are the keyboard and single-pointer path (WCAG 2.2 2.5.7).

import type { Attachment } from "svelte/attachments";

import { durationTokenInMilliseconds, lengthTokenInPixels } from "./tokens";

export type DragStep = "none" | "start" | "move" | "cancel";

/**
 * The gesture alone, without the DOM: `press` on pointerdown, `move` on each pointermove, then
 * `release` (true when a drag was dropped) or `cancel`. The timer functions are passed in for tests.
 */
export function createDragGesture(options: {
  readonly startDistance: number;
  readonly holdDelayMilliseconds: number;
  readonly moveTolerance: number;
  readonly schedule?: (callback: () => void, delayMilliseconds: number) => unknown;
  readonly cancelSchedule?: (handle: unknown) => void;
}): {
  press: (pointerType: string, x: number, y: number) => void;
  move: (x: number, y: number) => DragStep;
  release: () => boolean;
  cancel: () => void;
  /** A finger held long enough: its moves must not scroll the list any more. */
  isHoldingOrDragging: () => boolean;
  isDragging: () => boolean;
} {
  const schedule = options.schedule ?? ((callback, delay) => setTimeout(callback, delay));
  const cancelSchedule = options.cancelSchedule ?? ((handle) => clearTimeout(handle as ReturnType<typeof setTimeout>));
  let state: "idle" | "pressing" | "touch-waiting" | "touch-held" | "dragging" = "idle";
  let origin = { x: 0, y: 0 };
  let pending: unknown = null;

  function clearTimer() {
    if (pending !== null) {
      cancelSchedule(pending);
      pending = null;
    }
  }

  function cancel() {
    clearTimer();
    state = "idle";
  }

  return {
    press(pointerType, x, y) {
      cancel();
      origin = { x, y };
      if (pointerType === "touch") {
        state = "touch-waiting";
        pending = schedule(() => {
          pending = null;
          if (state === "touch-waiting") {
            state = "touch-held";
          }
        }, options.holdDelayMilliseconds);
      } else {
        state = "pressing";
      }
    },
    move(x, y) {
      const distance = Math.hypot(x - origin.x, y - origin.y);
      switch (state) {
        case "idle":
          return "none";
        case "dragging":
          return "move";
        case "pressing":
          if (distance > options.startDistance) {
            state = "dragging";
            return "start";
          }
          return "none";
        case "touch-waiting":
          // Moving before the hold is a scroll or a swipe: not a drag.
          if (distance > options.moveTolerance) {
            cancel();
            return "cancel";
          }
          return "none";
        case "touch-held":
          if (distance > options.moveTolerance) {
            state = "dragging";
            return "start";
          }
          return "none";
      }
    },
    release() {
      const wasDragging = state === "dragging";
      cancel();
      return wasDragging;
    },
    cancel,
    isHoldingOrDragging: () => state === "touch-held" || state === "dragging",
    isDragging: () => state === "dragging",
  };
}

export interface DragHandlers {
  /** Read at each press: false while another move is in flight, or without the permission. */
  readonly canStart: () => boolean;
  /** `source` is the element carried, for its size and place. */
  readonly onstart: (x: number, y: number, source: HTMLElement) => void;
  readonly onmove: (x: number, y: number) => void;
  /** `isDropped` is false for Esc and for a pointer the browser took over. */
  readonly onend: (isDropped: boolean) => void;
}

/** `{@attach channelDragSource(handlers)}` on a row or a header. */
export function channelDragSource(handlers: DragHandlers): Attachment<HTMLElement> {
  return (element) => {
    const gesture = createDragGesture({
      startDistance: lengthTokenInPixels("component-channel-drag-start-distance"),
      holdDelayMilliseconds: durationTokenInMilliseconds("motion-delay-long-press"),
      moveTolerance: lengthTokenInPixels("component-context-menu-long-press-move-tolerance"),
    });
    let pointerId: number | null = null;

    // While pressed, moves and the release are followed on the window: a quick flick leaves the
    // row before it travels the start distance, and the row may be rebuilt under the pointer.
    // A press that may become a drag selects no text on its way (a row, a heading, an open menu).
    const preventSelection = (event: Event) => event.preventDefault();

    function listenToWindow(isListening: boolean) {
      if (isListening) {
        window.addEventListener("pointermove", handlePointerMove);
        window.addEventListener("pointerup", handlePointerUp);
        window.addEventListener("pointercancel", handlePointerCancel);
        document.addEventListener("selectstart", preventSelection);
      } else {
        window.removeEventListener("pointermove", handlePointerMove);
        window.removeEventListener("pointerup", handlePointerUp);
        window.removeEventListener("pointercancel", handlePointerCancel);
        document.removeEventListener("selectstart", preventSelection);
      }
    }

    function end(isDropped: boolean) {
      const wasDragging = gesture.isDragging();
      gesture.cancel();
      pointerId = null;
      listenToWindow(false);
      window.removeEventListener("keydown", handleEscape, true);
      if (wasDragging) {
        handlers.onend(isDropped);
      }
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        end(false);
      }
    }

    function handlePointerDown(event: PointerEvent) {
      if (!event.isPrimary || event.button !== 0 || !handlers.canStart()) {
        return;
      }
      end(false);
      pointerId = event.pointerId;
      gesture.press(event.pointerType, event.clientX, event.clientY);
      listenToWindow(true);
    }

    function handlePointerMove(event: PointerEvent) {
      if (event.pointerId !== pointerId) {
        return;
      }
      const step = gesture.move(event.clientX, event.clientY);
      if (step === "start") {
        window.addEventListener("keydown", handleEscape, true);
        handlers.onstart(event.clientX, event.clientY, element);
        handlers.onmove(event.clientX, event.clientY);
      } else if (step === "move") {
        handlers.onmove(event.clientX, event.clientY);
      } else if (step === "cancel") {
        end(false);
      }
    }

    function handlePointerUp(event: PointerEvent) {
      if (event.pointerId !== pointerId) {
        return;
      }
      if (gesture.isDragging()) {
        // The release is not a click: it would open the channel or fold the category now under the
        // pointer (the drop may have rebuilt the row there). The guard ends with this turn.
        const swallowClick = (click: Event) => {
          click.stopImmediatePropagation();
          click.preventDefault();
        };
        window.addEventListener("click", swallowClick, { capture: true, once: true });
        setTimeout(() => window.removeEventListener("click", swallowClick, { capture: true }), 0);
      }
      end(true);
    }

    function handlePointerCancel(event: PointerEvent) {
      if (event.pointerId === pointerId) {
        end(false);
      }
    }

    function handleTouchMove(event: TouchEvent) {
      // Once a finger held still, the list must not scroll under the carried row.
      if (gesture.isHoldingOrDragging() && event.cancelable) {
        event.preventDefault();
      }
    }

    element.addEventListener("pointerdown", handlePointerDown);
    element.addEventListener("touchmove", handleTouchMove, { passive: false });
    return () => {
      end(false);
      element.removeEventListener("pointerdown", handlePointerDown);
      element.removeEventListener("touchmove", handleTouchMove);
    };
  };
}
