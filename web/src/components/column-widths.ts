// The widths the reader gives the columns of the wide layout (DESIGN.md 4.5): the channel list
// in the navigator, the docked members side sheet and the docked search results. Remembered on
// this device, checked on the way in, and fitted to the window so the conversation keeps its
// minimum width. Pure: sizes in CSS pixels and storage are passed in.

export type ResizableColumn = "channels" | "members" | "search";

export interface ColumnBounds {
  minimum: number;
  maximum: number;
  /** The width a double-click puts back. */
  default: number;
}

export type ColumnWidths = Record<ResizableColumn, number>;

const COLUMN_WIDTHS_STORAGE_KEY = "mellow.column-widths";

/** The width kept within the column's bounds and within `room`, though never under its minimum. */
export function clampColumnWidth(width: number, bounds: ColumnBounds, room: number): number {
  const widest = largestColumnWidth(bounds, room);
  return Math.round(Math.min(Math.max(width, bounds.minimum), widest));
}

/** The widest the column may be now: its maximum, or less when the window leaves less room. */
export function largestColumnWidth(bounds: ColumnBounds, room: number): number {
  return Math.max(bounds.minimum, Math.min(bounds.maximum, room));
}

/**
 * The widths laid out for the navigator's channel list and the docked side sheet (members or
 * results; `null` when none is docked), given the room the window leaves beside a conversation
 * of minimum width. The side sheet gives way first, down to its minimum, so opening it moves the
 * channel list only when even that does not fit. Below the breakpoints nothing fits and every
 * column stays at its minimum: the drawers take over there.
 */
export function fitColumnWidths(
  room: number,
  channels: { width: number; bounds: ColumnBounds },
  sideSheet: { width: number; bounds: ColumnBounds } | null,
): { channels: number; sideSheet: number | null } {
  const fittedChannels = clampColumnWidth(channels.width, channels.bounds, room - (sideSheet?.bounds.minimum ?? 0));
  return {
    channels: fittedChannels,
    sideSheet: sideSheet ? clampColumnWidth(sideSheet.width, sideSheet.bounds, room - fittedChannels) : null,
  };
}

/**
 * The width after a key on the handle (WAI-ARIA window splitter): the arrows move the edge one
 * step left or right, Home and End to the narrowest and widest. `null` for any other key.
 * `growsRightwards` says whether moving the edge to the right widens this column: the channel
 * list's trailing edge does, the side sheet's leading edge does not (and both flip right to left).
 */
export function widthAfterKey(
  key: string,
  width: number,
  bounds: ColumnBounds,
  growsRightwards: boolean,
  step: number,
): number | null {
  const towardsWider = growsRightwards ? step : -step;
  switch (key) {
    case "ArrowRight":
      return clampColumnWidth(width + towardsWider, bounds, bounds.maximum);
    case "ArrowLeft":
      return clampColumnWidth(width - towardsWider, bounds, bounds.maximum);
    case "Home":
      return bounds.minimum;
    case "End":
      return bounds.maximum;
    default:
      return null;
  }
}

/** The widths this browser remembers, each within its bounds; the default where none or garbage is stored. */
export function readColumnWidths(
  storage: Pick<Storage, "getItem"> | null,
  boundsByColumn: Record<ResizableColumn, ColumnBounds>,
): ColumnWidths {
  let stored: unknown = null;
  try {
    stored = JSON.parse(storage?.getItem(COLUMN_WIDTHS_STORAGE_KEY) ?? "null");
  } catch (error) {
    console.warn("unreadable column widths", error);
  }
  const widthOf = (column: ResizableColumn): number => {
    const bounds = boundsByColumn[column];
    const width: unknown = typeof stored === "object" && stored !== null ? Reflect.get(stored, column) : undefined;
    return typeof width === "number" && Number.isFinite(width)
      ? clampColumnWidth(width, bounds, bounds.maximum)
      : bounds.default;
  };
  return { channels: widthOf("channels"), members: widthOf("members"), search: widthOf("search") };
}

/** Stores the widths when the browser lets us; a full or blocked storage keeps them for this tab only. */
export function writeColumnWidths(storage: Pick<Storage, "setItem"> | null, widths: ColumnWidths): void {
  try {
    storage?.setItem(COLUMN_WIDTHS_STORAGE_KEY, JSON.stringify(widths));
  } catch (error) {
    console.warn("couldn't store the column widths", error);
  }
}
