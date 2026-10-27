// Where a menu opened from a button sits (DESIGN.md §6.33): under the button, or above it when the
// window has no room below (a message or a friend near the bottom of a phone screen), and never
// past the window's top.

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
