// Search results as the side sheet lists them (DESIGN.md 5.12): labelled by channel where the
// channel changes, and facets that narrow the query when chosen.

import type { MessageSearchHit, PublicUser, Snowflake } from "../lib/protocol";
import { type SearchFilterKey, channelFilterValue, searchQueryParts, withSearchFilter } from "./search-query";

export interface SearchResultRow {
  hit: MessageSearchHit;
  /** The first result of a run from one channel: its label shows above it. */
  startsChannelRun: boolean;
}

/** Results in the instance's order; a channel label opens each run of results from one channel. */
export function searchResultRows(hits: readonly MessageSearchHit[]): SearchResultRow[] {
  return hits.map((hit, index) => ({
    hit,
    startsChannelRun: index === 0 || hits[index - 1]?.message.channel_id !== hit.message.channel_id,
  }));
}

/**
 * The query narrowed to a channel: by name in a server (when the name stays one part), by id for
 * a conversation, whose only name is its people's.
 */
export function queryNarrowedToChannel(query: string, channel: { id: Snowflake; name: string | null }): string {
  return withSearchFilter(query, "in", channel.name === null ? channel.id : channelFilterValue({ id: channel.id, name: channel.name }));
}

export function queryNarrowedToAuthor(query: string, author: Pick<PublicUser, "username">): string {
  return withSearchFilter(query, "from", author.username);
}

/** Whether the query already narrows by `key`: its facets then have nothing left to offer. */
export function isNarrowedBy(query: string, key: SearchFilterKey): boolean {
  return searchQueryParts(query).some((part) => part.filter?.key === key);
}
