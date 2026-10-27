// Group conversations on the web (DESIGN.md §5.6, ADR-0026): how a conversation is named, who the
// "New message" picker offers, how many more fit, and who owns a group after its owner leaves.

import { strings } from "../i18n";
import {
  type DirectMessageChannel,
  MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS,
  type PublicUser,
  type Snowflake,
} from "../lib/protocol";
import { displayName } from "./format";

export function isGroupConversation(conversation: DirectMessageChannel): boolean {
  return conversation.owner_id !== undefined;
}

/** Everyone in the conversation but the signed-in user, in the instance's order. */
export function otherParticipants(conversation: DirectMessageChannel, currentUserId: Snowflake | null): PublicUser[] {
  return conversation.recipients.filter((recipient) => recipient.id !== currentUserId);
}

/** "Ana, Bob and 2 others" from the people in it, whatever its own name. */
export function participantNamesOf(conversation: DirectMessageChannel, currentUserId: Snowflake | null): string {
  return strings.groups.participantNames(otherParticipants(conversation, currentUserId).map(displayName));
}

/** A group's own name when it has one, else its people's names; a one-to-one is the other person. */
export function conversationDisplayName(conversation: DirectMessageChannel, currentUserId: Snowflake | null): string {
  const ownName = isGroupConversation(conversation) ? (conversation.name ?? "").trim() : "";
  return ownName || participantNamesOf(conversation, currentUserId);
}

/**
 * Who locks the composer: the other person of a one-to-one conversation when you blocked them
 * (ADR-0025). A group never locks, whatever the relationships in it (ADR-0026): their messages fold.
 */
export function blockedRecipientOf(
  conversation: DirectMessageChannel,
  currentUserId: Snowflake | null,
  isBlocked: (userId: Snowflake) => boolean,
): PublicUser | null {
  if (isGroupConversation(conversation)) {
    return null;
  }
  return otherParticipants(conversation, currentUserId).find((recipient) => isBlocked(recipient.id)) ?? null;
}

/** Everyone in a group, you included. */
export function participantCount(conversation: DirectMessageChannel, currentUserId: Snowflake | null): number {
  return otherParticipants(conversation, currentUserId).length + 1;
}

/**
 * How many more people the picker may add: a new group holds you and 9 others; an existing one
 * what is left of its 10 places. Never negative.
 */
export function pickerRoom(participantsAlreadyIn: number, pickedCount: number): number {
  return Math.max(MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS - participantsAlreadyIn - pickedCount, 0);
}

/** Case and accents ignored, so "chloe" finds Chloé. */
function foldedForSearch(text: string): string {
  return text.normalize("NFD").replace(/\p{Diacritic}/gu, "").toLocaleLowerCase();
}

/** The friends whose display name or username contains `query`, minus those already in the group. */
export function friendsMatching(
  friends: readonly PublicUser[],
  query: string,
  excludedUserIds: ReadonlySet<Snowflake>,
): PublicUser[] {
  const term = foldedForSearch(query.trim());
  return friends.filter(
    (friend) =>
      !excludedUserIds.has(friend.id) &&
      (foldedForSearch(displayName(friend)).includes(term) || foldedForSearch(friend.username).includes(term)),
  );
}

/**
 * Who owns the group once the signed-in owner leaves: the participant with the lowest id
 * (ADR-0026). `null` when the user is not the owner or is the last one in it.
 */
export function nextOwnerAfterLeaving(
  conversation: DirectMessageChannel,
  currentUserId: Snowflake | null,
): PublicUser | null {
  if (conversation.owner_id !== currentUserId) {
    return null;
  }
  // Snowflakes are 64-bit: compared as BigInt, never as strings or doubles.
  return otherParticipants(conversation, currentUserId).reduce<PublicUser | null>(
    (lowest, candidate) => (lowest === null || BigInt(candidate.id) < BigInt(lowest.id) ? candidate : lowest),
    null,
  );
}
