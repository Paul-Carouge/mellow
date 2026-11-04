// Whether a message mentions its reader (ADR-0040): the one rule behind the message tint,
// notifications and live unread mention badges.

import type { Message, Snowflake } from "./protocol";

/** Who reads, and where: `guildId` and `roleIds` are null and empty in a direct conversation. */
export interface MentionReader {
  readonly userId: Snowflake;
  readonly guildId: Snowflake | null;
  readonly roleIds: readonly Snowflake[];
}

/**
 * The server decided who a message mentions: `mention_ids` names people, `mention_role_ids` the
 * roles the author could mention and the guild's id for `@everyone`. The reader is mentioned by
 * name, by a role they hold now, or by `@everyone` in their guild; never by their own message,
 * even one naming a role they hold.
 */
export function doesMessageMentionReader(
  message: Pick<Message, "author" | "mention_ids" | "mention_role_ids">,
  reader: MentionReader,
): boolean {
  if (message.author.id === reader.userId) {
    return false;
  }
  if ((message.mention_ids ?? []).includes(reader.userId)) {
    return true;
  }
  const roleIds = message.mention_role_ids ?? [];
  return (
    reader.guildId !== null &&
    roleIds.length > 0 &&
    (roleIds.includes(reader.guildId) || reader.roleIds.some((roleId) => roleIds.includes(roleId)))
  );
}
