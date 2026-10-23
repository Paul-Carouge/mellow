// `@name` mentions in message text (DESIGN.md §5.4).
// shortcut: the protocol has no mention syntax or mention list yet, so mentions are found here by
// matching "@username" (or a one-word display name) against the people this client knows; move to
// the instance's own mention ids once the protocol carries them.

import type { PublicUser } from "../lib/protocol";

export type MentionSegment = { kind: "text"; text: string } | { kind: "mention"; text: string; user: PublicUser };

/** People by the lowercase names they can be mentioned with. */
export type MentionablePeople = ReadonlyMap<string, PublicUser>;

/** Usernames and display names are at most 32 characters (`account_rules.rs` on the server). */
const MAXIMUM_NAME_LENGTH = 32;
/**
 * An "@" at the start or after a space, then a name's characters (dots allowed inside), one more
 * for a closing full stop. Bounded and followed by no name character, so a long run of dots or
 * letters is never a mention and costs a constant amount of work.
 */
const MENTION_CANDIDATE = new RegExp(
  `(?<=^|\\s)@([\\p{L}\\p{N}_.]{1,${MAXIMUM_NAME_LENGTH + 1}})(?![\\p{L}\\p{N}_.])`,
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
