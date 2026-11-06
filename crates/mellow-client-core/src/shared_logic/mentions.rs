//! `@name` mentions in message text (`web/src/components/mentions.ts`, DESIGN.md §5.4).
//!
//! shortcut: the protocol has no mention syntax for people yet, so mentions are found by
//! matching the name written after an "@" against the people this client knows, the longest name
//! first since names may hold spaces and dashes (ADR-0048); move to the instance's own mention
//! ids once the protocol carries them.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use mellow_protocol::{PublicUser, Snowflake};

use super::format::display_name;
use super::search_text::fold_for_search;
use super::unicode_classes::{character_at, character_before, is_js_whitespace, js_trim};

/// The word that mentions everyone who can see the channel, in any case; never a person's name.
pub const EVERYONE_MENTION_WORD: &str = "everyone";
/// Names are at most 32 characters (`account_names.rs` on the server).
const MAXIMUM_NAME_LENGTH: usize = 32;
pub const MAXIMUM_MENTION_SUGGESTIONS: usize = 10;

/// People by the lowercase names they can be mentioned with.
pub type MentionablePeople = HashMap<String, PublicUser>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MentionSegment {
    Text(String),
    Mention {
        text: String,
        user: PublicUser,
    },
    /// `@everyone` as written, when the server kept it as a mention (ADR-0040).
    EveryoneMention {
        text: String,
    },
}

/// `[\p{Alphabetic}\p{N}_.]`: what a name is made of. `Alphabetic` keeps combining vowel signs,
/// as the server's `char::is_alphanumeric` does.
fn is_name_character(character: char) -> bool {
    character.is_alphabetic() || character.is_numeric() || character == '_' || character == '.'
}

/// What may follow a name's first character inside a candidate: names hold spaces and dashes.
fn is_candidate_character(character: char) -> bool {
    is_name_character(character) || character == ' ' || character == '-'
}

#[must_use]
pub fn mentionable_people(people: &[PublicUser]) -> MentionablePeople {
    let mut by_name = MentionablePeople::new();
    // A display name two people share is no one's, as on the server (`domain/mentions.rs`):
    // whoever copies a name must not take its mentions.
    let mut shared_display_names = HashSet::new();
    // Display names first, so a username always wins over someone else's display name.
    for person in people {
        let shown = js_trim(&person.display_name);
        if !shown.is_empty() {
            let name = shown.to_lowercase();
            if by_name
                .get(&name)
                .is_some_and(|holder| holder.id != person.id)
            {
                shared_display_names.insert(name.clone());
            }
            by_name.insert(name, person.clone());
        }
    }
    for name in &shared_display_names {
        by_name.remove(name);
    }
    for person in people {
        by_name.insert(person.username.to_lowercase(), person.clone());
    }
    by_name
}

/// What follows the "@" at byte `at` when a mention may start there: an "@" at the start or
/// after whitespace, a name character, then up to a name's length of name characters, spaces and
/// dashes. Bounded, so a long run is read in constant work.
fn candidate_run_after(text: &str, at: usize) -> Option<&str> {
    let may_start_here = at == 0 || character_before(text, at).is_some_and(is_js_whitespace);
    let run_start = at + 1;
    let rest = text.get(run_start..)?;
    let mut characters = rest.char_indices();
    let (_, first) = characters.next()?;
    if !may_start_here || !is_name_character(first) {
        return None;
    }
    let mut run_end = first.len_utf8();
    for (offset, character) in characters.take(MAXIMUM_NAME_LENGTH) {
        if !is_candidate_character(character) {
            break;
        }
        run_end = offset + character.len_utf8();
    }
    rest.get(..run_end)
}

