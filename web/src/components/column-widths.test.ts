import { describe, expect, it, vi } from "vitest";

import { memoryStorage } from "../test-memory-storage";
import {
  type ColumnBounds,
  clampColumnWidth,
  fitColumnWidths,
  largestColumnWidth,
  readColumnWidths,
  widthAfterKey,
  writeColumnWidths,
} from "./column-widths";

const CHANNELS: ColumnBounds = { minimum: 240, maximum: 360, default: 240 };
const MEMBERS: ColumnBounds = { minimum: 240, maximum: 320, default: 240 };
const SEARCH: ColumnBounds = { minimum: 400, maximum: 560, default: 400 };
const BOUNDS = { channels: CHANNELS, members: MEMBERS, search: SEARCH };
const STEP = 16;

describe("clampColumnWidth", () => {
  it("keeps a width within the column's bounds", () => {
    expect(clampColumnWidth(300, CHANNELS, 1_000)).toBe(300);
    expect(clampColumnWidth(100, CHANNELS, 1_000)).toBe(240);
    expect(clampColumnWidth(900, CHANNELS, 1_000)).toBe(360);
  });

  it("gives up width when the window leaves less room, never under the minimum", () => {
    expect(clampColumnWidth(360, CHANNELS, 300)).toBe(300);
    expect(clampColumnWidth(360, CHANNELS, 100)).toBe(240);
    expect(clampColumnWidth(360, CHANNELS, -50)).toBe(240);
    expect(largestColumnWidth(CHANNELS, 300)).toBe(300);
    expect(largestColumnWidth(CHANNELS, 100)).toBe(240);
  });

  it("lands on whole pixels", () => {
    expect(clampColumnWidth(250.6, CHANNELS, 1_000)).toBe(251);
  });
});

describe("fitColumnWidths", () => {
  it("lays out the reader's widths when the window holds them", () => {
    expect(fitColumnWidths(1_000, { width: 360, bounds: CHANNELS }, { width: 320, bounds: MEMBERS })).toEqual({
      channels: 360,
      sideSheet: 320,
    });
  });

  it("narrows the side sheet first, then the channel list", () => {
    // 360 + 320 wanted, 620 of room: the side sheet gives its 60 px.
    expect(fitColumnWidths(620, { width: 360, bounds: CHANNELS }, { width: 320, bounds: MEMBERS })).toEqual({
      channels: 360,
      sideSheet: 260,
    });
    // At its minimum it gives no more: the channel list gives the rest.
    expect(fitColumnWidths(560, { width: 360, bounds: CHANNELS }, { width: 320, bounds: MEMBERS })).toEqual({
      channels: 320,
      sideSheet: 240,
    });
  });

  it("keeps every column at its minimum where nothing fits, below the breakpoints", () => {
    expect(fitColumnWidths(200, { width: 360, bounds: CHANNELS }, { width: 500, bounds: SEARCH })).toEqual({
      channels: 240,
      sideSheet: 400,
    });
  });

  it("gives the channel list all the room when no side sheet is docked", () => {
    expect(fitColumnWidths(330, { width: 360, bounds: CHANNELS }, null)).toEqual({ channels: 330, sideSheet: null });
  });
});

describe("widthAfterKey", () => {
  it("moves the channel list's trailing edge with the arrows", () => {
    expect(widthAfterKey("ArrowRight", 240, CHANNELS, true, STEP)).toBe(256);
    expect(widthAfterKey("ArrowLeft", 256, CHANNELS, true, STEP)).toBe(240);
  });

  it("moves the side sheet's leading edge with the arrows: left widens it", () => {
    expect(widthAfterKey("ArrowLeft", 240, MEMBERS, false, STEP)).toBe(256);
    expect(widthAfterKey("ArrowRight", 256, MEMBERS, false, STEP)).toBe(240);
  });

  it("stops at the bounds, and Home and End reach them", () => {
    expect(widthAfterKey("ArrowLeft", 240, CHANNELS, true, STEP)).toBe(240);
    expect(widthAfterKey("ArrowRight", 350, CHANNELS, true, STEP)).toBe(360);
    expect(widthAfterKey("Home", 300, CHANNELS, true, STEP)).toBe(240);
    expect(widthAfterKey("End", 300, CHANNELS, true, STEP)).toBe(360);
  });

  it("ignores other keys", () => {
    expect(widthAfterKey("ArrowUp", 300, CHANNELS, true, STEP)).toBeNull();
    expect(widthAfterKey("Enter", 300, CHANNELS, true, STEP)).toBeNull();
  });
});

describe("stored column widths", () => {
  it("remembers the widths across visits", () => {
    const storage = memoryStorage();
    writeColumnWidths(storage, { channels: 300, members: 280, search: 480 });

    expect(readColumnWidths(storage, BOUNDS)).toEqual({ channels: 300, members: 280, search: 480 });
  });

  it("starts at the defaults when nothing or something unreadable is stored", () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const storage = memoryStorage();
    const defaults = { channels: 240, members: 240, search: 400 };
    expect(readColumnWidths(storage, BOUNDS)).toEqual(defaults);
    expect(readColumnWidths(null, BOUNDS)).toEqual(defaults);

    for (const garbage of ["{not json", "[300, 280]", '"300"', "null", '{"channels":"300","members":null}']) {
      storage.setItem("mellow.column-widths", garbage);
      expect(readColumnWidths(storage, BOUNDS)).toEqual(defaults);
    }
  });

  it("brings stored widths back within the bounds", () => {
    const storage = memoryStorage();
    storage.setItem("mellow.column-widths", JSON.stringify({ channels: 9_000, members: -4, search: 450.4 }));

    expect(readColumnWidths(storage, BOUNDS)).toEqual({ channels: 360, members: 240, search: 450 });
  });

  it("keeps the widths for this tab when the storage refuses them", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const fullStorage = {
      setItem: () => {
        throw new DOMException("full", "QuotaExceededError");
      },
    };

    expect(() => writeColumnWidths(fullStorage, { channels: 300, members: 240, search: 400 })).not.toThrow();
    expect(warn).toHaveBeenCalled();
  });
});
