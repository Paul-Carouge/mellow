//! Group conversations (ADR-0026): creating one with friends, adding friends, leaving, removing
//! people (the owner) and renaming. Their messages go through the usual message routes.

use std::collections::HashSet;

use mellow_protocol::{
    CreateGroupConversationRequest, DirectMessageChannel, GatewayEvent,
    MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS, Snowflake, UpdateGroupConversationRequest,
};

use super::direct_messages::{conversation_seen_by, conversation_views};
use super::{MessageError, Messages};
use crate::domain::guild_rules::{GuildRuleViolation, normalize_group_conversation_name};
use crate::domain::guilds::GuildError;
use crate::storage::direct_messages::{self as storage, GroupAddOutcome};
use crate::storage::relationships as relationship_storage;

impl Messages {
    /// A new group of the caller and `request.recipient_ids`, each a friend of theirs; anyone
    /// else answers `NotFound`, like an unknown account. Every participant hears of it at once.
    pub(crate) async fn create_group_conversation(
        &self,
        user_id: Snowflake,
        request: CreateGroupConversationRequest,
        now_unix_milliseconds: u64,
    ) -> Result<DirectMessageChannel, MessageError> {
        let others: HashSet<Snowflake> = request.recipient_ids.iter().copied().collect();
        if others.len() != request.recipient_ids.len()
            || others.contains(&user_id)
            || !(2..MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS).contains(&others.len())
        {
            return Err(GuildError::from(GuildRuleViolation::GroupConversationRecipients).into());
        }
        let name = normalize_group_conversation_name(&request.name).map_err(GuildError::from)?;
        for other_id in &others {
            if !self.is_friend(user_id, *other_id).await? {
                return Err(GuildError::NotFound.into());
            }
        }
        let channel_id = self.ids.next_id(now_unix_milliseconds)?;
        let mut participant_ids: Vec<Snowflake> = others.into_iter().collect();
        participant_ids.push(user_id);
        participant_ids.sort_unstable_by_key(|id| id.raw());
        storage::create_group(&self.database, channel_id, user_id, &participant_ids, &name)
            .await
            .map_err(GuildError::from)?;
        let views = conversation_views(&self.database, channel_id, &participant_ids).await?;
        let created = views
            .iter()
            .find(|(participant_id, _)| *participant_id == user_id)
            .map(|(_, view)| view.clone())
            .ok_or(GuildError::NotFound)?;
        for (participant_id, view) in views {
            self.guilds.publish_to_users(
                &[participant_id],
                &GatewayEvent::DirectMessageChannelCreate(view),
            );
        }
        Ok(created)
    }

    /// Any participant adds a friend of theirs while the group has room; the newcomer reads its
    /// whole history (ADR-0026).
    pub(crate) async fn add_group_participant(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        new_participant_id: Snowflake,
    ) -> Result<DirectMessageChannel, MessageError> {
        self.require_group_participant(user_id, channel_id).await?;
        if !self.is_friend(user_id, new_participant_id).await? {
            return Err(GuildError::NotFound.into());
        }
        match storage::add_group_participant(
            &self.database,
            channel_id,
            new_participant_id,
            MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS,
        )
        .await
        .map_err(GuildError::from)?
        {
            GroupAddOutcome::Added | GroupAddOutcome::AlreadyIn => {}
            GroupAddOutcome::Full => {
                return Err(GuildError::LimitReached("people per group").into());
            }
            GroupAddOutcome::NotAGroup => return Err(GuildError::NotFound.into()),
        }
        self.announce_group_change(channel_id, Some(new_participant_id))
            .await;
        self.group_seen_by(user_id, channel_id).await
    }

    /// Anyone leaves; only the owner removes someone else. The person who goes stops reading it.
    pub(crate) async fn remove_group_participant(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        removed_id: Snowflake,
    ) -> Result<(), MessageError> {
        let owner_id = self.require_group_participant(user_id, channel_id).await?;
        if removed_id != user_id && owner_id != user_id {
            return Err(GuildError::MissingPermissions.into());
        }
        let removal = storage::remove_group_participant(&self.database, channel_id, removed_id)
            .await
            .map_err(GuildError::from)?;
        if !removal.was_participant {
            return Err(GuildError::NotFound.into());
        }
        self.guilds.publish_to_users(
            &[removed_id],
            &GatewayEvent::DirectMessageChannelDelete { id: channel_id },
        );
        if !removal.deleted_conversation {
            self.announce_group_change(channel_id, None).await;
        }
        Ok(())
    }

