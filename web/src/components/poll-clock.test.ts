import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { pollClockNow, watchPollClock } from "./poll-clock.svelte";

describe("the shared poll clock", () => {
  beforeEach(() => vi.useFakeTimers({ now: 1_000 }));
  afterEach(() => vi.useRealTimers());

  it("runs one timer while anything watches it and stops with the last watcher", () => {
    const first = watchPollClock();
    const second = watchPollClock();
    expect(vi.getTimerCount()).toBe(1);
    vi.advanceTimersByTime(60_000);
    expect(pollClockNow()).toBe(61_000);
    first();
    expect(vi.getTimerCount()).toBe(1);
    second();
    expect(vi.getTimerCount()).toBe(0);
  });
});
