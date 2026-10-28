// Where a menu sits (DESIGN.md §6.33): under the button that opened it, or above it when the
// window has no room below (a message or a friend near the bottom of a phone screen), and never
// past the window's top; at the pointer for a right-click or a long press, turned back towards the
// window when it would leave it.

export interface MenuAnchorBounds {
  readonly top: number;
  readonly bottom: number;
  readonly left: number;
  readonly right: number;
}

/**
 * What opened the menu: a pointer (right-click, long press) at a point, or an element (a button,
 * or the focused item for Shift+F10 and the Menu key). `align` says which of the element's edges
 * the menu lines up with; `stretch` makes it as wide as the element.
 */
export type MenuOrigin =
  | { readonly kind: "point"; readonly x: number; readonly y: number }
  | { readonly kind: "anchor"; readonly bounds: MenuAnchorBounds; readonly align: "start" | "end" | "stretch" };

export interface MenuPlacement {
  readonly top: number;
  readonly left: number;
  /** Set for a stretched menu only; the others keep their own width. */
  readonly width: number | null;
  /** The menu opened above its origin: it lifts in rather than drops in. */
  readonly isAbove: boolean;
}

export function menuTopNextToAnchor(
  anchor: { readonly top: number; readonly bottom: number },
  menuHeight: number,
  viewportHeight: number,
  gap: number,
): number {
  const topWhenBelow = anchor.bottom + gap;
  if (topWhenBelow + menuHeight <= viewportHeight) {
    return topWhenBelow;
  }
  return Math.max(anchor.top - gap - menuHeight, 0);
}

/** `start` clamped into `[margin, limit - size - margin]`, or `margin` when the size fills it. */
function clampIntoWindow(start: number, size: number, limit: number, margin: number): number {
  return Math.max(Math.min(start, limit - size - margin), margin);
}

/**
 * The menu's top-left corner in the window. `gap` separates it from an element; `margin` is the
 * least room it keeps from the window's edges when it opens at a point.
 */
export function placeMenu(
  origin: MenuOrigin,
  menu: { readonly width: number; readonly height: number },
  viewport: { readonly width: number; readonly height: number },
  gap: number,
  margin: number,
): MenuPlacement {
  if (origin.kind === "point") {
    // Down and to the trailing side of the pointer, as desktop menus open; flipped back over the
    // pointer when that side has no room, then kept inside the window.
    const fitsRight = origin.x + menu.width + margin <= viewport.width;
    const fitsBelow = origin.y + menu.height + margin <= viewport.height;
    const top = clampIntoWindow(fitsBelow ? origin.y : origin.y - menu.height, menu.height, viewport.height, margin);
    return {
      top,
      left: clampIntoWindow(fitsRight ? origin.x : origin.x - menu.width, menu.width, viewport.width, margin),
      width: null,
      isAbove: top < origin.y,
    };
  }
  const { bounds, align } = origin;
  const top = menuTopNextToAnchor(bounds, menu.height, viewport.height, gap);
  const left =
    align === "end"
      ? Math.max(bounds.right - menu.width, 0)
      : align === "start"
        ? clampIntoWindow(bounds.left, menu.width, viewport.width, 0)
        : bounds.left;
  return { top, left, width: align === "stretch" ? bounds.right - bounds.left : null, isAbove: top < bounds.top };
}
