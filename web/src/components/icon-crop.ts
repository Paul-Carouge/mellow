// The square crop of a server or channel icon (DESIGN.md §6.54): pure maths over the picture's
// size, in its own pixels, so the crop dialog only draws what these functions return. The instance
// cuts the same square (`crop_x`, `crop_y`, `crop_size`); without one it keeps the centred square.

import type { IconCrop } from "../lib/api";

export interface PictureSize {
  width: number;
  height: number;
}

/** A square in picture pixels; fractional while the user drags, rounded only when sent. */
export interface CropSquare {
  x: number;
  y: number;
  size: number;
}

/** Zooming in further than this shows a handful of pixels: the icon would be a blur. */
export const MAXIMUM_ICON_ZOOM = 8;
/** The smallest square worth keeping, in picture pixels, whatever the zoom allows. */
const MINIMUM_CROP_SIDE = 16;

function shortSide(picture: PictureSize): number {
  return Math.min(picture.width, picture.height);
}

/** What the instance keeps when no crop is sent, and where the dialog starts. */
export function centredSquare(picture: PictureSize): CropSquare {
  const size = shortSide(picture);
  return { x: (picture.width - size) / 2, y: (picture.height - size) / 2, size };
}

/** At least 1 (the centred square); a tiny picture cannot be zoomed into. */
export function maximumZoom(picture: PictureSize): number {
  return Math.max(1, Math.min(MAXIMUM_ICON_ZOOM, shortSide(picture) / MINIMUM_CROP_SIDE));
}

/** 1 when the square spans the picture's short side. */
export function zoomOf(crop: CropSquare, picture: PictureSize): number {
  return shortSide(picture) / crop.size;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), maximum);
}

/** The nearest square that stays inside the picture and within the zoom range. */
export function clampCrop(crop: CropSquare, picture: PictureSize): CropSquare {
  const size = clamp(crop.size, shortSide(picture) / maximumZoom(picture), shortSide(picture));
  return {
    x: clamp(crop.x, 0, picture.width - size),
    y: clamp(crop.y, 0, picture.height - size),
    size,
  };
}

/**
 * The square at `zoom`, keeping the picture point `anchor` (the pointer, or the square's centre)
 * where it is in the frame, as maps and photo viewers zoom.
 */
export function cropZoomedTo(
  crop: CropSquare,
  picture: PictureSize,
  zoom: number,
  anchor: { x: number; y: number } = { x: crop.x + crop.size / 2, y: crop.y + crop.size / 2 },
): CropSquare {
  const size = shortSide(picture) / clamp(zoom, 1, maximumZoom(picture));
  const ratio = size / crop.size;
  return clampCrop(
    { x: anchor.x - (anchor.x - crop.x) * ratio, y: anchor.y - (anchor.y - crop.y) * ratio, size },
    picture,
  );
}

/** Moves the square by a distance in picture pixels; dragging the picture moves it the other way. */
export function cropMovedBy(crop: CropSquare, picture: PictureSize, deltaX: number, deltaY: number): CropSquare {
  return clampCrop({ ...crop, x: crop.x + deltaX, y: crop.y + deltaY }, picture);
}

/**
 * Whole pixels for the query, or null for the centred square, so an untouched crop leaves the
 * choice to the instance's own centring.
 */
export function cropToSend(crop: CropSquare, picture: PictureSize): IconCrop | null {
  const centred = centredSquare(picture);
  const isCentred =
    Math.abs(crop.x - centred.x) < 0.5 && Math.abs(crop.y - centred.y) < 0.5 && Math.abs(crop.size - centred.size) < 0.5;
  if (isCentred) {
    return null;
  }
  const size = clamp(Math.round(crop.size), 1, shortSide(picture));
  return {
    x: clamp(Math.round(crop.x), 0, picture.width - size),
    y: clamp(Math.round(crop.y), 0, picture.height - size),
    size,
  };
}

/**
 * Where to draw the picture so that `crop` fills a square box, in percent of the box's side: the
 * stage and the small samples share it, whatever their size.
 */
export function placementInPercent(crop: CropSquare, picture: PictureSize): { left: number; top: number; width: number } {
  return {
    left: (-crop.x / crop.size) * 100,
    top: (-crop.y / crop.size) * 100,
    width: (picture.width / crop.size) * 100,
  };
}
