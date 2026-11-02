import { describe, expect, it } from "vitest";

import {
  isScreenShareQuality,
  preferredCodecs,
  screenCaptureConstraints,
  screenCaptureOptions,
  screenEncodings,
  screenShareProblemOf,
  withActiveLayers,
  withQuality,
} from "./screen-share";

const SCREEN_ENCODINGS = screenEncodings("1080p");

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
    // The instance allows 37.5 kB/s on q, 125 kB/s on l and 1.25 MB/s on h (ADR-0035, ADR-0037).
    expect(SCREEN_ENCODINGS.map((encoding) => encoding.maxBitrate)).toEqual([150_000, 500_000, 5_000_000]);
    expect(SCREEN_ENCODINGS.every((encoding) => encoding.maxFramerate === 30)).toBe(true);
  });

  it("captures and sends each quality at 30 frames per second, and changes it live keeping paused layers", () => {
    expect(screenCaptureConstraints("720p")).toEqual({
      width: { max: 1_280 },
      height: { max: 720 },
      frameRate: { ideal: 30, max: 30 },
    });
    expect(screenCaptureConstraints("1440p").width).toEqual({ max: 2_560 });
    expect(screenEncodings("1440p").at(-1)?.maxBitrate).toBe(8_000_000);
    const paused = withActiveLayers(screenEncodings("720p"), ["q"]);
    const upgraded = withQuality(paused, "1440p");
    expect(upgraded.map((encoding) => [encoding.active, encoding.maxBitrate])).toEqual([
      [true, 150_000],
      [false, 500_000],
      [false, 8_000_000],
    ]);
    expect(isScreenShareQuality("1440p")).toBe(true);
    expect(isScreenShareQuality("4k")).toBe(false);
  });
});

describe("screen share capture", () => {
  it("asks for the shared sound untouched, and never offers the call's own tab", () => {
    const options = screenCaptureOptions("1080p");
    expect(options.video).toEqual(screenCaptureConstraints("1080p"));
    expect(options.audio).toEqual({ echoCancellation: false, noiseSuppression: false, autoGainControl: false });
    expect(options.systemAudio).toBe("include");
    expect(options.selfBrowserSurface).toBe("exclude");
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
