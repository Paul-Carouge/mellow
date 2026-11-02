// The crop of an icon, an avatar or a banner (DESIGN.md §6.54): pure maths over the picture's
// size, in its own pixels, so the crop dialog only draws what these functions return. The kept
// part has a fixed shape, `aspect` being its width over its height: square for icons and avatars,
// the profile card's band for banners. The instance cuts the same part (`crop_x`, `crop_y` and
// `crop_size` or `crop_width`); without one it keeps the largest centred part of that shape.

import type { PictureCrop } from "../lib/api";

export interface PictureSize {
  width: number;
  height: number;
}

/** A part of the picture in its pixels, `aspect` times wider than tall; fractional while the
 * user drags, rounded only when sent. */
export interface CropArea {
  x: number;
  y: number;
  width: number;
}

/** Icons and avatars keep a square. */
export const SQUARE_ASPECT = 1;
/** Banners keep the band the profile card shows, stored at 960 × 384 by the instance (ADR-0033). */
export const BANNER_ASPECT = 960 / 384;
/** Zooming in further than this shows a handful of pixels: the picture would be a blur. */
export const MAXIMUM_CROP_ZOOM = 8;
/** The smallest short side worth keeping, in picture pixels, whatever the zoom allows. */
const MINIMUM_CROP_SIDE = 16;

export function cropHeight(crop: CropArea, aspect: number): number {
  return crop.width / aspect;
}

/** What the instance keeps when no crop is sent, and where the dialog starts. */
export function largestCentredCrop(picture: PictureSize, aspect: number): CropArea {
  const width = Math.min(picture.width, picture.height * aspect);
  return { x: (picture.width - width) / 2, y: (picture.height - width / aspect) / 2, width };
}

/** At least 1 (the largest centred part); a tiny picture cannot be zoomed into. */
export function maximumZoom(picture: PictureSize, aspect: number): number {
  const largest = largestCentredCrop(picture, aspect);
  const shortSide = Math.min(largest.width, cropHeight(largest, aspect));
  return Math.max(1, Math.min(MAXIMUM_CROP_ZOOM, shortSide / MINIMUM_CROP_SIDE));
}

/** 1 when the crop is the largest part of its shape. */
export function zoomOf(crop: CropArea, picture: PictureSize, aspect: number): number {
  return largestCentredCrop(picture, aspect).width / crop.width;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), maximum);
}

/** The nearest crop that stays inside the picture and within the zoom range. */
export function clampCrop(crop: CropArea, picture: PictureSize, aspect: number): CropArea {
  const largestWidth = largestCentredCrop(picture, aspect).width;
  const width = clamp(crop.width, largestWidth / maximumZoom(picture, aspect), largestWidth);
  return {
    x: clamp(crop.x, 0, picture.width - width),
    y: clamp(crop.y, 0, picture.height - width / aspect),
    width,
  };
}

/**
 * The crop at `zoom`, keeping the picture point `anchor` (the pointer, or the crop's centre) where
 * it is in the frame, as maps and photo viewers zoom.
 */
export function cropZoomedTo(
  crop: CropArea,
  picture: PictureSize,
  aspect: number,
  zoom: number,
  anchor: { x: number; y: number } = { x: crop.x + crop.width / 2, y: crop.y + cropHeight(crop, aspect) / 2 },
): CropArea {
  const width = largestCentredCrop(picture, aspect).width / clamp(zoom, 1, maximumZoom(picture, aspect));
  const ratio = width / crop.width;
  return clampCrop(
    { x: anchor.x - (anchor.x - crop.x) * ratio, y: anchor.y - (anchor.y - crop.y) * ratio, width },
    picture,
    aspect,
  );
}

/** Moves the crop by a distance in picture pixels; dragging the picture moves it the other way. */
export function cropMovedBy(
  crop: CropArea,
  picture: PictureSize,
  aspect: number,
  deltaX: number,
  deltaY: number,
): CropArea {
  return clampCrop({ ...crop, x: crop.x + deltaX, y: crop.y + deltaY }, picture, aspect);
}

/**
 * Whole pixels for the query, or null for the largest centred part, so an untouched crop leaves
 * the choice to the instance's own centring. The height is rounded up when checking the bottom
 * edge, so the instance never has to narrow the crop to fit it.
 */
export function cropToSend(crop: CropArea, picture: PictureSize, aspect: number): PictureCrop | null {
  const centred = largestCentredCrop(picture, aspect);
  const isCentred =
    Math.abs(crop.x - centred.x) < 0.5 && Math.abs(crop.y - centred.y) < 0.5 && Math.abs(crop.width - centred.width) < 0.5;
  if (isCentred) {
    return null;
  }
  const width = clamp(Math.round(crop.width), 1, Math.floor(centred.width));
  return {
    x: clamp(Math.round(crop.x), 0, picture.width - width),
    y: clamp(Math.round(crop.y), 0, picture.height - Math.ceil(width / aspect)),
    width,
  };
}

/**
 * Where to draw the picture so that `crop` fills a box of its shape, in percent of the box's
 * width (left, width) and height (top): the stage and the small samples share it, whatever their
 * size.
 */
export function placementInPercent(
  crop: CropArea,
  picture: PictureSize,
  aspect: number,
): { left: number; top: number; width: number } {
  return {
    left: (-crop.x / crop.width) * 100,
    top: (-crop.y / cropHeight(crop, aspect)) * 100,
    width: (picture.width / crop.width) * 100,
  };
}

/**
 * The picture's first frame as a still object URL (revoke it when done), and its size in its own
 * pixels, EXIF orientation applied, as the instance measures it. Rejects what the browser cannot
 * decode. Still, because a server icon rests still in the strip (DESIGN.md §5.2).
 */
export async function stillPictureOf(source: Blob): Promise<{ url: string; picture: PictureSize }> {
  const bitmap = await createImageBitmap(source);
  const picture = { width: bitmap.width, height: bitmap.height };
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  canvas.getContext("2d")?.drawImage(bitmap, 0, 0);
  bitmap.close();
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve));
  if (!blob) {
    throw new Error("the first frame could not be drawn");
  }
  return { url: URL.createObjectURL(blob), picture };
}
