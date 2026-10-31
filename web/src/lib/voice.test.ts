import { afterEach, describe, expect, it, vi } from "vitest";

import { latencyQuality, microphoneProblemOf, participantChange, roundTripMillisecondsOf, waitForIceGathering } from "./voice";

describe("latencyQuality", () => {
  it("reads good up to 100 ms, medium up to 250 ms, poor above", () => {
    expect(latencyQuality(0)).toBe("good");
    expect(latencyQuality(100)).toBe("good");
    expect(latencyQuality(101)).toBe("medium");
    expect(latencyQuality(250)).toBe("medium");
    expect(latencyQuality(251)).toBe("poor");
  });
});

describe("roundTripMillisecondsOf", () => {
  const pair = (id: string, seconds: number, extra: Record<string, unknown> = {}) => ({
    type: "candidate-pair",
    id,
    currentRoundTripTime: seconds,
    ...extra,
  });

  it("prefers the transport's selected pair", () => {
    const stats = [
      { type: "transport", selectedCandidatePairId: "b" },
      pair("a", 0.5, { nominated: true, state: "succeeded" }),
      pair("b", 0.0424),
    ];
    expect(roundTripMillisecondsOf(stats)).toBe(42);
  });

  it("falls back to a nominated pair that succeeded (Firefox has no selected pair id)", () => {
    const stats = [pair("a", 0.3, { nominated: false, state: "succeeded" }), pair("b", 0.12, { nominated: true, state: "succeeded" })];
    expect(roundTripMillisecondsOf(stats)).toBe(120);
  });

  it("is null before anything was measured, and on unexpected shapes", () => {
    expect(roundTripMillisecondsOf([])).toBeNull();
    expect(roundTripMillisecondsOf([pair("a", Number.NaN, { nominated: true, state: "succeeded" })])).toBeNull();
    expect(roundTripMillisecondsOf([null, 3, "x", { type: "candidate-pair" }])).toBeNull();
  });
});

describe("microphoneProblemOf", () => {
  const named = (name: string) => Object.assign(new Error(name), { name });

  it("tells a refusal, a missing device and a busy one apart", () => {
    expect(microphoneProblemOf(named("NotAllowedError"))).toBe("denied");
    expect(microphoneProblemOf(named("SecurityError"))).toBe("denied");
    expect(microphoneProblemOf(named("NotFoundError"))).toBe("missing");
    expect(microphoneProblemOf(named("OverconstrainedError"))).toBe("missing");
    expect(microphoneProblemOf(named("NotReadableError"))).toBe("busy");
    expect(microphoneProblemOf(named("AbortError"))).toBe("busy");
  });

  it("reads anything else as unavailable (no mediaDevices outside HTTPS)", () => {
    expect(microphoneProblemOf(new TypeError("navigator.mediaDevices is unavailable"))).toBe("unavailable");
    expect(microphoneProblemOf("nope")).toBe("unavailable");
  });
});

describe("participantChange", () => {
  const inChannel = (channelId: string) => ({ channel_id: channelId });

  it("says who came into or left your channel, moves included", () => {
    expect(participantChange(undefined, inChannel("a"), "a")).toBe("joined");
    expect(participantChange(inChannel("b"), inChannel("a"), "a")).toBe("joined");
    expect(participantChange(inChannel("a"), undefined, "a")).toBe("left");
    expect(participantChange(inChannel("a"), inChannel("b"), "a")).toBe("left");
  });

  it("ignores changes elsewhere, mute changes, and everything while you are not connected", () => {
    expect(participantChange(inChannel("a"), inChannel("a"), "a")).toBeNull();
    expect(participantChange(undefined, inChannel("b"), "a")).toBeNull();
    expect(participantChange(undefined, inChannel("a"), null)).toBeNull();
  });
});

describe("waitForIceGathering", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  function fakePeer(state: RTCIceGatheringState) {
    const listeners: (() => void)[] = [];
    return {
      iceGatheringState: state,
      addEventListener: (_type: string, listener: () => void) => void listeners.push(listener),
      removeEventListener: () => void listeners.splice(0),
      complete() {
        this.iceGatheringState = "complete";
        listeners.forEach((listener) => listener());
      },
      listeners,
    };
  }

  it("resolves at once when gathering is already complete", async () => {
    await expect(waitForIceGathering(fakePeer("complete"), 1_000)).resolves.toBeUndefined();
  });

  it("resolves when gathering completes, and stops listening", async () => {
    const peer = fakePeer("gathering");
    const waiting = waitForIceGathering(peer, 1_000);
    peer.complete();
    await waiting;
    expect(peer.listeners).toHaveLength(0);
  });

  it("gives up after the timeout, so the offer leaves with what was found", async () => {
    vi.useFakeTimers();
    let isResolved = false;
    void waitForIceGathering(fakePeer("gathering"), 1_000).then(() => (isResolved = true));
    await vi.advanceTimersByTimeAsync(999);
    expect(isResolved).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(isResolved).toBe(true);
  });
});