    /// Any participant renames the group; an empty name clears it.
    pub(crate) async fn rename_group_conversation(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        request: UpdateGroupConversationRequest,
    ) -> Result<DirectMessageChannel, MessageError> {
        self.require_group_participant(user_id, channel_id).await?;
        let name = normalize_group_conversation_name(&request.name).map_err(GuildError::from)?;
        if !storage::rename_group(&self.database, channel_id, &name)
            .await
            .map_err(GuildError::from)?
        {
            return Err(GuildError::NotFound.into());
        }
        self.announce_group_change(channel_id, None).await;
        self.group_seen_by(user_id, channel_id).await
    }

    /// The group's owner, after checking `user_id` takes part; `NotFound` otherwise, so
    /// outsiders cannot tell a group from an unknown id.
    async fn require_group_participant(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
    ) -> Result<Snowflake, MessageError> {
        let owner_id = storage::group_owner_id(&self.database, channel_id)
            .await
            .map_err(GuildError::from)?
            .ok_or(GuildError::NotFound)?;
        let participant_ids = storage::list_participant_ids(&self.database, channel_id)
            .await
            .map_err(GuildError::from)?;
        if participant_ids.contains(&user_id) {
            Ok(owner_id)
        } else {
            Err(GuildError::NotFound.into())
        }
    }

    async fn is_friend(
        &self,
        user_id: Snowflake,
        other_id: Snowflake,
    ) -> Result<bool, MessageError> {
        Ok(
            relationship_storage::are_friends(&self.database, user_id, other_id)
                .await
                .map_err(GuildError::from)?,
        )
    }

    async fn group_seen_by(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
    ) -> Result<DirectMessageChannel, MessageError> {
        Ok(conversation_seen_by(&self.database, user_id, channel_id)
            .await?
            .ok_or(GuildError::NotFound)?)
    }

