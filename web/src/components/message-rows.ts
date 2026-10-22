// How the shell lays out channels and messages: Discord's rules, as the desktop client applies them.

import type { PendingMessage } from "../lib/client.svelte";
import { type Message, type Snowflake, compareSnowflakes } from "../lib/protocol";
import { unixMillisecondsOfSnowflake } from "./format";

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

export { type ChannelSection, channelSections } from "../lib/channels";
