//! Who a message mentions, with the rule the web client uses to draw mention chips
//! (`web/src/components/mentions.ts`): "@" at the start, after a space or a formatting mark, then
//! a person's name, outside Markdown code; names may hold spaces and dashes (ADR-0048), so the
//! longest name written there wins ("@Paul Carouge" over "@Paul"). The protocol has no mention
//! syntax yet, so the server finds them in the text the same way.
//! shortcut: escapes ("\*@paul") are not read as Markdown here, so the rare mention right after
//! one shows a chip but counts for no one; send mention ids from the composer to drop the guess.
//!
//! Roles are mentioned by id, `<@&id>`, which the composer writes when a role is picked, and
//! everyone by `@everyone` (ADR-0040); both outside code too.

use std::collections::{HashMap, HashSet};

use mellow_protocol::Snowflake;

/// Usernames and display names are at most 32 characters (`account_rules.rs`); one more for a
/// closing full stop.
const MAXIMUM_CANDIDATE_LENGTH: usize = 33;
/// A message naming more people than this mentions only the first ones, so a crafted message
/// costs a bounded amount of work.
pub(crate) const MAXIMUM_MENTIONS_PER_MESSAGE: usize = 50;

/// The word that mentions every member who can view the channel (ADR-0040), never a person.
const EVERYONE_MENTION_WORD: &str = "everyone";
/// What opens a role mention, `<@&id>`.
const ROLE_MENTION_OPENING: &str = "<@&";

/// Someone who may be mentioned: their id, username and display name.
pub(crate) struct MentionablePerson {
    pub(crate) id: Snowflake,
    pub(crate) username: String,
    pub(crate) display_name: String,
}

/// What may come right before an "@": the web client looks for mentions in each piece of text
/// Markdown leaves, so "**@paul**" and "||@paul||" mention too.
fn may_precede_a_mention(character: char) -> bool {
    character.is_whitespace() || matches!(character, '*' | '_' | '~' | '|')
}

fn is_name_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '.'
}