    /// Sends every participant the group as they see it: `newcomer_id` as a new conversation,
    /// everyone else as an update. Never fails: the change is committed, so a participant whose
    /// view cannot be read catches up at their next `Ready`.
    async fn announce_group_change(&self, channel_id: Snowflake, newcomer_id: Option<Snowflake>) {
        let participant_ids = match storage::list_participant_ids(&self.database, channel_id).await
        {
            Ok(participant_ids) => participant_ids,
            Err(error) => {
                tracing::warn!(%error, "cannot announce a group change");
                return;
            }
        };
        match conversation_views(&self.database, channel_id, &participant_ids).await {
            Ok(views) => {
                for (participant_id, view) in views {
                    let event = if Some(participant_id) == newcomer_id {
                        GatewayEvent::DirectMessageChannelCreate(view)
                    } else {
                        GatewayEvent::DirectMessageChannelUpdate(view)
                    };
                    self.guilds.publish_to_users(&[participant_id], &event);
                }
            }
            Err(error) => tracing::warn!(%error, "cannot announce a group change"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mellow_protocol::{
        MessageHistoryQuery, SendFriendRequest, SendMessageRequest, SetRelationshipKind,
    };

    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::Accounts;
    use crate::domain::accounts::tests::{NOW, migrated_database, register_request};
    use crate::domain::guilds::Guilds;
    use crate::domain::relationships::Relationships;
    use crate::domain::snowflake_generator::SnowflakeGenerator;
    use crate::events::{EventHub, SharedEvent};
    use crate::storage::Database;

    struct Fixture {
        _directory: tempfile::TempDir,
        database: Database,
        events: Arc<EventHub>,
        messages: Messages,
        relationships: Relationships,
        users: Vec<Snowflake>,
    }

    /// Five people sharing no guild; the first is friends with the three next ones, and the
    /// second with the fifth.
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
        let names = ["ana", "bob", "cleo", "dan", "eve"];
        let mut users = Vec::new();
        for name in names {
            users.push(
                accounts
                    .register(register_request(name, &format!("{name}@example.org")), NOW)
                    .await
                    .unwrap()
                    .user
                    .id,
            );
        }
        let relationships = Relationships::new(database.clone(), events.clone());
        for (asker, answerer) in [(0, 1), (0, 2), (0, 3), (1, 4)] {
            relationships
                .send_friend_request(
                    users[asker],
                    SendFriendRequest {
                        username: names[answerer].into(),
                    },
                    NOW,
                )
                .await
                .unwrap();
            relationships
                .set(
                    users[answerer],
                    users[asker],
                    SetRelationshipKind::Friend,
                    NOW,
                )
                .await
                .unwrap();
        }
        let guilds = Arc::new(Guilds::new(database.clone(), ids.clone(), events.clone()));
        Fixture {
            _directory: directory,
            messages: Messages::new(database.clone(), ids, guilds),
            relationships,
            database,
            events,
            users,
        }
    }

    fn live_session(
        fixture: &Fixture,
        user_id: Snowflake,
        token_byte: u8,
    ) -> tokio::sync::mpsc::Receiver<SharedEvent> {
        let (key, events) = fixture
            .events
            .register_session(user_id, [token_byte; 32], 4)
            .unwrap();
        fixture.events.activate_session(key, &[]);
        events
    }

    fn event_types(events: &mut tokio::sync::mpsc::Receiver<SharedEvent>) -> Vec<String> {
        let mut types = Vec::new();
        while let Ok(event) = events.try_recv() {
            let event: serde_json::Value = serde_json::from_str(&event.payload_json).unwrap();
            types.push(event["t"].as_str().unwrap().to_owned());
        }
        types
    }

    fn create(recipient_ids: &[Snowflake], name: &str) -> CreateGroupConversationRequest {
        CreateGroupConversationRequest {
            recipient_ids: recipient_ids.to_vec(),
            name: name.into(),
        }
    }

    fn text(content: &str) -> SendMessageRequest {
        SendMessageRequest {
            content: content.into(),
            reply_to_id: None,
            nonce: None,
            attachment_ids: Vec::new(),
        }
    }

    #[tokio::test]
    async fn groups_start_with_friends_and_everyone_hears_of_them_at_once() {
        let fixture = fixture().await;
        let [ana, bob, cleo, _, eve] = fixture.users[..] else {
            unreachable!()
        };
        let mut bob_events = live_session(&fixture, bob, 2);
        let messages = &fixture.messages;
        let with_a_stranger = messages
            .create_group_conversation(ana, create(&[bob, eve], ""), NOW)
            .await;
        let too_small = messages
            .create_group_conversation(ana, create(&[bob], ""), NOW)
            .await;
        let with_oneself = messages
            .create_group_conversation(ana, create(&[bob, ana], ""), NOW)
            .await;
        let group = messages
            .create_group_conversation(ana, create(&[bob, cleo], "  Trip  "), NOW)
            .await
            .unwrap();
        let bob_heard = event_types(&mut bob_events);
        let bob_lists = messages.list_direct_message_channels(bob).await.unwrap();
        let cleo_writes = messages.send(cleo, group.id, text("hello all"), NOW).await;
        fixture.database.close().await;

        assert!(matches!(
            with_a_stranger,
            Err(MessageError::Guild(GuildError::NotFound))
        ));
        assert!(too_small.is_err() && with_oneself.is_err());
        assert_eq!(group.owner_id, Some(ana));
        assert_eq!(group.name, "Trip");
        assert_eq!(group.recipients.len(), 2);
        assert_eq!(bob_heard, ["DIRECT_MESSAGE_CHANNEL_CREATE"]);
        assert!(
            bob_lists.iter().any(|channel| channel.id == group.id),
            "listed before its first message"
        );
        assert!(cleo_writes.is_ok(), "{cleo_writes:?}");
    }

    #[tokio::test]
    async fn friends_join_the_owner_removes_people_and_the_last_one_out_deletes_it() {
        let fixture = fixture().await;
        let [ana, bob, cleo, dan, eve] = fixture.users[..] else {
            unreachable!()
        };
        let messages = &fixture.messages;
        let group = messages
            .create_group_conversation(ana, create(&[bob, cleo], ""), NOW)
            .await
            .unwrap();
        messages
            .send(ana, group.id, text("before eve"), NOW)
            .await
            .unwrap();
        let mut ana_events = live_session(&fixture, ana, 1);
        let mut eve_events = live_session(&fixture, eve, 5);

        // Bob adds his friend Eve, who is no friend of Ana's.
        messages
            .add_group_participant(bob, group.id, eve)
            .await
            .unwrap();
        let after_join = (event_types(&mut ana_events), event_types(&mut eve_events));
        let eve_reads = messages
            .list_messages(eve, group.id, MessageHistoryQuery::default())
            .await
            .unwrap();
        let cleo_adds_a_stranger = messages.add_group_participant(cleo, group.id, dan).await;
        let bob_removes_cleo = messages.remove_group_participant(bob, group.id, cleo).await;
        messages
            .remove_group_participant(ana, group.id, eve)
            .await
            .unwrap();
        let eve_after_removal = (
            event_types(&mut eve_events),
            messages
                .list_messages(eve, group.id, MessageHistoryQuery::default())
                .await,
        );
        messages
            .remove_group_participant(ana, group.id, ana)
            .await
            .unwrap();
        let new_owner = storage::group_owner_id(&fixture.database, group.id)
            .await
            .unwrap();
        let renamed = messages
            .rename_group_conversation(
                cleo,
                group.id,
                UpdateGroupConversationRequest {
                    name: "Ours".into(),
                },
            )
            .await
            .unwrap();
        let too_long = messages
            .rename_group_conversation(
                cleo,
                group.id,
                UpdateGroupConversationRequest {
                    name: "x".repeat(101),
                },
            )
            .await;
        for leaver in [bob, cleo] {
            messages
                .remove_group_participant(leaver, group.id, leaver)
                .await
                .unwrap();
        }
        let gone = storage::list_participant_ids(&fixture.database, group.id)
            .await
            .unwrap();
        fixture.database.close().await;

        assert_eq!(after_join.0, ["DIRECT_MESSAGE_CHANNEL_UPDATE"]);
        assert_eq!(after_join.1, ["DIRECT_MESSAGE_CHANNEL_CREATE"]);
        assert_eq!(eve_reads.len(), 1, "the newcomer reads the history");
        assert!(matches!(
            cleo_adds_a_stranger,
            Err(MessageError::Guild(GuildError::NotFound))
        ));
        assert!(matches!(
            bob_removes_cleo,
            Err(MessageError::Guild(GuildError::MissingPermissions))
        ));
        assert_eq!(eve_after_removal.0, ["DIRECT_MESSAGE_CHANNEL_DELETE"]);
        assert!(
            eve_after_removal.1.is_err(),
            "a removed person no longer reads it"
        );
        assert_eq!(new_owner, Some(bob.min(cleo)), "the lowest id takes over");
        assert_eq!(renamed.name, "Ours");
        assert!(too_long.is_err());
        assert_eq!(
            gone,
            Vec::<Snowflake>::new(),
            "the conversation went with its last person"
        );
    }

    #[tokio::test]
    async fn a_block_inside_a_group_does_not_silence_it_and_groups_have_a_size_limit() {
        let fixture = fixture().await;
        let [ana, bob, cleo, dan, eve] = fixture.users[..] else {
            unreachable!()
        };
        let group = fixture
            .messages
            .create_group_conversation(ana, create(&[bob, cleo], ""), NOW)
            .await
            .unwrap();
        fixture
            .relationships
            .set(cleo, ana, SetRelationshipKind::Blocked, NOW)
            .await
            .unwrap();
        let ana_writes = fixture
            .messages
            .send(ana, group.id, text("still here"), NOW)
            .await;
        let cleo_writes = fixture
            .messages
            .send(cleo, group.id, text("me too"), NOW)
            .await;
        let full = storage::add_group_participant(&fixture.database, group.id, dan, 3)
            .await
            .unwrap();
        let roomy = storage::add_group_participant(&fixture.database, group.id, eve, 4)
            .await
            .unwrap();
        fixture.database.close().await;

        assert!(ana_writes.is_ok() && cleo_writes.is_ok());
        assert_eq!(full, GroupAddOutcome::Full);
        assert_eq!(roomy, GroupAddOutcome::Added);
    }
}
