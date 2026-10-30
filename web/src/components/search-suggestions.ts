// Filter suggestions under the search field (DESIGN.md 5.12): the filters between words, a
// filter's name from its first letters, then its values (people, channels, kinds, days).

import type { PublicUser, Snowflake } from "../lib/protocol";
import { foldForSearch } from "../lib/search-text";
import { displayName } from "./format";
import {
  SEARCH_FILTER_KEYS,
  SEARCH_HAS_VALUES,
  type SearchCaretContext,
  type SearchFilterKey,
  type SearchHasValue,
  isSearchableDay,
  isoDayOf,
} from "./search-query";

/** People and channels listed at most, so the list stays a glance. */
export const MAXIMUM_SEARCH_VALUE_SUGGESTIONS = 8;
const DAYS_IN_A_WEEK = 7;
const DAYS_IN_A_MONTH = 30;

export type RelativeDay = "today" | "yesterday" | "a-week-ago" | "a-month-ago" | "typed";

export type SearchSuggestion =
  | { kind: "key"; key: SearchFilterKey }
  | { kind: "person"; key: "from" | "mentions"; user: PublicUser }
  /** A server channel by name, or a conversation (named by id: `in:` knows conversations by id only). */
  | { kind: "channel"; channelId: Snowflake; name: string; isConversation: boolean; value: string }
  | { kind: "has"; value: SearchHasValue }
  | { kind: "day"; key: SearchFilterKey; day: string; relativeDay: RelativeDay };

export interface SearchSuggestionSources {
  people: readonly PublicUser[];
  channels: readonly { id: Snowflake; name: string; isConversation: boolean; value: string }[];
  today: Date;
}

export function searchSuggestions(context: SearchCaretContext, sources: SearchSuggestionSources): SearchSuggestion[] {
  switch (context.kind) {
    case "none":
      return [];
    case "between":
      return SEARCH_FILTER_KEYS.map((key) => ({ kind: "key", key }));
    case "key": {
      const partial = context.partial.toLowerCase();
      return SEARCH_FILTER_KEYS.filter((key) => key.startsWith(partial)).map((key) => ({ kind: "key", key }));
    }
    case "value":
      return valueSuggestions(context.key, context.partial, sources);
  }
}

function valueSuggestions(key: SearchFilterKey, partial: string, sources: SearchSuggestionSources): SearchSuggestion[] {
  switch (key) {
    case "from":
    case "mentions": {
      const term = foldForSearch(partial.replace(/^@+/u, ""));
      return rankByName(
        sources.people,
        (person) => [person.username, displayName(person)],
        term,
      ).map((user) => ({ kind: "person", key, user }));
    }
    case "in": {
      const term = foldForSearch(partial.replace(/^#+/u, ""));
      return rankByName(sources.channels, (channel) => [channel.name], term).map((channel) => ({
        kind: "channel",
        channelId: channel.id,
        name: channel.name,
        isConversation: channel.isConversation,
        value: channel.value,
      }));
    }
    case "has": {
      const lowered = partial.toLowerCase();
      return SEARCH_HAS_VALUES.filter((value) => value.startsWith(lowered)).map((value) => ({ kind: "has", value }));
    }
    case "before":
    case "after":
    case "during":
      return daySuggestions(key, partial, sources.today);
  }
}

/** Names starting with the term first, then names containing it; at most the first eight. */
function rankByName<T>(candidates: readonly T[], namesOf: (candidate: T) => string[], term: string): T[] {
  const scored = candidates.flatMap((candidate) => {
    const names = namesOf(candidate).map(foldForSearch);
    if (names.some((name) => name.startsWith(term))) {
      return [{ candidate, rank: 0 }];
    }
    return names.some((name) => name.includes(term)) ? [{ candidate, rank: 1 }] : [];
  });
  return scored
    .sort((left, right) => left.rank - right.rank)
    .slice(0, MAXIMUM_SEARCH_VALUE_SUGGESTIONS)
    .map(({ candidate }) => candidate);
}

function daySuggestions(key: SearchFilterKey, partial: string, today: Date): SearchSuggestion[] {
  const daysAgo = (days: number) => {
    const day = new Date(today);
    day.setDate(day.getDate() - days);
    return isoDayOf(day);
  };
  const candidates: { day: string; relativeDay: RelativeDay }[] = [
    { day: isoDayOf(today), relativeDay: "today" },
    { day: daysAgo(1), relativeDay: "yesterday" },
    { day: daysAgo(DAYS_IN_A_WEEK), relativeDay: "a-week-ago" },
    { day: daysAgo(DAYS_IN_A_MONTH), relativeDay: "a-month-ago" },
  ];
  const matching = candidates.filter((candidate) => candidate.day.startsWith(partial));
  if (isSearchableDay(partial) && !matching.some((candidate) => candidate.day === partial)) {
    matching.unshift({ day: partial, relativeDay: "typed" });
  }
  return matching.map((candidate) => ({ kind: "day", key, ...candidate }));
}

/** What choosing a suggestion writes, and whether the part ends there (a space follows). */
export function suggestionInsertion(suggestion: SearchSuggestion): { text: string; endsPart: boolean } {
  switch (suggestion.kind) {
    case "key":
      return { text: `${suggestion.key}:`, endsPart: false };
    case "person":
      return { text: `${suggestion.key}:${suggestion.user.username}`, endsPart: true };
    case "channel":
      return { text: `in:${suggestion.value}`, endsPart: true };
    case "has":
      return { text: `has:${suggestion.value}`, endsPart: true };
    case "day":
      return { text: `${suggestion.key}:${suggestion.day}`, endsPart: true };
  }
}
