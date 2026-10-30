import { describe, expect, it } from "vitest";

import { nextEmojiCell, sectionAtScrollTop } from "./emoji-grid";

// Two sections of 4 columns: recent (6 cells: a full row and a row of 2), people (9 cells).
//   recent  0  1  2  3      people   6  7  8  9
//           4  5                    10 11 12 13
//                                   14
const sections = [6, 9];
const press = (key: string, isControlHeld = false) => ({ key, isControlHeld });
const move = (current: number, key: string, isControlHeld = false) =>
  nextEmojiCell(sections, 4, current, press(key, isControlHeld));

describe("nextEmojiCell", () => {
  it("moves one cell left and right across sections, stopping at both ends", () => {
    expect(move(5, "ArrowRight")).toBe(6);
    expect(move(6, "ArrowLeft")).toBe(5);
    expect(move(0, "ArrowLeft")).toBe(0);
    expect(move(14, "ArrowRight")).toBe(14);
  });

  it("moves down a row, onto a short last row's last cell, then into the next section's column", () => {
    expect(move(1, "ArrowDown")).toBe(5);
    expect(move(3, "ArrowDown")).toBe(5);
    expect(move(5, "ArrowDown")).toBe(7);
    expect(move(13, "ArrowDown")).toBe(14);
    expect(move(14, "ArrowDown")).toBe(14);
  });

  it("moves up a row, into the previous section's last row, and from the first row to the search", () => {
    expect(move(11, "ArrowUp")).toBe(7);
    expect(move(7, "ArrowUp")).toBe(5);
    expect(move(6, "ArrowUp")).toBe(4);
    expect(move(2, "ArrowUp")).toBe("search");
  });

  it("goes to the row's ends with Home and End, and to the first and last emoji with Ctrl", () => {
    expect(move(12, "Home")).toBe(10);
    expect(move(10, "End")).toBe(13);
    expect(move(4, "End")).toBe(5);
    expect(move(12, "Home", true)).toBe(0);
    expect(move(1, "End", true)).toBe(14);
  });

  it("jumps between sections with Page Up and Page Down", () => {
    expect(move(2, "PageDown")).toBe(6);
    expect(move(8, "PageDown")).toBe(14);
    expect(move(8, "PageUp")).toBe(6);
    expect(move(6, "PageUp")).toBe(0);
    expect(move(3, "PageUp")).toBe(0);
  });

  it("ignores other keys and cells outside the sections", () => {
    expect(move(3, "a")).toBeNull();
    expect(move(15, "ArrowRight")).toBeNull();
  });

  it("works with a single column", () => {
    expect(nextEmojiCell([2, 2], 1, 1, press("ArrowDown"))).toBe(2);
    expect(nextEmojiCell([2, 2], 1, 2, press("ArrowUp"))).toBe(1);
  });
});

describe("sectionAtScrollTop", () => {
  it("names the last section whose top has scrolled to the top of the list", () => {
    const tops = [0, 120, 480];
    expect(sectionAtScrollTop(tops, 0, false)).toBe(0);
    expect(sectionAtScrollTop(tops, 119.5, false)).toBe(1);
    expect(sectionAtScrollTop(tops, 300, false)).toBe(1);
    expect(sectionAtScrollTop(tops, 9000, false)).toBe(2);
    expect(sectionAtScrollTop([], 50, false)).toBe(0);
  });

  it("names the last section at the end of the list, even when its top cannot reach the top", () => {
    expect(sectionAtScrollTop([0, 120, 480], 400, true)).toBe(2);
    expect(sectionAtScrollTop([], 0, true)).toBe(0);
  });
});
