//! The conversation's pure rules: which actions a message offers (`message-actions.ts` and
//! `messageMenuActions` in `context-menus.ts`), the typing line (`typingSegments` in `format.ts`),
//! how the virtual list follows a new set of rows, and when it asks for older history.

use std::ops::Range;

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

/// The keys of a message's hover bar, in order (DESIGN.md §5.4).
/// shortcut: no Forward and no quick reactions yet; they come with forwards and the emoji picker
/// (M3 step 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverBarAction {
    Edit,
    Reply,
    CopyText,
    Delete,
    More,
}

/// What a message's actions depend on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // The web's facts, one flag each.
pub struct MessageFacts {
    pub is_own: bool,
    /// A forward copies someone's words: it cannot be edited (ADR-0039).
    pub is_forward: bool,
    /// A poll, checklist or progress bar is neither edited as text nor copied (ADR-0043, ADR-0047).
    pub is_rich: bool,
    pub has_text: bool,
    pub is_pinned: bool,
    pub can_send_messages: bool,
    /// Delete others' messages: `MANAGE_MESSAGES`.
    pub can_manage_messages: bool,
    /// Joining an existing reaction: the channel's history is readable.
    pub can_join_reactions: bool,
    /// `MANAGE_MESSAGES` in a server channel; any participant in a conversation.
    pub can_manage_pins: bool,
}

impl MessageFacts {
    fn can_edit(self) -> bool {
        self.is_own && self.can_send_messages && !self.is_forward && !self.is_rich
    }

    fn can_delete(self) -> bool {
        self.is_own || self.can_manage_messages
    }
}

