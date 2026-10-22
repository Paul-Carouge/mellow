import { describe, expect, it } from "vitest";

import { iconCropQuery } from "../lib/api";
import {
  MAXIMUM_ICON_ZOOM,
  centredSquare,
  clampCrop,
  cropMovedBy,
  cropToSend,
  cropZoomedTo,
  maximumZoom,
  placementInPercent,
  zoomOf,
} from "./icon-crop";

const LANDSCAPE = { width: 1200, height: 800 };
const PORTRAIT = { width: 600, height: 900 };

describe("centredSquare", () => {
  it("spans the short side, centred on the long one", () => {
    expect(centredSquare(LANDSCAPE)).toEqual({ x: 200, y: 0, size: 800 });
    expect(centredSquare(PORTRAIT)).toEqual({ x: 0, y: 150, size: 600 });
    expect(centredSquare({ width: 64, height: 64 })).toEqual({ x: 0, y: 0, size: 64 });
  });
});

describe("zoom", () => {
  it("starts at 1 on the centred square and caps how far a picture can be zoomed", () => {
    expect(zoomOf(centredSquare(LANDSCAPE), LANDSCAPE)).toBe(1);
    expect(maximumZoom(LANDSCAPE)).toBe(MAXIMUM_ICON_ZOOM);
    // 40 px tall: never below a 16 px square, so 2.5 at most.
    expect(maximumZoom({ width: 100, height: 40 })).toBe(2.5);
    expect(maximumZoom({ width: 10, height: 10 })).toBe(1);
  });

  it("zooms around the square's centre by default", () => {
    const zoomed = cropZoomedTo(centredSquare(LANDSCAPE), LANDSCAPE, 2);
    expect(zoomed).toEqual({ x: 400, y: 200, size: 400 });
  });

  it("keeps the anchor under the pointer while zooming", () => {
    const start = centredSquare(LANDSCAPE);
    const anchor = { x: 300, y: 100 };
    const zoomed = cropZoomedTo(start, LANDSCAPE, 2, anchor);
    // The anchor sat at an eighth across and down the square; it still does.
    expect((anchor.x - zoomed.x) / zoomed.size).toBeCloseTo((anchor.x - start.x) / start.size);
    expect((anchor.y - zoomed.y) / zoomed.size).toBeCloseTo((anchor.y - start.y) / start.size);
  });

  it("stays between the centred square and the maximum zoom", () => {
    const start = centredSquare(LANDSCAPE);
    expect(cropZoomedTo(start, LANDSCAPE, 0.2).size).toBe(800);
    expect(cropZoomedTo(start, LANDSCAPE, 100).size).toBe(800 / MAXIMUM_ICON_ZOOM);
  });

  it("never leaves the picture when zooming out near an edge", () => {
    const corner = { x: 1100, y: 700, size: 100 };
    expect(cropZoomedTo(corner, LANDSCAPE, 1)).toEqual({ x: 400, y: 0, size: 800 });
  });
});

describe("cropMovedBy", () => {
  it("moves within the picture and stops at its edges", () => {
    const start = centredSquare(LANDSCAPE);
    expect(cropMovedBy(start, LANDSCAPE, 50, 0)).toEqual({ x: 250, y: 0, size: 800 });
    expect(cropMovedBy(start, LANDSCAPE, -1000, -1000)).toEqual({ x: 0, y: 0, size: 800 });
    expect(cropMovedBy(start, LANDSCAPE, 1000, 1000)).toEqual({ x: 400, y: 0, size: 800 });
  });

  it("clamps a square larger than the picture to its short side", () => {
    expect(clampCrop({ x: -10, y: -10, size: 5000 }, PORTRAIT)).toEqual({ x: 0, y: 0, size: 600 });
  });
});

describe("cropToSend", () => {
  it("sends nothing for the centred square, so the instance centres it itself", () => {
    expect(cropToSend(centredSquare(LANDSCAPE), LANDSCAPE)).toBeNull();
    expect(cropToSend({ x: 200.3, y: 0.2, size: 799.8 }, LANDSCAPE)).toBeNull();
  });

  it("rounds to whole pixels that stay inside the picture", () => {
    expect(cropToSend({ x: 10.6, y: 20.4, size: 300.5 }, LANDSCAPE)).toEqual({ x: 11, y: 20, size: 301 });
    expect(cropToSend({ x: 899.7, y: 499.6, size: 300.4 }, LANDSCAPE)).toEqual({ x: 900, y: 500, size: 300 });
  });

  it("becomes the query the instance reads, or none", () => {
    expect(iconCropQuery({ x: 11, y: 20, size: 301 })).toBe("?crop_x=11&crop_y=20&crop_size=301");
    expect(iconCropQuery(null)).toBe("");
  });
});

describe("placementInPercent", () => {
  it("scales the picture so the square fills its box", () => {
    expect(placementInPercent({ x: 200, y: 0, size: 800 }, LANDSCAPE)).toEqual({ left: -25, top: -0, width: 150 });
    expect(placementInPercent({ x: 0, y: 150, size: 600 }, PORTRAIT)).toEqual({ left: -0, top: -25, width: 100 });
  });
});
