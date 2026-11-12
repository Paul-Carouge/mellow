//! The conversation's pure rules: which keys a message's hover bar offers (`message-actions.ts`;
//! its menu is client-core's `message_menu_actions`), the typing line (`typingSegments` in `format.ts`),
//! who reacted (`MessageReactions.svelte`), how the virtual list follows a new set of rows, and when
//! it asks for older history.

use std::ops::Range;

use mellow_client_core::shared_logic::context_menus_people::MessageMenuFacts;
use mellow_protocol::{DirectMessageChannel, PublicUser, Snowflake};

use crate::i18n::Strings;

// ---- Who may write ----

/// Who locks the composer: the other person of a one-to-one conversation the reader blocked
/// (ADR-0025). A group never locks, whatever the relationships in it (ADR-0026): their messages
/// fold. The web's `blockedRecipientOf`.
pub fn blocked_recipient_of(
    conversation: &DirectMessageChannel,
    current_user_id: Option<Snowflake>,
    is_blocked: impl Fn(Snowflake) -> bool,
) -> Option<&PublicUser> {
    if conversation.owner_id.is_some() {
        return None;
    }
    conversation
        .recipients
        .iter()
        .filter(|recipient| Some(recipient.id) != current_user_id)
        .find(|recipient| is_blocked(recipient.id))
}

// ---- Message actions ----

/// The keys of a message's hover bar after its reactions, in order (DESIGN.md §5.4). The quick
/// reactions and Add reaction lead the bar wherever `can_add_reactions` holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverBarAction {
    Edit,
    Reply,
    Forward,
    CopyText,
    Delete,
    More,
}

/// Edit takes Reply's place on a message you can edit, as on Discord; Delete stays in More
/// unless Shift is held, so the everyday bar stays short (`hoverBarActions`).
#[must_use]
pub fn hover_bar_actions(facts: MessageMenuFacts, is_shift_held: bool) -> Vec<HoverBarAction> {
    let mut actions = Vec::new();
    if facts.can_edit() {
        actions.push(HoverBarAction::Edit);
    } else if facts.can_send_messages {
        actions.push(HoverBarAction::Reply);
    }
    if !facts.is_rich {
        actions.push(HoverBarAction::Forward);
    }
    if !facts.is_rich && facts.has_text {
        actions.push(HoverBarAction::CopyText);
    }
    if is_shift_held && facts.can_delete() {
        actions.push(HoverBarAction::Delete);
    }
    // The menu always has Copy message ID.
    actions.push(HoverBarAction::More);
    actions
}

// ---- Who reacted ----

/// Others named before "and 3 others", as on Discord.
const NAMED_REACTORS: usize = 3;

/// "You, Ana, Bo and 3 others reacted with 👍" (`sentenceFor`): the reader first when they are
/// among the `count`, then up to three others from `users`, then how many are left.
#[must_use]
pub fn reactor_sentence(
    users: &[PublicUser],
    count: u32,
    is_mine: bool,
    reader_id: Option<Snowflake>,
    emoji: &str,
    strings: Strings,
) -> String {
    let named: Vec<&str> = users
        .iter()
        .filter(|user| Some(user.id) != reader_id)
        .take(NAMED_REACTORS)
        .map(mellow_client_core::shared_logic::format::display_name)
        .collect();
    let unnamed = usize::try_from(count)
        .unwrap_or(usize::MAX)
        .saturating_sub(usize::from(is_mine) + named.len());
    strings.reactions().reacted_with(
        is_mine,
        &named,
        i64::try_from(unnamed).unwrap_or(i64::MAX),
        emoji,
    )
}

// ---- Typing ----

const MAXIMUM_TYPING_NAMES_LISTED: usize = 2;
const MAXIMUM_TYPING_PEOPLE_COUNTED: usize = 5;

/// One piece of the typing line; names show in bold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypingSegment {
    pub text: String,
    pub is_name: bool,
}

