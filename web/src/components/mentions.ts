// `@name` mentions in message text (DESIGN.md §5.4).
// shortcut: the protocol has no mention syntax or mention list yet, so mentions are found here by
// matching "@username" (or a one-word display name) against the people this client knows; move to
// the instance's own mention ids once the protocol carries them.

import type { PublicUser, Snowflake } from "../lib/protocol";
import { foldForSearch } from "../lib/search-text";
import { displayName } from "./format";

export type MentionSegment = { kind: "text"; text: string } | { kind: "mention"; text: string; user: PublicUser };

/** People by the lowercase names they can be mentioned with. */
export type MentionablePeople = ReadonlyMap<string, PublicUser>;

/** Usernames and display names are at most 32 characters (`account_rules.rs` on the server). */
const MAXIMUM_NAME_LENGTH = 32;
/**
 * An "@" at the start or after a space, then a name's characters (dots allowed inside), one more
 * for a closing full stop. `Alphabetic` keeps combining vowel signs in names, as the server's
 * `char::is_alphanumeric` does. Bounded and followed by no name character, so a long run of dots or
 * letters is never a mention and costs a constant amount of work.
 */
const MENTION_CANDIDATE = new RegExp(
  `(?<=^|\\s)@([\\p{Alphabetic}\\p{N}_.]{1,${MAXIMUM_NAME_LENGTH + 1}})(?![\\p{Alphabetic}\\p{N}_.])`,
  "gu",
);

export function mentionablePeople(people: readonly PublicUser[]): MentionablePeople {
  const byName = new Map<string, PublicUser>();
  // A display name two people share is no one's, as on the server (`domain/mentions.rs`): whoever
  // copies a name must not take its mentions.
  const sharedDisplayNames = new Set<string>();
  // Display names first, so a username always wins over someone else's display name.
  for (const person of people) {
    const displayName = person.display_name.trim();
    if (displayName !== "" && !/\s/u.test(displayName)) {
      const name = displayName.toLowerCase();
      const holder = byName.get(name);
      if (holder && holder.id !== person.id) {
        sharedDisplayNames.add(name);
      }
      byName.set(name, person);
    }
  }
  for (const name of sharedDisplayNames) {
    byName.delete(name);
  }
  for (const person of people) {
    byName.set(person.username.toLowerCase(), person);
  }
  return byName;
}

/**
 * `mentionablePeople` that returns its previous map while the people are the same objects: a
 * member update (a role change, say) rebuilds the member list around the same users, and a new map
 * would make every message row parse its text again.
 */
export function memoizedMentionablePeople(): (people: readonly PublicUser[]) => MentionablePeople {
  let previousPeople: readonly PublicUser[] | null = null;
  let previousMap: MentionablePeople = new Map();
  return (people) => {
    const isUnchanged =
      previousPeople !== null &&
      people.length === previousPeople.length &&
      people.every((person, index) => person === previousPeople?.[index]);
    if (!isUnchanged) {
      previousPeople = people;
      previousMap = mentionablePeople(people);
    }
    return previousMap;
  };
}

function findMentioned(name: string, people: MentionablePeople): { user: PublicUser; length: number } | null {
  // "@paul." at the end of a sentence: the trailing dots are punctuation unless they are the name's.
  // Lengths come from the original text: lowercasing can change a string's length.
  for (const candidate of [name, name.replace(/\.+$/u, "")]) {
    const user = candidate === "" ? undefined : people.get(candidate.toLowerCase());
    if (user) {
      return { user, length: candidate.length };
    }
  }
  return null;
}

export function splitMentions(text: string, people: MentionablePeople): MentionSegment[] {
  const segments: MentionSegment[] = [];
  let textStart = 0;
  for (const match of text.matchAll(MENTION_CANDIDATE)) {
    const found = match[1] ? findMentioned(match[1], people) : null;
    if (!found) {
      continue;
    }
    if (match.index > textStart) {
      segments.push({ kind: "text", text: text.slice(textStart, match.index) });
    }
    const end = match.index + 1 + found.length;
    segments.push({ kind: "mention", text: text.slice(match.index, end), user: found.user });
    textStart = end;
  }
  if (textStart < text.length) {
    segments.push({ kind: "text", text: text.slice(textStart) });
  }
  return segments;
}

// --- Suggestions while typing "@na" (DESIGN.md §5.5) -----------------------------------------

