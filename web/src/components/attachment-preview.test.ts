import { describe, expect, it } from "vitest";

import type { Attachment } from "../lib/protocol";
import { imagePreviewWidth } from "./attachment-preview";

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
