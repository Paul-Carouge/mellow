import { type Attachment, PREVIEWABLE_IMAGE_TYPES } from "../lib/protocol";

export interface PixelSize {
  width: number;
  height: number;
}

/** The message's attachments the viewer can show, in message order (file cards are skipped). */
export function viewablePictures(attachments: readonly Attachment[]): Attachment[] {
  return attachments.filter((attachment) => PREVIEWABLE_IMAGE_TYPES.includes(attachment.content_type));
}

/** Previous and next wrap around, so neither button is ever disabled (docs/design/web-picture-viewer.md). */
export function wrappedPictureIndex(index: number, step: -1 | 1, total: number): number {
  return (((index + step) % total) + total) % total;
}

/** Only a plain primary click opens the viewer: a modified or middle click keeps the link's own behaviour. */
export function isPlainPrimaryClick(event: Pick<MouseEvent, "button" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey">): boolean {
  return event.button === 0 && !event.ctrlKey && !event.metaKey && !event.shiftKey && !event.altKey;
}

/** The picture fitted inside the box, never upscaled past its own pixels. */
export function fittedPictureSize(picture: PixelSize, box: PixelSize): PixelSize {
  const scale = Math.min(1, box.width / picture.width, box.height / picture.height);
  return { width: Math.max(1, Math.round(picture.width * scale)), height: Math.max(1, Math.round(picture.height * scale)) };
}

/** Actual size only means something when the fitted picture is smaller than its own pixels. */
export function isPictureLargerThanBox(picture: PixelSize, box: PixelSize): boolean {
  return picture.width > box.width || picture.height > box.height;
}

/**
 * Where the stage scrolls, along one axis, once the picture shows at its own size, so the point
 * that was clicked (at `clickRatio` of the fitted picture) stays under the pointer.
 */
export function actualSizeScrollStart(clickRatio: number, naturalLength: number, stagePadding: number, pointerOffset: number): number {
  return Math.max(0, clickRatio * naturalLength + stagePadding - pointerOffset);
}

/** A known, usable size from the instance (absent for older uploads and servers). */
export function attachmentPixelSize(attachment: Attachment): PixelSize | null {
  const { width, height } = attachment;
  return width !== undefined && height !== undefined && width > 0 && height > 0 ? { width, height } : null;
}
