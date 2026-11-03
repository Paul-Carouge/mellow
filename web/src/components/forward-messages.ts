// Forwarding a message (DESIGN.md §5.4, ADR-0039): the places the forward dialog offers, its
// search, and how a forward names the place it came from.

import { strings } from "../i18n";
import { ApiError } from "../lib/api";
import { channelSections } from "../lib/channels";
import { describeError, errorReason } from "../lib/client.svelte";
import {
  type DirectMessageChannel,
  type Guild,
  Permission,
  type PublicUser,
  type Snowflake,
  computeChannelPermissions,
  hasPermission,
} from "../lib/protocol";
import { foldForSearch } from "../lib/search-text";
import { displayName } from "./format";
import { blockedRecipientOf, conversationDisplayName, isGroupConversation, otherParticipants } from "./group-conversations";

/** As on Discord: one forward reaches at most this many places. */
export const MAXIMUM_FORWARD_DESTINATIONS = 5;

export interface ForwardDestination {
  /** The channel the forward is posted in. */
  readonly channelId: Snowflake;
  readonly kind: "channel" | "person" | "group";
  /** A channel's name (shown after its `#` glyph), the person, or the group's name. */
  readonly name: string;
  /** The second line: the server's name, "@ana", or "Groupe". */
  readonly where: string;
  /** A conversation's other people, for its avatar; empty for a channel. */
  readonly people: readonly PublicUser[];
  /** Name, place and usernames, folded once for the search. */
  readonly searchText: string;
}

export interface ForwardDestinationSources {
  readonly guilds: readonly Guild[];
  /** Most recent first, as the navigator lists them. */
  readonly conversations: readonly DirectMessageChannel[];
  readonly currentUserId: Snowflake;
  readonly isBlocked: (userId: Snowflake) => boolean;
  /** The message has files: the place needs `ATTACH_FILES` too, or the instance refuses. */
  readonly hasAttachments: boolean;
}

/**
 * The conversations first (most recent first), then each server's text channels in the sidebar's
 * order. Only places the reader may post the forward in; the instance decides anyway (ADR-0014).
 */
export function forwardDestinations(sources: ForwardDestinationSources): ForwardDestination[] {
  const { currentUserId } = sources;
  const conversations = sources.conversations
    // A one-to-one conversation with someone you blocked has no composer: nothing goes there.
    .filter((conversation) => blockedRecipientOf(conversation, currentUserId, sources.isBlocked) === null)
    .map((conversation): ForwardDestination => {
      const people = otherParticipants(conversation, currentUserId);
      const isGroup = isGroupConversation(conversation);
      const name = conversationDisplayName(conversation, currentUserId);
      const where = isGroup ? strings.groups.group : `@${people[0]?.username ?? ""}`;
      return {
        channelId: conversation.id,
        kind: isGroup ? "group" : "person",
        name,
        where,
        people,
        searchText: foldForSearch([name, where, ...people.flatMap((person) => [displayName(person), person.username])].join(" ")),
      };
    });
  const channels = sources.guilds.flatMap((guild) =>
    channelSections(guild)
      .flatMap((section) => section.channels)
      .filter((channel) => {
        if (channel.kind !== "text") {
          return false;
        }
        const permissions = computeChannelPermissions(
          guild.my_permissions,
          guild.id,
          currentUserId,
          guild.my_role_ids,
          channel.permission_overwrites,
        );
        return (
          hasPermission(permissions, Permission.VIEW_CHANNEL) &&
          hasPermission(permissions, Permission.SEND_MESSAGES) &&
          (!sources.hasAttachments || hasPermission(permissions, Permission.ATTACH_FILES))
        );
      })
      .map(
        (channel): ForwardDestination => ({
          channelId: channel.id,
          kind: "channel",
          name: channel.name,
          where: guild.name,
          people: [],
          searchText: foldForSearch(`${channel.name} ${guild.name}`),
        }),
      ),
  );
  return [...conversations, ...channels];
}

/**
 * `destinations` in the order of `channelIds` (the list when the dialog opened), places that
 * appeared since at the end: a forward sent moves its conversation to the top of the navigator,
 * which must not reshuffle the rows being picked.
 */
export function inOpeningOrder(destinations: readonly ForwardDestination[], channelIds: readonly Snowflake[]): ForwardDestination[] {
  const rankOf = new Map(channelIds.map((channelId, index) => [channelId, index]));
  const rank = (destination: ForwardDestination) => rankOf.get(destination.channelId) ?? channelIds.length;
  // `sort` is stable: places unknown at opening keep their own order after the others.
  return [...destinations].sort((left, right) => rank(left) - rank(right));
}

