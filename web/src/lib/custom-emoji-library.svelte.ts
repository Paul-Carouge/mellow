// The servers' custom emoji (ADR-0028): every picture this user may load, and each server's list
// edits. The lists live in the guild snapshots `MellowClient` owns, so this slice reads and
// changes them through ports.

import type { RestClient } from "./api";
import type { CustomEmoji, Guild, Snowflake } from "./protocol";

/** What the custom emoji read from, and change in, the rest of the client. */
export interface CustomEmojiPorts {
  readonly rest: Pick<RestClient, "createEmoji" | "renameEmoji" | "deleteEmoji">;
  /** Reactive: `known` follows it. */
  guilds(): readonly Guild[];
  updateGuild(guildId: Snowflake, change: (guild: Guild) => Guild): void;
}

export class CustomEmojiLibrary {
  /**
   * Every custom emoji of the user's guilds by id: the pictures this user may load, so the ones a
   * message or reaction can show. Others read as their name.
   */
  readonly known: ReadonlyMap<Snowflake, CustomEmoji> = $derived.by(() => {
    const lists = this.ports.guilds().map((guild) => guild.emojis);
    // Channel and role events replace the guild but keep its list: the same map then spares every
    // message row a new parse.
    const isUnchanged =
      lists.length === this.knownLists.length && lists.every((list, index) => list === this.knownLists[index]);
    if (!isUnchanged) {
      this.knownLists = lists;
      this.knownMap = new Map(lists.flatMap((list) => (list ?? []).map((emoji) => [emoji.id, emoji] as const)));
    }
    return this.knownMap;
  });
  /** The lists `known` was last built from; never reactive. */
  private knownLists: readonly (CustomEmoji[] | undefined)[] = [];
  private knownMap: ReadonlyMap<Snowflake, CustomEmoji> = new Map();

  constructor(private readonly ports: CustomEmojiPorts) {}

  /** The guild's custom emoji, oldest first; none from an older instance. */
  emojisOf(guildId: Snowflake): CustomEmoji[] {
    return this.ports.guilds().find((guild) => guild.id === guildId)?.emojis ?? [];
  }

  private applyEmojis(guildId: Snowflake, emojis: CustomEmoji[]): void {
    this.ports.updateGuild(guildId, (guild) => ({ ...guild, emojis }));
  }

  /** Processed by the instance first: an animated GIF takes a second or two. */
  async createEmoji(guildId: Snowflake, name: string, file: File): Promise<CustomEmoji> {
    const created = await this.ports.rest.createEmoji(guildId, name, file);
    const emojis = this.emojisOf(guildId);
    // The gateway's list may have come first.
    if (!emojis.some((emoji) => emoji.id === created.id)) {
      this.applyEmojis(guildId, [...emojis, created]);
    }
    return created;
  }

  /**
   * The new name shows with `GUILD_EMOJIS_UPDATE`, not with the answer: an answer arriving after
   * a later rename's event would put the older name back.
   */
  async renameEmoji(guildId: Snowflake, emojiId: Snowflake, name: string): Promise<void> {
    await this.ports.rest.renameEmoji(guildId, emojiId, name);
  }

  async deleteEmoji(guildId: Snowflake, emojiId: Snowflake): Promise<void> {
    await this.ports.rest.deleteEmoji(guildId, emojiId);
    this.applyEmojis(
      guildId,
      this.emojisOf(guildId).filter((emoji) => emoji.id !== emojiId),
    );
  }
}
