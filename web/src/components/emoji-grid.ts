// Moving through the emoji picker's sections (DESIGN.md §6.52): each section is its own grid of
// `columns` cells, the last row often short, and the keys move as in one grid that runs on from
// section to section, keeping the column where it can.

/** Where a key moves focus: a cell's index over all sections, "search" above the first row, or nowhere. */
export type EmojiGridMove = number | "search" | null;

export interface EmojiGridKeyPress {
  key: string;
  /** Ctrl (Cmd on macOS) turns Home and End into the first and last emoji of all. */
  isControlHeld: boolean;
}

interface Position {
  section: number;
  start: number;
  length: number;
  row: number;
  column: number;
}

function positionOf(sectionLengths: readonly number[], columns: number, index: number): Position | null {
  let start = 0;
  for (const [section, length] of sectionLengths.entries()) {
    if (index < start + length) {
      const offset = index - start;
      return { section, start, length, row: Math.floor(offset / columns), column: offset % columns };
    }
    start += length;
  }
  return null;
}

function startOf(sectionLengths: readonly number[], section: number): number {
  return sectionLengths.slice(0, section).reduce((total, length) => total + length, 0);
}

/**
 * The cell a key moves to from `current`. Arrows move by one cell or one row (Up from the first
 * row goes back to the search field), Home and End to the row's ends (with Ctrl, the first and
 * last emoji), Page Up and Page Down to the start of the previous and next section. Sections are
 * never empty: the picker leaves empty ones out.
 */
export function nextEmojiCell(
  sectionLengths: readonly number[],
  columns: number,
  current: number,
  press: EmojiGridKeyPress,
): EmojiGridMove {
  const total = sectionLengths.reduce((sum, length) => sum + length, 0);
  const position = positionOf(sectionLengths, Math.max(columns, 1), current);
  if (!position) {
    return null;
  }
  const width = Math.max(columns, 1);
  const { section, start, length, row, column } = position;
  const rowStart = start + row * width;
  const lastSection = sectionLengths.length - 1;
  switch (press.key) {
    case "ArrowRight":
      return Math.min(current + 1, total - 1);
    case "ArrowLeft":
      return Math.max(current - 1, 0);
    case "ArrowDown": {
      if (rowStart + width < start + length) {
        return Math.min(rowStart + width + column, start + length - 1);
      }
      if (section === lastSection) {
        return current;
      }
      const nextStart = start + length;
      return nextStart + Math.min(column, (sectionLengths[section + 1] ?? 1) - 1);
    }
    case "ArrowUp": {
      if (row > 0) {
        return rowStart - width + column;
      }
      if (section === 0) {
        return "search";
      }
      const previousLength = sectionLengths[section - 1] ?? 1;
      const previousStart = start - previousLength;
      const lastRowStart = Math.floor((previousLength - 1) / width) * width;
      return previousStart + Math.min(lastRowStart + column, previousLength - 1);
    }
    case "Home":
      return press.isControlHeld ? 0 : rowStart;
    case "End":
      return press.isControlHeld ? total - 1 : Math.min(rowStart + width, start + length) - 1;
    case "PageDown":
      return section === lastSection ? total - 1 : start + length;
    case "PageUp":
      return current > start || section === 0 ? start : startOf(sectionLengths, section - 1);
    default:
      return null;
  }
}

/**
 * The section being read: the last one whose top has reached the top of the scrolled list, or the
 * last of all once the list is scrolled to its end (a short last section never reaches the top).
 */
export function sectionAtScrollTop(sectionTops: readonly number[], scrollTop: number, isAtEnd: boolean): number {
  if (isAtEnd && sectionTops.length > 0) {
    return sectionTops.length - 1;
  }
  // A pixel of slack: scrolling to a section lands on fractional offsets.
  return sectionTops.reduce((reached, top, index) => (top <= scrollTop + 1 ? index : reached), 0);
}
