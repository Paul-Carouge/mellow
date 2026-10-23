// How the shell lays out channels and messages: Discord's rules, as the desktop client applies them.

import { formattingLocale, strings } from "../i18n";
import type { PendingMessage, UnreadMarker } from "../lib/client.svelte";
import { type Message, type Snowflake, compareSnowflakes } from "../lib/protocol";
import { messageTimestampLabel, unixMillisecondsOfSnowflake } from "./format";

/** Discord starts a new author group after this long, even for the same author. */
export const AUTHOR_GROUP_MAXIMUM_GAP_MILLISECONDS = 7 * 60 * 1_000;

export type MessageListRow =
  | {
      kind: "sent";
      key: string;
      message: Message;
      sentAt: number;
      startsAuthorGroup: boolean;
      /** The first message others sent after the visit's unread marker: the "New" divider goes above it. */
      isFirstUnread: boolean;
    }
  | { kind: "pending"; key: string; pending: PendingMessage; startsAuthorGroup: boolean };

/**
 * Confirmed messages oldest first, then the user's outbox; `now` dates the outbox rows. The unread
 * divider starts a new author group, as on Discord.
 */
export function buildMessageRows(
  messages: readonly Message[],
  pendingMessages: readonly PendingMessage[],
  currentUserId: Snowflake | null,
  now: number,
  unreadMarker: Snowflake | null = null,
): MessageListRow[] {
  const rows: MessageListRow[] = [];
  let previous: { authorId: Snowflake; sentAt: number } | null = null;
  let hasPlacedUnreadDivider = false;
  const continuesGroup = (authorId: Snowflake, sentAt: number, isReply: boolean) =>
    !isReply &&
    previous !== null &&
    previous.authorId === authorId &&
    sentAt - previous.sentAt <= AUTHOR_GROUP_MAXIMUM_GAP_MILLISECONDS;

  for (const message of messages) {
    const sentAt = unixMillisecondsOfSnowflake(message.id);
    const isFirstUnread: boolean =
      !hasPlacedUnreadDivider &&
      unreadMarker !== null &&
      compareSnowflakes(message.id, unreadMarker) > 0 &&
      message.author.id !== currentUserId;
    hasPlacedUnreadDivider ||= isFirstUnread;
    rows.push({
      kind: "sent",
      key: message.id,
      message,
      sentAt,
      startsAuthorGroup: isFirstUnread || !continuesGroup(message.author.id, sentAt, message.reply_to_id !== null),
      isFirstUnread,
    });
    previous = { authorId: message.author.id, sentAt };
  }
  for (const pending of pendingMessages) {
    const authorId = currentUserId ?? "";
    rows.push({
      kind: "pending",
      key: pending.nonce,
      pending,
      startsAuthorGroup: !continuesGroup(authorId, now, pending.replyToId !== null),
    });
    previous = { authorId, sentAt: now };
  }
  return rows;
}

export interface UnreadSummary {
  /** Messages from others that were new when the channel was opened, among those loaded. */
  count: number;
  /** Older new messages are not loaded yet: the count is a lower bound ("50+"). */
  isPartial: boolean;
}

/** What the jump bar counts (DESIGN.md §5.4): never live arrivals, never the user's own messages. */
export function unreadSummary(
  messages: readonly Message[],
  marker: UnreadMarker,
  currentUserId: Snowflake | null,
  hasReachedStart: boolean,
): UnreadSummary {
  const count = messages.filter(
    (message) =>
      compareSnowflakes(message.id, marker.lastReadMessageId) > 0 &&
      compareSnowflakes(message.id, marker.newestAtOpenMessageId) <= 0 &&
      message.author.id !== currentUserId,
  ).length;
  const oldestLoaded = messages[0];
  const isPartial =
    !hasReachedStart && oldestLoaded !== undefined && compareSnowflakes(oldestLoaded.id, marker.lastReadMessageId) > 0;
  return { count, isPartial };
}

/** True when the first new message is loaded, so the "New" divider can sit above it. */
export function isUnreadStartLoaded(messages: readonly Message[], marker: UnreadMarker, hasReachedStart: boolean): boolean {
  const oldestLoaded = messages[0];
  return hasReachedStart || (oldestLoaded !== undefined && compareSnowflakes(oldestLoaded.id, marker.lastReadMessageId) <= 0);
}

/** "3 new messages since today at 14:32", dated from the last message read before the visit. */
export function unreadBarLabel(summary: UnreadSummary, lastReadAt: number, now: Date): string {
  const since = messageTimestampLabel(lastReadAt, now);
  // "Today at" becomes "today at" mid-sentence; dates start with a digit, which this leaves alone.
  const sinceInSentence = since.charAt(0).toLocaleLowerCase(formattingLocale) + since.slice(1);
  return strings.chat.newMessagesSince(summary.count, summary.isPartial, sinceInSentence);
}

export { type ChannelSection, channelSections } from "../lib/channels";

/**
 * `next`, with each row equal to its previous version swapped back for that version: the list
 * then updates only the rows that changed, instead of every row on each event.
 */
export function reuseUnchangedRows(
  previous: ReadonlyMap<string, MessageListRow>,
  next: MessageListRow[],
): MessageListRow[] {
  return next.map((row) => {
    const old = previous.get(row.key);
    return old !== undefined && isSameRow(old, row) ? old : row;
  });
}

function isSameRow(left: MessageListRow, right: MessageListRow): boolean {
  if (left.kind === "sent" && right.kind === "sent") {
    return (
      left.message === right.message &&
      left.startsAuthorGroup === right.startsAuthorGroup &&
      left.isFirstUnread === right.isFirstUnread
    );
  }
  return (
    left.kind === "pending" &&
    right.kind === "pending" &&
    left.pending === right.pending &&
    left.startsAuthorGroup === right.startsAuthorGroup
  );
}
