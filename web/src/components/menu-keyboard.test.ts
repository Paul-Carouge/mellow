import { describe, expect, it } from "vitest";

import { nextMenuItemIndex, typeaheadMenuItemIndex } from "./menu-keyboard";

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

  describe("type-ahead", () => {
    const labels = ["Reply", "Edit", "Copy text", "Copy user ID", "Épingler", "Delete"];

    it("moves to the next item starting with the letter, ignoring case", () => {
      expect(typeaheadMenuItemIndex("c", 0, labels)).toBe(2);
      expect(typeaheadMenuItemIndex("C", 0, labels)).toBe(2);
    });

    it("cycles through the items sharing the letter, wrapping", () => {
      expect(typeaheadMenuItemIndex("c", 2, labels)).toBe(3);
      expect(typeaheadMenuItemIndex("c", 3, labels)).toBe(2);
    });

    it("ignores accents, so French labels are found", () => {
      expect(typeaheadMenuItemIndex("e", 1, labels)).toBe(4);
    });

    it("starts from the top when no item holds focus", () => {
      expect(typeaheadMenuItemIndex("r", -1, labels)).toBe(0);
    });

    it("leaves other keys and unknown letters alone", () => {
      expect(typeaheadMenuItemIndex("z", 0, labels)).toBeNull();
      expect(typeaheadMenuItemIndex(" ", 0, labels)).toBeNull();
      expect(typeaheadMenuItemIndex("Enter", 0, labels)).toBeNull();
    });
  });
});
