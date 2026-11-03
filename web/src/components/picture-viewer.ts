import type { Attachment } from "../lib/protocol";
import { attachmentDisplay } from "./attachment-preview";

export interface PixelSize {
  width: number;
  height: number;
}

/**
 * The message's attachments the viewer can show, in message order: pictures and the videos the
 * list plays (file cards and sounds are skipped).
 */
export function viewableMedia(attachments: readonly Attachment[]): Attachment[] {
  return attachments.filter((attachment) => isViewerPicture(attachment) || isViewerVideo(attachment));
}

export function isViewerPicture(attachment: Attachment): boolean {
  return attachmentDisplay(attachment, false, false) === "image";
}

export function isViewerVideo(attachment: Attachment): boolean {
  return attachmentDisplay(attachment, false, false) === "video";
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

/** A video fills the box (grown too, as a player does); its frames are not judged pixel by pixel. */
/** Most videos are landscape: the box a video holds until its own size is known. */
export const FALLBACK_VIDEO_PIXEL_SIZE: PixelSize = { width: 16, height: 9 };

export function fittedVideoSize(video: PixelSize, box: PixelSize): PixelSize {
  const scale = Math.min(box.width / video.width, box.height / video.height);
  return { width: Math.max(1, Math.round(video.width * scale)), height: Math.max(1, Math.round(video.height * scale)) };
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
