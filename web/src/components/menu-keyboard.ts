// Keyboard movement inside the web client's menus (DESIGN.md §6.33): arrows wrap, Home and End
// jump to the ends; Esc and light dismiss come from the popover itself.

/** The items of a menu, plain or radio, in document order. */
export function menuItemsIn(menu: HTMLElement | null): HTMLElement[] {
  return [...(menu?.querySelectorAll<HTMLElement>("[role^=menuitem]") ?? [])];
}

/** The index the key moves focus to, or `null` for keys a menu does not handle. */
export function nextMenuItemIndex(key: string, currentIndex: number, itemCount: number): number | null {
  if (itemCount === 0) {
    return null;
  }
  switch (key) {
    case "ArrowDown":
      return (currentIndex + 1) % itemCount;
    case "ArrowUp":
      // From nowhere (-1), up lands on the last item.
      return (Math.max(currentIndex, 0) - 1 + itemCount) % itemCount;
    case "Home":
      return 0;
    case "End":
      return itemCount - 1;
    default:
      return null;
  }
}

/** The menu's keydown handler: moves focus between its items. */
export function moveFocusWithinMenu(event: KeyboardEvent, menu: HTMLElement | null): void {
  const items = menuItemsIn(menu);
  const currentIndex = items.findIndex((item) => item === document.activeElement);
  const next = nextMenuItemIndex(event.key, currentIndex, items.length);
  if (next !== null) {
    event.preventDefault();
    items[next]?.focus();
  }
}
