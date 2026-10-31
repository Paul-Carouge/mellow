// How a guild's channels are listed: Discord's order, as the desktop client applies it.

import { type Channel, type Guild, compareSnowflakes } from "./protocol";

export interface ChannelSection {
  /** `null` for the channels outside any category, listed first. */
  category: Channel | null;
  channels: Channel[];
}

/**
 * Uncategorised channels, then each category with its channels, text and voice mixed by position
 * (the managers choose the order). A channel in a category hidden from the reader is listed with
 * the uncategorised ones: it is visible, and its heading is not. It keeps its `parent_id`, so a reorder sends that category back unchanged.
 */
export function channelSections(guild: Pick<Guild, "channels">): ChannelSection[] {
  const byPosition = (left: Channel, right: Channel) =>
    left.position - right.position || compareSnowflakes(left.id, right.id);
  const categories = guild.channels.filter((channel) => channel.kind === "category").sort(byPosition);
  const categoryIds = new Set(categories.map((category) => category.id));
  const listedChannels = guild.channels.filter((channel) => channel.kind !== "category").sort(byPosition);
  const uncategorised = listedChannels.filter(
    (channel) => channel.parent_id === null || !categoryIds.has(channel.parent_id),
  );
  return [
    ...(uncategorised.length > 0 ? [{ category: null, channels: uncategorised }] : []),
    ...categories.map((category) => ({
      category,
      channels: listedChannels.filter((channel) => channel.parent_id === category.id),
    })),
  ];
}
