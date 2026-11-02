import { afterEach, describe, expect, it, vi } from "vitest";

import { memoryStorage } from "../test-memory-storage";
import {
  type PictureAnimationChoice,
  readPictureAnimationChoice,
  shouldPictureAnimate,
  writePictureAnimationChoice,
} from "./picture-animation";

afterEach(() => {
  vi.restoreAllMocks();
});

describe("shouldPictureAnimate", () => {
  const cases: [PictureAnimationChoice, boolean, boolean, boolean][] = [
    // choice, reduced motion, plays by itself (hovered, or always-playing), expected
    ["on-hover", false, false, false],
    ["on-hover", false, true, true],
    ["on-hover", true, false, false],
    ["on-hover", true, true, false],
    ["always", false, false, true],
    ["always", false, true, true],
    ["always", true, false, true],
    ["always", true, true, true],
  ];
  it.each(cases)("%s, reduced motion %s, playing by itself %s: %s", (choice, isReduced, isPlaying, expected) => {
    expect(shouldPictureAnimate(choice, isReduced, isPlaying)).toBe(expected);
  });
});

describe("picture animation storage", () => {
  it("defaults to on hover when nothing, or something unknown, is stored", () => {
    expect(readPictureAnimationChoice(null)).toBe("on-hover");
    expect(readPictureAnimationChoice(memoryStorage())).toBe("on-hover");
    const storage = memoryStorage();
    for (const corrupt of ["never", "ALWAYS", '"always"', ""]) {
      storage.setItem("mellow.picture-animation", corrupt);
      expect(readPictureAnimationChoice(storage)).toBe("on-hover");
    }
  });

  it("reads back what it wrote", () => {
    const storage = memoryStorage();
    writePictureAnimationChoice(storage, "always");
    expect(readPictureAnimationChoice(storage)).toBe("always");
    writePictureAnimationChoice(storage, "on-hover");
    expect(readPictureAnimationChoice(storage)).toBe("on-hover");
  });

  it("falls back to the default when storage refuses, without throwing", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const refusing = {
      getItem: () => {
        throw new DOMException("denied", "SecurityError");
      },
      setItem: () => {
        throw new DOMException("full", "QuotaExceededError");
      },
    };
    expect(readPictureAnimationChoice(refusing)).toBe("on-hover");
    expect(() => writePictureAnimationChoice(refusing, "always")).not.toThrow();
    expect(warn).toHaveBeenCalledTimes(2);
  });
});
