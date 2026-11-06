//! Role and `@everyone` mentions (`web/src/components/role-mentions.ts`, ADR-0040, DESIGN.md
//! §5.4, §5.5): how `<@&id>` reads as text, which roles the writer may mention, the `@`
//! suggestions for them, and turning a picked role's `@Name` into its markup on send.

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

use mellow_protocol::{Role, Snowflake};

use super::markdown::{find_code_spans, with_custom_emoji_as_names};
use super::mentions::{EVERYONE_MENTION_WORD, MAXIMUM_MENTION_SUGGESTIONS};
use super::search_text::fold_for_search;
use super::unicode_classes::{character_at, character_before, is_js_whitespace, js_trim};

/// `@Modérateurs`, or `@` and `deleted_role_name` (the catalogue's « rôle-supprimé ») for a role
/// this client does not know.
#[must_use]
pub fn role_mention_label(role: Option<&Role>, deleted_role_name: &str) -> String {
    format!(
        "@{}",
        role.map_or(deleted_role_name, |role| role.name.as_str())
    )
}

/// A message as plain text (previews, reply lines, pins): emoji as `:name:`, roles as `@Name`.
#[must_use]
pub fn message_as_plain_text<S: BuildHasher>(
    content: &str,
    roles: &HashMap<Snowflake, Role, S>,
    deleted_role_name: &str,
) -> String {
    let text = with_custom_emoji_as_names(content);
    let mut converted = String::with_capacity(text.len());
    let mut copied_up_to = 0;
    let mut search_from = 0;
    while let Some(start) = text
        .get(search_from..)
        .and_then(|rest| rest.find("<@&"))
        .map(|offset| offset + search_from)
    {
        search_from = start + 1;
        let digits_start = start + "<@&".len();
        let rest = text.get(digits_start..).unwrap_or_default();
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if !(1..=20).contains(&digits) || rest.as_bytes().get(digits) != Some(&b'>') {
            continue;
        }
        let id = rest
            .get(..digits)
            .and_then(|digits| digits.parse().ok())
            .map(Snowflake::from_raw);
        converted.push_str(text.get(copied_up_to..start).unwrap_or_default());
        converted.push_str(&role_mention_label(
            id.and_then(|id| roles.get(&id)),
            deleted_role_name,
        ));
        copied_up_to = digits_start + digits + 1;
        search_from = copied_up_to;
    }
    converted.push_str(text.get(copied_up_to..).unwrap_or_default());
    converted
}

// --- What the writer may mention -------------------------------------------------------------

/// The roles the writer may mention in a channel, highest first: those anyone may mention, or
/// every role with `MENTION_EVERYONE` (ADR-0040). Never `@everyone`'s role (its id is the
/// guild's): everyone is mentioned by the word.
#[must_use]
pub fn roles_writer_may_mention(
    roles: &[Role],
    guild_id: Snowflake,
    can_mention_everyone: bool,
) -> Vec<&Role> {
    let mut mentionable: Vec<&Role> = roles
        .iter()
        .filter(|role| role.id != guild_id && (can_mention_everyone || role.is_mentionable))
        .collect();
    mentionable.sort_by_key(|role| std::cmp::Reverse(role.position));
    mentionable
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleMentionSuggestion<'a> {
    Everyone,
    Role(&'a Role),
}

/// 0 for a name starting with the folded query, 1 for a word of it starting with it, 2 holding it.
fn role_match_rank(name: &str, folded_query: &str) -> Option<u8> {
    let folded = fold_for_search(name);
    if folded.starts_with(folded_query) {
        return Some(0);
    }
    if folded
        .split(is_js_whitespace)
        .any(|word| word.starts_with(folded_query))
    {
        return Some(1);
    }
    folded.contains(folded_query).then_some(2)
}

/// What `@query` offers after the people: `@everyone` when the writer may use it and it
/// matches, then the matching roles of `mentionable_roles` (from [`roles_writer_may_mention`]),
/// best match first, then highest role, at most [`MAXIMUM_MENTION_SUGGESTIONS`].
#[must_use]
pub fn suggest_role_mentions<'r>(
    query: &str,
    mentionable_roles: &[&'r Role],
    can_mention_everyone: bool,
) -> Vec<RoleMentionSuggestion<'r>> {
    let folded_query = fold_for_search(query);
    let everyone = (can_mention_everyone
        && EVERYONE_MENTION_WORD.starts_with(folded_query.as_str()))
    .then_some(RoleMentionSuggestion::Everyone);
    let mut roles: Vec<(u8, usize, &Role)> = mentionable_roles
        .iter()
        .enumerate()
        .filter_map(|(order, role)| {
            role_match_rank(&role.name, &folded_query).map(|rank| (rank, order, *role))
        })
        .collect();
    // The order is unique, so the web's last tie-break (by name) never applies.
    roles.sort_by_key(|(rank, order, _)| (*rank, *order));
    everyone
        .into_iter()
        .chain(
            roles
                .into_iter()
                .map(|(_, _, role)| RoleMentionSuggestion::Role(role)),
        )
        .take(MAXIMUM_MENTION_SUGGESTIONS)
        .collect()
}

