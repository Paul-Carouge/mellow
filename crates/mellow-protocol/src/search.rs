//! Message search (ADR-0029): `GET /api/v1/guilds/{guild_id}/messages/search` searches the guild
//! channels the requester can read, `GET /api/v1/users/@me/messages/search` their conversations.

use serde::{Deserialize, Serialize};

use crate::{Message, PublicUser, Snowflake};

/// Results per page at most, and by default.
pub const MAXIMUM_SEARCH_RESULTS_PER_PAGE: u32 = 25;
/// Matches past this many are not counted: `total_count` stops there and
/// `total_count_is_lower_bound` is set, so clients show "1 000+". Facets count the newest this
/// many matches.
pub const MAXIMUM_COUNTED_SEARCH_MATCHES: u32 = 1_000;
/// Pages reach this many results at most (`offset + limit`), as on Discord: refine the query
/// rather than paging further.
pub const MAXIMUM_REACHABLE_SEARCH_RESULTS: u32 = 1_000;
/// The query text, in characters.
pub const MAXIMUM_SEARCH_QUERY_LENGTH: usize = 256;
/// Words, phrases and filters in one query.
pub const MAXIMUM_SEARCH_QUERY_PARTS: usize = 16;
/// Entries in each facet list, the largest counts first.
pub const MAXIMUM_SEARCH_FACETS: usize = 10;

/// Query of both search routes.
///
/// `q` is the query language: free words (all must match, each also as a word prefix, exact
/// words ranked first), `"exact phrase"`, `-excluded` (word or phrase), `word*` (explicit prefix),
/// and the filters `from:username`, `mentions:username`, `in:channel` (an id, or in a guild a
/// name with or without `#`), `has:file`, `has:image`, `has:link`, `has:pinned`,
/// `before:YYYY-MM-DD`, `after:YYYY-MM-DD` and `during:YYYY-MM-DD`. Repeated `from:`,
/// `mentions:` and `in:` filters mean any of them. Anything else, an unknown filter included, is
/// searched as words. Case and accents are ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSearchQuery {
    pub q: String,
    /// Defaults to `relevance`.
    pub sort: Option<MessageSearchSort>,
    /// Results to skip; `offset + limit` stays within `MAXIMUM_REACHABLE_SEARCH_RESULTS`.
    pub offset: Option<u32>,
    /// 1 to `MAXIMUM_SEARCH_RESULTS_PER_PAGE`, which is the default.
    pub limit: Option<u32>,
    /// The searcher's offset from UTC (minutes east, e.g. 60 for Paris in winter), so `before:`,
    /// `after:` and `during:` days are theirs. Defaults to 0 (UTC); -840 to 840.
    pub utc_offset_minutes: Option<i16>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageSearchSort {
    /// Best matches first (bm25), newest first among equals. Without words, same as `newest`.
    #[default]
    Relevance,
    Newest,
}

/// Answer of both search routes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSearchResults {
    /// The page, in the requested order.
    pub hits: Vec<MessageSearchHit>,
    /// Matches in the whole scope, counted up to `MAXIMUM_COUNTED_SEARCH_MATCHES`.
    pub total_count: u32,
    /// More matches exist than `total_count` says.
    pub total_count_is_lower_bound: bool,
    /// Matches per channel or conversation, among the counted (newest) ones; the largest first.
    pub channel_facets: Vec<ChannelSearchFacet>,
    /// Matches per author, among the counted (newest) ones; the largest first.
    pub author_facets: Vec<AuthorSearchFacet>,
}

/// One matching message. Blocked authors' messages are included like any other: clients fold
/// them as they do in the channel (ADR-0025).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSearchHit {
    /// With its attachments and the requester's reactions, as in history pages.
    pub message: Message,
    /// The matched words in `message.content`, in order and never overlapping; empty for a query
    /// made of filters only.
    pub highlights: Vec<HighlightRange>,
}

/// A span of `Message::content` in UTF-16 code units, the unit of JavaScript string indices:
/// `content.slice(start, end)`. No markup comes from the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighlightRange {
    pub start: u32,
    /// Exclusive.
    pub end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelSearchFacet {
    pub channel_id: Snowflake,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorSearchFacet {
    pub author: PublicUser,
    pub count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_defaults_and_wire_names() {
        let query: MessageSearchQuery =
            serde_json::from_str(r#"{"q":"hello","sort":"newest","utc_offset_minutes":-300}"#)
                .unwrap();
        assert_eq!(query.sort, Some(MessageSearchSort::Newest));
        assert_eq!(query.utc_offset_minutes, Some(-300));
        assert_eq!(query.offset, None);

        let range = serde_json::to_value(HighlightRange { start: 3, end: 8 }).unwrap();
        assert_eq!(range, serde_json::json!({"start": 3, "end": 8}));
        let facet = serde_json::to_value(ChannelSearchFacet {
            channel_id: Snowflake::from_raw(7),
            count: 2,
        })
        .unwrap();
        assert_eq!(facet, serde_json::json!({"channel_id": "7", "count": 2}));
    }
}
