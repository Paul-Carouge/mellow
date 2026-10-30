// A server's own emoji in the web client (DESIGN.md §6.58, ADR-0028): the composer's ":na"
// suggestions, turning a typed ":name:" into the markup on send, plain text for previews, the
// jumbo size of a message made only of emoji, and naming and checking a picture before upload.

import { strings } from "../i18n";
import { ApiError } from "../lib/api";
import { errorReason } from "../lib/client.svelte";
import { searchEmoji } from "../lib/emoji-catalogue";
import {
  type CustomEmoji,
  MAXIMUM_EMOJIS_PER_GUILD_AND_KIND,
  MAXIMUM_EMOJI_NAME_LENGTH,
  MAXIMUM_EMOJI_UPLOAD_BYTES,
  MINIMUM_EMOJI_NAME_LENGTH,
  PREVIEWABLE_IMAGE_TYPES,
  type Snowflake,
  customEmojiMarkup,
  customEmojiOfReactionKey,
  isValidEmojiName,
} from "../lib/protocol";
import { displayEmoji } from "../lib/reactions";
import { humanFileSize } from "./format";
import { type BlockNode, findCodeSpans } from "./markdown";

const EMOJI_NAME_CHARACTER = /[A-Za-z0-9_]/u;

// --- Suggestions while typing --------------------------------------------------------------

/** What is being typed after a ":": replaced from `start` to `end` when a suggestion is chosen. */
export interface EmojiSuggestionQuery {
  start: number;
  end: number;
  query: string;
}

/**
 * The ":na" being typed before the caret, as on Discord: a colon at the start or after a space,
 * then at least two name characters up to the caret. Null otherwise, so "12:30" or a URL's
 * "https:" never opens the list.
 */
export function emojiSuggestionQuery(text: string, caret: number): EmojiSuggestionQuery | null {
  let start = caret;
  while (start > 0 && caret - start <= MAXIMUM_EMOJI_NAME_LENGTH && EMOJI_NAME_CHARACTER.test(text[start - 1] ?? "")) {
    start -= 1;
  }
  const colon = start - 1;
  const length = caret - start;
  if (text[colon] !== ":" || length < MINIMUM_EMOJI_NAME_LENGTH || length > MAXIMUM_EMOJI_NAME_LENGTH) {
    return null;
  }
  if (colon > 0 && !/\s/u.test(text[colon - 1] ?? "")) {
    return null;
  }
  return { start: colon, end: caret, query: text.slice(start, caret) };
}

export type EmojiSuggestion =
  | { kind: "custom"; emoji: CustomEmoji }
  /** `name` is the emoji's first English search word, shown beside it. */
  | { kind: "unicode"; emoji: string; name: string };

export const MAXIMUM_EMOJI_SUGGESTIONS = 10;

/**
 * The server's emoji whose name holds the query (those starting with it first), then built-in
 * emoji whose words start with it, at most `MAXIMUM_EMOJI_SUGGESTIONS` in all.
 */
export function suggestEmoji(query: string, customEmojis: readonly CustomEmoji[]): EmojiSuggestion[] {
  const folded = query.toLowerCase();
  const containing = customEmojis.filter((emoji) => emoji.name.toLowerCase().includes(folded));
  const custom = [
    ...containing.filter((emoji) => emoji.name.toLowerCase().startsWith(folded)),
    ...containing.filter((emoji) => !emoji.name.toLowerCase().startsWith(folded)),
  ].map((emoji): EmojiSuggestion => ({ kind: "custom", emoji }));
  // Built-in emoji are searched by words: "thumbs_up" reads as "thumbs up".
  const unicode = searchEmoji(query.replaceAll("_", " ")).map(
    (entry): EmojiSuggestion => ({ kind: "unicode", emoji: entry.emoji, name: entry.keywords[0] ?? "" }),
  );
  return [...custom, ...unicode].slice(0, MAXIMUM_EMOJI_SUGGESTIONS);
}

/**
 * What choosing an emoji writes in the composer: a server emoji as its readable `:name:`, turned
 * into the markup on send (`withEmojiNamesAsMarkup`), a built-in one as itself.
 */
export function composerTextOf(emoji: string | CustomEmoji): string {
  return typeof emoji === "string" ? emoji : `:${emoji.name}:`;
}

/** The draft with `replacement` and a space in place of `start`–`end`, and where the caret goes. */
export function replaceInDraft(
  draft: string,
  start: number,
  end: number,
  replacement: string,
): { text: string; caret: number } {
  const inserted = draft[end] === " " ? replacement : `${replacement} `;
  return { text: draft.slice(0, start) + inserted + draft.slice(end), caret: start + inserted.length + (inserted === replacement ? 1 : 0) };
}

// --- Converting typed names ------------------------------------------------------------------

/** Markup already written: it stays as it is. */
const WRITTEN_MARKUP = /<a?:[A-Za-z0-9_]{2,32}:\d{1,20}>/gu;
const TYPED_EMOJI_NAME = /:([A-Za-z0-9_]{2,32}):/gu;

