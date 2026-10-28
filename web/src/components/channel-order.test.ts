import { describe, expect, it } from "vitest";

import { type ChannelSection, channelSections } from "../lib/channels";
import type { Channel } from "../lib/protocol";
import {
  type ListedBox,
  categoryDropAt,
  categoryStepIndex,
  channelDropAt,
  channelStepDestination,
  isPendingOrderSettled,
  isSameOrder,
  placementsOf,
  sectionsFromPlacements,
  withCategoryMoved,
  withChannelMoved,
} from "./channel-order";

function channel(id: string, kind: "text" | "category", position: number, parentId: string | null = null): Channel {
  return { id, guild_id: "g", kind, name: id, topic: "", parent_id: parentId, position, permission_overwrites: [] };
}

// lobby (uncategorised) · Text: general, links · Games: clips · Empty: (none)
const CHANNELS = [
  channel("lobby", "text", 0),
  channel("text", "category", 1),
  channel("general", "text", 2, "text"),
  channel("links", "text", 3, "text"),
  channel("games", "category", 4),
  channel("clips", "text", 5, "games"),
  channel("empty", "category", 6),
];
const SECTIONS = channelSections({ channels: CHANNELS });

/** Each section as [category or null, channel ids]. */
function shape(sections: readonly ChannelSection[] | null) {
  return sections?.map((section) => [section.category?.id ?? null, section.channels.map((item) => item.id)]) ?? null;
}

describe("placementsOf", () => {
  it("lists every channel once, each category before its channels", () => {
    expect(placementsOf(SECTIONS)).toEqual([
      { id: "lobby", parent_id: null },
      { id: "text", parent_id: null },
      { id: "general", parent_id: "text" },
      { id: "links", parent_id: "text" },
      { id: "games", parent_id: null },
      { id: "clips", parent_id: "games" },
      { id: "empty", parent_id: null },
    ]);
  });

  it("sends a hidden category back unchanged for a channel listed without it", () => {
    const sections = channelSections({ channels: [channel("secret", "text", 0, "hidden"), channel("open", "text", 1)] });
    expect(placementsOf(sections)).toEqual([
      { id: "secret", parent_id: "hidden" },
      { id: "open", parent_id: null },
    ]);
  });
});

describe("withChannelMoved", () => {
  it("moves down within a section, counting the moved row as listed", () => {
    expect(shape(withChannelMoved(SECTIONS, "general", { categoryId: "text", index: 2 }))).toEqual([
      [null, ["lobby"]],
      ["text", ["links", "general"]],
      ["games", ["clips"]],
      ["empty", []],
    ]);
  });

  it("moves into another category, an empty one, and out of any", () => {
    expect(shape(withChannelMoved(SECTIONS, "general", { categoryId: "games", index: 0 }))?.[2]).toEqual([
      "games",
      ["general", "clips"],
    ]);
    expect(shape(withChannelMoved(SECTIONS, "clips", { categoryId: "empty", index: 0 }))?.[3]).toEqual(["empty", ["clips"]]);
    const out = withChannelMoved(SECTIONS, "clips", { categoryId: null, index: 1 });
    expect(shape(out)?.[0]).toEqual([null, ["lobby", "clips"]]);
    expect(placementsOf(out ?? [])).toContainEqual({ id: "clips", parent_id: null });
  });

  it("creates the uncategorised rows when there were none, and drops them once empty", () => {
    const withoutLobby = channelSections({ channels: CHANNELS.filter((item) => item.id !== "lobby") });
    expect(shape(withChannelMoved(withoutLobby, "clips", { categoryId: null, index: 0 }))?.[0]).toEqual([null, ["clips"]]);
    expect(shape(withChannelMoved(SECTIONS, "lobby", { categoryId: "games", index: 1 }))?.[0]).toEqual(["text", ["general", "links"]]);
  });

  it("returns null when nothing changes or the channel or category is gone", () => {
    expect(withChannelMoved(SECTIONS, "general", { categoryId: "text", index: 0 })).toBeNull();
    expect(withChannelMoved(SECTIONS, "general", { categoryId: "text", index: 1 })).toBeNull();
    expect(withChannelMoved(SECTIONS, "missing", { categoryId: "text", index: 0 })).toBeNull();
    expect(withChannelMoved(SECTIONS, "general", { categoryId: "missing", index: 0 })).toBeNull();
  });

  it("keeps a hidden category's channel in it while moving it among the uncategorised rows", () => {
    const sections = channelSections({ channels: [channel("open", "text", 0), channel("secret", "text", 1, "hidden")] });
    const moved = withChannelMoved(sections, "secret", { categoryId: null, index: 0 });
    expect(placementsOf(moved ?? [])).toEqual([
      { id: "secret", parent_id: "hidden" },
      { id: "open", parent_id: null },
    ]);
  });
});

