import { MAXIMUM_MESSAGE_BYTES, MAXIMUM_MESSAGE_LENGTH } from "../lib/protocol";

/** Below this many characters left, the composer shows the count. */
export const MESSAGE_LENGTH_COUNTER_THRESHOLD = 500;

const utf8 = new TextEncoder();

/**
 * How many characters the message may still take, negative when it is too long. Counted as the
 * instance does: Unicode scalar values of the trimmed text, and at most `MAXIMUM_MESSAGE_BYTES`
 * of UTF-8, whichever runs out first (only emoji-heavy text reaches the byte bound).
 */
export function messageCharactersLeft(draft: string): number {
  const text = draft.trim();
  return Math.min(
    MAXIMUM_MESSAGE_LENGTH - [...text].length,
    MAXIMUM_MESSAGE_BYTES - utf8.encode(text).length,
  );
}
