//! Filter suggestions under the search field (`web/src/components/search-suggestions.ts`,
//! DESIGN.md 5.12): the filters between words, a filter's name from its first letters, then its
//! values (people, channels, kinds, days).

use mellow_protocol::{PublicUser, Snowflake};

use super::format::{CivilDate, display_name};
use super::search_query::{
    SearchCaretContext, SearchFilterKey, SearchHasValue, is_searchable_day, search_filter_text,
};
use super::search_text::fold_for_search;

/// People and channels listed at most, so the list stays a glance.
pub const MAXIMUM_SEARCH_VALUE_SUGGESTIONS: usize = 8;
const DAYS_IN_A_WEEK: i64 = 7;
const DAYS_IN_A_MONTH: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelativeDay {
    Today,
    Yesterday,
    AWeekAgo,
    AMonthAgo,
    Typed,
}

/// A channel the `in:` filter may name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchChannelSource {
    pub id: Snowflake,
    pub name: String,
    /// A conversation, named by id: `in:` knows conversations by id only.
    pub is_conversation: bool,
    /// What `in:` writes for it ([`super::search_query::channel_filter_value`]).
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchSuggestion<'a> {
    Key(SearchFilterKey),
    /// `key` is `From` or `Mentions`.
    Person {
        key: SearchFilterKey,
        user: &'a PublicUser,
    },
    Channel(&'a SearchChannelSource),
    Has(SearchHasValue),
    Day {
        key: SearchFilterKey,
        day: String,
        relative_day: RelativeDay,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct SearchSuggestionSources<'a> {
    pub people: &'a [PublicUser],
    pub channels: &'a [SearchChannelSource],
    /// The reader's local day.
    pub today: CivilDate,
}

#[must_use]
pub fn search_suggestions<'a>(
    context: &SearchCaretContext,
    sources: SearchSuggestionSources<'a>,
) -> Vec<SearchSuggestion<'a>> {
    match context {
        SearchCaretContext::None => Vec::new(),
        SearchCaretContext::Between { .. } => SearchFilterKey::ALL
            .into_iter()
            .map(SearchSuggestion::Key)
            .collect(),
        SearchCaretContext::Key { partial, .. } => {
            let partial = partial.to_lowercase();
            SearchFilterKey::ALL
                .into_iter()
                .filter(|key| key.as_str().starts_with(&partial))
                .map(SearchSuggestion::Key)
                .collect()
        }
        SearchCaretContext::Value { key, partial, .. } => value_suggestions(*key, partial, sources),
    }
}

fn value_suggestions<'a>(
    key: SearchFilterKey,
    partial: &str,
    sources: SearchSuggestionSources<'a>,
) -> Vec<SearchSuggestion<'a>> {
    match key {
        SearchFilterKey::From | SearchFilterKey::Mentions => {
            let term = fold_for_search(partial.trim_start_matches('@'));
            rank_by_name(
                sources.people,
                |person| vec![person.username.as_str(), display_name(person)],
                &term,
            )
            .into_iter()
            .map(|user| SearchSuggestion::Person { key, user })
            .collect()
        }
        SearchFilterKey::In => {
            let term = fold_for_search(partial.trim_start_matches('#'));
            rank_by_name(
                sources.channels,
                |channel| vec![channel.name.as_str()],
                &term,
            )
            .into_iter()
            .map(SearchSuggestion::Channel)
            .collect()
        }
        SearchFilterKey::Has => {
            let lowered = partial.to_lowercase();
            SearchHasValue::ALL
                .into_iter()
                .filter(|value| value.as_str().starts_with(&lowered))
                .map(SearchSuggestion::Has)
                .collect()
        }
        SearchFilterKey::Before | SearchFilterKey::After | SearchFilterKey::During => {
            day_suggestions(key, partial, sources.today)
        }
    }
}

