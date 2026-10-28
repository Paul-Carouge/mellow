import { describe, expect, it } from "vitest";

import { menuTopNextToAnchor, placeMenu } from "./menu-placement";

describe("menuTopNextToAnchor", () => {
  const button = { top: 100, bottom: 132 };

  it("opens under the button when the menu fits below it", () => {
    expect(menuTopNextToAnchor(button, 120, 800, 4)).toBe(136);
  });

  it("opens above the button near the bottom of the window", () => {
    const nearTheBottom = { top: 700, bottom: 732 };
    expect(menuTopNextToAnchor(nearTheBottom, 120, 800, 4)).toBe(576);
  });

  it("never goes past the top of a window too short for it either way", () => {
    expect(menuTopNextToAnchor(button, 400, 300, 4)).toBe(0);
  });
});

describe("placeMenu", () => {
  const menu = { width: 200, height: 150 };
  const desktop = { width: 1000, height: 800 };

  describe("at a point (right-click, long press)", () => {
    it("opens down and to the right of the pointer when it fits", () => {
      expect(placeMenu({ kind: "point", x: 300, y: 200 }, menu, desktop, 4, 4)).toEqual({
        top: 200,
        left: 300,
        width: null,
        isAbove: false,
      });
    });

    it("turns left of the pointer near the window's right edge", () => {
      expect(placeMenu({ kind: "point", x: 900, y: 200 }, menu, desktop, 4, 4).left).toBe(700);
    });

    it("opens above the pointer near the bottom, and says so for the lift-in", () => {
      const placement = placeMenu({ kind: "point", x: 300, y: 750 }, menu, desktop, 4, 4);
      expect(placement.top).toBe(600);
      expect(placement.isAbove).toBe(true);
    });

    it("stays inside a phone window too small for either side of the pointer", () => {
      const phone = { width: 320, height: 240 };
      const placement = placeMenu({ kind: "point", x: 160, y: 120 }, { width: 250, height: 200 }, phone, 4, 4);
      expect(placement.left).toBe(4);
      expect(placement.top).toBe(4);
    });

    it("keeps the margin at the top when the menu is taller than the window", () => {
      expect(placeMenu({ kind: "point", x: 10, y: 10 }, { width: 200, height: 900 }, desktop, 4, 4).top).toBe(4);
    });
  });

  describe("at an element (a button, or the focused item from the keyboard)", () => {
    const bounds = { top: 100, bottom: 132, left: 400, right: 640 };

    it("lines up with the element's trailing edge under it", () => {
      expect(placeMenu({ kind: "anchor", bounds, align: "end" }, menu, desktop, 4, 4)).toEqual({
        top: 136,
        left: 440,
        width: null,
        isAbove: false,
      });
    });

    it("lines up with the leading edge and stays inside the window", () => {
      const nearTheRight = { top: 100, bottom: 132, left: 900, right: 960 };
      expect(placeMenu({ kind: "anchor", bounds: nearTheRight, align: "start" }, menu, desktop, 4, 4).left).toBe(800);
    });

    it("takes the element's width when stretched", () => {
      expect(placeMenu({ kind: "anchor", bounds, align: "stretch" }, menu, desktop, 4, 4)).toMatchObject({
        left: 400,
        width: 240,
      });
    });

    it("opens above an element near the bottom", () => {
      const nearTheBottom = { top: 700, bottom: 732, left: 400, right: 640 };
      expect(placeMenu({ kind: "anchor", bounds: nearTheBottom, align: "end" }, menu, desktop, 4, 4)).toMatchObject({
        top: 546,
        isAbove: true,
      });
    });
  });
});
