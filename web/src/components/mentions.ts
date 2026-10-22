// `@name` mentions in message text (DESIGN.md §5.4).
// shortcut: the protocol has no mention syntax or mention list yet, so mentions are found here by
// matching "@username" (or a one-word display name) against the people this client knows; move to
// the instance's own mention ids once the protocol carries them.

import type { PublicUser, Snowflake } from "../lib/protocol";

export type MentionSegment = { kind: "text"; text: string } | { kind: "mention"; text: string; user: PublicUser };

/** People by the lowercase names they can be mentioned with. */
export type MentionablePeople = ReadonlyMap<string, PublicUser>;

/** An "@" at the start or after a space, then a username's characters (dots allowed inside). */
const MENTION_CANDIDATE = /(?<=^|\s)@([\p{L}\p{N}_.]+)/gu;

export function mentionablePeople(people: readonly PublicUser[]): MentionablePeople {
  const byName = new Map<string, PublicUser>();
  // Display names first, so a username always wins over someone else's display name.
  for (const person of people) {
    const displayName = person.display_name.trim();
    if (displayName !== "" && !/\s/u.test(displayName)) {
      byName.set(displayName.toLowerCase(), person);
    }
  }
  for (const person of people) {
    byName.set(person.username.toLowerCase(), person);
  }
  return byName;
}

function findMentioned(name: string, people: MentionablePeople): { user: PublicUser; length: number } | null {
  // "@paul." at the end of a sentence: the trailing dots are punctuation unless they are the name's.
  for (let candidate = name; candidate !== ""; candidate = candidate.replace(/\.$/u, "")) {
    const user = people.get(candidate.toLowerCase());
    if (user) {
      return { user, length: candidate.length };
    }
    if (!candidate.endsWith(".")) {
      return null;
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

export function mentionsUser(content: string, people: MentionablePeople, userId: Snowflake): boolean {
  return splitMentions(content, people).some((segment) => segment.kind === "mention" && segment.user.id === userId);
}