/// The longest name among `people` that `run` starts with, ending where a word does: before a
/// space, a dash, punctuation or the text's end (`after` is the character right after the run).
/// Returns the person and the name's byte length in `run`.
fn find_mentioned<'p>(
    run: &str,
    after: Option<char>,
    people: &'p MentionablePeople,
) -> Option<(&'p PublicUser, usize)> {
    let ends = run
        .char_indices()
        .map(|(offset, character)| offset + character.len_utf8())
        .rev();
    for end in ends {
        let next = if end < run.len() {
            character_at(run, end)
        } else {
            after
        };
        let ends_a_word = character_before(run, end).is_some_and(is_name_character)
            && !next.is_some_and(is_name_character);
        if !ends_a_word {
            continue;
        }
        let name = run.get(..end)?;
        // "@paul." at the end of a sentence: the trailing dots are punctuation unless they are the name's.
        for candidate in [name, name.trim_end_matches('.')] {
            if candidate.is_empty() {
                continue;
            }
            if let Some(user) = people.get(&candidate.to_lowercase()) {
                return Some((user, candidate.len()));
            }
        }
    }
    None
}

/// Whether `run` starts with the everyone word as a whole word, a closing full stop allowed.
fn is_everyone_word(run: &str, after: Option<char>) -> bool {
    let first_word_length = run
        .find(|character: char| !is_name_character(character))
        .unwrap_or(run.len());
    let first_word = run.get(..first_word_length).unwrap_or_default();
    let is_whole_word = first_word_length < run.len() || !after.is_some_and(is_name_character);
    is_whole_word && first_word.trim_end_matches('.').to_lowercase() == EVERYONE_MENTION_WORD
}

/// `text` cut into plain text and mentions. `@everyone` is never a person: it is a mention when
/// `is_everyone_mentioned` (the server kept it, ADR-0040), plain text otherwise.
#[must_use]
pub fn split_mentions(
    text: &str,
    people: &MentionablePeople,
    is_everyone_mentioned: bool,
) -> Vec<MentionSegment> {
    let mut segments = Vec::new();
    let mut text_start = 0;
    let mut search_from = 0;
    while let Some(offset) = text.get(search_from..).and_then(|rest| rest.find('@')) {
        let at = search_from + offset;
        let Some(run) = candidate_run_after(text, at) else {
            search_from = at + 1;
            continue;
        };
        let run_end = at + 1 + run.len();
        search_from = run_end;
        let after = character_at(text, run_end);
        let is_everyone = is_everyone_word(run, after);
        let found = if is_everyone {
            None
        } else {
            find_mentioned(run, after, people)
        };
        if found.is_none() && !(is_everyone && is_everyone_mentioned) {
            continue;
        }
        if at > text_start {
            segments.push(MentionSegment::Text(
                text.get(text_start..at).unwrap_or_default().to_owned(),
            ));
        }
        let end = at + 1 + found.map_or(EVERYONE_MENTION_WORD.len(), |(_, length)| length);
        let written = text.get(at..end).unwrap_or_default().to_owned();
        segments.push(match found {
            Some((user, _)) => MentionSegment::Mention {
                text: written,
                user: user.clone(),
            },
            None => MentionSegment::EveryoneMention { text: written },
        });
        text_start = end;
    }
    if text_start < text.len() {
        segments.push(MentionSegment::Text(
            text.get(text_start..).unwrap_or_default().to_owned(),
        ));
    }
    segments
}

// --- Suggestions while typing "@na" (DESIGN.md §5.5) -----------------------------------------

/// The "@" and the name being typed before the caret, which a chosen person replaces (bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionSuggestionQuery {
    pub start: usize,
    pub end: usize,
    pub query: String,
}

