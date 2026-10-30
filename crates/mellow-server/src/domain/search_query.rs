//! The message search query language (ADR-0029, `mellow_protocol::MessageSearchQuery`), parsed
//! into FTS5 expressions made only of quoted strings, so nothing a user types is ever read as
//! FTS5 syntax, and into filters that storage binds as parameters.

pub(crate) use crate::storage::search::RequiredContents;
use mellow_protocol::{
    HighlightRange, MAXIMUM_REACHABLE_SEARCH_RESULTS, MAXIMUM_SEARCH_QUERY_LENGTH,
    MAXIMUM_SEARCH_QUERY_PARTS, MAXIMUM_SEARCH_RESULTS_PER_PAGE,
    MAXIMUM_TIMESTAMP_MILLISECONDS_SINCE_EPOCH, MELLOW_EPOCH_UNIX_MILLISECONDS, MessageSearchQuery,
    MessageSearchSort, Snowflake,
};

const MILLISECONDS_PER_DAY: i64 = 86_400_000;
const MILLISECONDS_PER_MINUTE: i64 = 60_000;
/// A word matches as a prefix ("mell" finds "Mellow") only when its last token is this long or
/// longer, the shortest prefix the index holds (`prefix='2 3'`): a one-character prefix would
/// walk every word of the index.
const MINIMUM_PREFIX_CHARACTERS: usize = 2;
/// Prefix terms (implicit or explicit, excluded ones included) per query; later words match
/// exactly. Each prefix term reads a range of the index, so a query of many short prefixes is the
/// most expensive one can write.
pub(crate) const MAXIMUM_PREFIX_TERMS: usize = 4;
/// Marks the highlighter puts around matched words. Messages cannot hold control characters
/// (`message_rules`), so these never collide with content.
pub(crate) const HIGHLIGHT_START_MARK: char = '\u{1}';
pub(crate) const HIGHLIGHT_END_MARK: char = '\u{2}';

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SearchQueryViolation {
    #[error(
        "search queries are at most {MAXIMUM_SEARCH_QUERY_LENGTH} characters and {MAXIMUM_SEARCH_QUERY_PARTS} words, phrases and filters"
    )]
    TooLarge,
    #[error(
        "search pages hold 1 to {MAXIMUM_SEARCH_RESULTS_PER_PAGE} results and reach {MAXIMUM_REACHABLE_SEARCH_RESULTS} results at most"
    )]
    Page,
    #[error(
        "the UTC offset is -{MAXIMUM_UTC_OFFSET_MINUTES} to {MAXIMUM_UTC_OFFSET_MINUTES} minutes"
    )]
    UtcOffset,
}

/// UTC-14:00 to UTC+14:00, the extremes in use.
const MAXIMUM_UTC_OFFSET_MINUTES: i16 = 840;

/// A search request, checked before anything is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchRequest {
    pub(crate) parsed: ParsedSearchQuery,
    pub(crate) sort: MessageSearchSort,
    pub(crate) offset: u32,
    pub(crate) limit: u32,
    pub(crate) utc_offset_minutes: i16,
}

/// # Errors
/// A `SearchQueryViolation` for an oversized query, a page out of range or an impossible offset.
pub(crate) fn validated_search_request(
    query: &MessageSearchQuery,
) -> Result<SearchRequest, SearchQueryViolation> {
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(MAXIMUM_SEARCH_RESULTS_PER_PAGE);
    if !(1..=MAXIMUM_SEARCH_RESULTS_PER_PAGE).contains(&limit)
        || offset.saturating_add(limit) > MAXIMUM_REACHABLE_SEARCH_RESULTS
    {
        return Err(SearchQueryViolation::Page);
    }
    let utc_offset_minutes = query.utc_offset_minutes.unwrap_or(0);
    if !(-MAXIMUM_UTC_OFFSET_MINUTES..=MAXIMUM_UTC_OFFSET_MINUTES).contains(&utc_offset_minutes) {
        return Err(SearchQueryViolation::UtcOffset);
    }
    Ok(SearchRequest {
        parsed: parse_search_query(&query.q)?,
        sort: query.sort.unwrap_or_default(),
        offset,
        limit,
        utc_offset_minutes,
    })
}