function isEscapedAt(text: string, position: number): boolean {
  let backslashes = 0;
  for (let index = position - 1; index >= 0 && text[index] === "\\"; index -= 1) {
    backslashes += 1;
  }
  return backslashes % 2 === 1;
}

function convertNames(text: string, emojiByFoldedName: ReadonlyMap<string, CustomEmoji>): string {
  let converted = "";
  let copiedUpTo = 0;
  TYPED_EMOJI_NAME.lastIndex = 0;
  for (let match = TYPED_EMOJI_NAME.exec(text); match !== null; match = TYPED_EMOJI_NAME.exec(text)) {
    const name = match[1] ?? "";
    const emoji = emojiByFoldedName.get(name.toLowerCase());
    if (emoji && !isEscapedAt(text, match.index)) {
      converted += text.slice(copiedUpTo, match.index) + customEmojiMarkup(emoji);
      copiedUpTo = TYPED_EMOJI_NAME.lastIndex;
    } else {
      // The closing colon may open the next name (":not_one:party:").
      TYPED_EMOJI_NAME.lastIndex = match.index + 1 + name.length;
    }
  }
  return converted + text.slice(copiedUpTo);
}

/** `convertNames` around the markup already written in `text` (outside code). */
function convertNamesOutsideMarkup(text: string, emojiByFoldedName: ReadonlyMap<string, CustomEmoji>): string {
  let converted = "";
  let copiedUpTo = 0;
  WRITTEN_MARKUP.lastIndex = 0;
  for (let match = WRITTEN_MARKUP.exec(text); match !== null; match = WRITTEN_MARKUP.exec(text)) {
    converted += convertNames(text.slice(copiedUpTo, match.index), emojiByFoldedName) + match[0];
    copiedUpTo = WRITTEN_MARKUP.lastIndex;
  }
  return converted + convertNames(text.slice(copiedUpTo), emojiByFoldedName);
}

/**
 * The message as sent: each `:name:` of one of `emojis` (the open server's; names are unique
 * whatever the case) becomes its markup, unless escaped with a backslash or in code, found by
 * the renderer's own scan (`findCodeSpans`), so what stays literal is what shows as code.
 */
export function withEmojiNamesAsMarkup(text: string, emojis: readonly CustomEmoji[]): string {
  if (emojis.length === 0 || !text.includes(":")) {
    return text;
  }
  const emojiByFoldedName = new Map(emojis.map((emoji) => [emoji.name.toLowerCase(), emoji]));
  let converted = "";
  let copiedUpTo = 0;
  // In increasing order of start: the scan runs forward.
  for (const [start, span] of findCodeSpans(text)) {
    converted += convertNamesOutsideMarkup(text.slice(copiedUpTo, start), emojiByFoldedName) + text.slice(start, span.end);
    copiedUpTo = span.end;
  }
  return converted + convertNamesOutsideMarkup(text.slice(copiedUpTo), emojiByFoldedName);
}

/** A message as plain text (previews, reply lines): each emoji's markup reads `:name:`. */
export function withCustomEmojiAsNames(text: string): string {
  return text.replace(/<a?:([A-Za-z0-9_]{2,32}):\d{1,20}>/gu, ":$1:");
}

// --- Jumbo emoji -----------------------------------------------------------------------------

/** As on Discord: a message of up to 30 emoji and nothing else shows them large. */
export const MAXIMUM_JUMBO_EMOJI = 30;
/** A grapheme drawn as an emoji: emoji presentation, a selector asking for it, a keycap or a flag. */
const EMOJI_GRAPHEME = /\p{Emoji_Presentation}|\u{FE0F}|\u{20E3}|\p{Regional_Indicator}/u;
const graphemes = new Intl.Segmenter(undefined, { granularity: "grapheme" });

/** How many emoji `text` holds, null if it holds anything but emoji and spaces. */
function countEmojiOnly(text: string): number | null {
  let count = 0;
  for (const { segment } of graphemes.segment(text)) {
    if (/^\s+$/u.test(segment)) {
      continue;
    }
    if (!EMOJI_GRAPHEME.test(segment)) {
      return null;
    }
    count += 1;
  }
  return count;
}

/** Whether a parsed message is one paragraph of 1 to 30 emoji, shown pictured, and spaces. */
export function isJumboEmojiMessage(blocks: readonly BlockNode[]): boolean {
  const [block] = blocks;
  if (blocks.length !== 1 || block?.kind !== "paragraph") {
    return false;
  }
  let count = 0;
  for (const node of block.children) {
    const added = node.kind === "customEmoji" ? 1 : node.kind === "text" ? countEmojiOnly(node.text) : null;
    if (added === null) {
      return false;
    }
    count += added;
    if (count > MAXIMUM_JUMBO_EMOJI) {
      return false;
    }
  }
  return count > 0;
}

