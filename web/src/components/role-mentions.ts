// Role and `@everyone` mentions (ADR-0040, DESIGN.md §5.4, §5.5): how `<@&id>` reads as text,
// which roles the writer may mention, the `@` suggestions for them, and turning a picked role's
// `@Name` into its markup on send.

import { strings } from "../i18n";
import type { Role, Snowflake } from "../lib/protocol";
import { foldForSearch } from "../lib/search-text";
import { withCustomEmojiAsNames } from "./custom-emoji";
import { findCodeSpans } from "./markdown";
import { EVERYONE_MENTION_WORD, MAXIMUM_MENTION_SUGGESTIONS } from "./mentions";

/** `@Modérateurs`, or `@rôle-supprimé` for a role this client does not know. */
export function roleMentionLabel(role: Role | null): string {
  return `@${role?.name ?? strings.messages.deletedRoleName}`;
}

const ROLE_MENTION_MARKUP = /<@&(\d{1,20})>/gu;

/** A message as plain text (previews, reply lines, pins): emoji as `:name:`, roles as `@Name`. */
export function messageAsPlainText(content: string, roles: ReadonlyMap<Snowflake, Role>): string {
  return withCustomEmojiAsNames(content).replace(ROLE_MENTION_MARKUP, (_markup, id: string) =>
    roleMentionLabel(roles.get(id) ?? null),
  );
}

// --- What the writer may mention -------------------------------------------------------------

/**
 * The roles the writer may mention in a channel, highest first: those anyone may mention, or
 * every role with `MENTION_EVERYONE` (ADR-0040). Never `@everyone`'s role (its id is the guild's):
 * everyone is mentioned by the word.
 */
export function rolesWriterMayMention(roles: readonly Role[], guildId: Snowflake, canMentionEveryone: boolean): Role[] {
  return roles
    .filter((role) => role.id !== guildId && (canMentionEveryone || role.is_mentionable === true))
    .sort((left, right) => right.position - left.position);
}

export type RoleMentionSuggestion = { kind: "everyone" } | { kind: "role"; role: Role };

const roleNameCollator = new Intl.Collator();

/** 0 for a name starting with the folded query, 1 for a word of it starting with it, 2 holding it. */
function roleMatchRank(name: string, foldedQuery: string): number | null {
  const folded = foldForSearch(name);
  if (folded.startsWith(foldedQuery)) {
    return 0;
  }
  if (folded.split(/\s+/u).some((word) => word.startsWith(foldedQuery))) {
    return 1;
  }
  return folded.includes(foldedQuery) ? 2 : null;
}

/**
 * What `@query` offers after the people: `@everyone` when the writer may use it and it matches,
 * then the matching roles of `mentionableRoles` (from `rolesWriterMayMention`), best match first,
 * then highest role, at most `MAXIMUM_MENTION_SUGGESTIONS`.
 */
export function suggestRoleMentions(
  query: string,
  mentionableRoles: readonly Role[],
  canMentionEveryone: boolean,
): RoleMentionSuggestion[] {
  const foldedQuery = foldForSearch(query);
  const everyone: RoleMentionSuggestion[] =
    canMentionEveryone && EVERYONE_MENTION_WORD.startsWith(foldedQuery) ? [{ kind: "everyone" }] : [];
  const roles = mentionableRoles
    .map((role, order) => ({ role, order, rank: roleMatchRank(role.name, foldedQuery) }))
    .filter((entry): entry is { role: Role; order: number; rank: number } => entry.rank !== null)
    .sort((left, right) => left.rank - right.rank || left.order - right.order || roleNameCollator.compare(left.role.name, right.role.name))
    .map((entry): RoleMentionSuggestion => ({ kind: "role", role: entry.role }));
  return [...everyone, ...roles].slice(0, MAXIMUM_MENTION_SUGGESTIONS);
}

// --- Sending ---------------------------------------------------------------------------------

/** A role name may be followed by punctuation, not by more of a word. */
const WORD_CHARACTER = /[\p{Alphabetic}\p{N}_]/u;
/** A username may hold dots ("mod.ana"): a dot followed by a name character continues the name. */
const USERNAME_CHARACTER = /[\p{Alphabetic}\p{N}_.]/u;
/** What may come right before a mention's "@", as the server reads it. */
const MENTION_PRECEDER = /[\s*_~|]/u;

function isEscapedAt(text: string, position: number): boolean {
  let backslashes = 0;
  for (let index = position - 1; index >= 0 && text[index] === "\\"; index -= 1) {
    backslashes += 1;
  }
  return backslashes % 2 === 1;
}

/** The role whose name, in any case, is written right after the "@" at `at`, longest name first. */
function roleNamedAt(text: string, at: number, rolesByLongestName: readonly Role[]): Role | null {
  for (const role of rolesByLongestName) {
    const end = at + 1 + role.name.length;
    if (
      text.slice(at + 1, end).toLocaleLowerCase() === role.name.toLocaleLowerCase() &&
      !WORD_CHARACTER.test(text[end] ?? "") &&
      // "@mod.ana" names a person even with a role "Mod"; "@Mod." ending a sentence is the role.
      !(text[end] === "." && USERNAME_CHARACTER.test(text[end + 1] ?? ""))
    ) {
      return role;
    }
  }
  return null;
}

function convertRoleNames(text: string, rolesByLongestName: readonly Role[]): string {
  let converted = "";
  let copiedUpTo = 0;
  for (let at = text.indexOf("@"); at !== -1; at = text.indexOf("@", at + 1)) {
    if (at < copiedUpTo || (at > 0 && !MENTION_PRECEDER.test(text[at - 1] ?? "")) || isEscapedAt(text, at)) {
      continue;
    }
    const role = roleNamedAt(text, at, rolesByLongestName);
    if (role) {
      converted += `${text.slice(copiedUpTo, at)}<@&${role.id}>`;
      copiedUpTo = at + 1 + role.name.length;
    }
  }
  return converted + text.slice(copiedUpTo);
}

/**
 * The message as sent: each `@Name` of one of `mentionableRoles` (the roles the writer may
 * mention) becomes `<@&id>`, which the composer cannot show, as `:name:` becomes an emoji's
 * markup. Not in code or after a backslash; never a name that is someone's username in
 * `peopleNames` (lowercase), so picking a person keeps mentioning that person, nor "everyone".
 */
export function withRoleNamesAsMarkup(
  text: string,
  mentionableRoles: readonly Role[],
  peopleNames: ReadonlySet<string>,
): string {
  if (mentionableRoles.length === 0 || !text.includes("@")) {
    return text;
  }
  const rolesByLongestName = mentionableRoles
    .filter((role) => {
      const name = role.name.toLowerCase();
      return name.trim() !== "" && name !== EVERYONE_MENTION_WORD && !peopleNames.has(name);
    })
    .sort((left, right) => right.name.length - left.name.length);
  let converted = "";
  let copiedUpTo = 0;
  // In increasing order of start: the scan runs forward.
  for (const [start, span] of findCodeSpans(text)) {
    converted += convertRoleNames(text.slice(copiedUpTo, start), rolesByLongestName) + text.slice(start, span.end);
    copiedUpTo = span.end;
  }
  return converted + convertRoleNames(text.slice(copiedUpTo), rolesByLongestName);
}
