//! What the Friends space lists under each tab, and what a friend request's answer means for the
//! person who sent it (`web/src/components/friends.ts`, DESIGN.md §5.6.1, ADR-0025).

use mellow_protocol::{PresenceStatus, Relationship, RelationshipKind, Snowflake};

use crate::state::ClientState;

/// The Friends space's tabs, in their order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendsTab {
    Online,
    All,
    Pending,
    Blocked,
    /// The "Add friend" form; it lists nobody.
    Add,
}

impl FriendsTab {
    pub const ALL: [Self; 5] = [
        Self::Online,
        Self::All,
        Self::Pending,
        Self::Blocked,
        Self::Add,
    ];
}

/// A group's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendsGroupKind {
    Online,
    All,
    Incoming,
    Outgoing,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendsGroup<'a> {
    pub kind: FriendsGroupKind,
    pub relationships: Vec<&'a Relationship>,
}

/// The groups a list tab shows, empty groups left out: friends who are not offline (invisible
/// people read as offline), every friend, requests received then sent, or blocks. `Add` lists
/// nobody. `relationships_of_kind` gives each kind sorted by name, as
/// [`ClientState::relationships_of_kind`] does.
pub fn friends_groups_of<'a>(
    tab: FriendsTab,
    relationships_of_kind: impl Fn(RelationshipKind) -> Vec<&'a Relationship>,
    presence_of: impl Fn(Snowflake) -> PresenceStatus,
) -> Vec<FriendsGroup<'a>> {
    let group = |kind, relationships| FriendsGroup {
        kind,
        relationships,
    };
    let groups = match tab {
        FriendsTab::Online => vec![group(
            FriendsGroupKind::Online,
            relationships_of_kind(RelationshipKind::Friend)
                .into_iter()
                .filter(|relationship| presence_of(relationship.user.id) != PresenceStatus::Offline)
                .collect(),
        )],
        FriendsTab::All => vec![group(
            FriendsGroupKind::All,
            relationships_of_kind(RelationshipKind::Friend),
        )],
        FriendsTab::Pending => vec![
            group(
                FriendsGroupKind::Incoming,
                relationships_of_kind(RelationshipKind::IncomingRequest),
            ),
            group(
                FriendsGroupKind::Outgoing,
                relationships_of_kind(RelationshipKind::OutgoingRequest),
            ),
        ],
        FriendsTab::Blocked => vec![group(
            FriendsGroupKind::Blocked,
            relationships_of_kind(RelationshipKind::Blocked),
        )],
        FriendsTab::Add => Vec::new(),
    };
    groups
        .into_iter()
        .filter(|group| !group.relationships.is_empty())
        .collect()
}

/// The groups of `tab` in the client's current state.
#[must_use]
pub fn friends_groups_in(state: &ClientState, tab: FriendsTab) -> Vec<FriendsGroup<'_>> {
    friends_groups_of(
        tab,
        |kind| state.relationships_of_kind(kind),
        |user_id| state.presence_of(user_id),
    )
}

/// What sending a request did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendRequestOutcome {
    Sent,
    Accepted,
    AlreadyFriends,
    AlreadySent,
}

