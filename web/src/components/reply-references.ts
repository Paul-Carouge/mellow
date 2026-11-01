// The quoted line above a reply (DESIGN.md §5.4): the original's author and text.

import { strings } from "../i18n";
import type { Message, PublicUser, Snowflake } from "../lib/protocol";
import { withCustomEmojiAsNames } from "./custom-emoji";
import { attachmentCountLabel } from "./format";

export interface ReplyReference {
  author: PublicUser | null;
  text: string;
}

export type ReplyReferenceLookup = (
  message: Message,
  messagesById: ReadonlyMap<Snowflake, Message>,
  isBlocked: (userId: Snowflake) => boolean,
) => ReplyReference | null;

/** The text, then "2 attachments" when files came with it (DESIGN.md §7.3). */
export function messageSummary(message: Message): string {
  return [withCustomEmojiAsNames(message.content), attachmentCountLabel(message.attachments?.length ?? 0)]
    .filter(Boolean)
    .join(" ");
}

/**
 * Finds the reply's original among the loaded messages. The answer stays the same object while the
 * original message and its author's blocked state do, so reply rows that did not change keep their
 * props. Messages are replaced, never changed in place, when they are edited.
 */
export function createReplyReferenceLookup(): ReplyReferenceLookup {
  const referencesByOriginal = new WeakMap<Message, ReplyReference>();
  let originalUnavailable: ReplyReference | null = null;
  let blockedReply: ReplyReference | null = null;
  return (message, messagesById, isBlocked) => {
    if (message.reply_to_id === null) {
      return null;
    }
    const original = messagesById.get(message.reply_to_id);
    if (!original) {
      return (originalUnavailable ??= { author: null, text: strings.messages.originalUnavailable });
    }
    // Someone you blocked is never quoted (ADR-0025).
    if (isBlocked(original.author.id)) {
      return (blockedReply ??= { author: null, text: strings.messages.blockedReply });
    }
    let reference = referencesByOriginal.get(original);
    if (!reference) {
      reference = { author: original.author, text: messageSummary(original) };
      referencesByOriginal.set(original, reference);
    }
    return reference;
  };
}