/// Names starting with the term first, then names containing it; at most the first eight.
fn rank_by_name<'a, T>(
    candidates: &'a [T],
    names_of: impl Fn(&'a T) -> Vec<&'a str>,
    term: &str,
) -> Vec<&'a T> {
    let mut scored: Vec<(u8, &T)> = candidates
        .iter()
        .filter_map(|candidate| {
            let names: Vec<String> = names_of(candidate)
                .into_iter()
                .map(fold_for_search)
                .collect();
            if names.iter().any(|name| name.starts_with(term)) {
                Some((0, candidate))
            } else {
                names
                    .iter()
                    .any(|name| name.contains(term))
                    .then_some((1, candidate))
            }
        })
        .collect();
    scored.sort_by_key(|(rank, _)| *rank);
    scored
        .into_iter()
        .take(MAXIMUM_SEARCH_VALUE_SUGGESTIONS)
        .map(|(_, candidate)| candidate)
        .collect()
}

fn day_suggestions<'a>(
    key: SearchFilterKey,
    partial: &str,
    today: CivilDate,
) -> Vec<SearchSuggestion<'a>> {
    let candidates = [
        (today, RelativeDay::Today),
        (today.plus_days(-1), RelativeDay::Yesterday),
        (today.plus_days(-DAYS_IN_A_WEEK), RelativeDay::AWeekAgo),
        (today.plus_days(-DAYS_IN_A_MONTH), RelativeDay::AMonthAgo),
    ];
    let mut matching: Vec<(String, RelativeDay)> = candidates
        .into_iter()
        .map(|(day, relative_day)| (day.iso_day(), relative_day))
        .filter(|(day, _)| day.starts_with(partial))
        .collect();
    if is_searchable_day(partial) && !matching.iter().any(|(day, _)| day == partial) {
        matching.insert(0, (partial.to_owned(), RelativeDay::Typed));
    }
    matching
        .into_iter()
        .map(|(day, relative_day)| SearchSuggestion::Day {
            key,
            day,
            relative_day,
        })
        .collect()
}