describe("withCategoryMoved", () => {
  it("moves a category with its channels, uncategorised rows staying first", () => {
    expect(shape(withCategoryMoved(SECTIONS, "games", 0))).toEqual([
      [null, ["lobby"]],
      ["games", ["clips"]],
      ["text", ["general", "links"]],
      ["empty", []],
    ]);
    expect(shape(withCategoryMoved(SECTIONS, "text", 3))?.map(([id]) => id)).toEqual([null, "games", "empty", "text"]);
  });

  it("returns null when nothing changes", () => {
    expect(withCategoryMoved(SECTIONS, "games", 1)).toBeNull();
    expect(withCategoryMoved(SECTIONS, "games", 2)).toBeNull();
    expect(withCategoryMoved(SECTIONS, "missing", 0)).toBeNull();
  });
});

describe("Move up and Move down", () => {
  it("steps a channel within its section, and offers nothing at an edge", () => {
    expect(channelStepDestination(SECTIONS, "general", -1)).toBeNull();
    expect(channelStepDestination(SECTIONS, "links", 1)).toBeNull();
    const down = channelStepDestination(SECTIONS, "general", 1);
    expect(shape(withChannelMoved(SECTIONS, "general", down ?? { categoryId: null, index: 0 }))?.[1]).toEqual([
      "text",
      ["links", "general"],
    ]);
    const up = channelStepDestination(SECTIONS, "links", -1);
    expect(shape(withChannelMoved(SECTIONS, "links", up ?? { categoryId: null, index: 0 }))?.[1]).toEqual([
      "text",
      ["links", "general"],
    ]);
  });

  it("steps a category among the categories", () => {
    expect(categoryStepIndex(SECTIONS, "text", -1)).toBeNull();
    expect(categoryStepIndex(SECTIONS, "empty", 1)).toBeNull();
    expect(shape(withCategoryMoved(SECTIONS, "text", categoryStepIndex(SECTIONS, "text", 1) ?? 0))?.map(([id]) => id)).toEqual([
      null,
      "games",
      "text",
      "empty",
    ]);
    expect(shape(withCategoryMoved(SECTIONS, "empty", categoryStepIndex(SECTIONS, "empty", -1) ?? 0))?.map(([id]) => id)).toEqual([
      null,
      "text",
      "empty",
      "games",
    ]);
  });
});

describe("sectionsFromPlacements", () => {
  it("shows a pending order with the channels as they are now", () => {
    const moved = withChannelMoved(SECTIONS, "clips", { categoryId: "text", index: 0 });
    const renamed = CHANNELS.map((item) => (item.id === "clips" ? { ...item, name: "Clips" } : item));
    const shown = sectionsFromPlacements(renamed, placementsOf(moved ?? []));
    expect(shape(shown)).toEqual(shape(moved));
    expect(shown?.[1]?.channels[0]?.name).toBe("Clips");
    expect(shown?.[1]?.channels[0]?.parent_id).toBe("text");
  });

  it("gives way once a channel was created or deleted meanwhile", () => {
    const placements = placementsOf(SECTIONS);
    expect(sectionsFromPlacements(CHANNELS.slice(1), placements)).toBeNull();
    expect(sectionsFromPlacements([...CHANNELS, channel("new", "text", 9)], placements)).toBeNull();
  });

  it("round-trips the instance's order", () => {
    expect(isSameOrder(placementsOf(sectionsFromPlacements(CHANNELS, placementsOf(SECTIONS)) ?? []), placementsOf(SECTIONS))).toBe(true);
  });
});