/// "Ana is typing…", "Ana and Léo are typing…", "Ana, Léo and 1 other are typing…", then
/// "Several people are typing…" (`typingSegments`).
#[must_use]
pub fn typing_segments(names: &[String], strings: Strings) -> Vec<TypingSegment> {
    let typing = strings.typing();
    let name = |text: &str| TypingSegment {
        text: text.to_owned(),
        is_name: true,
    };
    let words = |text: &str| TypingSegment {
        text: text.to_owned(),
        is_name: false,
    };
    match names {
        [] => Vec::new(),
        _ if names.len() > MAXIMUM_TYPING_PEOPLE_COUNTED => vec![words(typing.several())],
        [first] => vec![name(first), words(typing.is_typing())],
        [first, second] => vec![
            name(first),
            words(typing.and()),
            name(second),
            words(typing.are_typing()),
        ],
        [first, second, ..] => {
            let others = i64::try_from(names.len() - MAXIMUM_TYPING_NAMES_LISTED).unwrap_or(0);
            vec![
                name(first),
                words(typing.list_separator()),
                name(second),
                words(&typing.and_others_are_typing(others)),
            ]
        }
    }
}

// ---- The virtual list ----

/// What the list must remeasure after the rows changed: the rows between the unchanged start
/// and end are replaced, and kept rows whose content changed are measured again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListUpdate {
    /// Old rows replaced, and how many new rows take their place.
    pub splice: Option<(Range<usize>, usize)>,
    /// Indices in the new rows.
    pub remeasure: Vec<usize>,
}

/// Compares two sets of rows by key (where a row is) and version (what it shows). Older pages
/// land between the history start and the first old row, so the rows being read keep their
/// index after the splice and the list keeps them in place.
#[must_use]
pub fn list_update<K: PartialEq>(old: &[(K, u64)], new: &[(K, u64)]) -> ListUpdate {
    let prefix = old
        .iter()
        .zip(new)
        .take_while(|(before, after)| before.0 == after.0)
        .count();
    let room = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(before, after)| before.0 == after.0)
        .count();
    let replaced = prefix..old.len() - suffix;
    let inserted = new.len() - prefix - suffix;
    let splice = (!replaced.is_empty() || inserted > 0).then_some((replaced, inserted));
    let new_suffix_start = new.len() - suffix;
    let remeasure = (0..prefix)
        .chain(new_suffix_start..new.len())
        .filter(|&index| {
            let old_index = if index < prefix {
                index
            } else {
                index - new_suffix_start + (old.len() - suffix)
            };
            old[old_index].1 != new[index].1
        })
        .collect();
    ListUpdate { splice, remeasure }
}

