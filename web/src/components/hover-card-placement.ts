// Where a profile hover card sits (DESIGN.md §5.7.2): beside the column a row belongs to (the
// member list, the voice participants), on the side with room and top-aligned with the row; or
// under a name in the conversation, start-aligned, above it when the window has no room below.
// Always inside the window by a margin.

export interface Box {
  readonly top: number;
  readonly bottom: number;
  readonly left: number;
  readonly right: number;
}

export interface Size {
  readonly width: number;
  readonly height: number;
}

export type HoverCardPlacement = "beside" | "below";

/** The side of the trigger the card ended up on: its entrance slides from there. */
export type HoverCardSide = "left" | "right" | "below" | "above";

export interface HoverCardPosition {
  readonly top: number;
  readonly left: number;
  readonly side: HoverCardSide;
}

export interface HoverCardSpacing {
  /** Between the card and its trigger, or the trigger's column. */
  readonly offset: number;
  /** Least space between the card and the window's edges. */
  readonly margin: number;
}

function clamp(value: number, least: number, most: number): number {
  return Math.max(least, Math.min(value, most));
}

function positionBelowOrAbove(trigger: Box, card: Size, viewport: Size, spacing: HoverCardSpacing): HoverCardPosition {
  const left = clamp(trigger.left, spacing.margin, viewport.width - spacing.margin - card.width);
  const below = trigger.bottom + spacing.offset;
  if (below + card.height <= viewport.height - spacing.margin) {
    return { top: below, left, side: "below" };
  }
  const above = trigger.top - spacing.offset - card.height;
  if (above >= spacing.margin) {
    return { top: above, left, side: "above" };
  }
  return { top: clamp(below, spacing.margin, viewport.height - spacing.margin - card.height), left, side: "below" };
}

/**
 * `column` is the box a "beside" card keeps clear of (the side sheet, the navigator); without one
 * the trigger itself is. A card that fits on neither side of it goes under the trigger instead.
 */
export function hoverCardPosition(
  placement: HoverCardPlacement,
  trigger: Box,
  column: Box | null,
  card: Size,
  viewport: Size,
  spacing: HoverCardSpacing,
): HoverCardPosition {
  if (placement === "below") {
    return positionBelowOrAbove(trigger, card, viewport, spacing);
  }
  const edges = column ?? trigger;
  const roomOnLeft = edges.left - spacing.offset - spacing.margin;
  const roomOnRight = viewport.width - edges.right - spacing.offset - spacing.margin;
  const top = clamp(trigger.top, spacing.margin, viewport.height - spacing.margin - card.height);
  const fitsLeft = roomOnLeft >= card.width;
  const fitsRight = roomOnRight >= card.width;
  if (fitsLeft && (!fitsRight || roomOnLeft >= roomOnRight)) {
    return { top, left: edges.left - spacing.offset - card.width, side: "left" };
  }
  if (fitsRight) {
    return { top, left: edges.right + spacing.offset, side: "right" };
  }
  return positionBelowOrAbove(trigger, card, viewport, spacing);
}
