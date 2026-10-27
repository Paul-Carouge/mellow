// The quick switcher's results (DESIGN.md §5.10, ADR-0022 §1): channels, people and servers whose
// name contains what was typed; "#", "@" and "*" narrow to one kind, grouped as in the mockup.

import type { IconName } from "./Icon.svelte";
import type { DirectMessageChannel, Guild, PublicUser, Snowflake } from "../lib/protocol";
import { strings } from "../i18n";
import { channelSections } from "../lib/channels";
import { displayName } from "./format";
import { conversationDisplayName, isGroupConversation, otherParticipants } from "./group-conversations";

export type QuickSwitcherTarget =
  | { kind: "channel"; guildId: Snowflake; channelId: Snowflake }
  | { kind: "person"; userId: Snowflake; conversationId: Snowflake | null }
  | { kind: "server"; guildId: Snowflake }
  | { kind: "conversation"; conversationId: Snowflake };

export interface QuickSwitcherItem {
  key: string;
  label: string;
  /** Where it leads: the server's name, "Direct message", "Group" or "Server". */
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
  /** Friends can be messaged even when they share no loaded server (ADR-0025). */
  isFriend: (userId: Snowflake) => boolean;
  hasUnread: (channelId: Snowflake) => boolean;
  canSendIn: (channelId: Snowflake) => boolean;
}

const MAXIMUM_ITEMS_PER_KIND_BEFORE_TYPING = 20;

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
    const isGroup = isGroupConversation(conversation);
    for (const recipient of conversation.recipients) {
      // A person leads to their one-to-one conversation, never to a group they are in. Someone met
      // only in a group is listed when the instance would open a conversation with them: a friend
      // here, or a member of a loaded server below.
      if (!isGroup) {
        conversationByUserId.set(recipient.id, conversation);
        people.set(recipient.id, recipient);
      } else if (sources.isFriend(recipient.id)) {
        people.set(recipient.id, recipient);
      }
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
        where: strings.quickSwitcher.directMessage,
        icon: "at",
        hasUnread: conversation !== null && sources.hasUnread(conversation.id),
        target: { kind: "person", userId: person.id, conversationId: conversation?.id ?? null },
      };
    });
  // Groups by their name or by anyone in them (ADR-0026), after the people.
  const groupItems: QuickSwitcherItem[] = sources.conversations
    .filter(isGroupConversation)
    .map((conversation) => ({ conversation, label: conversationDisplayName(conversation, sources.currentUserId) }))
    .filter(
      ({ conversation, label }) =>
        matches(label) ||
        otherParticipants(conversation, sources.currentUserId).some(
          (person) => matches(displayName(person)) || matches(person.username),
        ),
    )
    .map(({ conversation, label }) => ({
      key: `group:${conversation.id}`,
      label,
      where: strings.groups.group,
      icon: "users",
      hasUnread: sources.hasUnread(conversation.id),
      target: { kind: "conversation", conversationId: conversation.id },
    }));

  const servers: QuickSwitcherItem[] = sources.guilds
    .filter((guild) => matches(guild.name))
    .map((guild) => ({
      key: `server:${guild.id}`,
      label: guild.name,
      where: strings.quickSwitcher.server,
      icon: "users",
      hasUnread: guild.channels.some((channel) => sources.hasUnread(channel.id)),
      target: { kind: "server", guildId: guild.id },
    }));

  // Before anything is typed, a short list per kind: a large instance would otherwise render
  // every channel and member at once.
  const shown = (items: QuickSwitcherItem[]) => (term === "" ? items.slice(0, MAXIMUM_ITEMS_PER_KIND_BEFORE_TYPING) : items);
  return [
    { label: strings.quickSwitcher.channels, items: !kind || kind === "channels" ? shown(channels) : [] },
    { label: strings.quickSwitcher.people, items: !kind || kind === "people" ? shown([...peopleItems, ...groupItems]) : [] },
    { label: strings.quickSwitcher.servers, items: !kind || kind === "servers" ? shown(servers) : [] },
  ].filter((group) => group.items.length > 0);
}
