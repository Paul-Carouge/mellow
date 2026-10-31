import { describe, expect, it } from "vitest";

import { problemLook, signalLook } from "./voice-panel";

describe("signalLook", () => {
  it("drops a bar as the round trip grows, and says it in words", () => {
    expect(signalLook(42)).toEqual({ icon: "cell-signal-high", quality: "good", description: "Latency: 42 ms, good connection" });
    expect(signalLook(180)).toMatchObject({ icon: "cell-signal-medium", quality: "medium" });
    expect(signalLook(400)).toMatchObject({ icon: "cell-signal-low", quality: "poor" });
  });

  it("measures before judging", () => {
    expect(signalLook(null)).toEqual({ icon: "cell-signal-high", quality: null, description: "Measuring latency…" });
  });
});

describe("problemLook", () => {
  it("offers a retry only where waiting may help", () => {
    expect(problemLook({ kind: "refused", channelId: "1", reason: "channel_full" }, "lounge")).toEqual({
      title: "Couldn't join lounge",
      sentence: "The channel is full. Try again when someone leaves.",
      tone: "error",
      canRetry: true,
    });
    expect(problemLook({ kind: "refused", channelId: "1", reason: "missing_permissions" }, "lounge").canRetry).toBe(false);
    expect(problemLook({ kind: "unreachable", channelId: "1" }, "lounge").canRetry).toBe(true);
    expect(problemLook({ kind: "disconnected", channelId: "1", reason: "media_timeout" }, "lounge").canRetry).toBe(true);
    expect(problemLook({ kind: "disconnected", channelId: "1", reason: "channel_deleted" }, "lounge").canRetry).toBe(false);
    expect(problemLook({ kind: "unsupported" }, "lounge").canRetry).toBe(false);
  });

  it("tells a move to another tab as a notice, not an error", () => {
    expect(problemLook({ kind: "disconnected", channelId: "1", reason: "moved_elsewhere" }, "lounge")).toMatchObject({
      title: "Voice moved",
      tone: "notice",
      canRetry: false,
    });
  });

  it("still explains a reason a newer instance sends", () => {
    const fromTheFuture = { kind: "refused", channelId: "1", reason: "solar_flare" } as unknown as Parameters<typeof problemLook>[0];
    expect(problemLook(fromTheFuture, "lounge").sentence).toBe("The instance ended the connection.");
  });
});
