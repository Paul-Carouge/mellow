// The emoji this browser picked lately, shared by the picker and the hover bar's quick reactions.

import { browserStorage, quickReactions, readRecentEmoji, rememberEmoji } from "../lib/reactions";

const recent = $state({ emoji: readRecentEmoji(browserStorage()) });

export function recentEmoji(): readonly string[] {
  return recent.emoji;
}

export function quickReactionEmoji(): string[] {
  return quickReactions(recent.emoji);
}

/** Called for every reaction the user adds, from the picker or a quick reaction. */
export function noteEmojiUsed(emoji: string): void {
  recent.emoji = rememberEmoji(browserStorage(), emoji);
}