/** The "@" and the name being typed before the caret, which a chosen person replaces. */
export interface MentionSuggestionQuery {
  start: number;
  end: number;
  query: string;
}

const NAME_CHARACTER = /[\p{Alphabetic}\p{N}_.]/u;

/**
 * The "@na" ending at `caret`: an "@" where a mention may start (the start, a space, or a
 * formatting mark as in "**@na", as the server reads them), then up to a name's length of name
 * characters; a lone "@" asks for everyone, as on Discord. The replaced range runs to the end of
 * the name, so choosing with the caret inside "@pa|ul" leaves no "ul" behind.
 */
export function mentionSuggestionQuery(text: string, caret: number): MentionSuggestionQuery | null {
  let start = caret;
  while (start > 0 && caret - start <= MAXIMUM_NAME_LENGTH && NAME_CHARACTER.test(text[start - 1] ?? "")) {
    start -= 1;
  }
  const at = start - 1;
  if (text[at] !== "@" || caret - start > MAXIMUM_NAME_LENGTH) {
    return null;
  }
  if (at > 0 && !/[\s*_~|]/u.test(text[at - 1] ?? "")) {
    return null;
  }
  let end = caret;
  while (end < text.length && NAME_CHARACTER.test(text[end] ?? "")) {
    end += 1;
  }
  return { start: at, end, query: text.slice(start, caret) };
}

export const MAXIMUM_MENTION_SUGGESTIONS = 10;

interface FoldedNames {
  username: string;
  shownName: string;
  shownNameWords: readonly string[];
}

// Members keep their object while unchanged, so their folded names are worked out once, not on
// every keystroke.
const foldedNamesByPerson = new WeakMap<PublicUser, FoldedNames>();
const shownNameCollator = new Intl.Collator();

function foldedNamesOf(person: PublicUser): FoldedNames {
  let names = foldedNamesByPerson.get(person);
  if (!names) {
    const shownName = foldForSearch(person.display_name.trim());
    names = { username: foldForSearch(person.username), shownName, shownNameWords: shownName.split(/\s+/u) };
    foldedNamesByPerson.set(person, names);
  }
  return names;
}

/**
 * How well `person` matches the folded query, lower first: a name starting with it, then a word
 * of the display name starting with it, then a name holding it; null for no match.
 */
function mentionMatchRank(person: PublicUser, foldedQuery: string): number | null {
  const { username, shownName, shownNameWords } = foldedNamesOf(person);
  if (username.startsWith(foldedQuery) || shownName.startsWith(foldedQuery)) {
    return 0;
  }
  if (shownNameWords.some((word) => word.startsWith(foldedQuery))) {
    return 1;
  }
  return username.includes(foldedQuery) || shownName.includes(foldedQuery) ? 2 : null;
}

/**
 * The people `query` may name, best match first and then by shown name, at most
 * `MAXIMUM_MENTION_SUGGESTIONS`, without `excludedUserId` (the writer: mentioning oneself counts
 * for no one) and each person once. Case and accents are ignored, as in the message search.
 */
export function suggestMentions(
  query: string,
  people: readonly PublicUser[],
  excludedUserId: Snowflake,
): PublicUser[] {
  const foldedQuery = foldForSearch(query);
  const seen = new Set<Snowflake>([excludedUserId]);
  // Only the best few are kept as people are read: a lone "@" in a big server matches everyone,
  // and sorting them all on every keystroke would cost more than reading them.
  const best: { person: PublicUser; rank: number; name: string }[] = [];
  const isBefore = (left: (typeof best)[number], right: (typeof best)[number]) =>
    left.rank - right.rank || shownNameCollator.compare(left.name, right.name);
  for (const person of people) {
    if (seen.has(person.id)) {
      continue;
    }
    seen.add(person.id);
    const rank = mentionMatchRank(person, foldedQuery);
    if (rank === null) {
      continue;
    }
    const entry = { person, rank, name: displayName(person) };
    const last = best[best.length - 1];
    if (best.length === MAXIMUM_MENTION_SUGGESTIONS && last && isBefore(last, entry) <= 0) {
      continue;
    }
    const index = best.findIndex((kept) => isBefore(entry, kept) < 0);
    best.splice(index === -1 ? best.length : index, 0, entry);
    best.length = Math.min(best.length, MAXIMUM_MENTION_SUGGESTIONS);
  }
  return best.map((entry) => entry.person);
}
