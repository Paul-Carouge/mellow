// The quick switcher's results (DESIGN.md §5.10, ADR-0022 §1): channels, people and servers whose
// name contains what was typed; "#", "@" and "*" narrow to one kind, grouped as in the mockup.

import type { IconName } from "./Icon.svelte";
import type { DirectMessageChannel, Guild, PublicUser, Snowflake } from "../lib/protocol";
import { channelSections } from "../lib/channels";
import { displayName } from "./format";

export type QuickSwitcherTarget =
  | { kind: "channel"; guildId: Snowflake; channelId: Snowflake }
  | { kind: "person"; userId: Snowflake; conversationId: Snowflake | null }
  | { kind: "server"; guildId: Snowflake };

export interface QuickSwitcherItem {
  key: string;
  label: string;
  /** Where it leads: the server's name, "Direct message" or "Server". */
  where: string;
  icon: IconName;
  hasUnread: boolean;
  target: QuickSwitcherTarget;
}

export interface QuickSwitcherGroup {
  label: string;
  items: QuickSwitcherItem[];
}

export interface QuickSwitcherSources {
  guilds: readonly Guild[];
  conversations: readonly DirectMessageChannel[];
  /** Members of the servers loaded so far, for people without a conversation yet. */
  knownPeople: readonly PublicUser[];
  currentUserId: Snowflake | null;
  hasUnread: (channelId: Snowflake) => boolean;
  canSendIn: (channelId: Snowflake) => boolean;
}

function kindOfPrefix(prefix: string): "channels" | "people" | "servers" | null {
  return prefix === "#" ? "channels" : prefix === "@" ? "people" : prefix === "*" ? "servers" : null;
}

export function quickSwitcherResults(query: string, sources: QuickSwitcherSources): QuickSwitcherGroup[] {
  const trimmed = query.trim().toLowerCase();
  const kind = kindOfPrefix(trimmed.charAt(0));
  const term = kind ? trimmed.slice(1).trim() : trimmed;
  const matches = (name: string) => name.toLowerCase().includes(term);

  const channels: QuickSwitcherItem[] = sources.guilds.flatMap((guild) =>
    channelSections(guild)
      .flatMap((section) => section.channels)
      .filter((channel) => matches(channel.name))
      .map((channel) => ({
        key: `channel:${channel.id}`,
        label: channel.name,
        where: guild.name,
        icon: sources.canSendIn(channel.id) ? "hash" : "lock",
        hasUnread: sources.hasUnread(channel.id),
        target: { kind: "channel", guildId: guild.id, channelId: channel.id },
      })),
  );

  const conversationByUserId = new Map<Snowflake, DirectMessageChannel>();
  const people = new Map<Snowflake, PublicUser>();
  for (const conversation of sources.conversations) {
    for (const recipient of conversation.recipients) {
      conversationByUserId.set(recipient.id, conversation);
      people.set(recipient.id, recipient);
    }
  }
  for (const person of sources.knownPeople) {
    if (!people.has(person.id)) {
      people.set(person.id, person);
    }
  }
  const peopleItems: QuickSwitcherItem[] = [...people.values()]
    .filter((person) => person.id !== sources.currentUserId)
    .filter((person) => matches(displayName(person)) || matches(person.username))
    .sort((left, right) => displayName(left).localeCompare(displayName(right)))
    .map((person) => {
      const conversation = conversationByUserId.get(person.id) ?? null;
      return {
        key: `person:${person.id}`,
        label: displayName(person),
        where: "Direct message",
        icon: "at",
        hasUnread: conversation !== null && sources.hasUnread(conversation.id),
        target: { kind: "person", userId: person.id, conversationId: conversation?.id ?? null },
      };
    });

  const servers: QuickSwitcherItem[] = sources.guilds
    .filter((guild) => matches(guild.name))
    .map((guild) => ({
      key: `server:${guild.id}`,
      label: guild.name,
      where: "Server",
      icon: "users",
      hasUnread: guild.channels.some((channel) => sources.hasUnread(channel.id)),
      target: { kind: "server", guildId: guild.id },
    }));

  return [
    { label: "Channels", items: !kind || kind === "channels" ? channels : [] },
    { label: "People", items: !kind || kind === "people" ? peopleItems : [] },
    { label: "Servers", items: !kind || kind === "servers" ? servers : [] },
  ].filter((group) => group.items.length > 0);
}
