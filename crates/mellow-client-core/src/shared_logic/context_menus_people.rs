//! Which items a message's, a person's and a group conversation's context menus show
//! (`messageMenuActions`, `personMenuActions` and `groupMenuActions` in
//! `web/src/components/context-menus.ts`, DESIGN.md §6.33, `docs/design/web-context-menus.md`):
//! only actions the client can already do, each only when the reader may do it. Items come in
//! sections separated by a line; danger items are always the last section. How each item reads
//! (`menuItemLook`) belongs to the client's catalogue and icons.

use mellow_protocol::RelationshipKind;

/// One line of a menu: an action, or the line between two sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuLine<Action> {
    Action(Action),
    Separator,
}

/// The non-empty sections, with a separator between each (`joinSections`).
fn join_sections<Action>(sections: Vec<Vec<Option<Action>>>) -> Vec<MenuLine<Action>> {
    let mut lines = Vec::new();
    for section in sections {
        let mut actions = section.into_iter().flatten().peekable();
        if actions.peek().is_none() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(MenuLine::Separator);
        }
        lines.extend(actions.map(MenuLine::Action));
    }
    lines
}

// ---- A message ----

/// What a message's actions depend on (`MessageMenuFacts`, also read by the hover bar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // The web's facts, one flag each.
pub struct MessageMenuFacts {
    pub is_own: bool,
    /// A forward copies someone's words: it cannot be edited (ADR-0039).
    pub is_forward: bool,
    /// A poll, checklist or progress bar is neither edited as text nor copied (ADR-0043, ADR-0047).
    pub is_rich: bool,
    /// A poll still taking votes: its author may end it early (ADR-0043).
    pub is_poll_open: bool,
    pub has_text: bool,
    pub is_pinned: bool,
    pub can_send_messages: bool,
    /// Delete others' messages: `MANAGE_MESSAGES`.
    pub can_manage_messages: bool,
    pub has_reactions: bool,
    /// Joining an existing reaction: the channel's history is readable.
    pub can_join_reactions: bool,
    /// Starting a new one also needs `ADD_REACTIONS`.
    pub can_add_reactions: bool,
    /// Other people's reactions: `MANAGE_MESSAGES` in a server channel whose history is readable.
    pub can_remove_reactions: bool,
    /// `MANAGE_MESSAGES` in a server channel; any participant in a conversation.
    pub can_manage_pins: bool,
}

impl MessageMenuFacts {
    #[must_use]
    pub fn can_edit(self) -> bool {
        self.is_own && self.can_send_messages && !self.is_forward && !self.is_rich
    }

    #[must_use]
    pub fn can_delete(self) -> bool {
        self.is_own || self.can_manage_messages
    }
}

/// A message menu item, as the web's `MessageMenuAction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageMenuAction {
    /// Opens the emoji picker to react with any emoji.
    AddReaction,
    Reply,
    /// Opens the forward dialog (ADR-0039).
    Forward,
    Edit,
    CopyText,
    Pin,
    Unpin,
    /// Who reacted with what, in a dialog.
    ViewReactions,
    /// Its author stops the votes now, after a confirmation.
    EndPoll,
    /// A moderator takes every reaction off the message, after a confirmation.
    RemoveAllReactions,
    Delete,
}

