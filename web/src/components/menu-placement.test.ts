import { describe, expect, it } from "vitest";

import { menuTopNextToAnchor } from "./menu-placement";

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
