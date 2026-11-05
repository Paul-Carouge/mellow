// The quoted line above a reply (DESIGN.md §5.4): the original's author and text.

import { strings } from "../i18n";
import type { Message, PublicUser, Role, Snowflake } from "../lib/protocol";
import { attachmentCountLabel } from "./format";
import { pollSummary } from "./poll-view";
import { trackerSummary } from "./tracker-view";
import { messageAsPlainText } from "./role-mentions";

export interface ReplyReference {
  author: PublicUser | null;
  text: string;
}

export type ReplyReferenceLookup = (
  message: Message,
  messagesById: ReadonlyMap<Snowflake, Message>,
  isBlocked: (userId: Snowflake) => boolean,
  roles: ReadonlyMap<Snowflake, Role>,
) => ReplyReference | null;

/**
 * The text, roles named, then "2 attachments" when files came with it (DESIGN.md §7.3); a poll's
 * question for a poll (ADR-0043), a checklist's title or a progress bar's label (ADR-0047).
 */
export function messageSummary(message: Message, roles: ReadonlyMap<Snowflake, Role>): string {
  if (message.poll) {
    return pollSummary(message.poll);
  }
  const tracker = trackerSummary(message);
  if (tracker !== null) {
    return tracker;
  }
  return [messageAsPlainText(message.content, roles), attachmentCountLabel(message.attachments?.length ?? 0)]
    .filter(Boolean)
    .join(" ");
}

/**
 * Finds the reply's original among the loaded messages. The answer stays the same object while the
 * original message and its author's blocked state do, so reply rows that did not change keep their
 * props. Messages are replaced, never changed in place, when they are edited; a new roles map
 * (a role renamed) words every quote again.
 */
export function createReplyReferenceLookup(): ReplyReferenceLookup {
  let referencesByOriginal = new WeakMap<Message, ReplyReference>();
  let rolesQuoted: ReadonlyMap<Snowflake, Role> | null = null;
  let originalUnavailable: ReplyReference | null = null;
  let blockedReply: ReplyReference | null = null;
  return (message, messagesById, isBlocked, roles) => {
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
    if (roles !== rolesQuoted) {
      rolesQuoted = roles;
      referencesByOriginal = new WeakMap();
    }
    let reference = referencesByOriginal.get(original);
    if (!reference) {
      reference = { author: original.author, text: messageSummary(original, roles) };
      referencesByOriginal.set(original, reference);
    }
    return reference;
  };
}
