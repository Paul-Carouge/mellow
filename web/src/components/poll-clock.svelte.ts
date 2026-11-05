// One clock for every open poll on screen (docs/design/web-polls.md): it ticks once a minute while
// at least one poll watches it, so a long channel of polls never runs a timer per row.

const POLL_CLOCK_TICK_MILLISECONDS = 60_000;

const clock = $state({ now: Date.now() });
let watchers = 0;
let timer: ReturnType<typeof setInterval> | null = null;

/** The time as of the last tick; read inside a component to follow it. */
export function pollClockNow(): number {
  return clock.now;
}

/** From an open poll's `$effect`; the returned function stops watching. */
export function watchPollClock(): () => void {
  watchers += 1;
  if (timer === null) {
    clock.now = Date.now();
    timer = setInterval(() => (clock.now = Date.now()), POLL_CLOCK_TICK_MILLISECONDS);
  }
  return () => {
    watchers -= 1;
    if (watchers === 0 && timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  };
}