/// The More button, a right-click and Shift+F10 open the same menu, as on Discord and the web
/// (`messageMenuActions`): Add reaction, then the everyday actions, then the irreversible ones.
/// The message's ID is not offered: neither client has a developer mode yet.
#[must_use]
pub fn message_menu_actions(facts: MessageMenuFacts) -> Vec<MenuLine<MessageMenuAction>> {
    use MessageMenuAction as Action;
    join_sections(vec![
        vec![facts.can_add_reactions.then_some(Action::AddReaction)],
        vec![
            facts.can_send_messages.then_some(Action::Reply),
            // Forwarding needs somewhere else to send to, which the dialog finds; reading is enough.
            (!facts.is_rich).then_some(Action::Forward),
            facts.can_edit().then_some(Action::Edit),
            (facts.has_text && !facts.is_rich).then_some(Action::CopyText),
            facts.can_manage_pins.then_some(if facts.is_pinned {
                Action::Unpin
            } else {
                Action::Pin
            }),
            (facts.has_reactions && facts.can_join_reactions).then_some(Action::ViewReactions),
        ],
        vec![
            (facts.is_own && facts.is_poll_open).then_some(Action::EndPoll),
            (facts.has_reactions && facts.can_remove_reactions)
                .then_some(Action::RemoveAllReactions),
            facts.can_delete().then_some(Action::Delete),
        ],
    ])
}

/// The quick reactions the menu shows as one row of emoji keys above its items (the web phone
/// sheet's header row, `MessageMenuQuickReactions.svelte`): none where the reader may not react.
#[must_use]
pub fn message_menu_quick_reactions(
    facts: MessageMenuFacts,
    quick_reactions: &[String],
) -> Vec<String> {
    if facts.can_add_reactions {
        quick_reactions.to_vec()
    } else {
        Vec::new()
    }
}

// ---- A person (a member, a message's author, a friend, a one-to-one conversation) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonMenuAction {
    Profile,
    Message,
    AddFriend,
    AcceptRequest,
    IgnoreRequest,
    CancelRequest,
    Unblock,
    Mute,
    Unmute,
    NotificationSettings,
    CopyUserId,
    CopyChannelId,
    RemoveFriend,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // The web's facts, one flag each.
pub struct PersonMenuFacts {
    pub is_self: bool,
    pub relationship: Option<RelationshipKind>,
    /// The member list shows profiles; elsewhere there is none to show.
    pub can_show_profile: bool,
    /// Not from the conversation itself, which is already open.
    pub can_open_conversation: bool,
    /// A one-to-one conversation's row also copies the conversation's ID and mutes it.
    pub is_conversation_row: bool,
    /// That conversation is muted (read only with `is_conversation_row`).
    pub is_conversation_muted: bool,
}

#[must_use]
pub fn person_menu_actions(facts: PersonMenuFacts) -> Vec<MenuLine<PersonMenuAction>> {
    use PersonMenuAction as Action;
    let relationship = facts.relationship;
    let is_blocked = relationship == Some(RelationshipKind::Blocked);
    let is_other = !facts.is_self;
    let has = |kind: RelationshipKind| is_other && relationship == Some(kind);
    join_sections(vec![
        vec![
            facts.can_show_profile.then_some(Action::Profile),
            (is_other && !is_blocked && facts.can_open_conversation).then_some(Action::Message),
        ],
        vec![
            (is_other && relationship.is_none()).then_some(Action::AddFriend),
            has(RelationshipKind::IncomingRequest).then_some(Action::AcceptRequest),
            has(RelationshipKind::IncomingRequest).then_some(Action::IgnoreRequest),
            has(RelationshipKind::OutgoingRequest).then_some(Action::CancelRequest),
            has(RelationshipKind::Blocked).then_some(Action::Unblock),
        ],
        if facts.is_conversation_row {
            vec![
                Some(if facts.is_conversation_muted {
                    Action::Unmute
                } else {
                    Action::Mute
                }),
                Some(Action::NotificationSettings),
            ]
        } else {
            Vec::new()
        },
        vec![
            Some(Action::CopyUserId),
            facts.is_conversation_row.then_some(Action::CopyChannelId),
        ],
        vec![
            (relationship == Some(RelationshipKind::Friend)).then_some(Action::RemoveFriend),
            (is_other && !is_blocked).then_some(Action::Block),
        ],
    ])
}

// ---- A group conversation's row ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupMenuAction {
    RenameGroup,
    Mute,
    Unmute,
    NotificationSettings,
    CopyChannelId,
    LeaveGroup,
}