// --- Reactions -------------------------------------------------------------------------------

/** How a reaction's emoji shows: a built-in emoji, a server emoji's picture, or a deleted one. */
export type ReactionAppearance =
  | { kind: "unicode"; emoji: string; label: string }
  | { kind: "custom"; emoji: CustomEmoji; label: string }
  /** Its server deleted it: the reaction stays, joined and taken back, but never started anew. */
  | { kind: "deleted"; label: string };

/** `customEmojis` are those of the user's servers: a custom key missing there is a deleted emoji. */
export function reactionAppearance(key: string, customEmojis: ReadonlyMap<Snowflake, CustomEmoji>): ReactionAppearance {
  const customEmojiId = customEmojiOfReactionKey(key);
  if (customEmojiId === null) {
    const emoji = displayEmoji(key);
    return { kind: "unicode", emoji, label: emoji };
  }
  const emoji = customEmojis.get(customEmojiId);
  return emoji ? { kind: "custom", emoji, label: `:${emoji.name}:` } : { kind: "deleted", label: strings.reactions.deletedEmoji };
}

// --- Uploading -------------------------------------------------------------------------------

export interface EmojiCounts {
  still: number;
  animated: number;
}

export function emojiCounts(emojis: readonly CustomEmoji[]): EmojiCounts {
  const animated = emojis.filter((emoji) => emoji.is_animated).length;
  return { still: emojis.length - animated, animated };
}

/**
 * A name from a file's name: accents dropped, every other character outside letters, digits and
 * `_` made `_` ("emoji" when nothing is left), cut to 32 and padded to 2, then `_2`, `_3`… while
 * `takenNames` has it whatever the case (the instance would refuse it).
 */
export function emojiNameFromFileName(fileName: string, takenNames: Iterable<string>): string {
  const readable = fileName
    .replace(/\.[^.]*$/u, "")
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .replace(/[^A-Za-z0-9_]+/gu, "_")
    .replace(/^_+|_+$/gu, "");
  const base = (readable || "emoji").slice(0, MAXIMUM_EMOJI_NAME_LENGTH).padEnd(MINIMUM_EMOJI_NAME_LENGTH, "_");
  const taken = new Set([...takenNames].map((name) => name.toLowerCase()));
  let candidate = base;
  for (let suffix = 2; taken.has(candidate.toLowerCase()); suffix += 1) {
    const ending = `_${suffix}`;
    candidate = base.slice(0, MAXIMUM_EMOJI_NAME_LENGTH - ending.length) + ending;
  }
  return candidate;
}

/** Why a new name cannot be given to `emojiId`, checked before asking; null lets the instance decide. */
export function emojiRenameProblem(
  name: string,
  emojiId: Snowflake,
  emojis: readonly CustomEmoji[],
): "invalid" | "taken" | null {
  if (!isValidEmojiName(name)) {
    return "invalid";
  }
  const folded = name.toLowerCase();
  return emojis.some((emoji) => emoji.id !== emojiId && emoji.name.toLowerCase() === folded) ? "taken" : null;
}

/** The sentence end saying why the instance refused a picture (DESIGN.md §8). */
export function emojiUploadFailureReason(error: unknown, file: Pick<File, "size">): string {
  const reasons = strings.serverSettings.emoji;
  if (!(error instanceof ApiError)) {
    return errorReason(error);
  }
  switch (error.code) {
    case "FILE_TOO_LARGE":
      return reasons.tooLarge(humanFileSize(file.size));
    case "LIMIT_REACHED":
      return reasons.noRoom;
    // Names are checked before sending: what is left is a picture it could not use.
    case "VALIDATION_FAILED":
      return reasons.unusable;
    // The picture budget each person shares with avatars and icons (ADR-0028).
    case "RATE_LIMITED":
      return reasons.tooMany;
    default:
      return errorReason(error);
  }
}

/** Why a picture cannot be added, known before sending it; null lets the instance decide. */
export type EmojiFileProblem = "not-a-picture" | "too-large" | "no-room";

/** PNG and JPEG are always still; a GIF or WebP may be either, which only the instance can tell. */
const ALWAYS_STILL_TYPES: readonly string[] = ["image/png", "image/jpeg"];

export function emojiFileProblem(file: Pick<File, "type" | "size">, counts: EmojiCounts): EmojiFileProblem | null {
  if (!PREVIEWABLE_IMAGE_TYPES.includes(file.type)) {
    return "not-a-picture";
  }
  if (file.size > MAXIMUM_EMOJI_UPLOAD_BYTES) {
    return "too-large";
  }
  const isStillFull = counts.still >= MAXIMUM_EMOJIS_PER_GUILD_AND_KIND;
  const isAnimatedFull = counts.animated >= MAXIMUM_EMOJIS_PER_GUILD_AND_KIND;
  return isStillFull && (isAnimatedFull || ALWAYS_STILL_TYPES.includes(file.type)) ? "no-room" : null;
}
