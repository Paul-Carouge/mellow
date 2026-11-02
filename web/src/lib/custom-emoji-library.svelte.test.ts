import { describe, expect, it } from "vitest";

import type { CustomEmoji, Guild } from "./protocol";
import { type CustomEmojiPorts, CustomEmojiLibrary } from "./custom-emoji-library.svelte";

const CLUB = "100";
const BAND = "200";
const WAVE: CustomEmoji = { id: "700", name: "wave", is_animated: false };
const PARTY: CustomEmoji = { id: "701", name: "party", is_animated: true };
const TADA: CustomEmoji = { id: "702", name: "tada", is_animated: false };

function guild(id: string, emojis: CustomEmoji[] | undefined): Guild {
  return { id, name: `g${id}`, owner_id: "2", my_permissions: "0", my_role_ids: [], roles: [], channels: [], emojis };
}

/** A library over reactive guilds, as `MellowClient` holds them. */
function library(rest: Partial<CustomEmojiPorts["rest"]> = {}) {
  const world = $state({ guilds: [guild(CLUB, [WAVE]), guild(BAND, undefined)] });
  const emojis = new CustomEmojiLibrary({
    rest: {
      createEmoji: () => Promise.reject(new Error("unexpected createEmoji")),
      renameEmoji: () => Promise.reject(new Error("unexpected renameEmoji")),
      deleteEmoji: () => Promise.resolve(),
      ...rest,
    },
    guilds: () => world.guilds,
    updateGuild: (guildId, change) => {
      world.guilds = world.guilds.map((candidate) => (candidate.id === guildId ? change(candidate) : candidate));
    },
  });
  return { emojis, world };
}

describe("CustomEmojiLibrary", () => {
  it("knows every server's emoji, and follows their lists", () => {
    const { emojis, world } = library();
    expect([...emojis.known.keys()]).toEqual([WAVE.id]);
    world.guilds = [world.guilds[0]!, guild(BAND, [PARTY])];
    expect([...emojis.known.keys()]).toEqual([WAVE.id, PARTY.id]);
    expect(emojis.emojisOf(BAND)).toEqual([PARTY]);
    expect(emojis.emojisOf("999")).toEqual([]);
  });

  it("keeps the same map when a server changes but not its list", () => {
    const { emojis, world } = library();
    const before = emojis.known;
    world.guilds = world.guilds.map((candidate) => ({ ...candidate, name: "renamed" }));
    expect(emojis.known).toBe(before);
  });

  it("adds a created emoji once, even after the gateway's list", async () => {
    const { emojis, world } = library({ createEmoji: () => Promise.resolve(TADA) });
    await emojis.createEmoji(CLUB, "tada", new File([], "tada.png"));
    expect(emojis.emojisOf(CLUB)).toEqual([WAVE, TADA]);
    world.guilds = [guild(CLUB, [WAVE, TADA]), world.guilds[1]!];
    await emojis.createEmoji(CLUB, "tada", new File([], "tada.png"));
    expect(emojis.emojisOf(CLUB)).toEqual([WAVE, TADA]);
  });

  it("drops a deleted emoji at once", async () => {
    const { emojis } = library();
    await emojis.deleteEmoji(CLUB, WAVE.id);
    expect(emojis.emojisOf(CLUB)).toEqual([]);
    expect(emojis.known.size).toBe(0);
  });
});