/// What choosing a suggestion writes, and whether the part ends there (a space follows).
#[must_use]
pub fn suggestion_insertion(suggestion: &SearchSuggestion<'_>) -> (String, bool) {
    match suggestion {
        SearchSuggestion::Key(key) => (format!("{}:", key.as_str()), false),
        SearchSuggestion::Person { key, user } => (search_filter_text(*key, &user.username), true),
        SearchSuggestion::Channel(channel) => (format!("in:{}", channel.value), true),
        SearchSuggestion::Has(value) => (format!("has:{}", value.as_str()), true),
        SearchSuggestion::Day { key, day, .. } => (format!("{}:{day}", key.as_str()), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: u64, username: &str, display_name: &str) -> PublicUser {
        PublicUser {
            id: Snowflake::from_raw(id),
            username: username.into(),
            display_name: display_name.into(),
            avatar: None,
            banner: None,
        }
    }

    fn value(key: SearchFilterKey, partial: &str) -> SearchCaretContext {
        SearchCaretContext::Value {
            key,
            partial: partial.into(),
            start: 0,
            end: partial.len(),
        }
    }

    struct Fixture {
        people: Vec<PublicUser>,
        channels: Vec<SearchChannelSource>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                people: vec![
                    user(1, "ada", "Ada Lovelace"),
                    user(2, "bob", "Bobby"),
                    user(3, "zoe", "Zoé Abadie"),
                ],
                channels: vec![
                    SearchChannelSource {
                        id: Snowflake::from_raw(10),
                        name: "général".into(),
                        is_conversation: false,
                        value: "général".into(),
                    },
                    SearchChannelSource {
                        id: Snowflake::from_raw(11),
                        name: "Game night".into(),
                        is_conversation: false,
                        value: "11".into(),
                    },
                ],
            }
        }

        fn sources(&self) -> SearchSuggestionSources<'_> {
            SearchSuggestionSources {
                people: &self.people,
                channels: &self.channels,
                today: CivilDate {
                    year: 2026,
                    month: 10,
                    day: 30,
                },
            }
        }
    }

    #[test]
    fn lists_every_filter_between_parts_and_the_filters_a_word_could_start() {
        let fixture = Fixture::new();
        assert_eq!(
            search_suggestions(
                &SearchCaretContext::Between { start: 0, end: 0 },
                fixture.sources()
            )
            .len(),
            7
        );
        let key = |partial: &str| SearchCaretContext::Key {
            partial: partial.into(),
            start: 0,
            end: partial.len(),
        };
        assert_eq!(
            search_suggestions(&key("M"), fixture.sources()),
            vec![SearchSuggestion::Key(SearchFilterKey::Mentions)]
        );
        assert_eq!(search_suggestions(&key("pizza"), fixture.sources()), vec![]);
    }

    #[test]
    fn suggests_people_by_username_or_name_ignoring_accents_starting_matches_first() {
        let fixture = Fixture::new();
        let people = search_suggestions(&value(SearchFilterKey::From, "@a"), fixture.sources());
        let usernames: Vec<&str> = people
            .iter()
            .filter_map(|suggestion| {
                if let SearchSuggestion::Person { user, .. } = suggestion {
                    Some(user.username.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(usernames, vec!["ada", "zoe"]);
        assert_eq!(
            people.first().map(suggestion_insertion),
            Some(("from:ada".into(), true))
        );
        let paul = user(1, "Paul Carouge", "Ada Lovelace");
        assert_eq!(
            suggestion_insertion(&SearchSuggestion::Person {
                key: SearchFilterKey::Mentions,
                user: &paul
            })
            .0,
            "mentions:\"Paul Carouge\""
        );
    }

    #[test]
    fn suggests_channels_by_name_writing_the_value_that_stays_one_part() {
        let fixture = Fixture::new();
        let channels = search_suggestions(&value(SearchFilterKey::In, "#ga"), fixture.sources());
        assert_eq!(
            channels
                .iter()
                .map(|suggestion| suggestion_insertion(suggestion).0)
                .collect::<Vec<_>>(),
            vec!["in:11"]
        );
        assert_eq!(
            search_suggestions(&value(SearchFilterKey::In, "gene"), fixture.sources()).len(),
            1
        );
    }

    #[test]
    fn suggests_kinds_and_days_the_typed_day_first_when_it_is_a_real_one() {
        let fixture = Fixture::new();
        assert_eq!(
            search_suggestions(&value(SearchFilterKey::Has, "i"), fixture.sources()),
            vec![SearchSuggestion::Has(SearchHasValue::Image)]
        );
        let days: Vec<(String, RelativeDay)> =
            search_suggestions(&value(SearchFilterKey::Before, ""), fixture.sources())
                .into_iter()
                .filter_map(|suggestion| {
                    if let SearchSuggestion::Day {
                        day, relative_day, ..
                    } = suggestion
                    {
                        Some((day, relative_day))
                    } else {
                        None
                    }
                })
                .collect();
        assert_eq!(
            days,
            vec![
                ("2026-10-30".into(), RelativeDay::Today),
                ("2026-10-29".into(), RelativeDay::Yesterday),
                ("2026-10-23".into(), RelativeDay::AWeekAgo),
                ("2026-09-30".into(), RelativeDay::AMonthAgo),
            ]
        );
        assert_eq!(
            search_suggestions(
                &value(SearchFilterKey::During, "2024-02-29"),
                fixture.sources()
            ),
            vec![SearchSuggestion::Day {
                key: SearchFilterKey::During,
                day: "2024-02-29".into(),
                relative_day: RelativeDay::Typed
            }]
        );
        assert_eq!(
            search_suggestions(
                &value(SearchFilterKey::During, "2023-02-29"),
                fixture.sources()
            ),
            vec![]
        );
    }
}