/// What sending a request did, from the relationship before it and the instance's answer: the
/// instance answers an unchanged friendship or request with that same relationship.
#[must_use]
pub fn friend_request_outcome(
    before: Option<RelationshipKind>,
    after: RelationshipKind,
) -> FriendRequestOutcome {
    if after == RelationshipKind::Friend {
        if before == Some(RelationshipKind::Friend) {
            FriendRequestOutcome::AlreadyFriends
        } else {
            FriendRequestOutcome::Accepted
        }
    } else if before == Some(RelationshipKind::OutgoingRequest) {
        FriendRequestOutcome::AlreadySent
    } else {
        FriendRequestOutcome::Sent
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::PublicUser;

    use super::*;

    fn relationship(id: u64, kind: RelationshipKind) -> Relationship {
        Relationship {
            user: PublicUser {
                id: Snowflake::from_raw(id),
                username: format!("user{id}"),
                display_name: String::new(),
                avatar: None,
                banner: None,
            },
            kind,
            since_unix_milliseconds: 0,
        }
    }

    fn relationships() -> Vec<Relationship> {
        vec![
            relationship(1, RelationshipKind::Friend),
            relationship(2, RelationshipKind::Friend),
            relationship(3, RelationshipKind::Friend),
            relationship(4, RelationshipKind::IncomingRequest),
            relationship(5, RelationshipKind::OutgoingRequest),
            relationship(6, RelationshipKind::Blocked),
        ]
    }

    fn presence_of(user_id: Snowflake) -> PresenceStatus {
        match user_id.raw() {
            1 | 4 | 6 => PresenceStatus::Online,
            2 => PresenceStatus::DoNotDisturb,
            _ => PresenceStatus::Offline,
        }
    }

    fn ids_in(tab: FriendsTab) -> Vec<(FriendsGroupKind, Vec<u64>)> {
        let all = relationships();
        let of_kind = |kind| {
            all.iter()
                .filter(|candidate| candidate.kind == kind)
                .collect()
        };
        friends_groups_of(tab, of_kind, presence_of)
            .into_iter()
            .map(|group| {
                (
                    group.kind,
                    group
                        .relationships
                        .iter()
                        .map(|candidate| candidate.user.id.raw())
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn lists_friends_who_are_not_offline_under_online_whatever_their_status() {
        assert_eq!(
            ids_in(FriendsTab::Online),
            vec![(FriendsGroupKind::Online, vec![1, 2])]
        );
    }

    #[test]
    fn lists_every_friend_under_all_and_only_friends() {
        assert_eq!(
            ids_in(FriendsTab::All),
            vec![(FriendsGroupKind::All, vec![1, 2, 3])]
        );
    }

    #[test]
    fn lists_requests_received_before_requests_sent_under_pending() {
        assert_eq!(
            ids_in(FriendsTab::Pending),
            vec![
                (FriendsGroupKind::Incoming, vec![4]),
                (FriendsGroupKind::Outgoing, vec![5]),
            ]
        );
    }

    #[test]
    fn lists_blocks_under_blocked_online_or_not_and_nobody_under_add() {
        assert_eq!(
            ids_in(FriendsTab::Blocked),
            vec![(FriendsGroupKind::Blocked, vec![6])]
        );
        assert_eq!(ids_in(FriendsTab::Add), vec![]);
    }

    #[test]
    fn leaves_empty_groups_out_so_an_empty_tab_has_no_group() {
        assert_eq!(
            friends_groups_of(FriendsTab::Pending, |_| Vec::new(), presence_of),
            vec![]
        );
        let all = relationships();
        let of_kind = |kind| {
            all.iter()
                .filter(|candidate| candidate.kind == kind)
                .collect()
        };
        assert_eq!(
            friends_groups_of(FriendsTab::Online, of_kind, |_| PresenceStatus::Offline),
            vec![]
        );
    }

    #[test]
    fn tells_a_new_request_from_one_already_waiting() {
        assert_eq!(
            friend_request_outcome(None, RelationshipKind::OutgoingRequest),
            FriendRequestOutcome::Sent
        );
        assert_eq!(
            friend_request_outcome(
                Some(RelationshipKind::OutgoingRequest),
                RelationshipKind::OutgoingRequest
            ),
            FriendRequestOutcome::AlreadySent
        );
    }

    #[test]
    fn tells_accepting_their_request_from_an_existing_friendship() {
        assert_eq!(
            friend_request_outcome(
                Some(RelationshipKind::IncomingRequest),
                RelationshipKind::Friend
            ),
            FriendRequestOutcome::Accepted
        );
        assert_eq!(
            friend_request_outcome(None, RelationshipKind::Friend),
            FriendRequestOutcome::Accepted
        );
        assert_eq!(
            friend_request_outcome(Some(RelationshipKind::Friend), RelationshipKind::Friend),
            FriendRequestOutcome::AlreadyFriends
        );
    }
}