describe("isPendingOrderSettled", () => {
  const before = placementsOf(SECTIONS);
  const pending = [...before].reverse().map((placement) => ({ ...placement, parent_id: null }));
  const otherChannels = [...CHANNELS];

  it("settles when the instance's snapshot matches, even before the answer", () => {
    expect(isPendingOrderSettled(pending, null, pending, CHANNELS)).toBe(true);
  });

  it("keeps showing until an answered order is echoed or the channels are replaced", () => {
    expect(isPendingOrderSettled(pending, null, before, otherChannels)).toBe(false);
    expect(isPendingOrderSettled(pending, CHANNELS, before, CHANNELS)).toBe(false);
  });

  it("gives way to a different order from the instance once it answered", () => {
    expect(isPendingOrderSettled(pending, CHANNELS, before, otherChannels)).toBe(true);
  });
});

describe("dropping", () => {
  // Rows 32 px tall, headers 24 px, top to bottom as the sidebar lays them out.
  const BOXES: ListedBox[] = [
    { kind: "channel", id: "lobby", top: 0, bottom: 32 },
    { kind: "category", id: "text", top: 48, bottom: 72 },
    { kind: "channel", id: "general", top: 72, bottom: 104 },
    { kind: "channel", id: "links", top: 106, bottom: 138 },
    { kind: "category", id: "games", top: 154, bottom: 178 },
    { kind: "channel", id: "clips", top: 178, bottom: 210 },
    { kind: "category", id: "empty", top: 226, bottom: 250 },
  ];

  it("puts a channel at the nearest gap, with a line there", () => {
    expect(channelDropAt(SECTIONS, BOXES, "general", 140, true)).toEqual({
      destination: { categoryId: "text", index: 2 },
      indicator: { kind: "line", y: 138 },
    });
    expect(channelDropAt(SECTIONS, BOXES, "general", 205, true)).toEqual({
      destination: { categoryId: "games", index: 1 },
      indicator: { kind: "line", y: 210 },
    });
  });

  it("takes a channel into a category from its header", () => {
    expect(channelDropAt(SECTIONS, BOXES, "clips", 60, true)).toEqual({
      destination: { categoryId: "text", index: 0 },
      indicator: { kind: "category", id: "text" },
    });
    expect(channelDropAt(SECTIONS, BOXES, "clips", 240, true)?.destination).toEqual({ categoryId: "empty", index: 0 });
  });

  it("shows nothing where the channel already is", () => {
    expect(channelDropAt(SECTIONS, BOXES, "general", 80, true)).toBeNull();
    expect(channelDropAt(SECTIONS, BOXES, "general", 104, true)).toBeNull();
  });

  it("keeps a channel in its section without the permission to change its category", () => {
    expect(channelDropAt(SECTIONS, BOXES, "general", 205, false)?.destination).toEqual({ categoryId: "text", index: 2 });
    expect(channelDropAt(SECTIONS, BOXES, "general", 166, false)?.destination).toEqual({ categoryId: "text", index: 2 });
  });

  it("offers the top of the list as uncategorised when no channel is", () => {
    const sections = channelSections({ channels: CHANNELS.filter((item) => item.id !== "lobby") });
    const boxes = BOXES.slice(1);
    expect(channelDropAt(sections, boxes, "clips", 40, true)).toEqual({
      destination: { categoryId: null, index: 0 },
      indicator: { kind: "line", y: 48 },
    });
  });

  it("puts a category before the nearest header or after the last row", () => {
    expect(categoryDropAt(SECTIONS, BOXES, "games", 50)).toEqual({ index: 0, y: 48 });
    expect(categoryDropAt(SECTIONS, BOXES, "games", 260)).toEqual({ index: 3, y: 250 });
    expect(categoryDropAt(SECTIONS, BOXES, "games", 160)).toBeNull();
  });
});