/** Every word typed, in any order, ignoring case and accents; a leading `#` or `@` is only a habit. */
export function destinationsMatching(destinations: readonly ForwardDestination[], query: string): ForwardDestination[] {
  const words = foldForSearch(query.replace(/^\s*[#@]/u, "")).split(/\s+/u).filter((word) => word !== "");
  return destinations.filter((destination) => words.every((word) => destination.searchText.includes(word)));
}

/** How a destination is named in sentences: "#général", "Ana", or a group's name. */
export function destinationLabel(destination: ForwardDestination): string {
  return destination.kind === "channel" ? `#${destination.name}` : destination.name;
}

/**
 * Where a forward's original was written, as its footer names it: "#général" in a server, the
 * conversation's name at Home; null when the reader does not know that channel (ADR-0039).
 */
export function forwardOriginPlace(
  channelId: Snowflake,
  guilds: readonly Guild[],
  conversations: readonly DirectMessageChannel[],
  currentUserId: Snowflake | null,
): { readonly label: string; readonly guildId: Snowflake | null } | null {
  for (const guild of guilds) {
    const channel = guild.channels.find((candidate) => candidate.id === channelId);
    if (channel) {
      return { label: `#${channel.name}`, guildId: guild.id };
    }
  }
  const conversation = conversations.find((candidate) => candidate.id === channelId);
  return conversation ? { label: conversationDisplayName(conversation, currentUserId), guildId: null } : null;
}

// --- Sending ------------------------------------------------------------------------------------

/** What a forward dialog remembers between tries; the functions below fill it. */
export interface ForwardAttempts {
  /** One nonce for the forward and one for the note per place, for the dialog's life: a retry
   * after a lost answer gets the message already made instead of a copy. */
  readonly noncesByChannelId: Map<Snowflake, { readonly forward: string; readonly note: string }>;
  /** Places whose forward the instance confirmed: a retry only sends their note. */
  readonly forwardedChannelIds: Set<Snowflake>;
}

export interface ForwardSending {
  readonly forward: (channelId: Snowflake, nonce: string) => Promise<void>;
  readonly sendNote: (channelId: Snowflake, text: string, nonce: string) => Promise<void>;
  readonly createNonce: () => string;
}

export interface ForwardRefusal {
  readonly destination: ForwardDestination;
  /** "Couldn't forward to #news: you can't send messages or files there anymore." */
  readonly sentence: string;
  /** The sentence's end alone, for the place's row. */
  readonly reason: string;
}

export interface ForwardOutcome {
  /** Places that got the forward and the note (when there is one). */
  readonly reached: readonly ForwardDestination[];
  readonly refused: readonly ForwardRefusal[];
}

/** A forward's own refusals read better than the generic ones. */
function forwardRefusalReason(error: unknown): string {
  if (error instanceof ApiError && error.code === "NOT_FOUND") {
    return strings.forward.originGone;
  }
  if (error instanceof ApiError && error.code === "MISSING_PERMISSIONS") {
    return strings.forward.cannotSendThere;
  }
  return errorReason(error);
}

async function deliverToOne(
  destination: ForwardDestination,
  note: string,
  attempts: ForwardAttempts,
  sending: ForwardSending,
): Promise<ForwardRefusal | null> {
  const { channelId } = destination;
  let nonces = attempts.noncesByChannelId.get(channelId);
  if (nonces === undefined) {
    nonces = { forward: sending.createNonce(), note: sending.createNonce() };
    attempts.noncesByChannelId.set(channelId, nonces);
  }
  const label = destinationLabel(destination);
  if (!attempts.forwardedChannelIds.has(channelId)) {
    try {
      await sending.forward(channelId, nonces.forward);
    } catch (error) {
      const reason = forwardRefusalReason(error);
      return { destination, sentence: strings.errors.withReason(strings.forward.forwardFailed(label), reason), reason };
    }
    attempts.forwardedChannelIds.add(channelId);
  }
  if (note === "") {
    return null;
  }
  try {
    await sending.sendNote(channelId, note, nonces.note);
  } catch (error) {
    return { destination, sentence: describeError(error, strings.forward.noteFailed(label)), reason: errorReason(error) };
  }
  return null;
}

/**
 * Sends the forward, then `note` when not empty, to each place on its own and in parallel: one
 * refusing does not hold the others back. `attempts` carries over to the next try.
 */
export async function forwardToEach(
  destinations: readonly ForwardDestination[],
  note: string,
  attempts: ForwardAttempts,
  sending: ForwardSending,
): Promise<ForwardOutcome> {
  const refusals = await Promise.all(
    destinations.map((destination) => deliverToOne(destination, note, attempts, sending)),
  );
  return {
    reached: destinations.filter((_destination, index) => refusals[index] === null),
    refused: refusals.filter((refusal): refusal is ForwardRefusal => refusal !== null),
  };
}
