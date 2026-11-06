//! The message search query language as the web client reads it
//! (`web/src/components/search-query.ts`, ADR-0029): the same parts the instance's parser finds
//! (`domain::search_query`), so suggestions, chips and limits agree with what will be searched.
//! The query text stays the single source of truth: chips and facets only add or remove parts of it.
//! Spans are byte offsets.

use mellow_protocol::{MAXIMUM_SEARCH_QUERY_LENGTH, MAXIMUM_SEARCH_QUERY_PARTS, Snowflake};

use super::format::CivilDate;
use super::unicode_classes::{
    character_at, character_before, is_js_whitespace, is_letter_or_number, js_trim, js_trim_end,
    js_trim_start,
};

const FIRST_SEARCHABLE_YEAR: i32 = 1970;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchFilterKey {
    From,
    Mentions,
    In,
    Has,
    Before,
    After,
    During,
}

impl SearchFilterKey {
    pub const ALL: [Self; 7] = [
        Self::From,
        Self::Mentions,
        Self::In,
        Self::Has,
        Self::Before,
        Self::After,
        Self::During,
    ];

    /// The key as the query writes it, without its colon.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::From => "from",
            Self::Mentions => "mentions",
            Self::In => "in",
            Self::Has => "has",
            Self::Before => "before",
            Self::After => "after",
            Self::During => "during",
        }
    }

    fn from_lowercase(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == key)
    }

    /// Filters whose value may be quoted, as names hold spaces (ADR-0048): `from:"Paul Carouge"`.
    fn takes_a_quoted_name(self) -> bool {
        matches!(self, Self::From | Self::Mentions)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchHasValue {
    Link,
    File,
    Image,
    Pinned,
}

impl SearchHasValue {
    pub const ALL: [Self; 4] = [Self::Link, Self::File, Self::Image, Self::Pinned];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Link => "link",
            Self::File => "file",
            Self::Image => "image",
            Self::Pinned => "pinned",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchFilter {
    pub key: SearchFilterKey,
    /// As typed after the colon, `@` and `#` included.
    pub value: String,
}

/// One word, phrase or filter of a query, with its span in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQueryPart {
    pub start: usize,
    pub end: usize,
    /// Excluded with a leading `-`: never a filter.
    pub is_excluded: bool,
    pub is_phrase: bool,
    /// The text searched: the word or phrase without its `-` and quotes.
    pub text: String,
    pub filter: Option<SearchFilter>,
}

fn find_quote_from(query: &str, from: usize) -> Option<usize> {
    query.get(from..)?.find('"').map(|offset| offset + from)
}

/// Splits `query` as the instance does: whitespace between parts, `"…"` phrases, `-` exclusions.
#[must_use]
pub fn search_query_parts(query: &str) -> Vec<SearchQueryPart> {
    let bytes = query.as_bytes();
    let mut parts = Vec::new();
    let mut index = 0;
    while let Some(character) = character_at(query, index) {
        if is_js_whitespace(character) {
            index += character.len_utf8();
            continue;
        }
        let start = index;
        let is_excluded = character == '-'
            && character_at(query, index + 1).is_some_and(|next| !is_js_whitespace(next));
        if character == '-' {
            index += 1;
        }
        if bytes.get(index) == Some(&b'"') {
            let closing = find_quote_from(query, index + 1);
            let end = closing.map_or(query.len(), |closing| closing + 1);
            let text = query
                .get(index + 1..closing.unwrap_or(query.len()))
                .unwrap_or_default()
                .to_owned();
            parts.push(SearchQueryPart {
                start,
                end,
                is_excluded,
                is_phrase: true,
                text,
                filter: None,
            });
            index = end;
            continue;
        }
        let token_start = index;
        index += query
            .get(index..)
            .unwrap_or_default()
            .find(|character: char| is_js_whitespace(character) || character == '"')
            .unwrap_or(query.len() - index);
        let token = query.get(token_start..index).unwrap_or_default();
        let is_quoted_name_key = token
            .strip_suffix(':')
            .and_then(|key| SearchFilterKey::from_lowercase(&key.to_lowercase()))
            .is_some_and(SearchFilterKey::takes_a_quoted_name);
        if !is_excluded && bytes.get(index) == Some(&b'"') && is_quoted_name_key {
            // As on the instance: the quoted name is the value; an empty one leaves the key a word.
            let closing = find_quote_from(query, index + 1);
            let end = closing.map_or(query.len(), |closing| closing + 1);
            let name = query
                .get(index + 1..closing.unwrap_or(query.len()))
                .unwrap_or_default();
            let whole = format!("{token}{name}");
            let filter = search_filter_of(&whole);
            let text = if filter.is_some() {
                whole
            } else {
                token.to_owned()
            };
            parts.push(SearchQueryPart {
                start,
                end,
                is_excluded,
                is_phrase: false,
                text,
                filter,
            });
            index = end;
            continue;
        }
        parts.push(SearchQueryPart {
            start,
            end: index,
            is_excluded,
            is_phrase: false,
            text: token.to_owned(),
            filter: if is_excluded {
                None
            } else {
                search_filter_of(token)
            },
        });
    }
    parts
}

/// `key:value` when the instance would apply it as a filter; `None` when it is searched as words.
#[must_use]
pub fn search_filter_of(token: &str) -> Option<SearchFilter> {
    let colon = token.find(':').filter(|colon| *colon > 0)?;
    let key = SearchFilterKey::from_lowercase(&token.get(..colon)?.to_lowercase())?;
    let value = token.get(colon + 1..)?;
    is_valid_filter_value(key, value).then(|| SearchFilter {
        key,
        value: value.to_owned(),
    })
}

fn is_valid_filter_value(key: SearchFilterKey, value: &str) -> bool {
    match key {
        SearchFilterKey::From | SearchFilterKey::Mentions => {
            !js_trim(value.trim_start_matches('@')).is_empty()
        }
        SearchFilterKey::In => !value.trim_start_matches('#').is_empty(),
        SearchFilterKey::Has => SearchHasValue::ALL
            .iter()
            .any(|has| has.as_str() == value.to_lowercase()),
        SearchFilterKey::Before | SearchFilterKey::After | SearchFilterKey::During => {
            is_searchable_day(value)
        }
    }
}

/// `YYYY-MM-DD`, a real day from 1970 on, as the instance accepts it.
#[must_use]
pub fn is_searchable_day(text: &str) -> bool {
    CivilDate::from_iso_day(text).is_some_and(|day| day.year >= FIRST_SEARCHABLE_YEAR)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchQueryProblem {
    TooLong,
    TooManyParts,
}

/// Why the instance would refuse `query`, checked before sending; `None` when it fits.
#[must_use]
pub fn search_query_problem(query: &str) -> Option<SearchQueryProblem> {
    // The instance counts characters (Unicode scalar values), not bytes.
    if query.chars().count() > MAXIMUM_SEARCH_QUERY_LENGTH {
        return Some(SearchQueryProblem::TooLong);
    }
    (search_query_parts(query).len() > MAXIMUM_SEARCH_QUERY_PARTS)
        .then_some(SearchQueryProblem::TooManyParts)
}

fn has_letter_or_digit(text: &str) -> bool {
    text.chars().any(is_letter_or_number)
}

/// Whether a word, phrase or filter would be searched: text of punctuation alone finds nothing.
#[must_use]
pub fn has_searchable_content(query: &str) -> bool {
    search_query_parts(query)
        .iter()
        .any(|part| part.filter.is_some() || has_letter_or_digit(&part.text))
}

/// Whether words are searched, so results can be ranked; filters alone always list newest first.
#[must_use]
pub fn has_searched_words(query: &str) -> bool {
    search_query_parts(query)
        .iter()
        .any(|part| part.filter.is_none() && !part.is_excluded && has_letter_or_digit(&part.text))
}

/// The words and phrases to search, for the results' title ("pizza", "café au lait").
#[must_use]
pub fn searched_words_of(query: &str) -> String {
    search_query_parts(query)
        .iter()
        .filter(|part| part.filter.is_none())
        .filter_map(|part| query.get(part.start..part.end))
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the caret is in, for suggestions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchCaretContext {
    /// Between parts: every filter is offered.
    Between { start: usize, end: usize },
    /// A word that may be the start of a filter's name ("fr" → "from:").
    Key {
        partial: String,
        start: usize,
        end: usize,
    },
    /// After a known `key:`, the value typed so far.
    Value {
        key: SearchFilterKey,
        partial: String,
        start: usize,
        end: usize,
    },
    /// A phrase or an excluded word: nothing to suggest.
    None,
}

#[must_use]
pub fn search_caret_context(query: &str, caret: usize) -> SearchCaretContext {
    let parts = search_query_parts(query);
    let Some(part) = parts
        .iter()
        .find(|candidate| candidate.start < caret && caret <= candidate.end)
    else {
        return SearchCaretContext::Between {
            start: caret,
            end: caret,
        };
    };
    if part.is_phrase || part.is_excluded {
        return SearchCaretContext::None;
    }
    let Some(typed) = query.get(part.start..caret) else {
        return SearchCaretContext::None;
    };
    let Some(colon) = typed.find(':') else {
        return if has_letter_or_digit(typed) {
            SearchCaretContext::Key {
                partial: typed.to_owned(),
                start: part.start,
                end: part.end,
            }
        } else {
            SearchCaretContext::None
        };
    };
    match typed
        .get(..colon)
        .and_then(|key| SearchFilterKey::from_lowercase(&key.to_lowercase()))
    {
        Some(key) => {
            let value = typed.get(colon + 1..).unwrap_or_default();
            let partial = value.strip_prefix('"').unwrap_or(value).to_owned();
            SearchCaretContext::Value {
                key,
                partial,
                start: part.start,
                end: part.end,
            }
        }
        None => SearchCaretContext::None,
    }
}

/// Whether typing is still inside a filter's value: searching now would search half a name.
#[must_use]
pub fn is_typing_filter_value(query: &str, caret: usize) -> bool {
    matches!(
        search_caret_context(query, caret),
        SearchCaretContext::Value { .. }
    )
}

/// `query` with `start..end` replaced by `replacement`, and the caret after what was written; a
/// replacement ending a part (`adds_space`) gets one space after it unless one follows already.
/// `None` when the range is not one of `query`'s.
#[must_use]
pub fn replace_in_search_query(
    query: &str,
    start: usize,
    end: usize,
    replacement: &str,
    adds_space: bool,
) -> Option<(String, usize)> {
    let before = query.get(..start)?;
    let after = query.get(end..)?;
    let needs_leading_space =
        character_before(before, before.len()).is_some_and(|last| !is_js_whitespace(last));
    let written = format!(
        "{}{replacement}",
        if needs_leading_space { " " } else { "" }
    );
    let space = if adds_space && !character_at(after, 0).is_some_and(is_js_whitespace) {
        " "
    } else {
        ""
    };
    let caret = before.len() + written.len() + usize::from(adds_space);
    Some((format!("{before}{written}{space}{after}"), caret))
}

/// `query` with `key:value` at its end, unless it already holds that filter.
#[must_use]
pub fn with_search_filter(query: &str, key: SearchFilterKey, value: &str) -> String {
    let lowered = value.to_lowercase();
    let is_already_there = search_query_parts(query).iter().any(|part| {
        part.filter
            .as_ref()
            .is_some_and(|filter| filter.key == key && filter.value.to_lowercase() == lowered)
    });
    if is_already_there {
        return query.to_owned();
    }
    let trimmed = js_trim_end(query);
    let separator = if trimmed.is_empty() { "" } else { " " };
    format!("{trimmed}{separator}{}", search_filter_text(key, value))
}

/// A filter value ends at whitespace or a quote: names holding either are named by id instead.
fn stays_one_part(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(|character: char| is_js_whitespace(character) || character == '"')
}

/// `key:value` as the query holds it: a value with spaces quoted (names, ADR-0048).
#[must_use]
pub fn search_filter_text(key: SearchFilterKey, value: &str) -> String {
    if stays_one_part(value) {
        format!("{}:{value}", key.as_str())
    } else {
        format!("{}:\"{value}\"", key.as_str())
    }
}

/// `query` without the part at `start..end`, and without the space it leaves behind.
#[must_use]
pub fn without_search_query_part(query: &str, start: usize, end: usize) -> String {
    let before = js_trim_end(query.get(..start).unwrap_or_default());
    let after = js_trim_start(query.get(end..).unwrap_or_default());
    js_trim(&format!("{before} {after}")).to_owned()
}

/// The value `in:` names a channel with: its name when it stays one part, else its id.
#[must_use]
pub fn channel_filter_value(id: Snowflake, name: &str) -> String {
    if stays_one_part(name) && !name.starts_with('#') {
        name.to_owned()
    } else {
        id.raw().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::unnecessary_wraps)] // Compared with the parts' optional filter.
    fn filter(key: SearchFilterKey, value: &str) -> Option<SearchFilter> {
        Some(SearchFilter {
            key,
            value: value.to_owned(),
        })
    }

    #[test]
    fn finds_words_phrases_exclusions_and_filters_as_the_instance_does() {
        let query = "from:ada \"café au lait\" -pizza in:#general has:LINK piz* -\"no way\" before:2026-02-30 mentions:";
        let parts: Vec<_> = search_query_parts(query)
            .into_iter()
            .map(|part| (part.text, part.is_excluded, part.is_phrase, part.filter))
            .collect();
        assert_eq!(
            parts,
            vec![
                (
                    "from:ada".into(),
                    false,
                    false,
                    filter(SearchFilterKey::From, "ada")
                ),
                ("café au lait".into(), false, true, None),
                ("pizza".into(), true, false, None),
                (
                    "in:#general".into(),
                    false,
                    false,
                    filter(SearchFilterKey::In, "#general")
                ),
                (
                    "has:LINK".into(),
                    false,
                    false,
                    filter(SearchFilterKey::Has, "LINK")
                ),
                ("piz*".into(), false, false, None),
                ("no way".into(), true, true, None),
                // Not a real day, and an empty value: both searched as words, as on the instance.
                ("before:2026-02-30".into(), false, false, None),
                ("mentions:".into(), false, false, None),
            ]
        );
    }

    #[test]
    fn ends_a_word_at_a_quote_and_keeps_an_unclosed_phrase_to_the_end() {
        let parts: Vec<_> = search_query_parts("foo\"bar baz")
            .into_iter()
            .map(|part| (part.text, part.is_phrase))
            .collect();
        assert_eq!(parts, vec![("foo".into(), false), ("bar baz".into(), true)]);
    }

    #[test]
    fn knows_the_instances_limits_before_sending() {
        assert_eq!(search_query_problem(&"a ".repeat(16)), None);
        assert_eq!(
            search_query_problem(&"a ".repeat(17)),
            Some(SearchQueryProblem::TooManyParts)
        );
        assert_eq!(search_query_problem(&"é".repeat(256)), None);
        assert_eq!(search_query_problem(&"😀".repeat(129)), None);
        assert_eq!(
            search_query_problem(&"x".repeat(257)),
            Some(SearchQueryProblem::TooLong)
        );
    }

    #[test]
    fn tells_searchable_text_from_punctuation_and_words_from_filters() {
        assert!(!has_searchable_content("  ?! "));
        assert!(has_searchable_content("has:file"));
        assert!(!has_searched_words("has:file from:ada"));
        assert!(has_searched_words("has:file pizza"));
        assert!(!has_searched_words("-pizza"));
        assert_eq!(
            searched_words_of("from:ada pizza \"au lait\" in:general"),
            "pizza \"au lait\""
        );
    }

    #[test]
    fn offers_filters_between_parts_a_filters_name_then_its_values() {
        assert_eq!(
            search_caret_context("", 0),
            SearchCaretContext::Between { start: 0, end: 0 }
        );
        assert_eq!(
            search_caret_context("pizza ", 6),
            SearchCaretContext::Between { start: 6, end: 6 }
        );
        assert_eq!(
            search_caret_context("pizza fr", 8),
            SearchCaretContext::Key {
                partial: "fr".into(),
                start: 6,
                end: 8
            }
        );
        assert_eq!(
            search_caret_context("pizza FROM:ad", 13),
            SearchCaretContext::Value {
                key: SearchFilterKey::From,
                partial: "ad".into(),
                start: 6,
                end: 13
            }
        );
        assert_eq!(
            search_caret_context("\"from:a", 7),
            SearchCaretContext::None
        );
        assert_eq!(search_caret_context("-from:a", 7), SearchCaretContext::None);
        assert_eq!(
            search_caret_context("color:red", 9),
            SearchCaretContext::None
        );
        assert!(is_typing_filter_value("pizza from:a", 12));
        assert!(!is_typing_filter_value("pizza from:ada ", 15));
    }

    #[test]
    fn writes_a_suggestion_in_place_of_the_part_with_a_space_after_a_finished_filter() {
        assert_eq!(
            replace_in_search_query("pizza fr", 6, 8, "from:", false),
            Some(("pizza from:".into(), 11))
        );
        assert_eq!(
            replace_in_search_query("pizza from:a", 6, 12, "from:ada", true),
            Some(("pizza from:ada ".into(), 15))
        );
        assert_eq!(
            replace_in_search_query("from:a pizza", 0, 6, "from:ada", true),
            Some(("from:ada pizza".into(), 9))
        );
        assert_eq!(
            replace_in_search_query("pizza", 5, 5, "has:", false),
            Some(("pizza has:".into(), 10))
        );
    }

    #[test]
    fn adds_a_facets_filter_once_and_removes_a_chips_part_with_its_space() {
        assert_eq!(
            with_search_filter("pizza ", SearchFilterKey::From, "ada"),
            "pizza from:ada"
        );
        assert_eq!(
            with_search_filter("pizza from:Ada", SearchFilterKey::From, "ada"),
            "pizza from:Ada"
        );
        assert_eq!(
            with_search_filter("", SearchFilterKey::In, "general"),
            "in:general"
        );
        let query = "pizza from:ada  has:link";
        let parts = search_query_parts(query);
        let from = parts
            .get(1)
            .map(|part| without_search_query_part(query, part.start, part.end));
        assert_eq!(from.as_deref(), Some("pizza has:link"));
    }

    #[test]
    fn quotes_names_holding_spaces_and_reads_them_back_as_one_filter() {
        let query = with_search_filter("pizza", SearchFilterKey::From, "Paul Carouge");
        assert_eq!(query, "pizza from:\"Paul Carouge\"");
        assert_eq!(
            with_search_filter(&query, SearchFilterKey::From, "paul carouge"),
            query
        );
        let parts: Vec<_> = search_query_parts(&format!("{query} mentions:\"\" hi"))
            .into_iter()
            .map(|part| (part.text, part.filter))
            .collect();
        assert_eq!(
            parts,
            vec![
                ("pizza".into(), None),
                (
                    "from:Paul Carouge".into(),
                    filter(SearchFilterKey::From, "Paul Carouge")
                ),
                ("mentions:".into(), None),
                ("hi".into(), None),
            ]
        );
        assert_eq!(
            search_caret_context("from:\"Paul C", 12),
            SearchCaretContext::Value {
                key: SearchFilterKey::From,
                partial: "Paul C".into(),
                start: 0,
                end: 12
            }
        );
    }

    #[test]
    fn names_a_channel_by_id_when_its_name_would_not_stay_one_part() {
        let id = Snowflake::from_raw(7);
        assert_eq!(channel_filter_value(id, "général"), "général");
        assert_eq!(channel_filter_value(id, "Game night"), "7");
        assert_eq!(channel_filter_value(id, "say \"hi\""), "7");
        assert_eq!(channel_filter_value(id, "#hash"), "7");
    }
}
