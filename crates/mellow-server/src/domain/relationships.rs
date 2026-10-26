//! Friends, friend requests and blocks (ADR-0025). Every change reaches the sessions of the people
//! it concerns, never anyone else; a block is never shown to the person blocked.

use std::sync::Arc;

use mellow_protocol::{
    GatewayEvent, MAXIMUM_RELATIONSHIPS_PER_USER, PublicUser, Relationship, RelationshipKind,
    SendFriendRequest, SetRelationshipKind, Snowflake,
};

use crate::domain::guilds::GuildError;
use crate::domain::profile_images::fill_profile_images;
use crate::events::EventHub;
use crate::storage::Database;
use crate::storage::guilds as guild_storage;
use crate::storage::messages as message_storage;
use crate::storage::relationships::{self as storage, FriendRequestOutcome, Removal};

/// Usernames are at most 32 characters (`account_rules.rs`): anything longer names no one.
const MAXIMUM_USERNAME_LENGTH: usize = 32;

fn internal(error: impl std::error::Error + Send + Sync + 'static) -> GuildError {
    GuildError::Internal(Box::new(error))
}

#[derive(Debug)]
pub(crate) struct Relationships {
    database: Database,
    events: Arc<EventHub>,
}

impl Relationships {
    pub(crate) const fn new(database: Database, events: Arc<EventHub>) -> Self {
        Self { database, events }
    }

    /// The user's friends, requests and blocks, oldest first.
    pub(crate) async fn list(&self, user_id: Snowflake) -> Result<Vec<Relationship>, GuildError> {
        let mut relationships: Vec<Relationship> =
            storage::list_relationships(&self.database, user_id)
                .await?
                .into_iter()
                .map(|stored| Relationship {
                    user: PublicUser {
                        id: stored.user_id,
                        username: stored.username,
                        display_name: stored.display_name,
                        avatar: None,
                        banner: None,
                    },
                    kind: stored.kind,
                    since_unix_milliseconds: stored.since_unix_milliseconds,
                })
                .collect();
        fill_profile_images(
            &self.database,
            relationships
                .iter_mut()
                .map(|relationship| &mut relationship.user),
        )
        .await
        .map_err(internal)?;
        Ok(relationships)
    }

    /// The ids of the user's friends, for the event hub at `Ready`.
    pub(crate) async fn friend_ids(
        &self,
        user_id: Snowflake,
    ) -> Result<Vec<Snowflake>, GuildError> {
        Ok(storage::friend_ids(&self.database, user_id).await?)
    }

    /// Asks `request.username` to be friends, or accepts their request. A missing account,
    /// oneself and a block on either side all answer `NotFound` (ADR-0025).
    pub(crate) async fn send_friend_request(
        &self,
        user_id: Snowflake,
        request: SendFriendRequest,
        now_unix_milliseconds: u64,
    ) -> Result<Relationship, GuildError> {
        let username = request.username.trim();
        if username.is_empty() || username.chars().count() > MAXIMUM_USERNAME_LENGTH {
            return Err(GuildError::NotFound);
        }
        let other_id = storage::find_enabled_user_id_by_username(&self.database, username)
            .await?
            .filter(|other_id| *other_id != user_id)
            .ok_or(GuildError::NotFound)?;
        let outcome = storage::send_friend_request(
            &self.database,
            user_id,
            other_id,
            now_unix_milliseconds,
            MAXIMUM_RELATIONSHIPS_PER_USER,
        )
        .await?;
        let (mine, theirs) = match outcome {
            FriendRequestOutcome::Refused => return Err(GuildError::NotFound),
            FriendRequestOutcome::LimitReached => {
                return Err(GuildError::LimitReached("relationships per user"));
            }
            FriendRequestOutcome::Unchanged => {
                return self
                    .list(user_id)
                    .await?
                    .into_iter()
                    .find(|relationship| relationship.user.id == other_id)
                    .ok_or(GuildError::Conflict);
            }
            FriendRequestOutcome::Requested => (
                RelationshipKind::OutgoingRequest,
                RelationshipKind::IncomingRequest,
            ),
            FriendRequestOutcome::Accepted => (RelationshipKind::Friend, RelationshipKind::Friend),
        };
        let relationship = self
            .announce_pair(user_id, other_id, mine, theirs, now_unix_milliseconds)
            .await?;
        if outcome == FriendRequestOutcome::Accepted {
            self.events.link_friends(user_id, other_id);
        }
        Ok(relationship)
    }

