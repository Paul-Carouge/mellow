// Reactions as the client keeps them: the instance's emoji keys, how each one shows, the counts
// updated from events and from the user's own clicks, and the emoji the user picked lately.

import { EMOJI_CATALOGUE } from "./emoji-catalogue";
import type { ReactionSummary } from "./protocol";

const VARIATION_SELECTORS = /[︎️]/gu;
const EMOJI_PRESENTATION_SELECTOR = "️";
const KEYCAP = "⃣";

/** The key the instance stores a reaction under: the emoji without variation selectors. */
export function reactionKey(emoji: string): string {
  return emoji.replace(VARIATION_SELECTORS, "");
}

const CATALOGUE_EMOJI_BY_KEY = new Map(EMOJI_CATALOGUE.map((entry) => [reactionKey(entry.emoji), entry.emoji]));

/**
 * How a stored key shows: as the picker writes it when it knows it (the selectors put back), else
 * a lone symbol or keycap asks for its emoji look, which some fonts only give with the selector.
 */
export function displayEmoji(key: string): string {
  const known = CATALOGUE_EMOJI_BY_KEY.get(key);
  if (known) {
    return known;
  }
  const codePoints = [...key];
  if (codePoints.length === 2 && codePoints[1] === KEYCAP) {
    return `${codePoints[0]}${EMOJI_PRESENTATION_SELECTOR}${KEYCAP}`;
  }
  return codePoints.length === 1 ? `${key}${EMOJI_PRESENTATION_SELECTOR}` : key;
}

/**
 * One more reaction with `key`. The user's own is counted once: their click already counted it,
 * so the event that follows changes nothing.
 */
export function withReactionAdded(
  reactions: readonly ReactionSummary[],
  key: string,
  isMine: boolean,
): ReactionSummary[] {
  const existing = reactions.find((reaction) => reaction.emoji === key);
  if (!existing) {
    return [...reactions, { emoji: key, count: 1, me: isMine }];
  }
  if (isMine && existing.me) {
    return [...reactions];
  }
  return reactions.map((reaction) =>
    reaction === existing ? { ...reaction, count: reaction.count + 1, me: reaction.me || isMine } : reaction,
  );
}

/** One reaction with `key` fewer; the emoji goes at zero. The user's own is removed once. */
export function withReactionRemoved(
  reactions: readonly ReactionSummary[],
  key: string,
  isMine: boolean,
): ReactionSummary[] {
  const existing = reactions.find((reaction) => reaction.emoji === key);
  if (!existing || (isMine && !existing.me)) {
    return [...reactions];
  }
  return reactions
    .map((reaction) =>
      reaction === existing ? { ...reaction, count: reaction.count - 1, me: reaction.me && !isMine } : reaction,
    )
    .filter((reaction) => reaction.count > 0);
}

export function withoutEmoji(reactions: readonly ReactionSummary[], key: string): ReactionSummary[] {
  return reactions.filter((reaction) => reaction.emoji !== key);
}

// --- Recently used emoji --------------------------------------------------------------------

const RECENT_EMOJI_STORAGE_KEY = "mellow.recent-emoji";
export const MAXIMUM_RECENT_EMOJI = 16;
/** The hover bar's quick reactions until the user has picked some of their own, as on Discord. */
export const DEFAULT_QUICK_REACTIONS: readonly string[] = ["👍", "❤️", "😂"];
const QUICK_REACTION_COUNT = 3;

/** `localStorage` itself can throw (blocked site data): `null` then. */
export function browserStorage(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch (error) {
    console.warn("local storage unavailable", error);
    return null;
  }
}

/** What this browser remembers, kept only if it is a list of emoji the picker knows. */
export function readRecentEmoji(storage: Pick<Storage, "getItem"> | null): string[] {
  let stored: unknown;
  try {
    stored = JSON.parse(storage?.getItem(RECENT_EMOJI_STORAGE_KEY) ?? "[]");
  } catch (error) {
    console.warn("unreadable recent emoji", error);
    return [];
  }
  if (!Array.isArray(stored)) {
    return [];
  }
  return stored
    .filter((value): value is string => typeof value === "string" && CATALOGUE_EMOJI_BY_KEY.has(reactionKey(value)))
    .slice(0, MAXIMUM_RECENT_EMOJI);
}

/** `emoji` first, without duplicates; stored when the browser lets us, returned either way. */
export function rememberEmoji(storage: Pick<Storage, "getItem" | "setItem"> | null, emoji: string): string[] {
  const key = reactionKey(emoji);
  const recent = [emoji, ...readRecentEmoji(storage).filter((candidate) => reactionKey(candidate) !== key)].slice(
    0,
    MAXIMUM_RECENT_EMOJI,
  );
  try {
    storage?.setItem(RECENT_EMOJI_STORAGE_KEY, JSON.stringify(recent));
  } catch (error) {
    // A full or blocked storage only loses the memory, not the reaction.
    console.warn("couldn't remember the emoji", error);
  }
  return recent;
}

/** The three emoji offered in the hover bar: the latest picked, then the defaults. */
export function quickReactions(recent: readonly string[]): string[] {
  const chosen: string[] = [];
  for (const emoji of [...recent, ...DEFAULT_QUICK_REACTIONS]) {
    if (chosen.length === QUICK_REACTION_COUNT) {
      break;
    }
    if (!chosen.some((candidate) => reactionKey(candidate) === reactionKey(emoji))) {
      chosen.push(emoji);
    }
  }
  return chosen;
}