// --- Sending ---------------------------------------------------------------------------------

/// `[\p{Alphabetic}\p{N}_]`: a role name may be followed by punctuation, not by more of a word.
fn is_word_character(character: char) -> bool {
    character.is_alphabetic() || character.is_numeric() || character == '_'
}

/// A username may hold dots ("mod.ana"): a dot followed by a name character continues the name.
fn is_username_character(character: char) -> bool {
    is_word_character(character) || character == '.'
}

/// What may come right before a mention's "@", as the server reads it.
fn is_mention_preceder(character: char) -> bool {
    is_js_whitespace(character) || matches!(character, '*' | '_' | '~' | '|')
}

fn is_escaped_at(text: &str, position: usize) -> bool {
    let backslashes = text
        .as_bytes()
        .get(..position)
        .unwrap_or_default()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    backslashes % 2 == 1
}

/// The role whose name, in any case, is written right after the "@" at `at`, longest name first.
fn role_named_at<'r>(
    text: &str,
    at: usize,
    roles_by_longest_name: &[&'r Role],
) -> Option<&'r Role> {
    roles_by_longest_name.iter().copied().find(|role| {
        let end = at + 1 + role.name.len();
        text.get(at + 1..end).is_some_and(|written| written.to_lowercase() == role.name.to_lowercase())
            && !character_at(text, end).is_some_and(is_word_character)
            // "@mod.ana" names a person even with a role "Mod"; "@Mod." ending a sentence is the role.
            && !(character_at(text, end) == Some('.') && character_at(text, end + 1).is_some_and(is_username_character))
    })
}

fn convert_role_names(text: &str, roles_by_longest_name: &[&Role]) -> String {
    let mut converted = String::with_capacity(text.len());
    let mut copied_up_to = 0;
    let mut search_from = 0;
    while let Some(at) = text
        .get(search_from..)
        .and_then(|rest| rest.find('@'))
        .map(|offset| offset + search_from)
    {
        search_from = at + 1;
        if at < copied_up_to
            || (at > 0 && !character_before(text, at).is_some_and(is_mention_preceder))
            || is_escaped_at(text, at)
        {
            continue;
        }
        if let Some(role) = role_named_at(text, at, roles_by_longest_name) {
            converted.push_str(text.get(copied_up_to..at).unwrap_or_default());
            converted.push_str("<@&");
            converted.push_str(&role.id.raw().to_string());
            converted.push('>');
            copied_up_to = at + 1 + role.name.len();
        }
    }
    converted.push_str(text.get(copied_up_to..).unwrap_or_default());
    converted
}

/// The message as sent: each `@Name` of one of `mentionable_roles` (the roles the writer may
/// mention) becomes `<@&id>`, which the composer cannot show, as `:name:` becomes an emoji's
/// markup. Not in code or after a backslash; never a name that is someone's username in
/// `people_names` (lowercase), so picking a person keeps mentioning that person, nor "everyone".
#[must_use]
pub fn with_role_names_as_markup<S: BuildHasher>(
    text: &str,
    mentionable_roles: &[&Role],
    people_names: &HashSet<String, S>,
) -> String {
    if mentionable_roles.is_empty() || !text.contains('@') {
        return text.to_owned();
    }
    let mut roles_by_longest_name: Vec<&Role> = mentionable_roles
        .iter()
        .copied()
        .filter(|role| {
            let name = role.name.to_lowercase();
            !js_trim(&name).is_empty()
                && name != EVERYONE_MENTION_WORD
                && !people_names.contains(&name)
        })
        .collect();
    roles_by_longest_name.sort_by(|left, right| {
        right
            .name
            .encode_utf16()
            .count()
            .cmp(&left.name.encode_utf16().count())
    });
    let mut converted = String::with_capacity(text.len());
    let mut copied_up_to = 0;
    // In increasing order of start: the scan runs forward.
    for (start, span) in find_code_spans(text) {
        converted.push_str(&convert_role_names(
            text.get(copied_up_to..start).unwrap_or_default(),
            &roles_by_longest_name,
        ));
        converted.push_str(text.get(start..span.end).unwrap_or_default());
        copied_up_to = span.end;
    }
    converted.push_str(&convert_role_names(
        text.get(copied_up_to..).unwrap_or_default(),
        &roles_by_longest_name,
    ));
    converted
}

#[cfg(test)]
mod tests {
    use mellow_protocol::Permissions;

    use super::*;

    const GUILD: u64 = 100;
    const DELETED: &str = "deleted-role";

    fn role(id: u64, name: &str, position: u32, is_mentionable: bool) -> Role {
        Role {
            id: Snowflake::from_raw(id),
            name: name.to_owned(),
            color: 0,
            position,
            permissions: Permissions::empty(),
            is_displayed_separately: false,
            is_mentionable,
            icon: None,
            unicode_emoji: None,
        }
    }

