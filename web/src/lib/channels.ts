// How a guild's channels are listed: Discord's order, as the desktop client applies it.

import { type Channel, type Guild, type Snowflake, compareSnowflakes } from "./protocol";

export interface ChannelSection {
  /** `null` for the channels outside any category, listed first. */
  category: Channel | null;
  channels: Channel[];
}

/** Uncategorised text channels, then each category with its text channels, by position. */
export function channelSections(guild: Guild): ChannelSection[] {
  const byPosition = (left: Channel, right: Channel) =>
    left.position - right.position || compareSnowflakes(left.id, right.id);
  const textIn = (parentId: Snowflake | null) =>
    guild.channels.filter((channel) => channel.kind === "text" && channel.parent_id === parentId).sort(byPosition);
  const categories = guild.channels.filter((channel) => channel.kind === "category").sort(byPosition);
  const uncategorised = textIn(null);
  return [
    ...(uncategorised.length > 0 ? [{ category: null, channels: uncategorised }] : []),
    ...categories.map((category) => ({ category, channels: textIn(category.id) })),
  ];
}
