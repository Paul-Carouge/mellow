// Where a picker popover (emoji, GIF) opens beside the button that opened it: below when there is
// room, else above, a gap away, and inside the window. A button inside a larger box (the composer)
// marks that box with `data-popover-edge`, so the panel clears the box rather than overlapping it.

export interface AnchorEdges {
  /** Top and bottom of the box to keep clear of. */
  readonly top: number;
  readonly bottom: number;
  /** The button's right edge, which the panel's right edge lines up with. */
  readonly right: number;
}

export interface Size {
  readonly width: number;
  readonly height: number;
}

export function popoverPositionBesideAnchor(anchor: AnchorEdges, panel: Size, viewport: Size, gap: number): { top: number; left: number } {
  const fitsBelow = anchor.bottom + gap + panel.height <= viewport.height;
  const top = fitsBelow ? anchor.bottom + gap : Math.max(anchor.top - gap - panel.height, 0);
  const left = Math.min(Math.max(anchor.right - panel.width, 0), viewport.width - panel.width);
  return { top, left: Math.max(left, 0) };
}

/** The edges `popoverPositionBesideAnchor` needs, from the button and its marked box if any. */
export function anchorEdgesOf(button: HTMLElement): AnchorEdges {
  const buttonBounds = button.getBoundingClientRect();
  const edgeBounds = (button.closest<HTMLElement>("[data-popover-edge]") ?? button).getBoundingClientRect();
  return { top: edgeBounds.top, bottom: edgeBounds.bottom, right: buttonBounds.right };
}