/// The text outside code, where mentions count. A run of backticks opens code up to the next run
/// of the same length, across lines too, which covers fenced blocks as the web client reads them;
/// a run never closed stays text.
pub(crate) fn text_outside_code(content: &str) -> String {
    let characters: Vec<char> = content.chars().collect();
    let mut kept = String::with_capacity(content.len());
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

/// What may continue a name after its first character: names hold spaces and dashes too.
fn may_continue_a_name(character: char) -> bool {
    is_name_character(character) || matches!(character, ' ' | '-')
}

/// The names that may be written after each "@", in order, each with its spellings to try,
/// lowercased and longest first: every run of words up to a name's length, each as written,
/// without trailing dots ("@paul." at the end of a sentence), and without a closing italic mark
/// ("_@paul_").
fn mention_candidates(content: &str) -> Vec<Vec<String>> {
    let text = text_outside_code(content);
    let characters: Vec<char> = text.chars().collect();
    let mut candidates = Vec::new();
    for (index, character) in characters.iter().enumerate() {
        let is_after_space_or_start = index == 0 || may_precede_a_mention(characters[index - 1]);
        if *character != '@' || !is_after_space_or_start {
            continue;
        }
        // One character past the longest name, so a longer run of name characters is no name.
        let run: Vec<char> = characters[index + 1..]
            .iter()
            .copied()
            .take(MAXIMUM_CANDIDATE_LENGTH + 1)
            .take_while(|character| may_continue_a_name(*character))
            .collect();
        if !run.first().is_some_and(|first| is_name_character(*first)) {
            continue;
        }
        let mut spellings = Vec::new();
        for end in (1..=run.len().min(MAXIMUM_CANDIDATE_LENGTH)).rev() {
            let ends_a_word = is_name_character(run[end - 1])
                && run.get(end).is_none_or(|next| !is_name_character(*next));
            if !ends_a_word {
                continue;
            }
            let name: String = run[..end].iter().collect::<String>().to_lowercase();
            let without_trailing_dots = name.trim_end_matches('.').to_owned();
            let without_closing_italic = name.trim_end_matches(['.', '_']).to_owned();
            spellings.extend([name, without_trailing_dots, without_closing_italic]);
        }
        if !spellings.is_empty() {
            candidates.push(spellings);
        }
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
        if !display_name.is_empty() {
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
            .filter(|spelling| !spelling.is_empty() && *spelling != EVERYONE_MENTION_WORD)
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

/// A role of the guild and whether anyone may mention it (`Role.is_mentionable`).
pub(crate) struct RoleWithMentionSetting {
    pub(crate) id: Snowflake,
    pub(crate) is_mentionable: bool,
}

/// What a role mention may reach, decided by the author's permissions in the channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoleMentionReach {
    /// Only roles anyone may mention.
    RoleWithMentionSettings,
    /// `MENTION_EVERYONE`: every role and `@everyone`.
    EveryRoleAndEveryone,
}

/// The role ids written as `<@&id>` outside code, in order, once each.
fn role_mention_candidates(text: &str) -> Vec<u64> {
    let mut candidates = Vec::new();
    for (start, _) in text.match_indices(ROLE_MENTION_OPENING) {
        let rest = &text[start + ROLE_MENTION_OPENING.len()..];
        let digits_length = rest
            .find(|character: char| !character.is_ascii_digit())
            .unwrap_or(rest.len());
        if digits_length == 0 || !rest[digits_length..].starts_with('>') {
            continue;
        }
        if let Ok(id) = rest[..digits_length].parse::<u64>()
            && !candidates.contains(&id)
        {
            candidates.push(id);
        }
    }
    candidates
}

/// The roles `content` mentions among `roles` that `reach` allows, in order of first mention,
/// with `guild_id` (the `@everyone` role's id) first when it mentions everyone. A role written
/// with the guild's id is not a mention: everyone is mentioned by `@everyone` only.
pub(crate) fn mentioned_role_ids(
    content: &str,
    guild_id: Snowflake,
    roles: &[RoleWithMentionSetting],
    reach: RoleMentionReach,
) -> Vec<Snowflake> {
    let may_mention_every_role = reach == RoleMentionReach::EveryRoleAndEveryone;
    let mut mentioned = Vec::new();
    let mentions_everyone = may_mention_every_role
        && mention_candidates(content).iter().any(|spellings| {
            spellings
                .iter()
                .any(|spelling| spelling == EVERYONE_MENTION_WORD)
        });
    if mentions_everyone {
        mentioned.push(guild_id);
    }
    for id in role_mention_candidates(&text_outside_code(content)) {
        let Some(role) = roles
            .iter()
            .find(|role| role.id.raw() == id && role.id != guild_id)
        else {
            continue;
        };
        if role.is_mentionable || may_mention_every_role {
            mentioned.push(role.id);
            if mentioned.len() == MAXIMUM_MENTIONS_PER_MESSAGE {
                break;
            }
        }
    }
    mentioned
}

/// Whether `content` has anything that could mention a role or everyone, so a message without
/// one never reads the guild's roles.
pub(crate) fn may_mention_a_role(content: &str) -> bool {
    content.contains(ROLE_MENTION_OPENING) || content.to_lowercase().contains("@everyone")
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
            person(7, "Paul Carouge", "Paul Carouge"),
            person(8, "Jean-Paul", "Jean-Paul"),
        ]
    }

    #[test]
    fn names_with_spaces_and_dashes_are_mentions_and_the_longest_name_wins() {
        assert_eq!(mentioned("@Paul Carouge salut"), [7]);
        assert_eq!(mentioned("@paul carouge."), [7], "a closing full stop");
        assert_eq!(mentioned("@Paul salut"), [1]);
        assert_eq!(mentioned("@Jean Bob et @jean-paul"), [3, 8]);
        assert_eq!(mentioned("@Paul-Henri"), [1], "a dash ends a shorter name");
        assert_eq!(mentioned("@Jean"), Vec::<u64>::new(), "no part of a name");
        assert_eq!(
            mentioned(&format!("@Paul{}", "x".repeat(40))),
            Vec::<u64>::new()
        );
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
        assert_eq!(mentioned("@Jean"), Vec::<u64>::new(), "no part of a name");
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
        assert_eq!(
            mentioned("```npm i```\n@paul look"),
            [1],
            "a fence on one line"
        );
        assert_eq!(
            mentioned("```\ncode\n``` @paul"),
            [1],
            "after the closing fence"
        );
        assert_eq!(mentioned("```\n@paul"), [1], "a fence never closed is text");
        assert_eq!(mentioned("**@paul** ||@bob|| _@alice_"), [1, 3, 2]);
        assert_eq!(mentioned("line\n@bob"), [3]);
    }

    fn role(id: u64, is_mentionable: bool) -> RoleWithMentionSetting {
        RoleWithMentionSetting {
            id: Snowflake::from_raw(id),
            is_mentionable,
        }
    }

    const GUILD: Snowflake = Snowflake::from_raw(500);

    fn mentioned_roles(content: &str, reach: RoleMentionReach) -> Vec<u64> {
        let roles = [role(500, false), role(501, true), role(502, false)];
        mentioned_role_ids(content, GUILD, &roles, reach)
            .into_iter()
            .map(Snowflake::raw)
            .collect()
    }

    #[test]
    fn roles_anyone_may_mention_are_mentioned_by_id_once_and_outside_code() {
        let reach = RoleMentionReach::RoleWithMentionSettings;
        assert_eq!(mentioned_roles("hi <@&501> and <@&501>", reach), [501]);
        assert_eq!(mentioned_roles("<@&502>", reach), Vec::<u64>::new());
        assert_eq!(mentioned_roles("`<@&501>`", reach), Vec::<u64>::new());
        assert_eq!(
            mentioned_roles("<@&999> <@&> <@&501", reach),
            Vec::<u64>::new()
        );
        assert_eq!(
            mentioned_roles("@everyone", reach),
            Vec::<u64>::new(),
            "everyone needs MENTION_EVERYONE"
        );
    }

    #[test]
    fn mention_everyone_reaches_every_role_and_everyone_but_not_the_guild_id_as_a_role() {
        let reach = RoleMentionReach::EveryRoleAndEveryone;
        assert_eq!(
            mentioned_roles("<@&502> **@everyone** <@&501>", reach),
            [500, 502, 501]
        );
        assert_eq!(mentioned_roles("<@&500>", reach), Vec::<u64>::new());
        assert_eq!(mentioned_roles("mail@everyone", reach), Vec::<u64>::new());
        assert_eq!(
            mentioned_roles("```\n@everyone\n```", reach),
            Vec::<u64>::new()
        );
        assert_eq!(mentioned_roles("hi @everyone.", reach), [500]);
        assert!(may_mention_a_role("x <@&1>"));
        assert!(may_mention_a_role("@Everyone"));
        assert!(!may_mention_a_role("@paul"));
    }

    #[test]
    fn everyone_is_never_a_person() {
        let people = [person(7, "everyone", "Everyone")];
        assert_eq!(
            mentioned_ids("@everyone", Snowflake::from_raw(99), &people),
            Vec::<Snowflake>::new()
        );
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