/// Edit takes Reply's place on a message you can edit, as on Discord; Delete stays in More
/// unless Shift is held, so the everyday bar stays short (`hoverBarActions`).
#[must_use]
pub fn hover_bar_actions(facts: MessageFacts, is_shift_held: bool) -> Vec<HoverBarAction> {
    let mut actions = Vec::new();
    if facts.can_edit() {
        actions.push(HoverBarAction::Edit);
    } else if facts.can_send_messages {
        actions.push(HoverBarAction::Reply);
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

/// A message menu item (DESIGN.md §6.33).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageMenuAction {
    /// Adds or takes back the reader's reaction with this stored key, one per existing reaction.
    /// shortcut: a new emoji needs the picker (M3 step 5); the menu offers the existing ones.
    ToggleReaction(String),
    Reply,
    Edit,
    CopyText,
    Pin,
    Unpin,
    CopyMessageId,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuLine {
    Action(MessageMenuAction),
    Separator,
}

/// The More button and a right-click open the same menu, as on Discord (`messageMenuActions`):
/// reactions, then the everyday actions, then Delete, each section only when not empty.
#[must_use]
pub fn message_menu_lines(facts: MessageFacts, reaction_keys: &[String]) -> Vec<MenuLine> {
    use MessageMenuAction as Action;
    let reactions: Vec<Action> = if facts.can_join_reactions {
        reaction_keys
            .iter()
            .map(|key| Action::ToggleReaction(key.clone()))
            .collect()
    } else {
        Vec::new()
    };
    let everyday: Vec<Action> = [
        facts.can_send_messages.then_some(Action::Reply),
        facts.can_edit().then_some(Action::Edit),
        (facts.has_text && !facts.is_rich).then_some(Action::CopyText),
        facts.can_manage_pins.then_some(if facts.is_pinned {
            Action::Unpin
        } else {
            Action::Pin
        }),
        Some(Action::CopyMessageId),
    ]
    .into_iter()
    .flatten()
    .collect();
    let danger: Vec<Action> = facts
        .can_delete()
        .then_some(Action::Delete)
        .into_iter()
        .collect();
    let mut lines = Vec::new();
    for section in [reactions, everyday, danger] {
        if section.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(MenuLine::Separator);
        }
        lines.extend(section.into_iter().map(MenuLine::Action));
    }
    lines
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

    const OTHERS_TEXT: MessageFacts = MessageFacts {
        is_own: false,
        is_forward: false,
        is_rich: false,
        has_text: true,
        is_pinned: false,
        can_send_messages: true,
        can_manage_messages: false,
        can_join_reactions: true,
        can_manage_pins: false,
    };
    const OWN_TEXT: MessageFacts = MessageFacts {
        is_own: true,
        ..OTHERS_TEXT
    };

    use HoverBarAction::{CopyText, Delete, Edit, More, Reply};

    // Ported from `message-actions.test.ts`, without Forward (not on the desktop yet).

    #[test]
    fn your_own_message_has_edit_copy_and_more_with_delete_left_in_more() {
        assert_eq!(hover_bar_actions(OWN_TEXT, false), [Edit, CopyText, More]);
    }

    #[test]
    fn others_messages_have_reply_copy_and_more_even_for_a_moderator() {
        let moderator = MessageFacts {
            can_manage_messages: true,
            ..OTHERS_TEXT
        };
        assert_eq!(hover_bar_actions(moderator, false), [Reply, CopyText, More]);
    }

    #[test]
    fn delete_joins_before_more_only_while_shift_is_held_and_only_where_you_may_delete() {
        assert_eq!(
            hover_bar_actions(OWN_TEXT, true),
            [Edit, CopyText, Delete, More]
        );
        let moderator = MessageFacts {
            can_manage_messages: true,
            ..OTHERS_TEXT
        };
        assert!(hover_bar_actions(moderator, true).contains(&Delete));
        assert!(!hover_bar_actions(OTHERS_TEXT, true).contains(&Delete));
    }

    #[test]
    fn a_message_without_text_has_nothing_to_copy() {
        let files_only = MessageFacts {
            has_text: false,
            ..OTHERS_TEXT
        };
        assert_eq!(hover_bar_actions(files_only, false), [Reply, More]);
    }

    #[test]
    fn your_own_forward_or_poll_offers_reply_since_it_cannot_be_edited() {
        let forward = MessageFacts {
            is_forward: true,
            ..OWN_TEXT
        };
        assert_eq!(hover_bar_actions(forward, false), [Reply, CopyText, More]);
        let poll = MessageFacts {
            is_rich: true,
            has_text: false,
            ..OWN_TEXT
        };
        assert_eq!(hover_bar_actions(poll, false), [Reply, More]);
    }

    #[test]
    fn neither_reply_nor_edit_where_you_cannot_send() {
        let read_only = MessageFacts {
            can_send_messages: false,
            ..OWN_TEXT
        };
        assert_eq!(hover_bar_actions(read_only, false), [CopyText, More]);
    }

    // Ported from `context-menus.test.ts` ("messageMenuActions"): reactions toggle the existing
    // ones, and Copy message ID joins the everyday section.

    fn actions(lines: &[MenuLine]) -> Vec<Option<MessageMenuAction>> {
        lines
            .iter()
            .map(|line| match line {
                MenuLine::Action(action) => Some(action.clone()),
                MenuLine::Separator => None,
            })
            .collect()
    }

    #[test]
    fn a_reader_of_someone_elses_message_gets_neither_edit_nor_delete() {
        use MessageMenuAction::{CopyMessageId, CopyText, Reply, ToggleReaction};
        assert_eq!(
            actions(&message_menu_lines(OTHERS_TEXT, &["👍".to_owned()])),
            [
                Some(ToggleReaction("👍".to_owned())),
                None,
                Some(Reply),
                Some(CopyText),
                Some(CopyMessageId)
            ]
        );
    }

    #[test]
    fn your_own_message_adds_edit_and_delete_last_and_separated() {
        use MessageMenuAction::{CopyMessageId, CopyText, Delete, Edit, Reply};
        assert_eq!(
            actions(&message_menu_lines(OWN_TEXT, &[])),
            [
                Some(Reply),
                Some(Edit),
                Some(CopyText),
                Some(CopyMessageId),
                None,
                Some(Delete)
            ]
        );
    }

    #[test]
    fn a_moderator_pins_and_deletes_anyones_message_and_unpins_a_pinned_one() {
        let moderator = MessageFacts {
            can_manage_messages: true,
            can_manage_pins: true,
            ..OTHERS_TEXT
        };
        let lines = message_menu_lines(moderator, &[]);
        assert!(lines.contains(&MenuLine::Action(MessageMenuAction::Pin)));
        assert!(lines.contains(&MenuLine::Action(MessageMenuAction::Delete)));
        let pinned = MessageFacts {
            is_pinned: true,
            ..moderator
        };
        assert!(
            message_menu_lines(pinned, &[]).contains(&MenuLine::Action(MessageMenuAction::Unpin))
        );
    }

    #[test]
    fn a_read_only_channel_leaves_out_reply_edit_and_reactions_without_history() {
        use MessageMenuAction::{CopyMessageId, CopyText, Delete};
        let read_only = MessageFacts {
            can_send_messages: false,
            can_join_reactions: false,
            ..OWN_TEXT
        };
        assert_eq!(
            actions(&message_menu_lines(read_only, &["👍".to_owned()])),
            [Some(CopyText), Some(CopyMessageId), None, Some(Delete)]
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
