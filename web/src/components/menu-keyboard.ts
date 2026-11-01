// Keyboard movement inside the web client's menus (DESIGN.md §6.33): arrows wrap, Home and End
// jump to the ends, a letter moves to the next item starting with it; Esc and light dismiss come
// from the popover itself.

/**
 * The items of a menu, plain, radio or checkbox, in document order, with the controls a menu
 * holds (`data-menu-control`, a volume slider): arrows up and down reach them too, while left and
 * right stay theirs.
 */
export function menuItemsIn(menu: HTMLElement | null): HTMLElement[] {
  return [...(menu?.querySelectorAll<HTMLElement>("[role^=menuitem], [data-menu-control]") ?? [])];
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

/** Lower case without accents, so "e" finds "Épingler". */
function foldedForTypeahead(text: string): string {
  return text.normalize("NFD").replace(/\p{Diacritic}/gu, "").trim().toLocaleLowerCase();
}

/**
 * Type-ahead: the next item after the current one whose label starts with the typed character,
 * wrapping; pressing it again cycles through the items that share it. `null` when none does or
 * the key is not a character.
 */
export function typeaheadMenuItemIndex(key: string, currentIndex: number, labels: readonly string[]): number | null {
  if ([...key].length !== 1 || key.trim() === "") {
    return null;
  }
  const typed = foldedForTypeahead(key);
  for (let step = 1; step <= labels.length; step += 1) {
    const index = (Math.max(currentIndex, -1) + step) % labels.length;
    if (foldedForTypeahead(labels[index] ?? "").startsWith(typed)) {
      return index;
    }
  }
  return null;
}

/** The menu's keydown handler: moves focus between its items. */
export function moveFocusWithinMenu(event: KeyboardEvent, menu: HTMLElement | null): void {
  const items = menuItemsIn(menu);
  const currentIndex = items.findIndex((item) => item === document.activeElement);
  const hasModifier = event.ctrlKey || event.metaKey || event.altKey;
  const next =
    nextMenuItemIndex(event.key, currentIndex, items.length) ??
    (hasModifier ? null : typeaheadMenuItemIndex(event.key, currentIndex, items.map((item) => item.textContent ?? "")));
  if (next !== null) {
    event.preventDefault();
    items[next]?.focus();
  }
}
