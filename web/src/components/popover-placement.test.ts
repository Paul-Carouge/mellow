import { describe, expect, it } from "vitest";
import { popoverPositionBesideAnchor } from "./popover-placement";

const VIEWPORT = { width: 1440, height: 900 };
const PANEL = { width: 400, height: 440 };
const GAP = 8;

describe("popoverPositionBesideAnchor", () => {
  it("opens below the anchor, a gap away, when it fits", () => {
    expect(popoverPositionBesideAnchor({ top: 100, bottom: 132, right: 900 }, PANEL, VIEWPORT, GAP)).toEqual({ top: 140, left: 500 });
  });

  it("opens above the composer box a gap clear of its top edge, not over it", () => {
    // The composer box runs from 828 to 884; the emoji button inside it from 840 to 872.
    const position = popoverPositionBesideAnchor({ top: 828, bottom: 884, right: 1400 }, PANEL, VIEWPORT, GAP);
    expect(position.top + PANEL.height).toBe(828 - GAP);
  });

  it("stays inside the window", () => {
    expect(popoverPositionBesideAnchor({ top: 200, bottom: 600, right: 100 }, PANEL, VIEWPORT, GAP)).toEqual({ top: 0, left: 0 });
    expect(popoverPositionBesideAnchor({ top: 100, bottom: 132, right: 2000 }, PANEL, VIEWPORT, GAP).left).toBe(1040);
  });
});
