import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ChannelSection } from "../lib/channels";
import type { Channel, ChannelPlacement, Guild, Snowflake } from "../lib/protocol";
import { ChannelReorder, type ChannelReorderClient, PENDING_ORDER_ECHO_TIMEOUT_MILLISECONDS } from "./channel-reorder.svelte";

// Tests compile Svelte for the server, where the class's effect never runs and its deriveds are
// read afresh each time: these check the order shown, not when a settled pending one is forgotten.

function channel(id: string, kind: "text" | "category", position: number, parentId: string | null = null): Channel {
  return { id, guild_id: "g", kind, name: id, topic: "", parent_id: parentId, position, permission_overwrites: [] };
}

// lobby (uncategorised) · text: general, links · games: clips
const CHANNELS = [
  channel("lobby", "text", 0),
  channel("text", "category", 1),
  channel("general", "text", 2, "text"),
  channel("links", "text", 3, "text"),
  channel("games", "category", 4),
  channel("clips", "text", 5, "games"),
];
const ORIGINAL_PLACEMENTS: readonly ChannelPlacement[] = [
  { id: "lobby", parent_id: null },
  { id: "text", parent_id: null },
  { id: "general", parent_id: "text" },
  { id: "links", parent_id: "text" },
  { id: "games", parent_id: null },
  { id: "clips", parent_id: "games" },
];
const ORIGINAL_SHAPE = [
  [null, ["lobby"]],
  ["text", ["general", "links"]],
  ["games", ["clips"]],
];
const GENERAL_DOWN_SHAPE = [
  [null, ["lobby"]],
  ["text", ["links", "general"]],
  ["games", ["clips"]],
];
const UNKNOWN_REASON = "something went wrong. Try again.";

function byId(id: string): Channel {
  const found = CHANNELS.find((candidate) => candidate.id === id);
  if (found === undefined) {
    throw new Error(`no channel ${id} in the fixture`);
  }
  return found;
}

/** Each section as [category or null, channel ids]. */
function shape(sections: readonly ChannelSection[]) {
  return sections.map((section) => [section.category?.id ?? null, section.channels.map((item) => item.id)]);
}

/** A fake instance whose answers the test releases one by one, and whose snapshots it sends. */
class FakeInstance implements ChannelReorderClient {
  guilds: Guild[] = [
    { id: "g", name: "Guild", owner_id: "1", my_permissions: "0", my_role_ids: [], roles: [], channels: CHANNELS },
  ];
  readonly sentOrders: (readonly ChannelPlacement[])[] = [];
  readonly #answers: { resolve: () => void; reject: (error: unknown) => void }[] = [];

  get selectedGuild(): Guild | null {
    return this.guilds[0] ?? null;
  }

