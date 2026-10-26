import { describe, expect, it } from "vitest";

import { nextMenuItemIndex } from "./menu-keyboard";

describe("menu keyboard", () => {
  it("wraps with the arrows", () => {
    expect(nextMenuItemIndex("ArrowDown", 0, 3)).toBe(1);
    expect(nextMenuItemIndex("ArrowDown", 2, 3)).toBe(0);
    expect(nextMenuItemIndex("ArrowUp", 0, 3)).toBe(2);
    expect(nextMenuItemIndex("ArrowUp", 2, 3)).toBe(1);
  });

  it("starts at an end when no item holds focus", () => {
    expect(nextMenuItemIndex("ArrowDown", -1, 3)).toBe(0);
    expect(nextMenuItemIndex("ArrowUp", -1, 3)).toBe(2);
  });

  it("jumps to the ends with Home and End", () => {
    expect(nextMenuItemIndex("Home", 1, 3)).toBe(0);
    expect(nextMenuItemIndex("End", 1, 3)).toBe(2);
  });

  it("ignores other keys and empty menus", () => {
    expect(nextMenuItemIndex("Enter", 1, 3)).toBeNull();
    expect(nextMenuItemIndex("ArrowDown", -1, 0)).toBeNull();
  });
});
