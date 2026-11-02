import { describe, expect, it } from "vitest";

import { preferredCodecs, SCREEN_ENCODINGS, screenShareProblemOf, withActiveLayers } from "./screen-share";

describe("screen share layers", () => {
  it("keeps only the watched layers on, and leaves them be when the list is empty", () => {
    const active = withActiveLayers(SCREEN_ENCODINGS, ["q", "h"]);
    expect(active.map((encoding) => [encoding.rid, encoding.active])).toEqual([
      ["q", true],
      ["l", false],
      ["h", true],
    ]);
    expect(withActiveLayers(active, [])).toEqual(active);
  });

  it("sends the RIDs the instance knows, within its per-layer budgets", () => {
    expect(SCREEN_ENCODINGS.map((encoding) => encoding.rid)).toEqual(["q", "l", "h"]);
    // The instance's budgets are twice these (ADR-0035): 37.5, 125 and 375 kB/s.
    expect(SCREEN_ENCODINGS.map((encoding) => encoding.maxBitrate)).toEqual([150_000, 500_000, 2_500_000]);
  });
});

describe("screen share problems and codecs", () => {
  it("tells a cancelled picker from a browser that cannot share", () => {
    expect(screenShareProblemOf(new DOMException("no", "NotAllowedError"))).toBe("denied");
    expect(screenShareProblemOf(new TypeError("no getDisplayMedia"))).toBe("unsupported");
  });

  it("keeps Opus, VP8 and its retransmissions, or nothing when one is missing", () => {
    const codec = (mimeType: string) => ({ mimeType, clockRate: 90_000 });
    const audio = [codec("audio/opus"), codec("audio/PCMU")];
    const video = [codec("video/H264"), codec("video/VP8"), codec("video/rtx"), codec("video/AV1")];
    expect(preferredCodecs(audio, video)).toEqual({
      audio: [codec("audio/opus")],
      video: [codec("video/VP8"), codec("video/rtx")],
    });
    expect(preferredCodecs(audio, [codec("video/H264")])).toBeNull();
  });
});