/// The "@na" ending at `caret` (a byte offset): an "@" where a mention may start (the start, a
/// space, or a formatting mark as in "**@na", as the server reads them), then up to a name's
/// length of name characters; a lone "@" asks for everyone, as on Discord. The replaced range
/// runs to the end of the name, so choosing with the caret inside "@pa|ul" leaves no "ul" behind.
#[must_use]
pub fn mention_suggestion_query(text: &str, caret: usize) -> Option<MentionSuggestionQuery> {
    text.get(..caret)?;
    let mut start = caret;
    let mut typed_characters = 0;
    while typed_characters <= MAXIMUM_NAME_LENGTH {
        match character_before(text, start) {
            Some(character) if is_name_character(character) => {
                start -= character.len_utf8();
                typed_characters += 1;
            }
            _ => break,
        }
    }
    let at = start.checked_sub(1)?;
    if text.as_bytes().get(at) != Some(&b'@') || typed_characters > MAXIMUM_NAME_LENGTH {
        return None;
    }
    if at > 0
        && !character_before(text, at)
            .is_some_and(|character| is_js_whitespace(character) || "*_~|".contains(character))
    {
        return None;
    }
    let end = caret
        + text
            .get(caret..)?
            .find(|character: char| !is_name_character(character))
            .unwrap_or(text.len() - caret);
    Some(MentionSuggestionQuery {
        start: at,
        end,
        query: text.get(start..caret)?.to_owned(),
    })
}

/// How well `person` matches the folded query, lower first: a name starting with it, then a
/// word of the display name starting with it, then a name holding it; `None` for no match.
fn mention_match_rank(person: &PublicUser, folded_query: &str) -> Option<u8> {
    let username = fold_for_search(&person.username);
    let shown_name = fold_for_search(js_trim(&person.display_name));
    if username.starts_with(folded_query) || shown_name.starts_with(folded_query) {
        return Some(0);
    }
    if shown_name
        .split(is_js_whitespace)
        .any(|word| word.starts_with(folded_query))
    {
        return Some(1);
    }
    (username.contains(folded_query) || shown_name.contains(folded_query)).then_some(2)
}

/// shortcut: the web compares shown names with `Intl.Collator`; without ICU collation data this
/// compares them folded (case and accents ignored), then as written. Pull in `icu_collator` if
/// people report a different order between the web and the desktop.
pub(crate) fn compare_names_for_people(left: &str, right: &str) -> Ordering {
    fold_for_search(left)
        .cmp(&fold_for_search(right))
        .then_with(|| left.cmp(right))
}