    /// `friend` accepts `other_id`'s request (`NotFound` without one); `blocked` blocks them.
    pub(crate) async fn set(
        &self,
        user_id: Snowflake,
        other_id: Snowflake,
        kind: SetRelationshipKind,
        now_unix_milliseconds: u64,
    ) -> Result<Relationship, GuildError> {
        if other_id == user_id {
            return Err(GuildError::NotFound);
        }
        match kind {
            SetRelationshipKind::Friend => {
                if !storage::accept_friend_request(
                    &self.database,
                    user_id,
                    other_id,
                    now_unix_milliseconds,
                )
                .await?
                {
                    return Err(GuildError::NotFound);
                }
                let relationship = self
                    .announce_pair(
                        user_id,
                        other_id,
                        RelationshipKind::Friend,
                        RelationshipKind::Friend,
                        now_unix_milliseconds,
                    )
                    .await?;
                self.events.link_friends(user_id, other_id);
                Ok(relationship)
            }
            SetRelationshipKind::Blocked => {
                if !guild_storage::user_exists(&self.database, other_id).await? {
                    return Err(GuildError::NotFound);
                }
                let removal =
                    storage::block_user(&self.database, user_id, other_id, now_unix_milliseconds)
                        .await?;
                let relationship = self
                    .relationship_with(other_id, RelationshipKind::Blocked, now_unix_milliseconds)
                    .await?;
                self.events.publish_to_user(
                    user_id,
                    &GatewayEvent::RelationshipAdd(relationship.clone()),
                );
                self.after_removal(user_id, other_id, removal);
                Ok(relationship)
            }
        }
    }

    /// Ends a friendship, refuses or cancels a request, or unblocks; `NotFound` without one.
    pub(crate) async fn remove(
        &self,
        user_id: Snowflake,
        other_id: Snowflake,
    ) -> Result<(), GuildError> {
        let removal = storage::remove_relationship(&self.database, user_id, other_id).await?;
        if removal.previous.is_none() {
            return Err(GuildError::NotFound);
        }
        self.events.publish_to_user(
            user_id,
            &GatewayEvent::RelationshipRemove { user_id: other_id },
        );
        self.after_removal(user_id, other_id, removal);
        Ok(())
    }

    /// Tells the other person their side went, and stops sharing presence between former friends.
    fn after_removal(&self, user_id: Snowflake, other_id: Snowflake, removal: Removal) {
        if removal.other_side_removed.is_some() {
            self.events
                .publish_to_user(other_id, &GatewayEvent::RelationshipRemove { user_id });
        }
        if removal.previous == Some(RelationshipKind::Friend) {
            self.events.unlink_friends(user_id, other_id);
        }
    }

    /// Sends each side its new relationship with the other, and answers the user's.
    async fn announce_pair(
        &self,
        user_id: Snowflake,
        other_id: Snowflake,
        mine: RelationshipKind,
        theirs: RelationshipKind,
        now_unix_milliseconds: u64,
    ) -> Result<Relationship, GuildError> {
        let with_other = self
            .relationship_with(other_id, mine, now_unix_milliseconds)
            .await?;
        let with_user = self
            .relationship_with(user_id, theirs, now_unix_milliseconds)
            .await?;
        self.events
            .publish_to_user(other_id, &GatewayEvent::RelationshipAdd(with_user));
        self.events
            .publish_to_user(user_id, &GatewayEvent::RelationshipAdd(with_other.clone()));
        Ok(with_other)
    }

