import { describe, expect, it } from "vitest";

import type { Attachment } from "../lib/protocol";
import {
  actualSizeScrollStart,
  attachmentPixelSize,
  fittedPictureSize,
  isPictureLargerThanBox,
  isPlainPrimaryClick,
  viewablePictures,
  wrappedPictureIndex,
} from "./picture-viewer";

function attachment(id: string, contentType: string, size: Partial<Pick<Attachment, "width" | "height">> = {}): Attachment {
  return { id, file_name: `${id}.bin`, content_type: contentType, size_bytes: 1, ...size };
}

const NO_MODIFIERS = { button: 0, ctrlKey: false, metaKey: false, shiftKey: false, altKey: false };

describe("picture viewer", () => {
  it("keeps only pictures, in message order", () => {
    const pictures = viewablePictures([
      attachment("1", "image/png"),
      attachment("2", "application/octet-stream"),
      attachment("3", "image/avif"),
    ]);
    expect(pictures.map((picture) => picture.id)).toEqual(["1", "3"]);
  });

  it("wraps previous and next around the ends", () => {
    expect(wrappedPictureIndex(3, 1, 4)).toBe(0);
    expect(wrappedPictureIndex(0, -1, 4)).toBe(3);
    expect(wrappedPictureIndex(1, 1, 4)).toBe(2);
    expect(wrappedPictureIndex(0, 1, 1)).toBe(0);
  });

  it("opens only on a plain primary click", () => {
    expect(isPlainPrimaryClick(NO_MODIFIERS)).toBe(true);
    expect(isPlainPrimaryClick({ ...NO_MODIFIERS, button: 1 })).toBe(false);
    expect(isPlainPrimaryClick({ ...NO_MODIFIERS, ctrlKey: true })).toBe(false);
    expect(isPlainPrimaryClick({ ...NO_MODIFIERS, metaKey: true })).toBe(false);
    expect(isPlainPrimaryClick({ ...NO_MODIFIERS, shiftKey: true })).toBe(false);
    expect(isPlainPrimaryClick({ ...NO_MODIFIERS, altKey: true })).toBe(false);
  });

  it("fits a large picture into the box and never upscales a small one", () => {
    expect(fittedPictureSize({ width: 2880, height: 1800 }, { width: 1440, height: 1440 })).toEqual({ width: 1440, height: 900 });
    expect(fittedPictureSize({ width: 1800, height: 2880 }, { width: 1440, height: 1440 })).toEqual({ width: 900, height: 1440 });
    expect(fittedPictureSize({ width: 480, height: 270 }, { width: 1440, height: 900 })).toEqual({ width: 480, height: 270 });
    expect(fittedPictureSize({ width: 4000, height: 1 }, { width: 100, height: 100 })).toEqual({ width: 100, height: 1 });
  });

  it("offers actual size only for a picture larger than its box", () => {
    expect(isPictureLargerThanBox({ width: 2880, height: 1800 }, { width: 1440, height: 1800 })).toBe(true);
    expect(isPictureLargerThanBox({ width: 480, height: 270 }, { width: 480, height: 270 })).toBe(false);
  });

  it("keeps the clicked point under the pointer at actual size", () => {
    // Clicked the middle of a 2880 px picture, the pointer 700 px into a stage padded by 16 px.
    expect(actualSizeScrollStart(0.5, 2880, 16, 700)).toBe(756);
    expect(actualSizeScrollStart(0, 2880, 16, 700)).toBe(0);
  });

  it("uses the instance's size only when it is usable", () => {
    expect(attachmentPixelSize(attachment("1", "image/png", { width: 10, height: 20 }))).toEqual({ width: 10, height: 20 });
    expect(attachmentPixelSize(attachment("1", "image/png"))).toBeNull();
    expect(attachmentPixelSize(attachment("1", "image/png", { width: 0, height: 20 }))).toBeNull();
  });
});
