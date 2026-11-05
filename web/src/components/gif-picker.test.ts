import { afterEach, describe, expect, it, vi } from "vitest";

import type { KlipyGif, KlipyItem } from "../lib/klipy";
import {
  GIF_SEARCH_DEBOUNCE_MILLISECONDS,
  type TileBounds,
  createLatestRequestGuard,
  debounce,
  gifsInSlugOrder,
  nextGifTile,
  tileAspectRatio,
  withFavouriteSet,
} from "./gif-picker";

function gif(slug: string, width = 200, height = 100): KlipyGif {
  const media = { url: `https://static.klipy.com/${slug}.webp`, width, height, isVideo: false };
  return { kind: "gif", slug, title: slug, tile: media, still: null, messageUrl: media.url, blurPreview: null };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("createLatestRequestGuard", () => {
  it("lets only the latest request change the picker", () => {
    const guard = createLatestRequestGuard();
    const forCa = guard.start();
    const forCat = guard.start();
    expect(guard.isLatest(forCa)).toBe(false);
    expect(guard.isLatest(forCat)).toBe(true);
  });
});

describe("debounce", () => {
  it("asks once typing paused, and at once when flushed", () => {
    vi.useFakeTimers();
    const action = vi.fn();
    const search = debounce(action, GIF_SEARCH_DEBOUNCE_MILLISECONDS);
    search.call();
    vi.advanceTimersByTime(GIF_SEARCH_DEBOUNCE_MILLISECONDS - 1);
    search.call();
    vi.advanceTimersByTime(GIF_SEARCH_DEBOUNCE_MILLISECONDS - 1);
    expect(action).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(action).toHaveBeenCalledTimes(1);
    search.call();
    search.flush();
    vi.advanceTimersByTime(GIF_SEARCH_DEBOUNCE_MILLISECONDS);
    expect(action).toHaveBeenCalledTimes(2);
    search.call();
    search.cancel();
    vi.advanceTimersByTime(GIF_SEARCH_DEBOUNCE_MILLISECONDS);
    expect(action).toHaveBeenCalledTimes(2);
  });
});

describe("nextGifTile", () => {
  // Two justified rows: [0 wide][1][2] above [3][4 wide].
  const rows: TileBounds[] = [
    { left: 0, top: 0, width: 200, height: 96 },
    { left: 204, top: 0, width: 100, height: 96 },
    { left: 308, top: 0, width: 100, height: 96 },
    { left: 0, top: 100, width: 100, height: 96 },
    { left: 104, top: 100, width: 304, height: 96 },
  ];

  it("steps through Klipy's order with Left and Right", () => {
    expect(nextGifTile(rows, 2, "ArrowRight")).toBe(3);
    expect(nextGifTile(rows, 3, "ArrowLeft")).toBe(2);
    expect(nextGifTile(rows, 0, "ArrowLeft")).toBeNull();
    expect(nextGifTile(rows, 4, "ArrowRight")).toBeNull();
    expect(nextGifTile(rows, 2, "Home")).toBe(0);
    expect(nextGifTile(rows, 0, "End")).toBe(4);
  });

  it("reaches the nearest tile of the row above or below", () => {
    expect(nextGifTile(rows, 0, "ArrowDown")).toBe(3);
    expect(nextGifTile(rows, 2, "ArrowDown")).toBe(4);
    expect(nextGifTile(rows, 4, "ArrowUp")).toBe(1);
    expect(nextGifTile(rows, 3, "ArrowUp")).toBe(0);
  });

  it("goes back to the search from the first row, and nowhere past the last", () => {
    expect(nextGifTile(rows, 1, "ArrowUp")).toBe("search");
    expect(nextGifTile(rows, 4, "ArrowDown")).toBeNull();
    expect(nextGifTile(rows, 1, "a")).toBeNull();
    expect(nextGifTile([], 0, "ArrowDown")).toBeNull();
  });
});

describe("favourites", () => {
  it("shows them in the instance's order, leaving out what Klipy no longer knows", () => {
    const fromKlipy: KlipyItem[] = [gif("b"), gif("a"), { kind: "ad", position: 2, width: null, height: null }];
    expect(gifsInSlugOrder(fromKlipy, ["a", "gone", "b"]).map((item) => item.slug)).toEqual(["a", "b"]);
  });

  it("puts a new star first and takes an unstarred one out", () => {
    expect(withFavouriteSet(["a", "b"], "c", true)).toEqual(["c", "a", "b"]);
    expect(withFavouriteSet(["a", "b"], "b", true)).toEqual(["b", "a"]);
    expect(withFavouriteSet(["a", "b"], "a", false)).toEqual(["b"]);
    expect(withFavouriteSet([], "a", false)).toEqual([]);
  });
});

describe("tileAspectRatio", () => {
  it("follows the GIF's shape within bounds, and gives an ad a square", () => {
    expect(tileAspectRatio(gif("a", 200, 100))).toBe(2);
    expect(tileAspectRatio(gif("a", 1000, 100))).toBe(2.5);
    expect(tileAspectRatio(gif("a", 10, 100))).toBe(0.5);
    expect(tileAspectRatio({ kind: "ad", position: 0, width: null, height: null })).toBe(1);
    expect(tileAspectRatio({ kind: "ad", position: 0, width: 300, height: 250 })).toBe(1.2);
  });
});
