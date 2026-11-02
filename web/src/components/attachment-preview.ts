import type { Attachment } from "../lib/protocol";

/**
 * The CSS width of an inline image's box before it loads, when the instance gave its size: the
 * size it will show at once loaded (never above its own pixels, within the container and both
 * preview maxima), so nothing moves when it arrives. `undefined` without a usable size: the
 * caller keeps its fallback box.
 */
export function imagePreviewWidth(attachment: Attachment): string | undefined {
  const { width, height } = attachment;
  if (!isPositiveInteger(width) || !isPositiveInteger(height)) {
    return undefined;
  }
  return `min(100%, var(--mw-component-attachment-image-max-width), calc(var(--mw-component-attachment-image-max-height) * ${width} / ${height}), ${width}px)`;
}

function isPositiveInteger(value: number | undefined): value is number {
  return Number.isInteger(value) && (value ?? 0) > 0;
}
