import { describe, expect, it } from "vitest";

import { pictureCropQuery } from "../lib/api";
import {
  BANNER_ASPECT,
  MAXIMUM_CROP_ZOOM,
  SQUARE_ASPECT,
  clampCrop,
  cropMovedBy,
  cropToSend,
  cropZoomedTo,
  largestCentredCrop,
  maximumZoom,
  placementInPercent,
  zoomOf,
} from "./picture-crop";

const LANDSCAPE = { width: 1200, height: 800 };
const PORTRAIT = { width: 600, height: 900 };
const SQUARE = SQUARE_ASPECT;
const BANNER = BANNER_ASPECT;

describe("largestCentredCrop", () => {
  it("spans the short side of a square, centred on the long one", () => {
    expect(largestCentredCrop(LANDSCAPE, SQUARE)).toEqual({ x: 200, y: 0, width: 800 });
    expect(largestCentredCrop(PORTRAIT, SQUARE)).toEqual({ x: 0, y: 150, width: 600 });
    expect(largestCentredCrop({ width: 64, height: 64 }, SQUARE)).toEqual({ x: 0, y: 0, width: 64 });
  });

  it("keeps the banner's 5:2 band, as wide as the picture allows", () => {
    // 1200 wide holds a 480 tall band, centred in 800.
    expect(largestCentredCrop(LANDSCAPE, BANNER)).toEqual({ x: 0, y: 160, width: 1200 });
    // 3000 × 800 is wider than 5:2: the band is 2000 wide, centred.
    expect(largestCentredCrop({ width: 3000, height: 800 }, BANNER)).toEqual({ x: 500, y: 0, width: 2000 });
  });
});

describe("zoom", () => {
  it("starts at 1 on the largest part and caps how far a picture can be zoomed", () => {
    expect(zoomOf(largestCentredCrop(LANDSCAPE, SQUARE), LANDSCAPE, SQUARE)).toBe(1);
    expect(zoomOf(largestCentredCrop(LANDSCAPE, BANNER), LANDSCAPE, BANNER)).toBe(1);
    expect(maximumZoom(LANDSCAPE, SQUARE)).toBe(MAXIMUM_CROP_ZOOM);
    // 40 px tall: never below a 16 px square, so 2.5 at most.
    expect(maximumZoom({ width: 100, height: 40 }, SQUARE)).toBe(2.5);
    expect(maximumZoom({ width: 10, height: 10 }, SQUARE)).toBe(1);
    // A 200 × 80 band: its 80 px height reaches 16 px at 5×.
    expect(maximumZoom({ width: 200, height: 80 }, BANNER)).toBe(5);
  });

  it("zooms around the crop's centre by default", () => {
    expect(cropZoomedTo(largestCentredCrop(LANDSCAPE, SQUARE), LANDSCAPE, SQUARE, 2)).toEqual({
      x: 400,
      y: 200,
      width: 400,
    });
    // The 1200 × 480 band at 2× is 600 × 240 around the same centre (600, 400).
    expect(cropZoomedTo(largestCentredCrop(LANDSCAPE, BANNER), LANDSCAPE, BANNER, 2)).toEqual({
      x: 300,
      y: 280,
      width: 600,
    });
  });

  it("keeps the anchor under the pointer while zooming", () => {
    for (const aspect of [SQUARE, BANNER]) {
      const start = largestCentredCrop(LANDSCAPE, aspect);
      const anchor = { x: 300, y: 250 };
      const zoomed = cropZoomedTo(start, LANDSCAPE, aspect, 2, anchor);
      // The anchor keeps its share across and down the crop.
      expect((anchor.x - zoomed.x) / zoomed.width).toBeCloseTo((anchor.x - start.x) / start.width);
      expect(((anchor.y - zoomed.y) * aspect) / zoomed.width).toBeCloseTo(((anchor.y - start.y) * aspect) / start.width);
    }
  });

  it("stays between the largest part and the maximum zoom", () => {
    const start = largestCentredCrop(LANDSCAPE, SQUARE);
    expect(cropZoomedTo(start, LANDSCAPE, SQUARE, 0.2).width).toBe(800);
    expect(cropZoomedTo(start, LANDSCAPE, SQUARE, 100).width).toBe(800 / MAXIMUM_CROP_ZOOM);
  });

  it("never leaves the picture when zooming out near an edge", () => {
    expect(cropZoomedTo({ x: 1100, y: 700, width: 100 }, LANDSCAPE, SQUARE, 1)).toEqual({ x: 400, y: 0, width: 800 });
    expect(cropZoomedTo({ x: 1000, y: 700, width: 200 }, LANDSCAPE, BANNER, 1)).toEqual({ x: 0, y: 320, width: 1200 });
  });
});

