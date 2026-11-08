//! Group conversations (`web/src/components/group-conversations.ts`, DESIGN.md §5.6, ADR-0026):
//! who is in one besides the reader, who the "New message" picker offers, how many more fit, and
//! who owns a group after its owner leaves. Naming a conversation takes the catalogue's words, so
//! it stays with each client.

use mellow_protocol::{
    DirectMessageChannel, MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS, PublicUser, Snowflake,
};

use super::format::display_name;
use super::search_text::fold_for_search;

#[must_use]
pub fn is_group_conversation(conversation: &DirectMessageChannel) -> bool {
    conversation.owner_id.is_some()
}

/// Everyone in the conversation but the signed-in user, in the instance's order.
#[must_use]
pub fn other_participants(
    conversation: &DirectMessageChannel,
    current_user_id: Option<Snowflake>,
) -> Vec<&PublicUser> {
    conversation
        .recipients
        .iter()
        .filter(|recipient| Some(recipient.id) != current_user_id)
        .collect()
}

/// Everyone in a group, you included.
#[must_use]
pub fn participant_count(
    conversation: &DirectMessageChannel,
    current_user_id: Option<Snowflake>,
) -> usize {
    other_participants(conversation, current_user_id).len() + 1
}

/// How many more people the picker may add: a new group holds you and 9 others; an existing one
/// what is left of its 10 places. Never negative.
#[must_use]
pub fn picker_room(participants_already_in: usize, picked_count: usize) -> usize {
    MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS
        .saturating_sub(participants_already_in)
        .saturating_sub(picked_count)
}

/// The friends whose display name or username contains `query` (case and accents ignored, so
/// "chloe" finds Chloé), minus those already in the group (a group holds at most 10).
#[must_use]
pub fn friends_matching<'a>(
    friends: &[&'a PublicUser],
    query: &str,
    excluded_user_ids: &[Snowflake],
) -> Vec<&'a PublicUser> {
    let term = fold_for_search(query.trim());
    friends
        .iter()
        .copied()
        .filter(|friend| {
            !excluded_user_ids.contains(&friend.id)
                && (fold_for_search(display_name(friend)).contains(&term)
                    || fold_for_search(&friend.username).contains(&term))
        })
        .collect()
}

/// Who owns the group once the signed-in owner leaves: the participant with the lowest id
/// (ADR-0026). `None` when the user is not the owner or is the last one in it.
#[must_use]
pub fn next_owner_after_leaving(
    conversation: &DirectMessageChannel,
    current_user_id: Option<Snowflake>,
) -> Option<&PublicUser> {
    if conversation.owner_id.is_none() || conversation.owner_id != current_user_id {
        return None;
    }
    other_participants(conversation, current_user_id)
        .into_iter()
        .min_by_key(|candidate| candidate.id.raw())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: Snowflake = Snowflake::from_raw(1);

    fn person(id: u64, display_name: &str, username: &str) -> PublicUser {
        PublicUser {
            id: Snowflake::from_raw(id),
            username: username.into(),
            display_name: display_name.into(),
            avatar: None,
            banner: None,
        }
    }

    fn ana() -> PublicUser {
        person(30, "Ana", "ana")
    }
    fn bob() -> PublicUser {
        person(12, "Bob", "bob")
    }
    fn chloe() -> PublicUser {
        person(25, "Chloé", "chloe_b")
    }
    fn david() -> PublicUser {
        person(9_000_000_000_000_000_001, "David", "david")
    }

    fn group(recipients: Vec<PublicUser>, owner_id: Snowflake) -> DirectMessageChannel {
        DirectMessageChannel {
            id: Snowflake::from_raw(500),
            recipients,
            last_message_id: None,
            owner_id: Some(owner_id),
            name: String::new(),
        }
    }

    #[test]
    fn never_counts_the_signed_in_user_among_the_others() {
        let with_me = group(vec![person(1, "Paul", "paul"), ana(), bob()], ME);
        assert_eq!(other_participants(&with_me, Some(ME)).len(), 2);
        assert_eq!(participant_count(&with_me, Some(ME)), 3);
        let one_to_one = DirectMessageChannel {
            owner_id: None,
            ..group(vec![ana()], ME)
        };
        assert!(!is_group_conversation(&one_to_one));
        assert!(is_group_conversation(&with_me));
    }

    #[test]
    fn finds_friends_by_display_name_or_username_ignoring_case_and_accents() {
        let (ana, bob, chloe, david) = (ana(), bob(), chloe(), david());
        let friends = vec![&ana, &bob, &chloe, &david];
        let none = [];
        assert_eq!(friends_matching(&friends, "", &none), friends);
        assert_eq!(friends_matching(&friends, "CHLOE", &none), vec![&chloe]);
        assert_eq!(friends_matching(&friends, "_b", &none), vec![&chloe]);
        assert_eq!(friends_matching(&friends, "  bo ", &none), vec![&bob]);
        assert_eq!(
            friends_matching(&friends, "zed", &none),
            Vec::<&PublicUser>::new()
        );
    }

    #[test]
    fn leaves_out_people_already_in_the_group() {
        let (ana, bob, chloe, david) = (ana(), bob(), chloe(), david());
        let friends = vec![&ana, &bob, &chloe, &david];
        let already_in = [bob.id, david.id];
        assert_eq!(
            friends_matching(&friends, "", &already_in),
            vec![&ana, &chloe]
        );
    }

    #[test]
    fn offers_9_others_for_a_new_group_and_what_is_left_of_10_places_for_an_existing_one() {
        assert_eq!(picker_room(1, 0), 9);
        assert_eq!(picker_room(1, 9), 0);
        assert_eq!(picker_room(7, 1), 2);
        assert_eq!(picker_room(10, 0), 0);
        assert_eq!(picker_room(10, 3), 0);
    }

    #[test]
    fn the_next_owner_is_the_participant_with_the_lowest_id_as_64_bit_numbers() {
        let owned = group(vec![ana(), chloe(), bob(), david()], ME);
        assert_eq!(next_owner_after_leaving(&owned, Some(ME)), Some(&bob()));
        // As strings "10" would come before "9"; as doubles the last two would be equal.
        let short_ids = group(
            vec![person(10, "Yann", "yann"), person(9, "Zoé", "zoe")],
            ME,
        );
        assert_eq!(
            next_owner_after_leaving(&short_ids, Some(ME)).map(display_name),
            Some("Zoé")
        );
        let high = group(
            vec![
                person(9_007_199_254_740_993, "Hugo", "hugo"),
                person(9_007_199_254_740_992, "Inès", "ines"),
            ],
            ME,
        );
        assert_eq!(
            next_owner_after_leaving(&high, Some(ME)).map(display_name),
            Some("Inès")
        );
    }

    #[test]
    fn nobody_owns_it_next_when_you_do_not_own_it_or_are_the_last_one_in_it() {
        let not_mine = group(vec![ana(), bob()], ana().id);
        assert_eq!(next_owner_after_leaving(&not_mine, Some(ME)), None);
        assert_eq!(next_owner_after_leaving(&group(vec![], ME), Some(ME)), None);
    }
}
