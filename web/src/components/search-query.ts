// The message search query language as the web client reads it (ADR-0029): the same parts the
// instance's parser finds (`domain::search_query`), so suggestions, chips and limits agree with
// what will be searched. The query text stays the single source of truth: chips and facets only
// add or remove parts of it.

import { MAXIMUM_SEARCH_QUERY_LENGTH, MAXIMUM_SEARCH_QUERY_PARTS, type Snowflake } from "../lib/protocol";

export const SEARCH_FILTER_KEYS = ["from", "mentions", "in", "has", "before", "after", "during"] as const;
export type SearchFilterKey = (typeof SEARCH_FILTER_KEYS)[number];
export const SEARCH_HAS_VALUES = ["link", "file", "image", "pinned"] as const;
export type SearchHasValue = (typeof SEARCH_HAS_VALUES)[number];
/** Keys whose value is a day, `YYYY-MM-DD` in the searcher's time zone. */
export const SEARCH_DATE_KEYS: readonly SearchFilterKey[] = ["before", "after", "during"];

const WHITESPACE = /\s/u;
const LETTER_OR_DIGIT = /[\p{L}\p{N}]/u;
const ISO_DAY = /^(\d{4})-(\d{2})-(\d{2})$/u;
const FIRST_SEARCHABLE_YEAR = 1970;
/** A filter value ends at whitespace or a quote: names holding either are named by id instead. */
const VALUE_THAT_STAYS_ONE_PART = /^[^\s"]+$/u;

export interface SearchFilter {
  key: SearchFilterKey;
  /** As typed after the colon, `@` and `#` included. */
  value: string;
}

/** One word, phrase or filter of a query, with its span in the text (UTF-16 units). */
export interface SearchQueryPart {
  start: number;
  end: number;
  /** Excluded with a leading `-`: never a filter. */
  isExcluded: boolean;
  isPhrase: boolean;
  /** The text searched: the word or phrase without its `-` and quotes. */
  text: string;
  filter: SearchFilter | null;
}

/** Splits `query` as the instance does: whitespace between parts, `"…"` phrases, `-` exclusions. */
export function searchQueryParts(query: string): SearchQueryPart[] {
  const parts: SearchQueryPart[] = [];
  let index = 0;
  while (index < query.length) {
    if (WHITESPACE.test(query.charAt(index))) {
      index += 1;
      continue;
    }
    const start = index;
    const isExcluded = query.charAt(index) === "-" && index + 1 < query.length && !WHITESPACE.test(query.charAt(index + 1));
    if (query.charAt(index) === "-") {
      index += 1;
    }
    if (query.charAt(index) === '"') {
      const closing = query.indexOf('"', index + 1);
      const end = closing === -1 ? query.length : closing + 1;
      parts.push({
        start,
        end,
        isExcluded,
        isPhrase: true,
        text: query.slice(index + 1, closing === -1 ? query.length : closing),
        filter: null,
      });
      index = end;
      continue;
    }
    const tokenStart = index;
    while (index < query.length && !WHITESPACE.test(query.charAt(index)) && query.charAt(index) !== '"') {
      index += 1;
    }
    const token = query.slice(tokenStart, index);
    parts.push({
      start,
      end: index,
      isExcluded,
      isPhrase: false,
      text: token,
      filter: isExcluded ? null : searchFilterOf(token),
    });
  }
  return parts;
}

/** `key:value` when the instance would apply it as a filter; null when it is searched as words. */
export function searchFilterOf(token: string): SearchFilter | null {
  const colon = token.indexOf(":");
  if (colon <= 0) {
    return null;
  }
  const key = token.slice(0, colon).toLowerCase();
  const value = token.slice(colon + 1);
  if (!isSearchFilterKey(key) || !isValidFilterValue(key, value)) {
    return null;
  }
  return { key, value };
}

function isSearchFilterKey(key: string): key is SearchFilterKey {
  return (SEARCH_FILTER_KEYS as readonly string[]).includes(key);
}

function isValidFilterValue(key: SearchFilterKey, value: string): boolean {
  switch (key) {
    case "from":
    case "mentions":
      return value.replace(/^@+/u, "") !== "";
    case "in":
      return value.replace(/^#+/u, "") !== "";
    case "has":
      return (SEARCH_HAS_VALUES as readonly string[]).includes(value.toLowerCase());
    case "before":
    case "after":
    case "during":
      return isSearchableDay(value);
  }
}

/** `YYYY-MM-DD`, a real day from 1970 on, as the instance accepts it. */
export function isSearchableDay(text: string): boolean {
  const match = ISO_DAY.exec(text);
  if (!match) {
    return false;
  }
  const [year, month, day] = [Number(match[1]), Number(match[2]), Number(match[3])];
  if (year < FIRST_SEARCHABLE_YEAR || month < 1 || month > 12 || day < 1) {
    return false;
  }
  // Day 0 of the next month is the last day of this one.
  return day <= new Date(Date.UTC(year, month, 0)).getUTCDate();
}

/** The local calendar day of `date` as `YYYY-MM-DD`. */
export function isoDayOf(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

export type SearchQueryProblem = "too-long" | "too-many-parts";

/** Why the instance would refuse `query`, checked before sending; null when it fits. */
export function searchQueryProblem(query: string): SearchQueryProblem | null {
  // The instance counts characters (Unicode scalar values), not UTF-16 units.
  if ([...query].length > MAXIMUM_SEARCH_QUERY_LENGTH) {
    return "too-long";
  }
  return searchQueryParts(query).length > MAXIMUM_SEARCH_QUERY_PARTS ? "too-many-parts" : null;
}

/** Whether a word, phrase or filter would be searched: text of punctuation alone finds nothing. */
export function hasSearchableContent(query: string): boolean {
  return searchQueryParts(query).some((part) => part.filter !== null || LETTER_OR_DIGIT.test(part.text));
}

/** Whether words are searched, so results can be ranked; filters alone always list newest first. */
export function hasSearchedWords(query: string): boolean {
  return searchQueryParts(query).some((part) => part.filter === null && !part.isExcluded && LETTER_OR_DIGIT.test(part.text));
}

/** The words and phrases to search, for the results' title ("pizza", "café au lait"). */
export function searchedWordsOf(query: string): string {
  return searchQueryParts(query)
    .filter((part) => part.filter === null)
    .map((part) => query.slice(part.start, part.end))
    .join(" ");
}

/** What the caret is in, for suggestions. */
export type SearchCaretContext =
  /** Between parts: every filter is offered. */
  | { kind: "between"; start: number; end: number }
  /** A word that may be the start of a filter's name ("fr" → "from:"). */
  | { kind: "key"; partial: string; start: number; end: number }
  /** After a known `key:`, the value typed so far. */
  | { kind: "value"; key: SearchFilterKey; partial: string; start: number; end: number }
  /** A phrase or an excluded word: nothing to suggest. */
  | { kind: "none" };

export function searchCaretContext(query: string, caret: number): SearchCaretContext {
  const part = searchQueryParts(query).find((candidate) => candidate.start < caret && caret <= candidate.end);
  if (!part) {
    return { kind: "between", start: caret, end: caret };
  }
  if (part.isPhrase || part.isExcluded) {
    return { kind: "none" };
  }
  const typed = query.slice(part.start, caret);
  const colon = typed.indexOf(":");
  if (colon === -1) {
    return LETTER_OR_DIGIT.test(typed) ? { kind: "key", partial: typed, start: part.start, end: part.end } : { kind: "none" };
  }
  const key = typed.slice(0, colon).toLowerCase();
  return isSearchFilterKey(key)
    ? { kind: "value", key, partial: typed.slice(colon + 1), start: part.start, end: part.end }
    : { kind: "none" };
}

/** Whether typing is still inside a filter's value: searching now would search half a name. */
export function isTypingFilterValue(query: string, caret: number): boolean {
  return searchCaretContext(query, caret).kind === "value";
}

/**
 * `query` with `start..end` replaced by `replacement`; a replacement ending a part (`addsSpace`)
 * gets one space after it unless one follows already. The caret goes after what was written.
 */
export function replaceInSearchQuery(
  query: string,
  start: number,
  end: number,
  replacement: string,
  addsSpace: boolean,
): { query: string; caret: number } {
  const before = query.slice(0, start);
  const after = query.slice(end);
  const needsLeadingSpace = before !== "" && !WHITESPACE.test(before.charAt(before.length - 1));
  const written = `${needsLeadingSpace ? " " : ""}${replacement}`;
  const space = addsSpace && !WHITESPACE.test(after.charAt(0)) ? " " : "";
  const caret = before.length + written.length + (addsSpace ? 1 : 0);
  return { query: `${before}${written}${space}${after}`, caret };
}

/** `query` with `key:value` at its end, unless it already holds that filter. */
export function withSearchFilter(query: string, key: SearchFilterKey, value: string): string {
  const lowered = value.toLowerCase();
  const alreadyThere = searchQueryParts(query).some(
    (part) => part.filter?.key === key && part.filter.value.toLowerCase() === lowered,
  );
  if (alreadyThere) {
    return query;
  }
  const trimmed = query.trimEnd();
  return `${trimmed}${trimmed === "" ? "" : " "}${key}:${value}`;
}

/** `query` without the part at `start..end`, and without the space it leaves behind. */
export function withoutSearchQueryPart(query: string, part: Pick<SearchQueryPart, "start" | "end">): string {
  return `${query.slice(0, part.start).trimEnd()} ${query.slice(part.end).trimStart()}`.trim();
}

/** The value `in:` names a channel with: its name when it stays one part, else its id. */
export function channelFilterValue(channel: { id: Snowflake; name: string }): string {
  return VALUE_THAT_STAYS_ONE_PART.test(channel.name) && !channel.name.startsWith("#") ? channel.name : channel.id;
}