  reorderChannels(_guildId: Snowflake, placements: readonly ChannelPlacement[]): Promise<void> {
    this.sentOrders.push(placements);
    return new Promise((resolve, reject) => this.#answers.push({ resolve, reject }));
  }

  agree() {
    this.#answers.shift()?.resolve();
  }

  refuse() {
    this.#answers.shift()?.reject(new Error("refused"));
  }

  /** The gateway's `GUILD_CREATE`: the guild's channels replaced, in the order given. */
  sendSnapshot(placements: readonly ChannelPlacement[]) {
    const channels = placements.map((placement, position) => ({ ...byId(placement.id), parent_id: placement.parent_id, position }));
    this.guilds = this.guilds.map((guild) => ({ ...guild, channels }));
  }
}

function setUp() {
  const instance = new FakeInstance();
  const announcements: string[] = [];
  const reorder = new ChannelReorder(() => instance, (sentence) => announcements.push(sentence));
  return { instance, reorder, announcements };
}

describe("ChannelReorder", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("shows a menu move at once and keeps it until the instance's snapshot echoes it", async () => {
    const { instance, reorder } = setUp();
    const outcome = reorder.moveChannelStep(byId("general"), 1);
    expect(shape(reorder.sections)).toEqual(GENERAL_DOWN_SHAPE);
    expect(reorder.isReordering).toBe(true);
    instance.agree();
    expect(await outcome).toEqual({ announcement: "Moved #general down." });
    expect(reorder.isReordering).toBe(false);
    expect(shape(reorder.sections)).toEqual(GENERAL_DOWN_SHAPE);
    instance.sendSnapshot(instance.sentOrders[0] ?? []);
    expect(shape(reorder.sections)).toEqual(GENERAL_DOWN_SHAPE);
  });

  it("lets the instance's order win when its snapshot differs from the move it agreed to", async () => {
    const { instance, reorder } = setUp();
    const outcome = reorder.moveChannelStep(byId("general"), 1);
    instance.agree();
    await outcome;
    instance.sendSnapshot(ORIGINAL_PLACEMENTS);
    expect(shape(reorder.sections)).toEqual(ORIGINAL_SHAPE);
  });

  it("goes back to the instance's order when the echo never comes", async () => {
    const { instance, reorder } = setUp();
    const outcome = reorder.moveChannelStep(byId("general"), 1);
    instance.agree();
    await outcome;
    await vi.advanceTimersByTimeAsync(PENDING_ORDER_ECHO_TIMEOUT_MILLISECONDS - 1);
    expect(shape(reorder.sections)).toEqual(GENERAL_DOWN_SHAPE);
    await vi.advanceTimersByTimeAsync(1);
    expect(shape(reorder.sections)).toEqual(ORIGINAL_SHAPE);
  });

  it("puts the old order back and says why in the menu when a move is refused", async () => {
    const { instance, reorder } = setUp();
    const outcome = reorder.moveCategoryStep(byId("games"), -1);
    expect(shape(reorder.sections)).toEqual([
      [null, ["lobby"]],
      ["games", ["clips"]],
      ["text", ["general", "links"]],
    ]);
    instance.refuse();
    expect(await outcome).toEqual({ error: `Couldn't move the category: ${UNKNOWN_REASON}` });
    expect(shape(reorder.sections)).toEqual(ORIGINAL_SHAPE);
    expect(reorder.error).toBe("");
  });

  it("sends nothing for a move past an edge", async () => {
    const { instance, reorder } = setUp();
    expect(await reorder.moveChannelStep(byId("links"), 1)).toEqual({ announcement: "" });
    expect(await reorder.moveCategoryStep(byId("text"), -1)).toEqual({ announcement: "" });
    expect(instance.sentOrders).toEqual([]);
  });

  it("announces a dropped channel by its new category", async () => {
    const { instance, reorder, announcements } = setUp();
    const dropped = reorder.dropChannel("clips", { categoryId: "text", index: 0 });
    expect(shape(reorder.sections)).toEqual([
      [null, ["lobby"]],
      ["text", ["clips", "general", "links"]],
      ["games", []],
    ]);
    instance.agree();
    await dropped;
    expect(announcements).toEqual(["Moved #clips to text."]);
  });

  it("keeps a refused drop's reason until the next move", async () => {
    const { instance, reorder, announcements } = setUp();
    const dropped = reorder.dropCategory(byId("games"), 0);
    instance.refuse();
    await dropped;
    expect(reorder.error).toBe(`Couldn't move the category: ${UNKNOWN_REASON}`);
    expect(announcements).toEqual([]);
    expect(shape(reorder.sections)).toEqual(ORIGINAL_SHAPE);
    const moved = reorder.dropChannel("general", { categoryId: null, index: 1 });
    expect(reorder.error).toBe("");
    instance.agree();
    await moved;
    expect(announcements).toEqual(["Moved #general out of any category."]);
  });

  it("moves a channel to the end of the category chosen and answers the dialog", async () => {
    const { instance, reorder, announcements } = setUp();
    const moved = reorder.moveChannelTo(byId("lobby"), "games");
    instance.agree();
    expect(await moved).toBeNull();
    expect(shape(reorder.sections)).toEqual([
      ["text", ["general", "links"]],
      ["games", ["clips", "lobby"]],
    ]);
    expect(announcements).toEqual(["Moved #lobby to games."]);
    const refused = reorder.moveChannelTo(byId("general"), null);
    instance.refuse();
    expect(await refused).toBe(`Couldn't move the channel: ${UNKNOWN_REASON}`);
  });
});
