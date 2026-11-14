// When a hover card shows, switches and goes (DESIGN.md §5.7.2): it opens after the pointer (or
// keyboard focus) rests on a trigger, stays while the pointer is on the trigger or on the card, and
// closes a short grace after it left both, so the pointer can travel to the card. Moving onto
// another trigger while a card is open switches after the same grace: no close-and-reopen flash,
// and a name crossed on the way to the card does not take it over.

export interface HoverCardTimers {
  set(callback: () => void, milliseconds: number): unknown;
  clear(handle: unknown): void;
}

const browserTimers: HoverCardTimers = {
  set: (callback, milliseconds) => setTimeout(callback, milliseconds),
  clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export interface HoverCardControllerOptions<Target> {
  /** Read when a wait starts, so a token changed meanwhile (tests, themes) is followed. */
  openDelayMilliseconds: () => number;
  closeDelayMilliseconds: () => number;
  /** The same trigger (an element): re-entering it keeps its card rather than reopening it. */
  isSameTrigger: (left: Target, right: Target) => boolean;
  /** Read when a wait ends: a trigger that left the page (a row regrouped) never gets its card. */
  isStillThere: (target: Target) => boolean;
  onchange: (shown: Target | null) => void;
  timers?: HoverCardTimers;
}

export class HoverCardController<Target> {
  readonly #options: HoverCardControllerOptions<Target>;
  readonly #timers: HoverCardTimers;
  #shown: Target | null = null;
  /** What the running wait will show: a card, or null to close. */
  #pending: Target | null = null;
  #timer: unknown = null;

  constructor(options: HoverCardControllerOptions<Target>) {
    this.#options = options;
    this.#timers = options.timers ?? browserTimers;
  }

  get shown(): Target | null {
    return this.#shown;
  }

  /** The pointer entered a trigger, or keyboard focus landed on one. */
  enterTrigger(target: Target): void {
    if (this.#shown !== null && this.#options.isSameTrigger(this.#shown, target)) {
      this.#cancelWait();
      return;
    }
    const delay = this.#shown === null ? this.#options.openDelayMilliseconds() : this.#options.closeDelayMilliseconds();
    this.#wait(delay, target);
  }

  /** The pointer left a trigger, or focus left it. */
  leaveTrigger(): void {
    if (this.#shown === null) {
      this.#cancelWait();
      return;
    }
    this.#wait(this.#options.closeDelayMilliseconds(), null);
  }

  /** The pointer reached the card: it stays, and a switch begun on the way is dropped. */
  enterCard(): void {
    if (this.#shown !== null) {
      this.#cancelWait();
    }
  }

  leaveCard(): void {
    this.leaveTrigger();
  }

  /**
   * A trigger left the page: a wait for its card is dropped. Returns whether its card is the one
   * shown, which the caller closes once it may change state.
   */
  dropTrigger(target: Target): boolean {
    if (this.#pending !== null && this.#options.isSameTrigger(this.#pending, target)) {
      this.#cancelWait();
    }
    return this.#shown !== null && this.#options.isSameTrigger(this.#shown, target);
  }

  /** Esc, a press, a scroll, the modal opening: the card goes at once and nothing is pending. */
  close(): void {
    this.#cancelWait();
    this.#show(null);
  }

  #wait(milliseconds: number, next: Target | null): void {
    this.#cancelWait();
    this.#pending = next;
    this.#timer = this.#timers.set(() => {
      this.#timer = null;
      this.#pending = null;
      // Switching to a trigger that is gone closes the card rather than leaving the old one up.
      this.#show(next !== null && this.#options.isStillThere(next) ? next : null);
    }, milliseconds);
  }

  #cancelWait(): void {
    if (this.#timer !== null) {
      this.#timers.clear(this.#timer);
      this.#timer = null;
    }
    this.#pending = null;
  }

  #show(next: Target | null): void {
    if (this.#shown === next) {
      return;
    }
    this.#shown = next;
    this.#options.onchange(next);
  }
}