/// How a word matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WordMatch {
    Exact,
    /// A free word: exactly, or as the start of a longer word (exact words rank first).
    ExactOrPrefix,
    /// Typed with a trailing `*`.
    Prefix,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SearchTerm {
    Word { text: String, matching: WordMatch },
    Phrase(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ParsedSearchQuery {
    pub(crate) included_terms: Vec<SearchTerm>,
    pub(crate) excluded_terms: Vec<SearchTerm>,
    /// Lowercase, without `@`.
    pub(crate) author_usernames: Vec<String>,
    /// Lowercase, without `@`.
    pub(crate) mentioned_usernames: Vec<String>,
    /// As typed, without `#`: an id or a channel name, resolved by the caller.
    pub(crate) channel_references: Vec<String>,
    pub(crate) required_contents: RequiredContents,
    /// Days since 1970-01-01 in the searcher's time zone: messages from this day on.
    pub(crate) first_day: Option<i64>,
    /// Messages before this day.
    pub(crate) end_day: Option<i64>,
}

impl ParsedSearchQuery {
    /// Nothing to search for: no word and no filter.
    pub(crate) fn is_empty(&self) -> bool {
        self.included_terms.is_empty()
            && self.excluded_terms.is_empty()
            && self.author_usernames.is_empty()
            && self.mentioned_usernames.is_empty()
            && self.channel_references.is_empty()
            && self.required_contents == RequiredContents::default()
            && self.first_day.is_none()
            && self.end_day.is_none()
    }

    /// The FTS5 `MATCH` expression of the included words, minus the excluded ones; `None`
    /// without included words.
    pub(crate) fn match_expression(&self) -> Option<String> {
        if self.included_terms.is_empty() {
            return None;
        }
        let included = self
            .included_terms
            .iter()
            .map(included_term_expression)
            .collect::<Vec<_>>()
            .join(" AND ");
        Some(match self.exclusion_expression_of_terms() {
            Some(excluded) => format!("({included}) NOT ({excluded})"),
            None => included,
        })
    }

    /// Excluded words with no included word to subtract them from: messages matching this are
    /// left out by the caller. `None` when `match_expression` already excludes them.
    pub(crate) fn standalone_exclusion_expression(&self) -> Option<String> {
        if self.included_terms.is_empty() {
            self.exclusion_expression_of_terms()
        } else {
            None
        }
    }

    fn exclusion_expression_of_terms(&self) -> Option<String> {
        if self.excluded_terms.is_empty() {
            return None;
        }
        Some(
            self.excluded_terms
                .iter()
                .map(excluded_term_expression)
                .collect::<Vec<_>>()
                .join(" OR "),
        )
    }

    /// The ids bounding the date filters, `[lower, upper)`, for a searcher `utc_offset_minutes`
    /// east of UTC.
    pub(crate) fn id_bounds(
        &self,
        utc_offset_minutes: i16,
    ) -> (Option<Snowflake>, Option<Snowflake>) {
        let bound = |day: i64| first_id_of_day(day, utc_offset_minutes);
        (self.first_day.map(bound), self.end_day.map(bound))
    }
}

/// An FTS5 string: double quotes inside are doubled, so the text is only ever a phrase.
fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

fn included_term_expression(term: &SearchTerm) -> String {
    match term {
        SearchTerm::Word {
            text,
            matching: WordMatch::Prefix,
        } => format!("{}*", quoted(text)),
        SearchTerm::Word {
            text,
            matching: WordMatch::ExactOrPrefix,
        } => {
            // Both phrases match the exact word, so bm25 ranks it above words merely starting so.
            let quoted_text = quoted(text);
            format!("({quoted_text} OR {quoted_text}*)")
        }
        SearchTerm::Word {
            text,
            matching: WordMatch::Exact,
        }
        | SearchTerm::Phrase(text) => quoted(text),
    }
}

fn excluded_term_expression(term: &SearchTerm) -> String {
    match term {
        SearchTerm::Word {
            text,
            matching: WordMatch::Prefix,
        } => format!("{}*", quoted(text)),
        SearchTerm::Word { text, .. } | SearchTerm::Phrase(text) => quoted(text),
    }
}

/// Whether the last word the tokenizer finds in `text` is long enough to be a prefix: FTS5
/// applies `*` to it alone ("a.b*" is "a" then "b*").
fn ends_with_a_prefixable_token(text: &str) -> bool {
    text.split(|character: char| !character.is_alphanumeric())
        .rfind(|token| !token.is_empty())
        .is_some_and(|token| token.chars().count() >= MINIMUM_PREFIX_CHARACTERS)
}

/// Text the tokenizer would find no word in (only punctuation or emoji) matches nothing, so it
/// is dropped rather than searched.
fn holds_a_word(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

/// Parses the query language described on `mellow_protocol::MessageSearchQuery`.
///
/// # Errors
/// `TooLarge` past the length or part limits.
pub(crate) fn parse_search_query(input: &str) -> Result<ParsedSearchQuery, SearchQueryViolation> {
    if input.chars().count() > MAXIMUM_SEARCH_QUERY_LENGTH {
        return Err(SearchQueryViolation::TooLarge);
    }
    let mut parsed = ParsedSearchQuery::default();
    let mut part_count = 0;
    let mut prefix_term_count = 0;
    // Control characters separate words: messages hold none but newlines and tabs, and a NUL would
    // end the FTS5 expression early, inside a quoted string.
    let mut characters = input
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .peekable();
    loop {
        while characters
            .next_if(|character| character.is_whitespace())
            .is_some()
        {}
        let Some(&first) = characters.peek() else {
            break;
        };
        part_count += 1;
        if part_count > MAXIMUM_SEARCH_QUERY_PARTS {
            return Err(SearchQueryViolation::TooLarge);
        }
        let is_excluded = first == '-' && {
            characters.next();
            characters
                .peek()
                .is_some_and(|character| !character.is_whitespace())
        };
        if characters.next_if_eq(&'"').is_some() {
            let phrase: String = characters.by_ref().take_while(|&c| c != '"').collect();
            if holds_a_word(&phrase) {
                let term = SearchTerm::Phrase(phrase);
                push_term(&mut parsed, term, is_excluded);
            }
            continue;
        }
        let mut token = String::new();
        while let Some(character) =
            characters.next_if(|character| !character.is_whitespace() && *character != '"')
        {
            token.push(character);
        }
        if is_excluded || !apply_filter(&mut parsed, &token) {
            let text = token.trim_end_matches('*');
            if holds_a_word(text) {
                let wanted = match (text.len() < token.len(), is_excluded) {
                    (true, _) => WordMatch::Prefix,
                    (false, false) => WordMatch::ExactOrPrefix,
                    (false, true) => WordMatch::Exact,
                };
                let matching = if wanted != WordMatch::Exact
                    && prefix_term_count < MAXIMUM_PREFIX_TERMS
                    && ends_with_a_prefixable_token(text)
                {
                    prefix_term_count += 1;
                    wanted
                } else {
                    WordMatch::Exact
                };
                let term = SearchTerm::Word {
                    text: text.to_owned(),
                    matching,
                };
                push_term(&mut parsed, term, is_excluded);
            }
        }
    }
    Ok(parsed)
}

fn push_term(parsed: &mut ParsedSearchQuery, term: SearchTerm, is_excluded: bool) {
    if is_excluded {
        parsed.excluded_terms.push(term);
    } else {
        parsed.included_terms.push(term);
    }
}

/// Applies `key:value` when it is a known filter with a valid value; `false` otherwise, and the
/// token is then searched as words.
fn apply_filter(parsed: &mut ParsedSearchQuery, token: &str) -> bool {
    let Some((key, value)) = token.split_once(':') else {
        return false;
    };
    if value.is_empty() {
        return false;
    }
    match key.to_lowercase().as_str() {
        "from" | "mentions" => {
            let username = value.trim_start_matches('@').to_lowercase();
            if username.is_empty() {
                return false;
            }
            if key.eq_ignore_ascii_case("from") {
                parsed.author_usernames.push(username);
            } else {
                parsed.mentioned_usernames.push(username);
            }
            true
        }
        "in" => {
            let channel = value.trim_start_matches('#');
            if channel.is_empty() {
                return false;
            }
            parsed.channel_references.push(channel.to_owned());
            true
        }
        "has" => {
            let required = &mut parsed.required_contents;
            match value.to_lowercase().as_str() {
                "file" => required.has_file = true,
                "image" => required.has_image = true,
                "link" => required.has_link = true,
                "pinned" => required.is_pinned = true,
                _ => return false,
            }
            true
        }
        "before" | "after" | "during" => {
            let Some(day) = days_since_unix_epoch(value) else {
                return false;
            };
            let (first_day, end_day) = match key.to_lowercase().as_str() {
                "before" => (None, Some(day)),
                "after" => (Some(day + 1), None),
                _ => (Some(day), Some(day + 1)),
            };
            if let Some(first_day) = first_day {
                parsed.first_day = Some(parsed.first_day.map_or(first_day, |d| d.max(first_day)));
            }
            if let Some(end_day) = end_day {
                parsed.end_day = Some(parsed.end_day.map_or(end_day, |d| d.min(end_day)));
            }
            true
        }
        _ => false,
    }
}

/// `YYYY-MM-DD` (years 1970 to 9999) as days since 1970-01-01; `None` for anything else.
fn days_since_unix_epoch(text: &str) -> Option<i64> {
    let mut fields = text.split('-');
    let (year, month, day) = (fields.next()?, fields.next()?, fields.next()?);
    if fields.next().is_some()
        || year.len() != 4
        || month.len() != 2
        || day.len() != 2
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
    {
        return None;
    }
    let (year, month, day): (i64, i64, i64) =
        (year.parse().ok()?, month.parse().ok()?, day.parse().ok()?);
    let is_leap_year = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year => 29,
        2 => 28,
        _ => return None,
    };
    if year < 1970 || !(1..=days_in_month).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

/// Howard Hinnant's `days_from_civil`: days since 1970-01-01 of a proleptic Gregorian date.
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The smallest id of a message sent at the start of `day` in a time zone `utc_offset_minutes`
/// east of UTC, clamped to the range ids can hold.
fn first_id_of_day(day: i64, utc_offset_minutes: i16) -> Snowflake {
    let unix_milliseconds =
        day * MILLISECONDS_PER_DAY - i64::from(utc_offset_minutes) * MILLISECONDS_PER_MINUTE;
    let since_mellow_epoch =
        i128::from(unix_milliseconds) - i128::from(MELLOW_EPOCH_UNIX_MILLISECONDS);
    let clamped =
        since_mellow_epoch.clamp(0, i128::from(MAXIMUM_TIMESTAMP_MILLISECONDS_SINCE_EPOCH));
    // Clamped into the 42-bit range above, so it fits and `from_parts` accepts it.
    let milliseconds = u64::try_from(clamped).unwrap_or(0);
    Snowflake::from_parts(milliseconds, 0).unwrap_or(Snowflake::from_raw(0))
}

/// The matched spans of `content`, from the highlighter's copy of it with marks around each
/// match. Empty if that copy is not `content` with marks added (it should always be).
pub(crate) fn highlight_ranges(marked_content: &str, content: &str) -> Vec<HighlightRange> {
    let mut ranges = Vec::new();
    let mut unmarked = String::with_capacity(content.len());
    let mut position: u32 = 0;
    let mut start = None;
    for character in marked_content.chars() {
        match character {
            HIGHLIGHT_START_MARK => start = Some(position),
            HIGHLIGHT_END_MARK => {
                if let Some(start) = start.take()
                    && start < position
                {
                    match ranges.last_mut() {
                        // Adjacent matches (a word found by two phrases) read as one.
                        Some(HighlightRange { end, .. }) if *end == start => *end = position,
                        _ => ranges.push(HighlightRange {
                            start,
                            end: position,
                        }),
                    }
                }
            }
            _ => {
                unmarked.push(character);
                position = position
                    .saturating_add(u32::try_from(character.len_utf16()).unwrap_or(u32::MAX));
            }
        }
    }
    if unmarked == content {
        ranges
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str) -> SearchTerm {
        SearchTerm::Word {
            text: text.to_owned(),
            matching: WordMatch::ExactOrPrefix,
        }
    }

    fn exact(text: &str) -> SearchTerm {
        SearchTerm::Word {
            text: text.to_owned(),
            matching: WordMatch::Exact,
        }
    }

    #[test]
    fn words_phrases_exclusions_and_prefixes() {
        let parsed = parse_search_query(r#"pizza "hot dog" -olive -"bad day" mell* a"#).unwrap();
        assert_eq!(
            parsed.included_terms,
            [
                word("pizza"),
                SearchTerm::Phrase("hot dog".into()),
                SearchTerm::Word {
                    text: "mell".into(),
                    matching: WordMatch::Prefix
                },
                exact("a"),
            ]
        );
        assert_eq!(
            parsed.match_expression().as_deref(),
            Some(
                r#"(("pizza" OR "pizza"*) AND "hot dog" AND "mell"* AND "a") NOT ("olive" OR "bad day")"#
            )
        );
        assert_eq!(parsed.standalone_exclusion_expression(), None);
    }

    #[test]
    fn only_excluded_words_are_left_to_the_caller() {
        let parsed = parse_search_query("-spoiler from:ada").unwrap();
        assert_eq!(parsed.match_expression(), None);
        assert_eq!(
            parsed.standalone_exclusion_expression().as_deref(),
            Some(r#""spoiler""#)
        );
        assert_eq!(parsed.author_usernames, ["ada"]);
    }

    #[test]
    fn filters_are_read_and_unknown_or_invalid_ones_are_words() {
        let parsed = parse_search_query(
            "FROM:@Ada mentions:bob in:#general in:42 has:file has:IMAGE has:link has:pinned \
             has:poll before:2026-13-01 colour:red",
        )
        .unwrap();
        assert_eq!(parsed.author_usernames, ["ada"]);
        assert_eq!(parsed.mentioned_usernames, ["bob"]);
        assert_eq!(parsed.channel_references, ["general", "42"]);
        assert_eq!(
            parsed.required_contents,
            RequiredContents {
                has_file: true,
                has_image: true,
                has_link: true,
                is_pinned: true
            }
        );
        assert_eq!(
            parsed.included_terms,
            [
                word("has:poll"),
                word("before:2026-13-01"),
                word("colour:red")
            ]
        );
        assert_eq!(parsed.first_day, None);
    }

    #[test]
    fn dates_narrow_to_their_intersection() {
        let day = days_since_unix_epoch("2026-10-30").unwrap();
        let parsed =
            parse_search_query("after:2026-10-01 during:2026-10-30 before:2026-12-01").unwrap();
        assert_eq!(parsed.first_day, Some(day));
        assert_eq!(parsed.end_day, Some(day + 1));
        assert!(!parsed.is_empty());
    }

    #[test]
    fn calendar_days_are_counted_like_unix_time() {
        assert_eq!(days_since_unix_epoch("1970-01-01"), Some(0));
        assert_eq!(days_since_unix_epoch("2026-01-01"), Some(20_454));
        assert_eq!(days_since_unix_epoch("2024-02-29"), Some(19_782));
        assert_eq!(days_since_unix_epoch("2025-02-29"), None);
        assert_eq!(days_since_unix_epoch("1969-12-31"), None);
        assert_eq!(days_since_unix_epoch("2026-1-01"), None);
        assert_eq!(days_since_unix_epoch("+026-01-01"), None);
    }

    #[test]
    fn day_bounds_follow_the_searchers_time_zone() {
        let parsed = parse_search_query("during:2026-01-02").unwrap();
        let (lower, upper) = parsed.id_bounds(60);
        // Midnight in UTC+1 is 23:00 the day before in UTC: one day minus one hour after the epoch.
        assert_eq!(
            lower.unwrap().milliseconds_since_epoch(),
            86_400_000 - 3_600_000
        );
        assert_eq!(
            upper.unwrap().milliseconds_since_epoch(),
            2 * 86_400_000 - 3_600_000
        );
        // Days before Mellow's epoch clamp to the first id.
        let (lower, _) = parse_search_query("after:2020-01-01").unwrap().id_bounds(0);
        assert_eq!(lower, Some(Snowflake::from_raw(0)));
    }

    #[test]
    fn punctuation_and_emoji_alone_search_nothing() {
        let parsed = parse_search_query(r#"!!! 🎉 "" - "?""#).unwrap();
        assert!(parsed.is_empty(), "{parsed:?}");
        assert!(parse_search_query("   ").unwrap().is_empty());
    }

    #[test]
    fn control_characters_separate_words() {
        // A NUL inside the FTS5 expression would end it in the middle of a quoted string.
        let parsed = parse_search_query("has:link\0\"x\u{7}y").unwrap();
        assert_eq!(parsed.included_terms, [SearchTerm::Phrase("x y".into())]);
        assert!(parsed.required_contents.has_link);
    }

    #[test]
    fn quotes_inside_words_cannot_escape_their_string() {
        let parsed = parse_search_query(r#"a"b OR c"#).unwrap();
        assert_eq!(
            parsed.included_terms,
            [exact("a"), SearchTerm::Phrase("b OR c".into())]
        );
        assert_eq!(quoted(r#"say "hi""#), r#""say ""hi""""#);
    }

    #[test]
    fn oversized_queries_are_refused() {
        assert!(parse_search_query(&"a".repeat(MAXIMUM_SEARCH_QUERY_LENGTH)).is_ok());
        assert_eq!(
            parse_search_query(&"a".repeat(MAXIMUM_SEARCH_QUERY_LENGTH + 1)),
            Err(SearchQueryViolation::TooLarge)
        );
        let parts = vec!["w"; MAXIMUM_SEARCH_QUERY_PARTS];
        assert!(parse_search_query(&parts.join(" ")).is_ok());
        assert_eq!(
            parse_search_query(&format!("{} w", parts.join(" "))),
            Err(SearchQueryViolation::TooLarge)
        );
    }

    #[test]
    fn highlight_marks_become_utf16_ranges() {
        let content = "Café ☕ 😀 café";
        let marked = "\u{1}Café\u{2} ☕ 😀 \u{1}caf\u{2}\u{1}é\u{2}";
        assert_eq!(
            highlight_ranges(marked, content),
            [
                HighlightRange { start: 0, end: 4 },
                // The emoji takes two UTF-16 units.
                HighlightRange { start: 10, end: 14 },
            ]
        );
        assert_eq!(highlight_ranges("\u{1}other\u{2}", content), []);
    }

    #[test]
    fn pages_and_time_zones_are_bounded() {
        let query =
            |offset: Option<u32>, limit: Option<u32>, utc: Option<i16>| MessageSearchQuery {
                q: "hello".into(),
                sort: None,
                offset,
                limit,
                utc_offset_minutes: utc,
            };
        let defaults = validated_search_request(&query(None, None, None)).unwrap();
        assert_eq!(
            (
                defaults.offset,
                defaults.limit,
                defaults.sort,
                defaults.utc_offset_minutes
            ),
            (
                0,
                MAXIMUM_SEARCH_RESULTS_PER_PAGE,
                MessageSearchSort::Relevance,
                0
            )
        );
        let last_page = MAXIMUM_REACHABLE_SEARCH_RESULTS - MAXIMUM_SEARCH_RESULTS_PER_PAGE;
        assert!(validated_search_request(&query(Some(last_page), None, Some(-840))).is_ok());
        for refused in [
            query(Some(last_page + 1), None, None),
            query(None, Some(0), None),
            query(None, Some(MAXIMUM_SEARCH_RESULTS_PER_PAGE + 1), None),
            query(Some(u32::MAX), Some(1), None),
        ] {
            assert_eq!(
                validated_search_request(&refused),
                Err(SearchQueryViolation::Page)
            );
        }
        assert_eq!(
            validated_search_request(&query(None, None, Some(841))),
            Err(SearchQueryViolation::UtcOffset)
        );
    }

    #[test]
    fn prefixes_need_a_last_token_of_two_characters() {
        let parsed = parse_search_query("a* a.b* x. ab* mellow. -cd*").unwrap();
        assert_eq!(
            parsed.included_terms,
            [
                exact("a"),
                exact("a.b"),
                exact("x."),
                SearchTerm::Word {
                    text: "ab".into(),
                    matching: WordMatch::Prefix
                },
                word("mellow."),
            ]
        );
        assert_eq!(
            parsed.excluded_terms,
            [SearchTerm::Word {
                text: "cd".into(),
                matching: WordMatch::Prefix
            }]
        );
        assert_eq!(
            parsed.match_expression().as_deref(),
            Some(r#"("a" AND "a.b" AND "x." AND "ab"* AND ("mellow." OR "mellow."*)) NOT ("cd"*)"#)
        );
    }

    #[test]
    fn prefix_terms_are_capped_and_later_words_match_exactly() {
        let parsed = parse_search_query("one -two* three four five six*").unwrap();
        let matchings: Vec<WordMatch> = parsed
            .included_terms
            .iter()
            .chain(&parsed.excluded_terms)
            .map(|term| match term {
                SearchTerm::Word { matching, .. } => *matching,
                SearchTerm::Phrase(_) => WordMatch::Exact,
            })
            .collect();
        // Included: one, three, four, five, six; then excluded: two.
        assert_eq!(
            matchings,
            [
                WordMatch::ExactOrPrefix,
                WordMatch::ExactOrPrefix,
                WordMatch::ExactOrPrefix,
                WordMatch::Exact,
                WordMatch::Exact,
                WordMatch::Prefix,
            ]
        );
    }

    mod properties {
        use proptest::prelude::*;
        use sqlx::Connection as _;

        use super::super::*;

        /// Pieces chosen to look like FTS5 syntax and like the query language, plus any character.
        fn query_text() -> impl Strategy<Value = String> {
            let piece = prop_oneof![
                Just("\"".to_owned()),
                Just("\"\"".to_owned()),
                Just("-".to_owned()),
                Just("*".to_owned()),
                Just(" ".to_owned()),
                Just(":".to_owned()),
                Just("(".to_owned()),
                Just(")".to_owned()),
                Just("^".to_owned()),
                Just("+".to_owned()),
                Just("{content}".to_owned()),
                Just("AND".to_owned()),
                Just("OR".to_owned()),
                Just("NOT".to_owned()),
                Just("NEAR(".to_owned()),
                Just("from:".to_owned()),
                Just("in:#".to_owned()),
                Just("has:link".to_owned()),
                Just("during:2026-02-29".to_owned()),
                "[a-zé0-9]{1,6}",
                any::<char>().prop_map(String::from),
            ];
            proptest::collection::vec(piece, 0..40).prop_map(|pieces| pieces.concat())
        }

        proptest! {
            #[test]
            fn any_query_parses_or_is_refused_as_too_large(input in query_text()) {
                match parse_search_query(&input) {
                    Ok(parsed) => {
                        let terms = parsed.included_terms.len() + parsed.excluded_terms.len();
                        prop_assert!(terms <= MAXIMUM_SEARCH_QUERY_PARTS);
                    }
                    Err(SearchQueryViolation::TooLarge) => prop_assert!(
                        input.chars().count() > MAXIMUM_SEARCH_QUERY_LENGTH
                            || input.split_whitespace().count() > MAXIMUM_SEARCH_QUERY_PARTS
                            || input.contains('"')
                    ),
                    Err(other) => prop_assert!(false, "unexpected {other:?}"),
                }
            }
        }

        /// Every expression the parser builds is valid FTS5 for the index's own definition.
        #[test]
        fn every_parsed_expression_is_accepted_by_fts5() {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let connection = runtime.block_on(async {
                let mut connection = sqlx::SqliteConnection::connect("sqlite::memory:")
                    .await
                    .unwrap();
                sqlx::raw_sql(
                    "CREATE VIRTUAL TABLE messages_fts USING fts5(content, \
                     tokenize = 'unicode61 remove_diacritics 2', prefix = '2 3'); \
                     INSERT INTO messages_fts (content) VALUES ('Le café est prêt'), ('NEAR AND OR');",
                )
                .execute(&mut connection)
                .await
                .unwrap();
                std::cell::RefCell::new(connection)
            });
            let mut runner = proptest::test_runner::TestRunner::default();
            runner
                .run(&query_text(), |input| {
                    let Ok(parsed) = parse_search_query(&input) else {
                        return Ok(());
                    };
                    let expressions = [
                        parsed.match_expression(),
                        parsed.standalone_exclusion_expression(),
                    ];
                    for expression in expressions.into_iter().flatten() {
                        let outcome = runtime.block_on(
                            sqlx::query(
                                "SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?",
                            )
                            .bind(&expression)
                            .fetch_all(&mut *connection.borrow_mut()),
                        );
                        prop_assert!(
                            outcome.is_ok(),
                            "{input:?} gave {expression:?}: {outcome:?}"
                        );
                    }
                    Ok(())
                })
                .unwrap();
            runtime.block_on(connection.into_inner().close()).unwrap();
        }
    }
}
