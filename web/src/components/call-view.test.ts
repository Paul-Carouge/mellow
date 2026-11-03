import { describe, expect, it } from "vitest";

import { type CallPerson, arrangeCallTiles, featuredPersonOf, gridShapeFor, nextSpeaker, tileSizeOf, watchedCamerasOf } from "./call-view";

const person = (userId: string, video: CallPerson["video"] = null): CallPerson => ({ userId, video });

describe("arrangeCallTiles", () => {
  it("puts pictures first, each group in join order, and the featured person before everyone", () => {
    const people = [person("1"), person("2", "camera"), person("3"), person("4", "screen")];
    expect(arrangeCallTiles(people, null).tiles.map((tile) => tile.userId)).toEqual(["2", "4", "1", "3"]);
    expect(arrangeCallTiles(people, "3").tiles.map((tile) => tile.userId)).toEqual(["3", "2", "4", "1"]);
  });

  it("keeps nine tiles and sends the others under them", () => {
    const people = Array.from({ length: 12 }, (_, index) => person(String(index), index === 11 ? "camera" : null));
    const { tiles, overflow } = arrangeCallTiles(people, null);
    expect(tiles).toHaveLength(9);
    expect(tiles[0]?.userId).toBe("11");
    expect(overflow.map((tile) => tile.userId)).toEqual(["8", "9", "10"]);
  });
});

describe("tileSizeOf", () => {
  it("follows ADR-0035's sizes", () => {
    expect(tileSizeOf("grid", 1, false)).toBe("large");
    expect(tileSizeOf("grid", 2, false)).toBe("medium");
    expect(tileSizeOf("grid", 4, false)).toBe("medium");
    expect(tileSizeOf("grid", 5, false)).toBe("small");
    expect(tileSizeOf("speaker", 5, true)).toBe("large");
    expect(tileSizeOf("speaker", 5, false)).toBe("small");
  });
});

describe("watchedCamerasOf", () => {
  it("asks for every picture on a tile but your own", () => {
    const tiles = [person("2", "camera"), person("1", "camera"), person("4", "screen"), person("3")];
    expect(watchedCamerasOf(tiles, "grid", "1")).toEqual([
      { userId: "2", size: "medium" },
      { userId: "4", size: "medium" },
    ]);
    expect(watchedCamerasOf(tiles, "speaker", "1")).toEqual([
      { userId: "2", size: "large" },
      { userId: "4", size: "small" },
    ]);
  });
});

describe("gridShapeFor", () => {
  it("chooses the columns that make the largest 16:9 tiles", () => {
    expect(gridShapeFor(1, 1000, 600, 8)).toEqual({ columns: 1, tileWidth: 1000 });
    expect(gridShapeFor(4, 1000, 600, 8).columns).toBe(2);
    expect(gridShapeFor(3, 1600, 400, 8).columns).toBe(3);
    // A phone held upright stacks them.
    expect(gridShapeFor(3, 360, 700, 8).columns).toBe(1);
    expect(gridShapeFor(9, 1200, 700, 8).columns).toBe(3);
  });

  it("never gives a negative size", () => {
    expect(gridShapeFor(9, 0, 0, 8).tileWidth).toBe(0);
  });
});

describe("nextSpeaker", () => {
  const present = ["1", "2", "3"];
  it("follows whoever else talks, stays on them while they talk, and keeps them through silences", () => {
    expect(nextSpeaker(null, new Set(["1"]), "1", present)).toBeNull();
    expect(nextSpeaker(null, new Set(["2"]), "1", present)).toBe("2");
    expect(nextSpeaker("2", new Set(["2", "3"]), "1", present)).toBe("2");
    expect(nextSpeaker("2", new Set(["3"]), "1", present)).toBe("3");
    expect(nextSpeaker("3", new Set(), "1", present)).toBe("3");
  });

  it("forgets someone who left", () => {
    expect(nextSpeaker("4", new Set(), "1", present)).toBeNull();
  });
});

describe("featuredPersonOf", () => {
  const people = [person("1", "camera"), person("2"), person("3", "camera")];
  it("prefers the one picked, then the last speaker, then someone else's picture", () => {
    expect(featuredPersonOf(people, "2", "3", "1")).toBe("2");
    expect(featuredPersonOf(people, null, "2", "1")).toBe("2");
    expect(featuredPersonOf(people, "9", null, "1")).toBe("3");
    expect(featuredPersonOf([person("1")], null, null, "1")).toBe("1");
    expect(featuredPersonOf([], null, null, "1")).toBeNull();
  });
});
