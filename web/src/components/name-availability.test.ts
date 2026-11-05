import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../lib/api";
import { NAME_CHECK_PAUSE_MILLISECONDS, NameAvailabilityCheck, tidiedName } from "./name-availability.svelte";

describe("name availability", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("asks once typing pauses, never for the current or an empty name", async () => {
    const asked: string[] = [];
    const check = new NameAvailabilityCheck((name) => {
      asked.push(name);
      return Promise.resolve(name !== "Basile");
    });
    check.update("  Pa", "Aurore");
    check.update("  Paul   C ", "Aurore");
    expect(check.state).toBe("checking");
    await vi.advanceTimersByTimeAsync(NAME_CHECK_PAUSE_MILLISECONDS);
    expect(asked).toEqual(["Paul C"]);
    expect(check.state).toBe("available");

    check.update("Basile", "Aurore");
    await vi.advanceTimersByTimeAsync(NAME_CHECK_PAUSE_MILLISECONDS);
    expect(check.state).toBe("taken");

    check.update(" Aurore ", "Aurore");
    check.update("", "Aurore");
    await vi.advanceTimersByTimeAsync(NAME_CHECK_PAUSE_MILLISECONDS);
    expect(asked).toHaveLength(2);
    expect(check.state).toBe("idle");
  });

  it("keeps the newest draft's answer and tells a refused name from a lost answer", async () => {
    const answers: ((isAvailable: boolean) => void)[] = [];
    const check = new NameAvailabilityCheck(
      () => new Promise<boolean>((resolve) => answers.push(resolve)),
    );
    check.update("Old", "Aurore");
    await vi.advanceTimersByTimeAsync(NAME_CHECK_PAUSE_MILLISECONDS);
    check.update("New", "Aurore");
    await vi.advanceTimersByTimeAsync(NAME_CHECK_PAUSE_MILLISECONDS);
    answers[1]?.(true);
    await Promise.resolve();
    answers[0]?.(false);
    await Promise.resolve();
    expect(check.state).toBe("available");

    const refusing = new NameAvailabilityCheck(() => Promise.reject(new ApiError(400, "VALIDATION_FAILED")));
    refusing.update("a", "Aurore");
    await vi.advanceTimersByTimeAsync(NAME_CHECK_PAUSE_MILLISECONDS);
    expect(refusing.state).toBe("invalid");
    const offline = new NameAvailabilityCheck(() => Promise.reject(new ApiError(0, "NETWORK")));
    offline.update("Paul", "Aurore");
    await vi.advanceTimersByTimeAsync(NAME_CHECK_PAUSE_MILLISECONDS);
    expect(offline.state).toBe("unknown");
    offline.update("Paula", "Aurore");
    offline.cancel();
    await vi.advanceTimersByTimeAsync(NAME_CHECK_PAUSE_MILLISECONDS);
    expect(offline.state).toBe("idle");
  });

  it("tidies names as the instance stores them", () => {
    expect(tidiedName("  Jean \t Paul ")).toBe("Jean Paul");
  });
});
