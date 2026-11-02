import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { retryWhenOnlineAndPeriodically } from "./unreachable-retry";

const LONGEST_RETRY_DELAY_MILLISECONDS = 30_000;

/** A retry that stays pending until the test settles it. */
function controlledRetry() {
  const settlers: (() => void)[] = [];
  const retry = vi.fn(() => new Promise<void>((resolve) => settlers.push(resolve)));
  return { retry, settleLatest: () => settlers.at(-1)?.() };
}

describe("retryWhenOnlineAndPeriodically", () => {
  let fakeWindow: EventTarget;

  beforeEach(() => {
    vi.useFakeTimers();
    fakeWindow = new EventTarget();
    vi.stubGlobal("window", fakeWindow);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("retries when the window goes back online", () => {
    const { retry } = controlledRetry();
    retryWhenOnlineAndPeriodically(retry);

    fakeWindow.dispatchEvent(new Event("online"));

    expect(retry).toHaveBeenCalledTimes(1);
  });

  it("retries again within 30 s after each failed attempt", async () => {
    const { retry, settleLatest } = controlledRetry();
    retryWhenOnlineAndPeriodically(retry);

    vi.advanceTimersByTime(LONGEST_RETRY_DELAY_MILLISECONDS);
    expect(retry).toHaveBeenCalledTimes(1);
    settleLatest();
    await vi.advanceTimersByTimeAsync(LONGEST_RETRY_DELAY_MILLISECONDS);

    expect(retry).toHaveBeenCalledTimes(2);
  });

  it("never starts a retry while one is still running", () => {
    const { retry } = controlledRetry();
    retryWhenOnlineAndPeriodically(retry);

    fakeWindow.dispatchEvent(new Event("online"));
    fakeWindow.dispatchEvent(new Event("online"));
    vi.advanceTimersByTime(LONGEST_RETRY_DELAY_MILLISECONDS * 3);

    expect(retry).toHaveBeenCalledTimes(1);
  });

  it("stops retrying once stopped, even after a running attempt ends", async () => {
    const { retry, settleLatest } = controlledRetry();
    const stop = retryWhenOnlineAndPeriodically(retry);
    fakeWindow.dispatchEvent(new Event("online"));

    stop();
    settleLatest();
    await vi.advanceTimersByTimeAsync(LONGEST_RETRY_DELAY_MILLISECONDS * 3);
    fakeWindow.dispatchEvent(new Event("online"));

    expect(retry).toHaveBeenCalledTimes(1);
  });
});
