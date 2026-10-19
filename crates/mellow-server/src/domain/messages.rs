//! Message and read-state use cases. Access goes through the channel's guild permissions
//! (`Guilds::channel_and_member_permissions`, ADR-0007).

use std::sync::Arc;

use mellow_protocol::{
    AcknowledgeRequest, ChannelKind, EditMessageRequest, GatewayEvent, Message,
    MessageHistoryQuery, Permissions, PublicUser, ReadState, SendMessageRequest, Snowflake,
};
use sqlx::SqlitePool;

use crate::domain::guilds::{GuildError, Guilds};
use crate::domain::message_rules::{
    MessageRuleViolation, history_page_size, normalize_line_endings, validate_message_content,
    validate_nonce,
};
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::events::EventHub;
use crate::storage::guilds::StoredChannel;
use crate::storage::messages::{
    self as message_storage, HistoryPage, MessageStorageError, NewMessage, StoredMessage,
};

#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error(transparent)]
    InvalidInput(#[from] MessageRuleViolation),
    #[error(transparent)]
    Guild(#[from] GuildError),
    /// Unknown message, or one of another channel.
    #[error("message not found")]
    MessageNotFound,
    #[error("internal error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl From<MessageStorageError> for MessageError {
    fn from(error: MessageStorageError) -> Self {
        Self::Internal(Box::new(error))
    }
}

impl From<mellow_protocol::SnowflakeError> for MessageError {
    fn from(error: mellow_protocol::SnowflakeError) -> Self {
        Self::Internal(Box::new(error))
    }
}

fn message_from_stored(message: StoredMessage, nonce: Option<String>) -> Message {
    Message {
        id: message.id,
        channel_id: message.channel_id,
        author: PublicUser {
            id: message.author_id,
            username: message.author_username,
            display_name: message.author_display_name,
        },
        content: message.content,
        reply_to_id: message.reply_to_id,
        edited_at_unix_milliseconds: message.edited_at_unix_milliseconds,
        nonce,
    }
}

#[derive(Debug)]
pub(crate) struct Messages {
    pool: SqlitePool,
    ids: Arc<SnowflakeGenerator>,
    guilds: Arc<Guilds>,
    events: Arc<EventHub>,
}

impl Messages {
    pub(crate) fn new(
        pool: SqlitePool,
        ids: Arc<SnowflakeGenerator>,
        guilds: Arc<Guilds>,
        events: Arc<EventHub>,
    ) -> Self {
        Self {
            pool,
            ids,
            guilds,
            events,
        }
    }

    /// The text channel, after checking the member has every permission in `needed`.
    async fn text_channel_and_permissions_requiring(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        needed: Permissions,
    ) -> Result<(StoredChannel, Permissions), MessageError> {
        let (channel, permissions) = self
            .guilds
            .channel_and_member_permissions(user_id, channel_id)
            .await?;
        if !permissions.contains(needed) {
            return Err(GuildError::MissingPermissions.into());
        }
        if channel.kind != ChannelKind::Text {
            return Err(MessageRuleViolation::NotATextChannel.into());
        }
        Ok((channel, permissions))
    }

    async fn message_in_channel(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Result<StoredMessage, MessageError> {
        message_storage::find_message_in_channel(&self.pool, channel_id, message_id)
            .await?
            .ok_or(MessageError::MessageNotFound)
    }

    /// Needs `VIEW_CHANNEL` and `SEND_MESSAGES`. The nonce is echoed back, not stored.
    pub(crate) async fn send(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        request: SendMessageRequest,
        now_unix_milliseconds: u64,
    ) -> Result<Message, MessageError> {
        let (channel, _) = self
            .text_channel_and_permissions_requiring(
                user_id,
                channel_id,
                Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
            )
            .await?;
        let content = normalize_line_endings(&request.content);
        validate_message_content(&content)?;
        if let Some(nonce) = &request.nonce {
            validate_nonce(nonce)?;
        }
        if let Some(reply_to_id) = request.reply_to_id {
            message_storage::find_message_in_channel(&self.pool, channel.id, reply_to_id)
                .await?
                .ok_or(MessageRuleViolation::ReplyTarget)?;
        }
        let stored = message_storage::insert_message(
            &self.pool,
            &NewMessage {
                id: self.ids.next_id(now_unix_milliseconds)?,
                channel_id: channel.id,
                author_id: user_id,
                content,
                reply_to_id: request.reply_to_id,
            },
        )
        .await?;
        // The nonce goes back to the sender only, never in the guild-wide event.
        self.events.publish_to_guild(
            channel.guild_id,
            &GatewayEvent::MessageCreate(message_from_stored(stored.clone(), None)),
        );
        Ok(message_from_stored(stored, request.nonce))
    }

    /// Needs `VIEW_CHANNEL` and `READ_MESSAGE_HISTORY`. Chronological order, oldest first.
    pub(crate) async fn list_messages(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        query: MessageHistoryQuery,
    ) -> Result<Vec<Message>, MessageError> {
        let (channel, _) = self
            .text_channel_and_permissions_requiring(
                user_id,
                channel_id,
                Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY,
            )
            .await?;
        let page = match (query.before, query.after) {
            (None, None) => HistoryPage::Newest,
            (Some(before), None) => HistoryPage::Before(before),
            (None, Some(after)) => HistoryPage::After(after),
            (Some(_), Some(_)) => return Err(MessageRuleViolation::HistoryQuery.into()),
        };
        let limit = history_page_size(query.limit)?;
        Ok(
            message_storage::list_messages(&self.pool, channel.id, page, limit)
                .await?
                .into_iter()
                .map(|message| message_from_stored(message, None))
                .collect(),
        )
    }

    /// Only the author may edit, as on Discord, and only while still allowed to send: a muted
    /// member cannot republish by editing old messages.
    pub(crate) async fn edit(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        request: EditMessageRequest,
        now_unix_milliseconds: u64,
    ) -> Result<Message, MessageError> {
        let (channel, _) = self
            .text_channel_and_permissions_requiring(
                user_id,
                channel_id,
                Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
            )
            .await?;
        let message = self.message_in_channel(channel.id, message_id).await?;
        if message.author_id != user_id {
            return Err(GuildError::MissingPermissions.into());
        }
        let content = normalize_line_endings(&request.content);
        validate_message_content(&content)?;
        let updated = message_storage::update_message_content(
            &self.pool,
            message.id,
            &content,
            now_unix_milliseconds,
        )
        .await?
        .ok_or(MessageError::MessageNotFound)?;
        let updated = message_from_stored(updated, None);
        self.events.publish_to_guild(
            channel.guild_id,
            &GatewayEvent::MessageUpdate(updated.clone()),
        );
        Ok(updated)
    }

    /// The author, or a member with `MANAGE_MESSAGES`, may delete.
    pub(crate) async fn delete(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Result<(), MessageError> {
        let (channel, permissions) = self
            .text_channel_and_permissions_requiring(user_id, channel_id, Permissions::VIEW_CHANNEL)
            .await?;
        let message = self.message_in_channel(channel.id, message_id).await?;
        if message.author_id != user_id && !permissions.contains(Permissions::MANAGE_MESSAGES) {
            return Err(GuildError::MissingPermissions.into());
        }
        message_storage::delete_message(&self.pool, message.id).await?;
        self.events.publish_to_guild(
            channel.guild_id,
            &GatewayEvent::MessageDelete {
                guild_id: channel.guild_id,
                channel_id: channel.id,
                id: message.id,
            },
        );
        Ok(())
    }

    /// Marks the channel read up to an existing message of that channel; never moves backwards.
    pub(crate) async fn acknowledge_read_state(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        request: AcknowledgeRequest,
    ) -> Result<(), MessageError> {
        let (channel, _) = self
            .text_channel_and_permissions_requiring(user_id, channel_id, Permissions::VIEW_CHANNEL)
            .await?;
        let message = self
            .message_in_channel(channel.id, request.last_read_message_id)
            .await?;
        message_storage::advance_read_state(&self.pool, user_id, channel.id, message.id).await?;
        Ok(())
    }

    /// Shows "… is typing" to the channel's members. Needs `SEND_MESSAGES`.
    pub(crate) async fn start_typing(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
    ) -> Result<(), MessageError> {
        let (channel, _) = self
            .text_channel_and_permissions_requiring(
                user_id,
                channel_id,
                Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
            )
            .await?;
        self.events.publish_to_guild(
            channel.guild_id,
            &GatewayEvent::TypingStart {
                guild_id: channel.guild_id,
                channel_id: channel.id,
                user_id,
            },
        );
        Ok(())
    }

    pub(crate) async fn list_my_read_states(
        &self,
        user_id: Snowflake,
    ) -> Result<Vec<ReadState>, MessageError> {
        Ok(message_storage::list_read_states(&self.pool, user_id)
            .await?
            .into_iter()
            .map(|(channel_id, last_read_message_id)| ReadState {
                channel_id,
                last_read_message_id,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{CreateGuildRequest, CreateInviteRequest, Guild};

    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::Accounts;
    use crate::domain::accounts::tests::{NOW, migrated_pool, register_request};

    struct Fixture {
        _directory: tempfile::TempDir,
        pool: SqlitePool,
        messages: Messages,
        guilds: Arc<Guilds>,
        owner: Snowflake,
        member: Snowflake,
        outsider: Snowflake,
        guild: Guild,
        general: Snowflake,
    }

    async fn fixture() -> Fixture {
        let (directory, pool) = migrated_pool().await;
        let ids = Arc::new(SnowflakeGenerator::default());
        let accounts = Accounts::new(
            pool.clone(),
            RegistrationMode::Open,
            ids.clone(),
            Arc::default(),
        );
        let mut users = Vec::new();
        for name in ["owner", "member", "outsider"] {
            let session = accounts
                .register(register_request(name, &format!("{name}@example.org")), NOW)
                .await
                .unwrap();
            users.push(session.user.id);
        }
        let guilds = Arc::new(Guilds::new(pool.clone(), ids.clone(), Arc::default()));
        let guild = guilds
            .create_guild(
                users[0],
                CreateGuildRequest {
                    name: "Club".into(),
                },
                NOW,
            )
            .await
            .unwrap();
        let invite = guilds
            .create_invite(users[0], guild.id, CreateInviteRequest::default(), NOW)
            .await
            .unwrap();
        guilds
            .join_with_invite(users[1], &invite.code, NOW)
            .await
            .unwrap();
        let general = guild.channels[1].id;
        Fixture {
            _directory: directory,
            messages: Messages::new(pool.clone(), ids, guilds.clone(), Arc::default()),
            pool,
            guilds,
            owner: users[0],
            member: users[1],
            outsider: users[2],
            guild,
            general,
        }
    }

    fn text(content: &str) -> SendMessageRequest {
        SendMessageRequest {
            content: content.into(),
            reply_to_id: None,
            nonce: None,
        }
    }

    #[tokio::test]
    async fn members_send_and_page_through_history_in_chronological_order() {
        let fixture = fixture().await;
        let mut sent = Vec::new();
        for index in 0..5u64 {
            let message = fixture
                .messages
                .send(
                    fixture.member,
                    fixture.general,
                    text(&format!("message {index}")),
                    NOW + index,
                )
                .await
                .unwrap();
            sent.push(message.id);
        }
        let newest_two = fixture
            .messages
            .list_messages(
                fixture.owner,
                fixture.general,
                MessageHistoryQuery {
                    limit: Some(2),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let before = fixture
            .messages
            .list_messages(
                fixture.owner,
                fixture.general,
                MessageHistoryQuery {
                    before: Some(sent[4]),
                    limit: Some(2),
                    after: None,
                },
            )
            .await
            .unwrap();
        let after = fixture
            .messages
            .list_messages(
                fixture.owner,
                fixture.general,
                MessageHistoryQuery {
                    after: Some(sent[2]),
                    limit: Some(1),
                    before: None,
                },
            )
            .await
            .unwrap();
        fixture.pool.close().await;

        let ids = |messages: &[Message]| {
            messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&newest_two), [sent[3], sent[4]]);
        assert_eq!(ids(&before), [sent[2], sent[3]]);
        assert_eq!(ids(&after), [sent[3]]);
        assert_eq!(newest_two[0].author.username, "member");
    }

    #[tokio::test]
    async fn replies_must_target_the_same_channel_and_nonces_are_echoed() {
        let fixture = fixture().await;
        let other_channel = fixture
            .guilds
            .create_channel(
                fixture.owner,
                fixture.guild.id,
                mellow_protocol::CreateChannelRequest {
                    kind: ChannelKind::Text,
                    name: "other".into(),
                    topic: String::new(),
                    parent_id: None,
                },
                NOW,
            )
            .await
            .unwrap();
        let original = fixture
            .messages
            .send(fixture.owner, fixture.general, text("question"), NOW)
            .await
            .unwrap();
        let reply = fixture
            .messages
            .send(
                fixture.member,
                fixture.general,
                SendMessageRequest {
                    content: "answer".into(),
                    reply_to_id: Some(original.id),
                    nonce: Some("outbox-1".into()),
                },
                NOW,
            )
            .await
            .unwrap();
        let cross_channel_reply = fixture
            .messages
            .send(
                fixture.member,
                other_channel.id,
                SendMessageRequest {
                    content: "x".into(),
                    reply_to_id: Some(original.id),
                    nonce: None,
                },
                NOW,
            )
            .await;
        fixture.pool.close().await;

        assert_eq!(reply.reply_to_id, Some(original.id));
        assert_eq!(reply.nonce.as_deref(), Some("outbox-1"));
        assert!(matches!(
            cross_channel_reply,
            Err(MessageError::InvalidInput(
                MessageRuleViolation::ReplyTarget
            ))
        ));
    }

    #[tokio::test]
    async fn only_authors_edit_and_authors_or_moderators_delete() {
        let fixture = fixture().await;
        let by_member = fixture
            .messages
            .send(fixture.member, fixture.general, text("typo"), NOW)
            .await
            .unwrap();
        let by_owner = fixture
            .messages
            .send(fixture.owner, fixture.general, text("rules"), NOW)
            .await
            .unwrap();
        let edit = |content: &str| EditMessageRequest {
            content: content.into(),
        };

        let edited = fixture
            .messages
            .edit(
                fixture.member,
                fixture.general,
                by_member.id,
                edit("fixed"),
                NOW + 5,
            )
            .await
            .unwrap();
        let owner_edits_member = fixture
            .messages
            .edit(
                fixture.owner,
                fixture.general,
                by_member.id,
                edit("hijack"),
                NOW,
            )
            .await;
        let member_deletes_owner = fixture
            .messages
            .delete(fixture.member, fixture.general, by_owner.id)
            .await;
        // The owner has MANAGE_MESSAGES (all permissions).
        fixture
            .messages
            .delete(fixture.owner, fixture.general, by_member.id)
            .await
            .unwrap();
        let history = fixture
            .messages
            .list_messages(
                fixture.owner,
                fixture.general,
                MessageHistoryQuery::default(),
            )
            .await
            .unwrap();
        fixture.pool.close().await;

        assert_eq!(edited.content, "fixed");
        assert_eq!(edited.edited_at_unix_milliseconds, Some(NOW + 5));
        assert!(matches!(
            owner_edits_member,
            Err(MessageError::Guild(GuildError::MissingPermissions))
        ));
        assert!(matches!(
            member_deletes_owner,
            Err(MessageError::Guild(GuildError::MissingPermissions))
        ));
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].id, by_owner.id);
    }

    #[tokio::test]
    async fn outsiders_categories_and_foreign_messages_are_refused() {
        let fixture = fixture().await;
        let category = fixture.guild.channels[0].id;
        let message = fixture
            .messages
            .send(fixture.owner, fixture.general, text("hello"), NOW)
            .await
            .unwrap();

        let outsider_sends = fixture
            .messages
            .send(fixture.outsider, fixture.general, text("hi"), NOW)
            .await;
        let outsider_reads = fixture
            .messages
            .list_messages(
                fixture.outsider,
                fixture.general,
                MessageHistoryQuery::default(),
            )
            .await;
        let into_category = fixture
            .messages
            .send(fixture.owner, category, text("hi"), NOW)
            .await;
        let both_cursors = fixture
            .messages
            .list_messages(
                fixture.owner,
                fixture.general,
                MessageHistoryQuery {
                    before: Some(message.id),
                    after: Some(message.id),
                    limit: None,
                },
            )
            .await;
        let unknown_message = fixture
            .messages
            .edit(
                fixture.owner,
                fixture.general,
                Snowflake::from_raw(u64::MAX),
                EditMessageRequest {
                    content: "x".into(),
                },
                NOW,
            )
            .await;
        fixture.pool.close().await;

        assert!(matches!(
            outsider_sends,
            Err(MessageError::Guild(GuildError::NotFound))
        ));
        assert!(matches!(
            outsider_reads,
            Err(MessageError::Guild(GuildError::NotFound))
        ));
        assert!(matches!(
            into_category,
            Err(MessageError::InvalidInput(
                MessageRuleViolation::NotATextChannel
            ))
        ));
        assert!(matches!(
            both_cursors,
            Err(MessageError::InvalidInput(
                MessageRuleViolation::HistoryQuery
            ))
        ));
        assert!(matches!(
            unknown_message,
            Err(MessageError::MessageNotFound)
        ));
    }

    #[tokio::test]
    async fn read_state_only_moves_forward_and_disappears_after_leaving() {
        let fixture = fixture().await;
        let first = fixture
            .messages
            .send(fixture.owner, fixture.general, text("1"), NOW)
            .await
            .unwrap();
        let second = fixture
            .messages
            .send(fixture.owner, fixture.general, text("2"), NOW)
            .await
            .unwrap();
        let acknowledge = |id| AcknowledgeRequest {
            last_read_message_id: id,
        };

        fixture
            .messages
            .acknowledge_read_state(fixture.member, fixture.general, acknowledge(second.id))
            .await
            .unwrap();
        fixture
            .messages
            .acknowledge_read_state(fixture.member, fixture.general, acknowledge(first.id))
            .await
            .unwrap();
        let states = fixture
            .messages
            .list_my_read_states(fixture.member)
            .await
            .unwrap();
        fixture
            .guilds
            .leave_guild(fixture.member, fixture.guild.id)
            .await
            .unwrap();
        let after_leaving = fixture
            .messages
            .list_my_read_states(fixture.member)
            .await
            .unwrap();
        fixture.pool.close().await;

        assert_eq!(
            states,
            [ReadState {
                channel_id: fixture.general,
                last_read_message_id: second.id
            }]
        );
        assert_eq!(after_leaving, Vec::new());
    }
}