/// Older history is asked for when the top of the list is this close (the web's
/// `component.message-history.load-distance`), unless the channel's start is loaded, a page is on
/// its way or the last one failed.
#[must_use]
pub fn should_load_older_history(
    distance_from_top: f32,
    load_distance: f32,
    has_older_history: bool,
    is_loading_or_failed: bool,
) -> bool {
    has_older_history && !is_loading_or_failed && distance_from_top < load_distance
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    fn conversation_with(recipients: &[PublicUser], owner_id: Option<u64>) -> DirectMessageChannel {
        DirectMessageChannel {
            id: Snowflake::from_raw(70),
            recipients: recipients.to_vec(),
            last_message_id: None,
            owner_id: owner_id.map(Snowflake::from_raw),
            name: String::new(),
        }
    }

    fn someone(id: u64, name: &str) -> PublicUser {
        PublicUser {
            id: Snowflake::from_raw(id),
            username: name.to_lowercase(),
            display_name: name.to_owned(),
            avatar: None,
            banner: None,
        }
    }

    #[test]
    fn a_one_to_one_conversation_with_someone_you_blocked_is_locked() {
        let me = Some(Snowflake::from_raw(1));
        let (ana, bob) = (someone(30, "Ana"), someone(12, "Bob"));
        let is_blocked = |user_id: Snowflake| user_id == bob.id;
        let with_bob = conversation_with(std::slice::from_ref(&bob), None);
        assert_eq!(blocked_recipient_of(&with_bob, me, is_blocked), Some(&bob));
        let with_ana = conversation_with(std::slice::from_ref(&ana), None);
        assert_eq!(blocked_recipient_of(&with_ana, me, is_blocked), None);
    }

    #[test]
    fn a_group_never_locks_even_with_someone_you_blocked_in_it() {
        let (ana, bob) = (someone(30, "Ana"), someone(12, "Bob"));
        let group = conversation_with(&[ana, bob.clone()], Some(1));
        let is_blocked = |user_id: Snowflake| user_id == bob.id;
        assert_eq!(
            blocked_recipient_of(&group, Some(Snowflake::from_raw(1)), is_blocked),
            None
        );
    }

    const OTHERS_TEXT: MessageMenuFacts = MessageMenuFacts {
        is_own: false,
        is_forward: false,
        is_rich: false,
        is_poll_open: false,
        has_text: true,
        is_pinned: false,
        can_send_messages: true,
        can_manage_messages: false,
        has_reactions: false,
        can_join_reactions: true,
        can_add_reactions: true,
        can_remove_reactions: false,
        can_manage_pins: false,
    };
    const OWN_TEXT: MessageMenuFacts = MessageMenuFacts {
        is_own: true,
        ..OTHERS_TEXT
    };

    use HoverBarAction::{CopyText, Delete, Edit, Forward, More, Reply};

    // Ported from `message-actions.test.ts`.

    #[test]
    fn your_own_message_has_edit_forward_copy_and_more_with_delete_left_in_more() {
        assert_eq!(
            hover_bar_actions(OWN_TEXT, false),
            [Edit, Forward, CopyText, More]
        );
    }

    #[test]
    fn others_messages_have_reply_forward_copy_and_more_even_for_a_moderator() {
        let moderator = MessageMenuFacts {
            can_manage_messages: true,
            ..OTHERS_TEXT
        };
        assert_eq!(
            hover_bar_actions(moderator, false),
            [Reply, Forward, CopyText, More]
        );
    }

    #[test]
    fn delete_joins_before_more_only_while_shift_is_held_and_only_where_you_may_delete() {
        assert_eq!(
            hover_bar_actions(OWN_TEXT, true),
            [Edit, Forward, CopyText, Delete, More]
        );
        let moderator = MessageMenuFacts {
            can_manage_messages: true,
            ..OTHERS_TEXT
        };
        assert!(hover_bar_actions(moderator, true).contains(&Delete));
        assert!(!hover_bar_actions(OTHERS_TEXT, true).contains(&Delete));
    }

    #[test]
    fn a_message_without_text_has_nothing_to_copy() {
        let files_only = MessageMenuFacts {
            has_text: false,
            ..OTHERS_TEXT
        };
        assert_eq!(hover_bar_actions(files_only, false), [Reply, Forward, More]);
    }

    #[test]
    fn your_own_forward_or_poll_offers_reply_since_it_cannot_be_edited() {
        let forward = MessageMenuFacts {
            is_forward: true,
            ..OWN_TEXT
        };
        assert_eq!(
            hover_bar_actions(forward, false),
            [Reply, Forward, CopyText, More]
        );
        let poll = MessageMenuFacts {
            is_rich: true,
            has_text: false,
            ..OWN_TEXT
        };
        assert_eq!(hover_bar_actions(poll, false), [Reply, More]);
    }

    #[test]
    fn neither_reply_nor_edit_where_you_cannot_send() {
        let read_only = MessageMenuFacts {
            can_send_messages: false,
            ..OWN_TEXT
        };
        assert_eq!(
            hover_bar_actions(read_only, false),
            [Forward, CopyText, More]
        );
    }

    fn named(id: u64, name: &str) -> PublicUser {
        PublicUser {
            id: Snowflake::from_raw(id),
            username: name.to_lowercase(),
            display_name: name.to_owned(),
            avatar: None,
            banner: None,
        }
    }

    #[test]
    fn who_reacted_names_the_reader_first_then_three_others_then_counts_the_rest() {
        let strings = Strings::new(Language::English);
        let reader = Snowflake::from_raw(1);
        let users = [
            named(1, "Me"),
            named(2, "Ana"),
            named(3, "Bo"),
            named(4, "Cy"),
            named(5, "Di"),
        ];
        assert_eq!(
            reactor_sentence(&users, 6, true, Some(reader), "👍", strings),
            "You, Ana, Bo, Cy and 2 others reacted with\u{a0}👍"
        );
        assert_eq!(
            reactor_sentence(&users[1..2], 1, false, Some(reader), "👍", strings),
            "Ana reacted with\u{a0}👍"
        );
        // A page that has not caught up yet never counts below zero.
        assert_eq!(
            reactor_sentence(&users[1..3], 1, false, Some(reader), "👍", strings),
            "Ana and Bo reacted with\u{a0}👍"
        );
    }

    #[test]
    fn the_typing_line_follows_the_design_wording_from_one_person_to_several() {
        let strings = Strings::new(Language::English);
        let line = |names: &[&str]| {
            let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
            typing_segments(&names, strings)
                .into_iter()
                .map(|segment| segment.text)
                .collect::<String>()
        };
        assert_eq!(line(&[]), "");
        assert_eq!(line(&["Ana"]), "Ana is typing…");
        assert_eq!(line(&["Ana", "Léo"]), "Ana and Léo are typing…");
        assert_eq!(
            line(&["Ana", "Léo", "Zoé"]),
            "Ana, Léo and 1 other are typing…"
        );
        assert_eq!(
            line(&["A", "B", "C", "D", "E"]),
            "A, B and 3 others are typing…"
        );
        assert_eq!(
            line(&["A", "B", "C", "D", "E", "F"]),
            "Several people are typing…"
        );
    }

    fn rows(keys: &[(&'static str, u64)]) -> Vec<(&'static str, u64)> {
        keys.to_vec()
    }

    #[test]
    fn an_older_page_is_spliced_after_the_history_start_so_the_rows_read_keep_their_place() {
        let before = rows(&[("start", 0), ("10", 0), ("11", 0)]);
        let after = rows(&[("start", 1), ("8", 0), ("9", 0), ("10", 0), ("11", 0)]);
        assert_eq!(
            list_update(&before, &after),
            ListUpdate {
                splice: Some((1..1, 2)),
                remeasure: vec![0],
            }
        );
    }

    #[test]
    fn a_new_message_is_appended_and_an_edit_is_only_measured_again() {
        let before = rows(&[("start", 0), ("10", 0), ("pending", 0)]);
        let sent = rows(&[("start", 0), ("10", 1), ("11", 0)]);
        assert_eq!(
            list_update(&before, &sent),
            ListUpdate {
                splice: Some((2..3, 1)),
                remeasure: vec![1],
            }
        );
        assert_eq!(
            list_update(&sent, &sent),
            ListUpdate {
                splice: None,
                remeasure: Vec::new(),
            }
        );
    }

    #[test]
    fn a_deleted_message_in_the_middle_is_spliced_out() {
        let before = rows(&[("start", 0), ("10", 0), ("11", 0), ("12", 0)]);
        let after = rows(&[("start", 0), ("10", 0), ("12", 0)]);
        assert_eq!(list_update(&before, &after).splice, Some((2..3, 0)));
    }

    #[test]
    fn older_history_loads_near_the_top_until_the_start_once_at_a_time() {
        assert!(should_load_older_history(100.0, 800.0, true, false));
        assert!(!should_load_older_history(900.0, 800.0, true, false));
        assert!(!should_load_older_history(0.0, 800.0, false, false));
        assert!(!should_load_older_history(0.0, 800.0, true, true));
    }
}