    fn moderators() -> Role {
        role(201, "Modérateurs", 3, true)
    }
    fn sound() -> Role {
        role(202, "Équipe son", 2, false)
    }
    fn admins() -> Role {
        role(203, "Admins", 5, false)
    }
    fn guild_roles() -> Vec<Role> {
        vec![
            role(GUILD, "@everyone", 0, false),
            sound(),
            moderators(),
            admins(),
        ]
    }

    #[test]
    fn names_a_role_or_says_it_was_deleted() {
        assert_eq!(
            role_mention_label(Some(&moderators()), DELETED),
            "@Modérateurs"
        );
        assert_eq!(role_mention_label(None, DELETED), "@deleted-role");
        let roles = HashMap::from([(moderators().id, moderators())]);
        assert_eq!(
            message_as_plain_text("<@&201> and <@&999> <:party:5>", &roles, DELETED),
            "@Modérateurs and @deleted-role :party:"
        );
    }

    #[test]
    fn keeps_only_mentionable_roles_without_mention_everyone_never_everyones_role() {
        let roles = guild_roles();
        assert_eq!(
            roles_writer_may_mention(&roles, Snowflake::from_raw(GUILD), false),
            vec![&moderators()]
        );
        assert_eq!(
            roles_writer_may_mention(&roles, Snowflake::from_raw(GUILD), true),
            vec![&admins(), &moderators(), &sound()]
        );
    }

    #[test]
    fn suggests_everyone_first_then_roles_by_match_then_rank() {
        let roles = guild_roles();
        let allowed = roles_writer_may_mention(&roles, Snowflake::from_raw(GUILD), true);
        let (admins, moderators, sound) = (admins(), moderators(), sound());
        assert_eq!(
            suggest_role_mentions("", &allowed, true),
            vec![
                RoleMentionSuggestion::Everyone,
                RoleMentionSuggestion::Role(&admins),
                RoleMentionSuggestion::Role(&moderators),
                RoleMentionSuggestion::Role(&sound)
            ]
        );
        assert_eq!(
            suggest_role_mentions("mod", &allowed, true),
            vec![RoleMentionSuggestion::Role(&moderators)]
        );
        assert_eq!(
            suggest_role_mentions("son", &allowed, true),
            vec![RoleMentionSuggestion::Role(&sound)]
        );
        assert_eq!(
            suggest_role_mentions("e", &allowed, true),
            vec![
                RoleMentionSuggestion::Everyone,
                RoleMentionSuggestion::Role(&sound),
                RoleMentionSuggestion::Role(&moderators)
            ]
        );
        assert_eq!(suggest_role_mentions("ev", &[&moderators], false), vec![]);
        let many: Vec<Role> = (0..20)
            .map(|index| {
                role(
                    300 + index,
                    &format!("Team {index}"),
                    u32::try_from(index).unwrap_or(0),
                    true,
                )
            })
            .collect();
        let many: Vec<&Role> = many.iter().collect();
        assert_eq!(
            suggest_role_mentions("team", &many, false).len(),
            MAXIMUM_MENTION_SUGGESTIONS
        );
    }

    #[test]
    fn leaves_a_dotted_username_that_starts_like_a_role_name_to_the_person() {
        let moderator = role(301, "Mod", 1, true);
        assert_eq!(
            with_role_names_as_markup(
                "@mod.ana hi",
                &[&moderator],
                &HashSet::from(["mod.ana".to_owned()])
            ),
            "@mod.ana hi"
        );
        assert_eq!(
            with_role_names_as_markup("ask @Mod.", &[&moderator], &HashSet::new()),
            "ask <@&301>."
        );
    }

    #[test]
    fn turns_a_picked_roles_name_into_its_markup_longest_name_first_any_case() {
        let (moderators, sound) = (moderators(), sound());
        let allowed = [&moderators, &sound];
        let nobody = HashSet::new();
        assert_eq!(
            with_role_names_as_markup("@Équipe son, @modérateurs!", &allowed, &nobody),
            "<@&202>, <@&201>!"
        );
        assert_eq!(
            with_role_names_as_markup("**@Modérateurs**", &allowed, &nobody),
            "**<@&201>**"
        );
        let left_alone = "`@Modérateurs` \\@Modérateurs @Modérateursx a@Modérateurs";
        assert_eq!(
            with_role_names_as_markup(left_alone, &allowed, &nobody),
            left_alone
        );
        assert_eq!(
            with_role_names_as_markup("@Admins", &allowed, &nobody),
            "@Admins"
        );
        assert_eq!(
            with_role_names_as_markup(
                "@modérateurs",
                &allowed,
                &HashSet::from(["modérateurs".to_owned()])
            ),
            "@modérateurs"
        );
        let everyone_named = role(204, "everyone", 1, true);
        assert_eq!(
            with_role_names_as_markup("@everyone", &[&everyone_named], &nobody),
            "@everyone"
        );
    }
}
