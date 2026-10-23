//! Who a message mentions, with the rule the web client uses to draw mention chips
//! (`web/src/components/mentions.ts`): "@" at the start or after a space, then a username, or a
//! display name of one word, outside Markdown code. The protocol has no mention syntax yet, so the
//! server finds them in the text the same way.

use std::collections::{HashMap, HashSet};

use mellow_protocol::Snowflake;

/// Usernames and display names are at most 32 characters (`account_rules.rs`); one more for a
/// closing full stop.
const MAXIMUM_CANDIDATE_LENGTH: usize = 33;
/// A message naming more people than this mentions only the first ones, so a crafted message
/// costs a bounded amount of work.
pub(crate) const MAXIMUM_MENTIONS_PER_MESSAGE: usize = 50;

/// Someone who may be mentioned: their id, username and display name.
pub(crate) struct MentionablePerson {
    pub(crate) id: Snowflake,
    pub(crate) username: String,
    pub(crate) display_name: String,
}

fn is_name_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '.'
}

/// The text outside fenced code blocks and inline code spans, where mentions count.
fn text_outside_code(content: &str) -> String {
    let mut outside = String::with_capacity(content.len());
    let mut is_in_fence = false;
    for line in content.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            is_in_fence = !is_in_fence;
            outside.push('\n');
            continue;
        }
        if !is_in_fence {
            outside.push_str(line);
        }
    }
    // Inline code: a run of backticks up to the next run of the same length, as in CommonMark.
    let characters: Vec<char> = outside.chars().collect();
    let mut kept = String::with_capacity(outside.len());
    let mut index = 0;
    while index < characters.len() {
        if characters[index] != '`' {
            kept.push(characters[index]);
            index += 1;
            continue;
        }
        let run_start = index;
        while index < characters.len() && characters[index] == '`' {
            index += 1;
        }
        let run_length = index - run_start;
        let run = vec!['`'; run_length];
        let closing = (index..characters.len()).find(|&start| {
            characters[start..].starts_with(&run)
                && characters.get(start + run_length) != Some(&'`')
                && (start == 0 || characters[start - 1] != '`')
        });
        match closing {
            // A space stands for the code, so "a`x`@b" does not glue "@b" to text.
            Some(start) => {
                kept.push(' ');
                index = start + run_length;
            }
            None => kept.extend(&characters[run_start..index]),
        }
    }
    kept
}

/// The names written after an "@", in order, each with its spelling without trailing dots
/// ("@paul." at the end of a sentence), both lowercased.
fn mention_candidates(content: &str) -> Vec<[String; 2]> {
    let text = text_outside_code(content);
    let characters: Vec<char> = text.chars().collect();
    let mut candidates = Vec::new();
    for (index, character) in characters.iter().enumerate() {
        let is_after_space_or_start = index == 0 || characters[index - 1].is_whitespace();
        if *character != '@' || !is_after_space_or_start {
            continue;
        }
        let name: String = characters[index + 1..]
            .iter()
            .take_while(|character| is_name_character(**character))
            .collect();
        let length = name.chars().count();
        if length == 0 || length > MAXIMUM_CANDIDATE_LENGTH {
            continue;
        }
        let name = name.to_lowercase();
        let without_trailing_dots = name.trim_end_matches('.').to_owned();
        candidates.push([name, without_trailing_dots]);
    }
    candidates
}

/// The people `content` mentions among `people`, in order of first mention, without the author.
/// A username wins over someone else's display name, as on the web client; a display name two
/// people share mentions no one.
pub(crate) fn mentioned_ids(
    content: &str,
    author_id: Snowflake,
    people: &[MentionablePerson],
) -> Vec<Snowflake> {
    // `None` for a display name two people share: whoever copies a name must not take its
    // mentions, so it mentions no one.
    let mut by_name: HashMap<String, Option<Snowflake>> = HashMap::new();
    for person in people {
        let display_name = person.display_name.trim();
        if !display_name.is_empty() && !display_name.contains(char::is_whitespace) {
            by_name
                .entry(display_name.to_lowercase())
                .and_modify(|owner| {
                    if *owner != Some(person.id) {
                        *owner = None;
                    }
                })
                .or_insert(Some(person.id));
        }
    }
    for person in people {
        by_name.insert(person.username.to_lowercase(), Some(person.id));
    }
    let mut seen = HashSet::new();
    let mut mentioned = Vec::new();
    for spellings in mention_candidates(content) {
        let Some(id) = spellings
            .iter()
            .filter(|spelling| !spelling.is_empty())
            .find_map(|spelling| by_name.get(spelling).copied().flatten())
        else {
            continue;
        };
        if id != author_id && seen.insert(id) {
            mentioned.push(id);
            if mentioned.len() == MAXIMUM_MENTIONS_PER_MESSAGE {
                break;
            }
        }
    }
    mentioned
}

/// Whether `content` has anything that could be a mention, so a message without one never reads
/// the member list.
pub(crate) fn may_mention_someone(content: &str) -> bool {
    content.contains('@') && !mention_candidates(content).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(id: u64, username: &str, display_name: &str) -> MentionablePerson {
        MentionablePerson {
            id: Snowflake::from_raw(id),
            username: username.into(),
            display_name: display_name.into(),
        }
    }

    fn people() -> Vec<MentionablePerson> {
        vec![
            person(1, "paul", "Paul"),
            person(2, "alice", "Ali"),
            person(3, "bob", "Jean Bob"),
            person(4, "ali", "Someone"),
            person(5, "eve", "Twin"),
            person(6, "mallory", "twin"),
        ]
    }

    fn mentioned(content: &str) -> Vec<u64> {
        mentioned_ids(content, Snowflake::from_raw(99), &people())
            .into_iter()
            .map(Snowflake::raw)
            .collect()
    }

    #[test]
    fn usernames_and_one_word_display_names_are_mentions_in_order_once() {
        assert_eq!(mentioned("@Paul hello @ALICE and @paul"), [1, 2]);
        assert_eq!(mentioned("hi @alice."), [2], "a closing full stop");
        assert_eq!(mentioned("@Jean"), Vec::<u64>::new(), "no multi-word names");
        assert_eq!(
            mentioned("@twin @eve"),
            [5],
            "a shared display name is no one's"
        );
        assert_eq!(
            mentioned("@ali"),
            [4],
            "a username wins over another display name"
        );
    }

    #[test]
    fn only_at_the_start_or_after_a_space_and_outside_code() {
        assert_eq!(mentioned("mail@paul"), Vec::<u64>::new());
        assert_eq!(mentioned("\\@paul"), Vec::<u64>::new(), "escaped");
        assert_eq!(mentioned("`@paul` and ``a ` @bob``"), Vec::<u64>::new());
        assert_eq!(mentioned("```\n@paul\n```\n@bob"), [3]);
        assert_eq!(mentioned("an `unclosed @paul"), [1]);
        assert_eq!(mentioned("line\n@bob"), [3]);
    }

    #[test]
    fn the_author_unknown_names_and_long_names_are_not_mentions() {
        assert_eq!(
            mentioned_ids("@paul @nobody", Snowflake::from_raw(1), &people()),
            Vec::<Snowflake>::new()
        );
        assert_eq!(
            mentioned(&format!("@{}", "a".repeat(40))),
            Vec::<u64>::new()
        );
        assert!(!may_mention_someone("no at sign"));
        assert!(!may_mention_someone("mail@example.org"));
        assert!(may_mention_someone("hey @someone"));
    }
}