describe("cropMovedBy", () => {
  it("moves within the picture and stops at its edges", () => {
    const start = largestCentredCrop(LANDSCAPE, SQUARE);
    expect(cropMovedBy(start, LANDSCAPE, SQUARE, 50, 0)).toEqual({ x: 250, y: 0, width: 800 });
    expect(cropMovedBy(start, LANDSCAPE, SQUARE, -1000, -1000)).toEqual({ x: 0, y: 0, width: 800 });
    expect(cropMovedBy(start, LANDSCAPE, SQUARE, 1000, 1000)).toEqual({ x: 400, y: 0, width: 800 });
  });

  it("moves a banner's band up and down, its bottom edge at the picture's", () => {
    const start = largestCentredCrop(LANDSCAPE, BANNER);
    expect(cropMovedBy(start, LANDSCAPE, BANNER, 0, -1000)).toEqual({ x: 0, y: 0, width: 1200 });
    expect(cropMovedBy(start, LANDSCAPE, BANNER, 0, 1000)).toEqual({ x: 0, y: 320, width: 1200 });
  });

  it("clamps a crop larger than the picture to the largest part", () => {
    expect(clampCrop({ x: -10, y: -10, width: 5000 }, PORTRAIT, SQUARE)).toEqual({ x: 0, y: 0, width: 600 });
    expect(clampCrop({ x: -10, y: 5000, width: 5000 }, PORTRAIT, BANNER)).toEqual({ x: 0, y: 660, width: 600 });
  });
});

describe("cropToSend", () => {
  it("sends nothing for the largest centred part, so the instance centres it itself", () => {
    expect(cropToSend(largestCentredCrop(LANDSCAPE, SQUARE), LANDSCAPE, SQUARE)).toBeNull();
    expect(cropToSend({ x: 200.3, y: 0.2, width: 799.8 }, LANDSCAPE, SQUARE)).toBeNull();
    expect(cropToSend(largestCentredCrop(LANDSCAPE, BANNER), LANDSCAPE, BANNER)).toBeNull();
  });

  it("rounds to whole pixels that stay inside the picture", () => {
    expect(cropToSend({ x: 10.6, y: 20.4, width: 300.5 }, LANDSCAPE, SQUARE)).toEqual({ x: 11, y: 20, width: 301 });
    expect(cropToSend({ x: 899.7, y: 499.6, width: 300.4 }, LANDSCAPE, SQUARE)).toEqual({ x: 900, y: 500, width: 300 });
  });

  it("keeps a banner's band whole at the bottom edge, its height rounded up", () => {
    // 601 wide is 240.4 tall: the band starts at 559 at most, so the instance keeps all 601.
    expect(cropToSend({ x: 0, y: 559.6, width: 601 }, LANDSCAPE, BANNER)).toEqual({ x: 0, y: 559, width: 601 });
  });

  it("becomes the query the instance reads, or none", () => {
    expect(pictureCropQuery({ x: 11, y: 20, width: 301 }, "square")).toBe("?crop_x=11&crop_y=20&crop_size=301");
    expect(pictureCropQuery({ x: 0, y: 559, width: 601 }, "banner")).toBe("?crop_x=0&crop_y=559&crop_width=601");
    expect(pictureCropQuery(null, "square")).toBe("");
    expect(pictureCropQuery(null, "banner")).toBe("");
  });
});

describe("placementInPercent", () => {
  it("scales the picture so the crop fills a box of its shape", () => {
    expect(placementInPercent({ x: 200, y: 0, width: 800 }, LANDSCAPE, SQUARE)).toEqual({ left: -25, top: -0, width: 150 });
    expect(placementInPercent({ x: 0, y: 150, width: 600 }, PORTRAIT, SQUARE)).toEqual({ left: -0, top: -25, width: 100 });
    // The band is 1200 × 480: 160 px down is a third of its height.
    expect(placementInPercent({ x: 0, y: 160, width: 1200 }, LANDSCAPE, BANNER)).toEqual({
      left: -0,
      top: (-160 / 480) * 100,
      width: 100,
    });
  });
});
