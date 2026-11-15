import { describe, expect, it } from "vitest";

import {
  nextPlaybackSpeed,
  normalizedPeaks,
  placeholderPeaks,
  playbackFraction,
  playbackSpeedNumber,
  playedBarCount,
  seekFractionAfterKey,
} from "./audio-player";

describe("normalizedPeaks", () => {
  it("scales the loudest bar to the full height", () => {
    expect(normalizedPeaks([0.1, 0.25, 0.05])).toEqual([0.4, 1, 0.2]);
  });

  it("keeps silence flat instead of dividing by zero", () => {
    expect(normalizedPeaks([0, 0, 0])).toEqual([0, 0, 0]);
  });
});

describe("placeholderPeaks", () => {
  it("draws a flat line of the given level", () => {
    expect(placeholderPeaks(3, 0.16)).toEqual([0.16, 0.16, 0.16]);
    expect(placeholderPeaks(-1, 0.16)).toEqual([]);
  });
});

describe("playedBarCount", () => {
  it("counts a bar once the playhead passed its middle", () => {
    expect(playedBarCount(0, 48)).toBe(0);
    expect(playedBarCount(0.01, 48)).toBe(0);
    expect(playedBarCount(0.5, 48)).toBe(24);
    expect(playedBarCount(1, 48)).toBe(48);
  });

  it("stays within the bars whatever it is given", () => {
    expect(playedBarCount(-0.5, 48)).toBe(0);
    expect(playedBarCount(3, 48)).toBe(48);
  });
});

describe("playbackFraction", () => {
  it("is where the sound is, 0 while its length is unknown", () => {
    expect(playbackFraction(30, 120)).toBe(0.25);
    expect(playbackFraction(30, null)).toBe(0);
    expect(playbackFraction(30, Number.NaN)).toBe(0);
    expect(playbackFraction(30, Number.POSITIVE_INFINITY)).toBe(0);
    expect(playbackFraction(130, 120)).toBe(1);
  });
});

describe("seekFractionAfterKey", () => {
  it("moves five seconds per arrow when the length is known", () => {
    expect(seekFractionAfterKey("ArrowRight", 0.5, 100)).toBeCloseTo(0.55);
    expect(seekFractionAfterKey("ArrowUp", 0.5, 100)).toBeCloseTo(0.55);
    expect(seekFractionAfterKey("ArrowLeft", 0.5, 100)).toBeCloseTo(0.45);
    expect(seekFractionAfterKey("ArrowDown", 0.5, 100)).toBeCloseTo(0.45);
  });

  it("moves a twentieth per arrow before the length is known", () => {
    expect(seekFractionAfterKey("ArrowRight", 0, null)).toBeCloseTo(0.05);
  });

  it("moves a tenth per page, and Home and End go to the ends", () => {
    expect(seekFractionAfterKey("PageUp", 0.2, 100)).toBeCloseTo(0.3);
    expect(seekFractionAfterKey("PageDown", 0.2, 100)).toBeCloseTo(0.1);
    expect(seekFractionAfterKey("Home", 0.7, 100)).toBe(0);
    expect(seekFractionAfterKey("End", 0.7, 100)).toBe(1);
  });

  it("never leaves the sound", () => {
    expect(seekFractionAfterKey("ArrowLeft", 0.01, 100)).toBe(0);
    expect(seekFractionAfterKey("ArrowRight", 0.99, 100)).toBe(1);
  });

  it("leaves other keys to the page", () => {
    expect(seekFractionAfterKey(" ", 0.5, 100)).toBeNull();
    expect(seekFractionAfterKey("Tab", 0.5, 100)).toBeNull();
  });
});

describe("nextPlaybackSpeed and playbackSpeedNumber", () => {
  it("cycles 1, 1.5, 2 and back", () => {
    expect(nextPlaybackSpeed(1)).toBe(1.5);
    expect(nextPlaybackSpeed(1.5)).toBe(2);
    expect(nextPlaybackSpeed(2)).toBe(1);
    expect(nextPlaybackSpeed(0.75)).toBe(1);
  });

  it("writes the speed as the reader's language does", () => {
    expect(playbackSpeedNumber(1.5, "fr")).toBe("1,5");
    expect(playbackSpeedNumber(1.5, "en")).toBe("1.5");
    expect(playbackSpeedNumber(2, "en")).toBe("2");
  });
});
