import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Snowflake } from "../lib/protocol";
import { IMAGE_RETRY_DELAY_MILLISECONDS, retryImageOnceThenMarkFailed } from "./image-retry";

describe("image retry", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("retries a failed image once after the delay, then gives up", () => {
    const id = "42" as Snowflake;
    const retriedIds = new Set<Snowflake>();
    const failedIds = new Set<Snowflake>();

    retryImageOnceThenMarkFailed(id, retriedIds, failedIds);
    vi.advanceTimersByTime(IMAGE_RETRY_DELAY_MILLISECONDS - 1);
    expect(retriedIds.has(id)).toBe(false);
    vi.advanceTimersByTime(1);
    expect(retriedIds.has(id)).toBe(true);
    expect(failedIds.has(id)).toBe(false);

    retryImageOnceThenMarkFailed(id, retriedIds, failedIds);
    vi.runAllTimers();
    expect(failedIds.has(id)).toBe(true);
  });
});
