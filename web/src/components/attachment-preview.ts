import { type Attachment, PLAYABLE_AUDIO_TYPES, PLAYABLE_VIDEO_TYPES, PREVIEWABLE_IMAGE_TYPES } from "../lib/protocol";

/** How an attachment shows under its message (DESIGN.md §5.4 "Attachments"). */
export type AttachmentDisplay = "image" | "video" | "audio" | "file";

/**
 * A pending upload is not readable by others yet, and a picture or media the browser could not
 * show (`hasFailed`) falls back to its download card, as does any other type.
 */
export function attachmentDisplay(attachment: Attachment, isPending: boolean, hasFailed: boolean): AttachmentDisplay {
  if (isPending || hasFailed) {
    return "file";
  }
  if (PREVIEWABLE_IMAGE_TYPES.includes(attachment.content_type)) {
    return "image";
  }
  if (PLAYABLE_VIDEO_TYPES.includes(attachment.content_type)) {
    return "video";
  }
  return PLAYABLE_AUDIO_TYPES.includes(attachment.content_type) ? "audio" : "file";
}

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

/** Most videos are landscape; a picture of another shape is letterboxed on the dark stage. */
const FALLBACK_VIDEO_ASPECT_RATIO = "16 / 9";

/**
 * A video's box, fixed before anything loads (nothing is fetched until it plays, so its own size
 * is unknown): its stored size when the instance gave one, else 16:9 within the image maxima.
 */
export function videoBoxSize(attachment: Attachment): { width: string; aspectRatio: string } {
  const { width, height } = attachment;
  if (isPositiveInteger(width) && isPositiveInteger(height)) {
    return { width: imagePreviewWidth(attachment) ?? "100%", aspectRatio: `${width} / ${height}` };
  }
  return {
    width: `min(100%, var(--mw-component-attachment-image-max-width), calc(var(--mw-component-attachment-image-max-height) * ${FALLBACK_VIDEO_ASPECT_RATIO}))`,
    aspectRatio: FALLBACK_VIDEO_ASPECT_RATIO,
  };
}

let playingMedia: Pick<HTMLMediaElement, "pause"> | null = null;

/** Only one attachment plays at a time, as on Discord: starting one pauses the one playing before. */
export function pauseOtherMediaWhenOneStarts(started: Pick<HTMLMediaElement, "pause">): void {
  if (playingMedia !== null && playingMedia !== started) {
    playingMedia.pause();
  }
  playingMedia = started;
}

function isPositiveInteger(value: number | undefined): value is number {
  return Number.isInteger(value) && (value ?? 0) > 0;
}