/// The people `query` may name, best match first and then by shown name, at most
/// [`MAXIMUM_MENTION_SUGGESTIONS`], without `excluded_user_id` (the writer: mentioning oneself
/// counts for no one) and each person once. Case and accents are ignored, as in the search.
#[must_use]
pub fn suggest_mentions<'p>(
    query: &str,
    people: &'p [PublicUser],
    excluded_user_id: Snowflake,
) -> Vec<&'p PublicUser> {
    let folded_query = fold_for_search(query);
    let mut seen = HashSet::from([excluded_user_id]);
    let mut matches: Vec<(u8, &PublicUser)> = people
        .iter()
        .filter(|person| seen.insert(person.id))
        .filter_map(|person| mention_match_rank(person, &folded_query).map(|rank| (rank, person)))
        .collect();
    matches.sort_by(|(left_rank, left), (right_rank, right)| {
        left_rank
            .cmp(right_rank)
            .then_with(|| compare_names_for_people(display_name(left), display_name(right)))
    });
    matches
        .into_iter()
        .take(MAXIMUM_MENTION_SUGGESTIONS)
        .map(|(_, person)| person)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn user(id: u64, username: &str, display_name: &str) -> PublicUser {
        PublicUser {
            id: Snowflake::from_raw(id),
            username: username.to_owned(),
            display_name: display_name.to_owned(),
            avatar: None,
            banner: None,
        }
    }

    fn text(value: &str) -> MentionSegment {
        MentionSegment::Text(value.to_owned())
    }

    fn mention(value: &str, person: &PublicUser) -> MentionSegment {
        MentionSegment::Mention {
            text: value.to_owned(),
            user: person.clone(),
        }
    }

    fn paul() -> PublicUser {
        user(1, "paul", "Paul Carouge")
    }
    fn zoe() -> PublicUser {
        user(2, "zoe", "Zoé")
    }
    fn dotted() -> PublicUser {
        user(3, "j.doe", "")
    }
    fn people() -> MentionablePeople {
        mentionable_people(&[paul(), zoe(), dotted()])
    }

    #[test]
    fn finds_usernames_and_one_word_display_names_in_any_case() {
        assert_eq!(
            split_mentions("@Paul can you review, @Zoé?", &people(), false),
            vec![
                mention("@Paul", &paul()),
                text(" can you review, "),
                mention("@Zoé", &zoe()),
                text("?")
            ]
        );
    }

    #[test]
    fn gives_a_display_name_two_people_share_to_no_one_but_keeps_usernames() {
        let eve = user(5, "eve", "Twin");
        let twins = mentionable_people(&[eve.clone(), user(6, "mallory", "twin")]);
        assert_eq!(
            split_mentions("@twin @eve", &twins, false),
            vec![text("@twin "), mention("@eve", &eve)]
        );
    }

    #[test]
    fn leaves_trailing_dots_unknown_names_and_addresses_as_text() {
        assert_eq!(
            split_mentions("thanks @j.doe.", &people(), false),
            vec![text("thanks "), mention("@j.doe", &dotted()), text(".")]
        );
        assert_eq!(
            split_mentions("write to paul@example.org or @nobody", &people(), false),
            vec![text("write to paul@example.org or @nobody")]
        );
    }

    #[test]
    fn mentions_the_longest_name_written_after_the_at_sign() {
        let paul_carouge = user(7, "Paul Carouge", "Paul Carouge");
        let short_paul = user(8, "Paul", "Paul");
        let jean_paul = user(9, "Jean-Paul", "Jean-Paul");
        let named =
            mentionable_people(&[paul_carouge.clone(), short_paul.clone(), jean_paul.clone()]);
        assert_eq!(
            split_mentions("@paul carouge, salut", &named, false),
            vec![mention("@paul carouge", &paul_carouge), text(", salut")]
        );
        assert_eq!(
            split_mentions("@Paul salut", &named, false),
            vec![mention("@Paul", &short_paul), text(" salut")]
        );
        assert_eq!(
            split_mentions("@Jean-Paul.", &named, false),
            vec![mention("@Jean-Paul", &jean_paul), text(".")]
        );
        assert_eq!(
            split_mentions("@Paul-Henri", &named, false),
            vec![mention("@Paul", &short_paul), text("-Henri")]
        );
    }

    #[test]
    fn reads_neither_a_long_run_of_dots_nor_a_name_longer_than_any_account_as_a_mention() {
        let dots = format!("@{}", ".".repeat(4_000));
        assert_eq!(split_mentions(&dots, &people(), false), vec![text(&dots)]);
        let long = format!("@paul{}", "x".repeat(40));
        assert_eq!(split_mentions(&long, &people(), false), vec![text(&long)]);
    }

    #[test]
    fn reads_everyone_only_when_the_server_kept_it() {
        let everyone_person = mentionable_people(&[user(9, "everyone", "")]);
        assert_eq!(
            split_mentions("@everyone", &everyone_person, false),
            vec![text("@everyone")]
        );
        assert_eq!(
            split_mentions("hey @Everyone.", &people(), true),
            vec![
                text("hey "),
                MentionSegment::EveryoneMention {
                    text: "@Everyone".to_owned()
                },
                text(".")
            ]
        );
    }

    #[allow(clippy::unnecessary_wraps)] // Compared with the function's own optional answer.
    fn query(start: usize, end: usize, typed: &str) -> Option<MentionSuggestionQuery> {
        Some(MentionSuggestionQuery {
            start,
            end,
            query: typed.to_owned(),
        })
    }

    #[test]
    fn reads_the_name_typed_after_an_at_sign_at_the_start_or_after_a_space() {
        assert_eq!(mention_suggestion_query("@pa", 3), query(0, 3, "pa"));
        assert_eq!(mention_suggestion_query("hi @Zo", 6), query(3, 6, "Zo"));
        assert_eq!(mention_suggestion_query("hi @", 4), query(3, 4, ""));
        assert_eq!(
            mention_suggestion_query("@j.do and more", 5),
            query(0, 5, "j.do")
        );
    }

    #[test]
    fn opens_after_a_formatting_mark_and_replaces_the_whole_name() {
        assert_eq!(mention_suggestion_query("**@pa", 5), query(2, 5, "pa"));
        assert_eq!(mention_suggestion_query("||@pa", 5), query(2, 5, "pa"));
        assert_eq!(
            mention_suggestion_query("hi @paul how", 6),
            query(3, 8, "pa")
        );
    }

    #[test]
    fn ignores_an_at_sign_inside_a_word_after_a_space_or_after_a_long_name() {
        assert_eq!(mention_suggestion_query("mail@pa", 7), None);
        assert_eq!(mention_suggestion_query("@pa ", 4), None);
        assert_eq!(
            mention_suggestion_query(&format!("@{}", "a".repeat(33)), 34),
            None
        );
        assert_eq!(
            mention_suggestion_query(&format!("@{}", "a".repeat(32)), 33)
                .map(|found| found.query.len()),
            Some(32)
        );
    }

    fn ids(found: &[&PublicUser]) -> Vec<u64> {
        found.iter().map(|person| person.id.raw()).collect()
    }

    #[test]
    fn ranks_names_starting_with_the_query_then_display_name_words_then_names_holding_it() {
        let ana = user(10, "ana", "Ana Paula");
        let pablo = user(11, "pablo", "");
        let everyone = [paul(), zoe(), dotted(), ana.clone(), pablo];
        assert_eq!(
            ids(&suggest_mentions("pa", &everyone, Snowflake::from_raw(0))),
            vec![11, 1, 10]
        );
        assert_eq!(
            ids(&suggest_mentions("ul", &everyone, Snowflake::from_raw(0))),
            vec![10, 1]
        );
        assert_eq!(
            ids(&suggest_mentions("ZOE", &everyone, Snowflake::from_raw(0))),
            vec![2]
        );
        assert_eq!(
            ids(&suggest_mentions(
                "zoé",
                &[zoe(), zoe()],
                Snowflake::from_raw(0)
            )),
            vec![2]
        );
        assert_eq!(
            ids(&suggest_mentions("pau", &everyone, Snowflake::from_raw(1))),
            vec![10]
        );
    }

    #[test]
    fn offers_everyone_for_a_lone_at_sign_by_name_and_at_most_a_bounded_number() {
        assert_eq!(
            ids(&suggest_mentions(
                "",
                &[zoe(), paul()],
                Snowflake::from_raw(0)
            )),
            vec![1, 2]
        );
        let crowd: Vec<PublicUser> = (0..20)
            .map(|index| user(100 + index, &format!("user{index}"), ""))
            .collect();
        assert_eq!(
            suggest_mentions("user", &crowd, Snowflake::from_raw(0)).len(),
            MAXIMUM_MENTION_SUGGESTIONS
        );
    }

    #[test]
    fn keeps_the_best_ones_in_order_whatever_order_the_people_come_in() {
        let mut crowd: Vec<PublicUser> = (0..25)
            .map(|index| user(200 + index, &format!("member{:02}", 24 - index), ""))
            .collect();
        let starting_with_query = user(300, "zz", "Membre Zed");
        crowd.push(starting_with_query.clone());
        let names: Vec<&str> = suggest_mentions("membe", &crowd, Snowflake::from_raw(0))
            .iter()
            .map(|person| person.username.as_str())
            .collect();
        assert_eq!(
            names,
            (0..10)
                .map(|index| format!("member{index:02}"))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            ids(&suggest_mentions("zed", &crowd, Snowflake::from_raw(0))),
            vec![300]
        );
    }
}