/// Anyone in a group may rename it or leave it (ADR-0026).
#[must_use]
pub fn group_menu_actions(is_muted: bool) -> Vec<MenuLine<GroupMenuAction>> {
    use GroupMenuAction as Action;
    join_sections(vec![
        vec![Some(Action::RenameGroup)],
        vec![
            Some(if is_muted {
                Action::Unmute
            } else {
                Action::Mute
            }),
            Some(Action::NotificationSettings),
        ],
        vec![Some(Action::CopyChannelId)],
        vec![Some(Action::LeaveGroup)],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actions<Action: Clone>(lines: &[MenuLine<Action>]) -> Vec<Option<Action>> {
        lines
            .iter()
            .map(|line| match line {
                MenuLine::Action(action) => Some(action.clone()),
                MenuLine::Separator => None,
            })
            .collect()
    }

    // Ported from `context-menus.test.ts` ("messageMenuActions"): the quick reactions come before
    // Add reaction, and Copy message ID joins the everyday section.

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

    #[test]
    fn a_reader_of_someone_elses_message_gets_neither_edit_nor_delete() {
        use MessageMenuAction::{AddReaction, CopyText, Forward, Reply, ViewReactions};
        let reacted = MessageMenuFacts {
            has_reactions: true,
            ..OTHERS_TEXT
        };
        assert_eq!(
            actions(&message_menu_actions(reacted)),
            [
                Some(AddReaction),
                None,
                Some(Reply),
                Some(Forward),
                Some(CopyText),
                Some(ViewReactions)
            ]
        );
    }

    #[test]
    fn the_quick_reactions_head_the_menu_only_for_a_reader_who_may_react() {
        let quick = ["👍".to_owned(), "❤️".to_owned(), "😂".to_owned()];
        assert_eq!(message_menu_quick_reactions(OTHERS_TEXT, &quick), quick);
        let cannot_react = MessageMenuFacts {
            can_add_reactions: false,
            ..OTHERS_TEXT
        };
        assert_eq!(
            message_menu_quick_reactions(cannot_react, &quick),
            Vec::<String>::new()
        );
    }

    #[test]
    fn your_own_message_adds_edit_and_delete_last_and_separated() {
        use MessageMenuAction::{AddReaction, CopyText, Delete, Edit, Forward, Reply};
        assert_eq!(
            actions(&message_menu_actions(OWN_TEXT)),
            [
                Some(AddReaction),
                None,
                Some(Reply),
                Some(Forward),
                Some(Edit),
                Some(CopyText),
                None,
                Some(Delete)
            ]
        );
    }

    #[test]
    fn a_forward_or_a_poll_is_never_edited_even_your_own() {
        let forward = MessageMenuFacts {
            is_forward: true,
            ..OWN_TEXT
        };
        let lines = message_menu_actions(forward);
        assert!(!lines.contains(&MenuLine::Action(MessageMenuAction::Edit)));
        let poll = MessageMenuFacts {
            is_rich: true,
            has_text: false,
            ..OWN_TEXT
        };
        let lines = message_menu_actions(poll);
        assert!(!lines.contains(&MenuLine::Action(MessageMenuAction::Edit)));
        assert!(!lines.contains(&MenuLine::Action(MessageMenuAction::CopyText)));
        assert!(!lines.contains(&MenuLine::Action(MessageMenuAction::Forward)));
        // Read-only channels still forward: that is where announcements live.
        let read_only = MessageMenuFacts {
            can_send_messages: false,
            ..OTHERS_TEXT
        };
        assert!(
            message_menu_actions(read_only).contains(&MenuLine::Action(MessageMenuAction::Forward))
        );
    }

    #[test]
    fn the_author_of_an_open_poll_ends_it_from_the_danger_section() {
        use MessageMenuAction::{AddReaction, Delete, EndPoll, Reply};
        let open_poll = MessageMenuFacts {
            is_rich: true,
            is_poll_open: true,
            has_text: false,
            ..OWN_TEXT
        };
        assert_eq!(
            actions(&message_menu_actions(open_poll)),
            [
                Some(AddReaction),
                None,
                Some(Reply),
                None,
                Some(EndPoll),
                Some(Delete)
            ]
        );
        let someone_elses = MessageMenuFacts {
            is_own: false,
            ..open_poll
        };
        assert!(
            !message_menu_actions(someone_elses)
                .contains(&MenuLine::Action(MessageMenuAction::EndPoll))
        );
    }

    #[test]
    fn a_moderator_pins_and_deletes_anyones_message_and_unpins_a_pinned_one() {
        let moderator = MessageMenuFacts {
            can_manage_messages: true,
            can_manage_pins: true,
            ..OTHERS_TEXT
        };
        let lines = message_menu_actions(moderator);
        assert!(lines.contains(&MenuLine::Action(MessageMenuAction::Pin)));
        assert!(lines.contains(&MenuLine::Action(MessageMenuAction::Delete)));
        let pinned = MessageMenuFacts {
            is_pinned: true,
            ..moderator
        };
        assert!(message_menu_actions(pinned).contains(&MenuLine::Action(MessageMenuAction::Unpin)));
    }

    #[test]
    fn a_moderator_removes_all_reactions_only_from_a_message_that_has_some() {
        use MessageMenuAction::{Delete, RemoveAllReactions};
        let moderator = MessageMenuFacts {
            can_manage_messages: true,
            can_remove_reactions: true,
            has_reactions: true,
            ..OTHERS_TEXT
        };
        let lines = actions(&message_menu_actions(moderator));
        assert_eq!(
            lines[lines.len() - 2..],
            [Some(RemoveAllReactions), Some(Delete)]
        );
        let without_reactions = MessageMenuFacts {
            has_reactions: false,
            ..moderator
        };
        assert!(
            !message_menu_actions(without_reactions)
                .contains(&MenuLine::Action(RemoveAllReactions))
        );
        let member = MessageMenuFacts {
            has_reactions: true,
            ..OTHERS_TEXT
        };
        assert!(!message_menu_actions(member).contains(&MenuLine::Action(RemoveAllReactions)));
    }

    #[test]
    fn a_read_only_channel_leaves_out_reply_edit_and_reactions_without_the_permission() {
        use MessageMenuAction::{CopyText, Delete, Forward};
        let read_only = MessageMenuFacts {
            can_send_messages: false,
            has_reactions: true,
            can_join_reactions: false,
            can_add_reactions: false,
            ..OWN_TEXT
        };
        assert_eq!(
            actions(&message_menu_actions(read_only)),
            [Some(Forward), Some(CopyText), None, Some(Delete)]
        );
        assert_eq!(
            message_menu_quick_reactions(read_only, &["👍".to_owned()]),
            Vec::<String>::new(),
            "no quick reactions without the permission to react"
        );
    }

    // Ported from `context-menus.test.ts` ("personMenuActions").

    const STRANGER: PersonMenuFacts = PersonMenuFacts {
        is_self: false,
        relationship: None,
        can_show_profile: true,
        can_open_conversation: true,
        is_conversation_row: false,
        is_conversation_muted: false,
    };

    #[test]
    fn someone_you_dont_know_gets_a_message_a_request_their_id_and_block() {
        use PersonMenuAction::{AddFriend, Block, CopyUserId, Message, Profile};
        assert_eq!(
            actions(&person_menu_actions(STRANGER)),
            [
                Some(Profile),
                Some(Message),
                None,
                Some(AddFriend),
                None,
                Some(CopyUserId),
                None,
                Some(Block)
            ]
        );
    }

    #[test]
    fn a_request_received_is_answered_and_one_sent_is_cancelled() {
        use PersonMenuAction::{AcceptRequest, AddFriend, CancelRequest, IgnoreRequest};
        let received = person_menu_actions(PersonMenuFacts {
            relationship: Some(RelationshipKind::IncomingRequest),
            ..STRANGER
        });
        assert!(received.contains(&MenuLine::Action(AcceptRequest)));
        assert!(received.contains(&MenuLine::Action(IgnoreRequest)));
        let sent = person_menu_actions(PersonMenuFacts {
            relationship: Some(RelationshipKind::OutgoingRequest),
            ..STRANGER
        });
        assert!(sent.contains(&MenuLine::Action(CancelRequest)));
        assert!(!sent.contains(&MenuLine::Action(AddFriend)));
    }

    #[test]
    fn a_friend_is_removed_in_the_danger_section() {
        use PersonMenuAction::{Block, RemoveFriend};
        let lines = person_menu_actions(PersonMenuFacts {
            relationship: Some(RelationshipKind::Friend),
            ..STRANGER
        });
        assert_eq!(
            actions(&lines[lines.len() - 3..]),
            [None, Some(RemoveFriend), Some(Block)]
        );
    }

    #[test]
    fn someone_you_blocked_is_only_unblocked_never_messaged_or_blocked_again() {
        use PersonMenuAction::{CopyUserId, Profile, Unblock};
        assert_eq!(
            actions(&person_menu_actions(PersonMenuFacts {
                relationship: Some(RelationshipKind::Blocked),
                ..STRANGER
            })),
            [Some(Profile), None, Some(Unblock), None, Some(CopyUserId)]
        );
    }

    #[test]
    fn you_see_only_your_profile_and_id() {
        use PersonMenuAction::{CopyUserId, Profile};
        assert_eq!(
            actions(&person_menu_actions(PersonMenuFacts {
                is_self: true,
                ..STRANGER
            })),
            [Some(Profile), None, Some(CopyUserId)]
        );
    }

    #[test]
    fn a_conversation_row_copies_its_id_too_and_leaves_out_message() {
        use PersonMenuAction::{
            Block, CopyChannelId, CopyUserId, Mute, NotificationSettings, RemoveFriend,
        };
        let row = PersonMenuFacts {
            relationship: Some(RelationshipKind::Friend),
            can_show_profile: false,
            can_open_conversation: false,
            is_conversation_row: true,
            ..STRANGER
        };
        assert_eq!(
            actions(&person_menu_actions(row)),
            [
                Some(Mute),
                Some(NotificationSettings),
                None,
                Some(CopyUserId),
                Some(CopyChannelId),
                None,
                Some(RemoveFriend),
                Some(Block)
            ]
        );
    }

    #[test]
    fn a_muted_conversation_is_unmuted_from_its_row_and_never_muted_from_elsewhere() {
        use PersonMenuAction::{NotificationSettings, Unmute};
        let row = PersonMenuFacts {
            relationship: Some(RelationshipKind::Friend),
            can_show_profile: false,
            can_open_conversation: false,
            is_conversation_row: true,
            is_conversation_muted: true,
            ..STRANGER
        };
        assert_eq!(
            actions(&person_menu_actions(row)[..2]),
            [Some(Unmute), Some(NotificationSettings)]
        );
        assert!(
            !person_menu_actions(PersonMenuFacts {
                is_conversation_muted: true,
                ..STRANGER
            })
            .contains(&MenuLine::Action(Unmute))
        );
    }

    // Ported from `context-menus.test.ts` ("groupMenuActions").

    #[test]
    fn a_group_is_renamed_its_id_copied_and_left_last() {
        use GroupMenuAction::{CopyChannelId, LeaveGroup, Mute, NotificationSettings, RenameGroup};
        assert_eq!(
            actions(&group_menu_actions(false)),
            [
                Some(RenameGroup),
                None,
                Some(Mute),
                Some(NotificationSettings),
                None,
                Some(CopyChannelId),
                None,
                Some(LeaveGroup)
            ]
        );
        assert!(group_menu_actions(true).contains(&MenuLine::Action(GroupMenuAction::Unmute)));
    }
}