    /// A relationship of `kind` with `other_id`, with their public profile.
    async fn relationship_with(
        &self,
        other_id: Snowflake,
        kind: RelationshipKind,
        now_unix_milliseconds: u64,
    ) -> Result<Relationship, GuildError> {
        let (_, username, display_name) =
            message_storage::mentionable_users(&self.database, &[other_id])
                .await
                .map_err(internal)?
                .into_iter()
                .next()
                .ok_or(GuildError::NotFound)?;
        let mut relationship = Relationship {
            user: PublicUser {
                id: other_id,
                username,
                display_name,
                avatar: None,
                banner: None,
            },
            kind,
            since_unix_milliseconds: now_unix_milliseconds,
        };
        fill_profile_images(&self.database, [&mut relationship.user])
            .await
            .map_err(internal)?;
        Ok(relationship)
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{
        OpenDirectMessageRequest, PresenceStatus, SendMessageRequest, SetRelationshipKind,
    };

    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::Accounts;
    use crate::domain::accounts::tests::{NOW, migrated_database, register_request};
    use crate::domain::guilds::Guilds;
    use crate::domain::messages::Messages;
    use crate::domain::snowflake_generator::SnowflakeGenerator;
    use crate::events::SharedEvent;

    struct Fixture {
        _directory: tempfile::TempDir,
        database: Database,
        events: Arc<EventHub>,
        accounts: Accounts,
        relationships: Relationships,
        messages: Messages,
        alice: Snowflake,
        bob: Snowflake,
        carol: Snowflake,
    }

    /// Three people who share no guild.
    async fn fixture() -> Fixture {
        let (directory, database) = migrated_database().await;
        let ids = Arc::new(SnowflakeGenerator::default());
        let events = Arc::new(EventHub::default());
        let accounts = Accounts::new(
            database.clone(),
            RegistrationMode::Open,
            ids.clone(),
            events.clone(),
        );
        let mut users = Vec::new();
        for name in ["alice", "bob", "carol"] {
            users.push(
                accounts
                    .register(register_request(name, &format!("{name}@example.org")), NOW)
                    .await
                    .unwrap()
                    .user
                    .id,
            );
        }
        let guilds = Arc::new(Guilds::new(database.clone(), ids.clone(), events.clone()));
        Fixture {
            _directory: directory,
            relationships: Relationships::new(database.clone(), events.clone()),
            messages: Messages::new(database.clone(), ids, guilds),
            accounts,
            database,
            events,
            alice: users[0],
            bob: users[1],
            carol: users[2],
        }
    }

    /// A live session of `user_id` with its presence and friends loaded, as `identify` does.
    async fn connect(
        fixture: &Fixture,
        user_id: Snowflake,
        token_byte: u8,
    ) -> tokio::sync::mpsc::Receiver<SharedEvent> {
        let (key, events) = fixture
            .events
            .register_session(user_id, [token_byte; 32], 4)
            .unwrap();
        fixture
            .accounts
            .load_presence_into_events(user_id)
            .await
            .unwrap();
        let friend_ids = fixture.relationships.friend_ids(user_id).await.unwrap();
        fixture.events.set_friends(user_id, &friend_ids);
        fixture.events.activate_session(key, &[]);
        events
    }

    fn received(events: &mut tokio::sync::mpsc::Receiver<SharedEvent>) -> Vec<GatewayEvent> {
        let mut received = Vec::new();
        while let Ok(event) = events.try_recv() {
            received.push(serde_json::from_str(&event.payload_json).unwrap());
        }
        received
    }

    fn request(username: &str) -> SendFriendRequest {
        SendFriendRequest {
            username: username.into(),
        }
    }

    fn kinds(relationships: &[Relationship]) -> Vec<(Snowflake, RelationshipKind)> {
        relationships
            .iter()
            .map(|relationship| (relationship.user.id, relationship.kind))
            .collect()
    }

    #[tokio::test]
    async fn a_request_reaches_both_sides_and_friends_write_without_a_shared_guild() {
        let fixture = fixture().await;
        let (alice, bob) = (fixture.alice, fixture.bob);
        let mut alice_events = connect(&fixture, alice, 1).await;
        let mut bob_events = connect(&fixture, bob, 2).await;
        let before_friendship = fixture
            .messages
            .open_direct_message(alice, OpenDirectMessageRequest { recipient_id: bob }, NOW)
            .await;

        let sent = fixture
            .relationships
            .send_friend_request(alice, request(" BOB "), NOW)
            .await
            .unwrap();
        let sent_again = fixture
            .relationships
            .send_friend_request(alice, request("bob"), NOW)
            .await
            .unwrap();
        let bob_view_of_request = fixture.relationships.list(bob).await.unwrap();
        let after_request = (received(&mut alice_events), received(&mut bob_events));
        let accepted = fixture
            .relationships
            .set(bob, alice, SetRelationshipKind::Friend, NOW)
            .await
            .unwrap();
        let after_accept = (received(&mut alice_events), received(&mut bob_events));
        let conversation = fixture
            .messages
            .open_direct_message(alice, OpenDirectMessageRequest { recipient_id: bob }, NOW)
            .await
            .unwrap();
        let sent_message = fixture
            .messages
            .send(
                alice,
                conversation.id,
                SendMessageRequest {
                    content: "hi friend".into(),
                    reply_to_id: None,
                    nonce: None,
                    attachment_ids: Vec::new(),
                },
                NOW,
            )
            .await;
        fixture.database.close().await;

        assert!(before_friendship.is_err(), "no shared guild, no friendship");
        assert_eq!(sent.kind, RelationshipKind::OutgoingRequest);
        assert_eq!(sent.user.id, bob);
        assert_eq!(sent_again.kind, RelationshipKind::OutgoingRequest);
        assert_eq!(
            kinds(&bob_view_of_request),
            [(alice, RelationshipKind::IncomingRequest)]
        );
        assert!(matches!(
            after_request.0.as_slice(),
            [GatewayEvent::RelationshipAdd(Relationship {
                kind: RelationshipKind::OutgoingRequest,
                ..
            })]
        ));
        assert!(matches!(
            after_request.1.as_slice(),
            [GatewayEvent::RelationshipAdd(Relationship {
                kind: RelationshipKind::IncomingRequest,
                ..
            })]
        ));
        assert_eq!(accepted.kind, RelationshipKind::Friend);
        for (events, other) in [(&after_accept.0, bob), (&after_accept.1, alice)] {
            assert!(events.iter().any(|event| matches!(
                event,
                GatewayEvent::RelationshipAdd(Relationship { kind: RelationshipKind::Friend, user, .. })
                    if user.id == other
            )));
            assert!(
                events.iter().any(|event| matches!(
                    event,
                    GatewayEvent::PresenceUpdate(presence)
                        if presence.user_id == other && presence.status == PresenceStatus::Online
                )),
                "new friends see each other online: {events:?}"
            );
        }
        assert!(sent_message.is_ok(), "{sent_message:?}");
    }

    #[tokio::test]
    async fn missing_people_oneself_and_blocks_all_refuse_alike_and_a_mutual_request_accepts() {
        let fixture = fixture().await;
        let (alice, bob, carol) = (fixture.alice, fixture.bob, fixture.carol);
        let relationships = &fixture.relationships;
        relationships
            .set(carol, alice, SetRelationshipKind::Blocked, NOW)
            .await
            .unwrap();
        let mut refusals = Vec::new();
        for (sender, username) in [
            (alice, "nobody"),
            (alice, "alice"),
            (alice, "carol"),
            (carol, "alice"),
            (alice, &"x".repeat(40)),
        ] {
            refusals.push(
                relationships
                    .send_friend_request(sender, request(username), NOW)
                    .await
                    .map(|relationship| relationship.kind),
            );
        }
        relationships
            .send_friend_request(bob, request("alice"), NOW)
            .await
            .unwrap();
        let mutual = relationships
            .send_friend_request(alice, request("bob"), NOW)
            .await
            .unwrap();
        fixture.database.close().await;

        for refusal in refusals {
            assert!(matches!(refusal, Err(GuildError::NotFound)), "{refusal:?}");
        }
        assert_eq!(mutual.kind, RelationshipKind::Friend);
    }

    #[tokio::test]
    async fn blocking_ends_the_friendship_silently_and_closes_conversations_both_ways() {
        let fixture = fixture().await;
        let (alice, bob) = (fixture.alice, fixture.bob);
        let relationships = &fixture.relationships;
        relationships
            .send_friend_request(alice, request("bob"), NOW)
            .await
            .unwrap();
        relationships
            .set(bob, alice, SetRelationshipKind::Friend, NOW)
            .await
            .unwrap();
        let conversation = fixture
            .messages
            .open_direct_message(alice, OpenDirectMessageRequest { recipient_id: bob }, NOW)
            .await
            .unwrap();
        let mut alice_events = connect(&fixture, alice, 1).await;
        let mut bob_events = connect(&fixture, bob, 2).await;
        received(&mut alice_events);
        received(&mut bob_events);

        let blocked = relationships
            .set(bob, alice, SetRelationshipKind::Blocked, NOW)
            .await
            .unwrap();
        let after_block = (received(&mut alice_events), received(&mut bob_events));
        let views = (
            relationships.list(alice).await.unwrap(),
            relationships.list(bob).await.unwrap(),
        );
        let write = |author| {
            fixture.messages.send(
                author,
                conversation.id,
                SendMessageRequest {
                    content: "still there?".into(),
                    reply_to_id: None,
                    nonce: None,
                    attachment_ids: Vec::new(),
                },
                NOW,
            )
        };
        let alice_writes = write(alice).await;
        let bob_writes = write(bob).await;
        let reopen = fixture
            .messages
            .open_direct_message(alice, OpenDirectMessageRequest { recipient_id: bob }, NOW)
            .await;
        relationships.remove(bob, alice).await.unwrap();
        let after_unblock = relationships.list(bob).await.unwrap();
        let unblock_again = relationships.remove(bob, alice).await;
        fixture.database.close().await;

        assert_eq!(blocked.kind, RelationshipKind::Blocked);
        assert!(
            matches!(after_block.0.as_slice(), [GatewayEvent::RelationshipRemove { user_id }] if *user_id == bob),
            "alice only sees the friendship end: {:?}",
            after_block.0
        );
        assert!(matches!(
            after_block.1.as_slice(),
            [GatewayEvent::RelationshipAdd(Relationship {
                kind: RelationshipKind::Blocked,
                ..
            })]
        ));
        assert_eq!(kinds(&views.0), []);
        assert_eq!(kinds(&views.1), [(alice, RelationshipKind::Blocked)]);
        assert!(alice_writes.is_err() && bob_writes.is_err());
        assert!(reopen.is_err());
        assert_eq!(kinds(&after_unblock), []);
        assert!(matches!(unblock_again, Err(GuildError::NotFound)));
    }
}
