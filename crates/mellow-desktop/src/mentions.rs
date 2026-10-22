//! `@name` mentions in message text (DESIGN.md §5.4), with the web client's semantics
//! (`web/src/components/mentions.ts`).
//!
//! shortcut: the protocol has no mention syntax or mention list yet, so mentions are found here by
//! matching "@username" (or a one-word display name) against the people this client knows; move to
//! the instance's own mention ids once the protocol carries them.

use std::collections::HashMap;
use std::ops::Range;

use mellow_protocol::{PublicUser, Snowflake};

/// People by the lowercase names they can be mentioned with.
#[derive(Debug, Default)]
pub struct MentionablePeople {
    user_ids_by_name: HashMap<String, Snowflake>,
}

impl MentionablePeople {
    pub fn new<'a>(people: impl IntoIterator<Item = &'a PublicUser> + Clone) -> Self {
        let mut user_ids_by_name = HashMap::new();
        // Display names first, so a username always wins over someone else's display name.
        for person in people.clone() {
            let display_name = person.display_name.trim();
            if !display_name.is_empty() && !display_name.chars().any(char::is_whitespace) {
                user_ids_by_name.insert(display_name.to_lowercase(), person.id);
            }
        }
        for person in people {
            user_ids_by_name.insert(person.username.to_lowercase(), person.id);
        }
        Self { user_ids_by_name }
    }

    /// For the demo, which has no accounts: each name stands for itself.
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        let user_ids_by_name = names
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                let id = u64::try_from(index).unwrap_or(u64::MAX);
                (name.to_lowercase(), Snowflake::from_raw(id))
            })
            .collect();
        Self { user_ids_by_name }
    }

    fn user_id(&self, name: &str) -> Option<Snowflake> {
        self.user_ids_by_name.get(&name.to_lowercase()).copied()
    }
}

/// One mention found in a text: the byte range of "@name" and who it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub range: Range<usize>,
    pub user_id: Snowflake,
}

fn is_name_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '.'
}

/// Every "@name" at the start of the text or after whitespace that names someone known. Trailing
/// dots are punctuation unless they belong to the name ("@paul." ends a sentence).
pub fn find_mentions(text: &str, people: &MentionablePeople) -> Vec<Mention> {
    let mut mentions = Vec::new();
    let mut previous: Option<char> = None;
    for (at_index, character) in text.char_indices() {
        let starts_word = previous.is_none_or(char::is_whitespace);
        previous = Some(character);
        if character != '@' || !starts_word {
            continue;
        }
        let name_start = at_index + character.len_utf8();
        let name_end = text[name_start..]
            .char_indices()
            .find(|(_, character)| !is_name_character(*character))
            .map_or(text.len(), |(offset, _)| name_start + offset);
        let mut candidate = &text[name_start..name_end];
        while !candidate.is_empty() {
            if let Some(user_id) = people.user_id(candidate) {
                mentions.push(Mention {
                    range: at_index..name_start + candidate.len(),
                    user_id,
                });
                break;
            }
            match candidate.strip_suffix('.') {
                Some(shorter) => candidate = shorter,
                None => break,
            }
        }
    }
    mentions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(id: u64, username: &str, display_name: &str) -> PublicUser {
        PublicUser {
            id: Snowflake::from_raw(id),
            username: username.into(),
            display_name: display_name.into(),
            avatar: None,
            banner: None,
        }
    }

    #[test]
    fn usernames_and_one_word_display_names_are_mentions() {
        let people = [person(1, "paul", "Paul Carouge"), person(2, "ana", "Ana")];
        let people = MentionablePeople::new(&people);
        let text = "@Paul can you check? cc @ana. Not an@email or @nobody";

        let mentions = find_mentions(text, &people);

        let found: Vec<_> = mentions
            .iter()
            .map(|mention| (&text[mention.range.clone()], mention.user_id.to_string()))
            .collect();
        assert_eq!(found, [("@Paul", "1".into()), ("@ana", "2".into())]);
    }

    #[test]
    fn a_username_wins_over_someone_elses_display_name() {
        let people = [person(1, "zoe", "Léo"), person(2, "léo", "Léo Martin")];
        let people = MentionablePeople::new(&people);

        let mentions = find_mentions("@léo", &people);

        assert_eq!(mentions[0].user_id, Snowflake::from_raw(2));
    }
}
