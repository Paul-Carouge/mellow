import type { Checklist, ChecklistTick, ProgressBar } from "./protocol";

/**
 * The newer of two copies of a checklist or progress bar (ADR-0047): `incoming` only when its
 * revision is higher, so a pin's `MESSAGE_UPDATE` or a history page read before a tick never undoes
 * it, and an equal copy keeps the cached object (row identity).
 */
export function newestTracker<Tracker extends Checklist | ProgressBar>(
  cached: Tracker | undefined,
  incoming: Tracker | undefined,
): Tracker | undefined {
  if (!cached || !incoming) {
    return cached ?? incoming;
  }
  return incoming.revision > cached.revision ? incoming : cached;
}

/** The checklist with one item ticked (`tick`) or unticked (`undefined`); the same object when nothing changes. */
export function withItemTick(checklist: Checklist, itemId: number, tick: ChecklistTick | undefined): Checklist {
  const item = checklist.items.find((candidate) => candidate.id === itemId);
  if (!item || (item.checked === undefined) === (tick === undefined)) {
    return checklist;
  }
  return {
    ...checklist,
    items: checklist.items.map((candidate) => {
      if (candidate.id !== itemId) {
        return candidate;
      }
      const { checked: _, ...unticked } = candidate;
      return tick ? { ...unticked, checked: tick } : unticked;
    }),
  };
}

/** How many items are ticked, out of all. */
export function checklistProgress(checklist: Checklist): { done: number; total: number } {
  return {
    done: checklist.items.filter((item) => item.checked !== undefined).length,
    total: checklist.items.length,
  };
}

/** A whole percentage, rounded down so 99.6 % never reads as done. */
export function progressPercent(value: number, maximum: number): number {
  return maximum > 0 ? Math.floor((Math.min(Math.max(value, 0), maximum) * 100) / maximum) : 0;
}
