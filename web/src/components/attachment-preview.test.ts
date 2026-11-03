import { describe, expect, it, vi } from "vitest";

import type { Attachment } from "../lib/protocol";
import { attachmentDisplay, imagePreviewWidth, pauseOtherMediaWhenOneStarts, videoBoxSize } from "./attachment-preview";

const PHOTO: Attachment = { id: "1", file_name: "photo.jpg", content_type: "image/jpeg", size_bytes: 10 };

describe("imagePreviewWidth", () => {
  it("fits the picture's own size within the container and both maxima", () => {
    expect(imagePreviewWidth({ ...PHOTO, width: 3000, height: 4000 })).toBe(
      "min(100%, var(--mw-component-attachment-image-max-width), calc(var(--mw-component-attachment-image-max-height) * 3000 / 4000), 3000px)",
    );
  });

  it("leaves the fallback box to pictures without a usable size", () => {
    expect(imagePreviewWidth(PHOTO)).toBeUndefined();
    expect(imagePreviewWidth({ ...PHOTO, width: 3000 })).toBeUndefined();
    expect(imagePreviewWidth({ ...PHOTO, width: 0, height: 10 })).toBeUndefined();
    expect(imagePreviewWidth({ ...PHOTO, width: 1.5, height: 10 })).toBeUndefined();
  });
});

describe("attachmentDisplay", () => {
  const file = (content_type: string): Attachment => ({ ...PHOTO, content_type });

  it("shows pictures, videos and audio by their sniffed type", () => {
    expect(attachmentDisplay(file("image/avif"), false, false)).toBe("image");
    expect(attachmentDisplay(file("video/quicktime"), false, false)).toBe("video");
    expect(attachmentDisplay(file("video/webm"), false, false)).toBe("video");
    expect(attachmentDisplay(file("audio/mpeg"), false, false)).toBe("audio");
    expect(attachmentDisplay(file("audio/flac"), false, false)).toBe("audio");
  });

  it("keeps the download card for other files, pending uploads and what failed to play", () => {
    expect(attachmentDisplay(file("application/octet-stream"), false, false)).toBe("file");
    expect(attachmentDisplay(file("video/x-matroska"), false, false)).toBe("file");
    expect(attachmentDisplay(file("video/mp4"), true, false)).toBe("file");
    expect(attachmentDisplay(file("audio/ogg"), false, true)).toBe("file");
    expect(attachmentDisplay(file("image/png"), false, true)).toBe("file");
  });
});

describe("videoBoxSize", () => {
  const clip: Attachment = { id: "2", file_name: "clip.mp4", content_type: "video/mp4", size_bytes: 10 };

  it("keeps a stored size's shape", () => {
    expect(videoBoxSize({ ...clip, width: 720, height: 1280 })).toEqual({
      width: imagePreviewWidth({ ...clip, width: 720, height: 1280 }),
      aspectRatio: "720 / 1280",
    });
  });

  it("falls back to 16:9 within the image maxima", () => {
    expect(videoBoxSize(clip)).toEqual({
      width: "min(100%, var(--mw-component-attachment-image-max-width), calc(var(--mw-component-attachment-image-max-height) * 16 / 9))",
      aspectRatio: "16 / 9",
    });
  });
});

describe("pauseOtherMediaWhenOneStarts", () => {
  it("pauses the one playing before, never the one starting", () => {
    const first = { pause: vi.fn() };
    const second = { pause: vi.fn() };
    pauseOtherMediaWhenOneStarts(first);
    pauseOtherMediaWhenOneStarts(first);
    expect(first.pause).not.toHaveBeenCalled();
    pauseOtherMediaWhenOneStarts(second);
    expect(first.pause).toHaveBeenCalledOnce();
    expect(second.pause).not.toHaveBeenCalled();
  });
});
