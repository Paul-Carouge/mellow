//! Message and read-state use cases. Access goes through the channel's guild permissions
//! (`Guilds::channel_and_member_permissions`, ADR-0007), or for a direct-message conversation
//! through its participants (`permissions::direct_message_permissions`).

mod direct_messages;
mod pins;
mod reactions;

pub(crate) use reactions::ReactionRemoval;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use mellow_protocol::{
    AcknowledgeRequest, Attachment, AuditAction, ChannelKind, EditMessageRequest, GatewayEvent,
    Message, MessageHistoryQuery, Permissions, PublicUser, ReactionSummary, ReadState,
    SendMessageRequest, Snowflake,
};
use sqlx::SqlitePool;

use crate::domain::guilds::{ChannelAudience, GuildError, Guilds};
use crate::domain::message_rules::{
    MAXIMUM_ATTACHMENTS_PER_MESSAGE, MessageRuleViolation, history_page_size,
    normalize_line_endings, validate_message_content, validate_nonce,
};
use crate::domain::permissions::direct_message_permissions;
use crate::domain::profile_images::{fill_profile_images, profile_images_of_user};
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::storage::attachments::{self as attachment_storage, StoredAttachment};
use crate::storage::direct_messages as direct_message_storage;
use crate::storage::guilds::StoredChannel;
use crate::storage::messages::{
    self as message_storage, HistoryPage, MessageStorageError, NewMessage, StoredMessage,
};
use crate::storage::reactions as reaction_storage;

#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error(transparent)]
    InvalidInput(#[from] MessageRuleViolation),
    #[error(transparent)]
    Guild(#[from] GuildError),
    /// Unknown message, or one of another channel.
    #[error("message not found")]
    MessageNotFound,
    /// Every file transfer slot is taken; the client tries again shortly.
    #[error("too many file transfers at once")]
    Busy,
    /// The user's uploads already take their whole storage quota.
    #[error("attachment storage quota reached")]
    QuotaReached,
    #[error("internal error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl From<MessageStorageError> for MessageError {
    fn from(error: MessageStorageError) -> Self {
        match error {
            MessageStorageError::AttachmentUnavailable => MessageRuleViolation::Attachments.into(),
            other => Self::Internal(Box::new(other)),
        }
    }
}

fn attachment_from_stored(attachment: StoredAttachment) -> Attachment {
    Attachment {
        id: attachment.id,
        file_name: attachment.file_name,
        content_type: attachment.content_type,
        size_bytes: attachment.size_bytes,
    }
}

impl From<mellow_protocol::SnowflakeError> for MessageError {
    fn from(error: mellow_protocol::SnowflakeError) -> Self {
        Self::Internal(Box::new(error))
    }
}

fn message_from_stored(
    message: StoredMessage,
    nonce: Option<String>,
    attachments: Vec<StoredAttachment>,
) -> Message {
    Message {
        id: message.id,
        channel_id: message.channel_id,
        author: PublicUser {
            id: message.author_id,
            username: message.author_username,
            display_name: message.author_display_name,
            avatar: None,
            banner: None,
        },
        content: message.content,
        reply_to_id: message.reply_to_id,
        edited_at_unix_milliseconds: message.edited_at_unix_milliseconds,
        nonce,
        attachments: attachments
            .into_iter()
            .map(attachment_from_stored)
            .collect(),
        // Per viewer: filled by `fill_reactions` for REST answers, never in events.
        reactions: Vec::new(),
        pinned: message.is_pinned,
    }
}

/// The reactions of these messages as `viewer_id` sees them (`me`), for REST answers only.
async fn fill_reactions(
    pool: &SqlitePool,
    viewer_id: Snowflake,
    messages: &mut [Message],
) -> Result<(), MessageError> {
    let message_ids: Vec<Snowflake> = messages.iter().map(|message| message.id).collect();
    let mut reactions =
        reaction_storage::reactions_of_messages(pool, viewer_id, &message_ids).await?;
    for message in messages {
        message.reactions = reactions
            .remove(&message.id)
            .unwrap_or_default()
            .into_iter()
            .map(|reaction| ReactionSummary {
                emoji: reaction.emoji,
                count: reaction.count,
                me: reaction.includes_viewer,
            })
            .collect();
    }
    Ok(())
}

/// The attachments of one message, for the paths that build a single one.
async fn attachments_of(
    pool: &SqlitePool,
    message_id: Snowflake,
) -> Result<Vec<StoredAttachment>, MessageError> {
    Ok(
        attachment_storage::attachments_of_messages(pool, &[message_id])
            .await?
            .remove(&message_id)
            .unwrap_or_default(),
    )
}

/// Where a message action happens.
enum MessageChannel {
    Guild(StoredChannel),
    Direct {
        id: Snowflake,
        participant_ids: Vec<Snowflake>,
    },
}

impl MessageChannel {
    fn id(&self) -> Snowflake {
        match self {
            Self::Guild(channel) => channel.id,
            Self::Direct { id, .. } => *id,
        }
    }

    fn guild_id(&self) -> Option<Snowflake> {
        match self {
            Self::Guild(channel) => Some(channel.guild_id),
            Self::Direct { .. } => None,
        }
    }
}

/// Who receives a channel's events, decided before the write (ADR-0015).
enum MessageAudience {
    Guild {
        guild_id: Snowflake,
        viewers: ChannelAudience,
    },
    Participants(Vec<Snowflake>),
}

async fn is_empty_channel(pool: &SqlitePool, channel_id: Snowflake) -> Result<bool, MessageError> {
    Ok(
        message_storage::list_messages(pool, channel_id, HistoryPage::Newest, 1)
            .await?
            .is_empty(),
    )
}

fn publish(guilds: &Guilds, audience: &MessageAudience, event: &GatewayEvent) {
    match audience {
        MessageAudience::Guild { guild_id, viewers } => {
            guilds.publish_to_audience(*guild_id, viewers, event);
        }
        MessageAudience::Participants(user_ids) => guilds.publish_to_users(user_ids, event),
    }
}

#[derive(Debug)]
pub(crate) struct Messages {
    pool: SqlitePool,
    ids: Arc<SnowflakeGenerator>,
    /// Also publishes message events, to the members who can view the channel (ADR-0015).
    guilds: Arc<Guilds>,
    /// Held from id generation until the `MESSAGE_CREATE` event is published, so messages commit
    /// and reach gateways in id order: an `after` cursor can never skip a message committed late.
    send_order: Arc<tokio::sync::Mutex<()>>,
}

impl Messages {
    pub(crate) fn new(pool: SqlitePool, ids: Arc<SnowflakeGenerator>, guilds: Arc<Guilds>) -> Self {
        Self {
            pool,
            ids,
            guilds,
            send_order: Arc::default(),
        }
    }

    /// The text channel or conversation, after checking the user has every permission in
    /// `needed`. A conversation the user is not part of is `NotFound`, like a hidden channel.
    async fn text_channel_and_permissions_requiring(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        needed: Permissions,
    ) -> Result<(MessageChannel, Permissions), MessageError> {
        let participant_ids = direct_message_storage::list_participant_ids(&self.pool, channel_id)
            .await
            .map_err(GuildError::from)?;
        let (channel, permissions) = if participant_ids.is_empty() {
            let (channel, permissions) = self
                .guilds
                .channel_and_member_permissions(user_id, channel_id)
                .await?;
            if channel.kind != ChannelKind::Text {
                return Err(MessageRuleViolation::NotATextChannel.into());
            }
            (MessageChannel::Guild(channel), permissions)
        } else {
            let is_participant = participant_ids.contains(&user_id);
            let shares_a_guild = is_participant
                && direct_messages::shares_a_guild_with_each(&self.pool, user_id, &participant_ids)
                    .await?;
            let permissions = direct_message_permissions(is_participant, shares_a_guild);
            if !permissions.contains(Permissions::VIEW_CHANNEL) {
                return Err(GuildError::NotFound.into());
            }
            let channel = MessageChannel::Direct {
                id: channel_id,
                participant_ids,
            };
            (channel, permissions)
        };
        if !permissions.contains(needed) {
            return Err(GuildError::MissingPermissions.into());
        }
        Ok((channel, permissions))
    }

    /// For attachments: the user may act in this text channel or conversation with `needed`.
    /// A channel they cannot view is `NotFound`.
    pub(crate) async fn require_channel_permissions(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        needed: Permissions,
    ) -> Result<(), MessageError> {
        self.text_channel_and_permissions_requiring(user_id, channel_id, needed)
            .await
            .map(|_| ())
    }

    async fn audience(&self, channel: &MessageChannel) -> Result<MessageAudience, MessageError> {
        Ok(match channel {
            MessageChannel::Guild(channel) => MessageAudience::Guild {
                guild_id: channel.guild_id,
                viewers: self.guilds.channel_audience(channel).await?,
            },
            MessageChannel::Direct {
                participant_ids, ..
            } => MessageAudience::Participants(participant_ids.clone()),
        })
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

    /// Needs `VIEW_CHANNEL` and `SEND_MESSAGES`. The nonce is echoed back to the sender only. A
    /// send repeating a nonce the author already used in this channel (an outbox retry after a
    /// lost response) returns that message as stored now, edits included, without a new event and
    /// whatever the new content. Once that message is deleted, the nonce is free again.
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
                // Files need the right to attach them when they are sent too, not only when they
                // were uploaded: taking it away stops publishing what is already uploaded.
                if request.attachment_ids.is_empty() {
                    Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES
                } else {
                    Permissions::VIEW_CHANNEL
                        | Permissions::SEND_MESSAGES
                        | Permissions::ATTACH_FILES
                },
            )
            .await?;
        let content = normalize_line_endings(&request.content);
        let distinct_attachments: HashSet<Snowflake> =
            request.attachment_ids.iter().copied().collect();
        if request.attachment_ids.len() > MAXIMUM_ATTACHMENTS_PER_MESSAGE
            || distinct_attachments.len() != request.attachment_ids.len()
        {
            return Err(MessageRuleViolation::Attachments.into());
        }
        // A message of files alone may have no text, as on Discord.
        if request.attachment_ids.is_empty() || !content.trim().is_empty() {
            validate_message_content(&content)?;
        }
        let content = if content.trim().is_empty() {
            String::new()
        } else {
            content
        };
        if let Some(nonce) = &request.nonce {
            validate_nonce(nonce)?;
        }
        // Decided before the lock and the write: a failure leaves nothing stored, and the scan of
        // a restricted channel's members does not hold up every other send (ADR-0015).
        let audience = self.audience(&channel).await?;
        let channel_id = channel.id();
        // Read before the write too, so nothing can fail between the insert and the events.
        let conversation_views = match &channel {
            MessageChannel::Direct {
                participant_ids, ..
            } => {
                direct_messages::conversation_views(&self.pool, channel_id, participant_ids).await?
            }
            MessageChannel::Guild(_) => Vec::new(),
        };
        let send_order = self.send_order.clone();
        let pool = self.pool.clone();
        let ids = self.ids.clone();
        let guilds = self.guilds.clone();
        // A detached task: a client hanging up mid-insert must not release the lock before the
        // insert commits and its event is published. The lock also makes the nonce lookup and
        // the insert atomic for concurrent retries.
        tokio::spawn(async move {
            let _send_order = send_order.lock_owned().await;
            if let Some(nonce) = &request.nonce
                && let Some(already_sent) =
                    message_storage::find_message_by_nonce(&pool, channel_id, user_id, nonce)
                        .await?
            {
                let attachments = attachments_of(&pool, already_sent.id).await?;
                let mut message = message_from_stored(already_sent, request.nonce, attachments);
                fill_profile_images(&pool, [&mut message.author]).await?;
                fill_reactions(&pool, user_id, std::slice::from_mut(&mut message)).await?;
                return Ok(message);
            }
            if let Some(reply_to_id) = request.reply_to_id {
                message_storage::find_message_in_channel(&pool, channel_id, reply_to_id)
                    .await?
                    .ok_or(MessageRuleViolation::ReplyTarget)?;
            }
            let is_first_direct_message =
                !conversation_views.is_empty() && is_empty_channel(&pool, channel_id).await?;
            let author_images = profile_images_of_user(&pool, user_id).await?;
            let stored = message_storage::insert_message(
                &pool,
                &NewMessage {
                    id: ids.next_id(now_unix_milliseconds)?,
                    channel_id,
                    author_id: user_id,
                    content,
                    reply_to_id: request.reply_to_id,
                    nonce: request.nonce.clone(),
                    attachment_ids: request.attachment_ids.clone(),
                },
            )
            .await?;
            let attachments = attachments_of(&pool, stored.id).await?;
            if is_first_direct_message {
                direct_messages::announce_conversation(&guilds, conversation_views, stored.id);
            }
            let mut message = message_from_stored(stored, None, attachments);
            message.author.avatar = author_images.avatar;
            message.author.banner = author_images.banner;
            // The nonce goes back to the sender only, never in the event.
            publish(
                &guilds,
                &audience,
                &GatewayEvent::MessageCreate(message.clone()),
            );
            message.nonce = request.nonce;
            Ok(message)
        })
        .await
        .map_err(|error| MessageError::Internal(Box::new(error)))?
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
        let messages =
            message_storage::list_messages(&self.pool, channel.id(), page, limit).await?;
        self.messages_for_viewer(user_id, messages).await
    }

    /// Stored messages as `viewer_id` sees them in a REST answer: with their attachments,
    /// reactions and author pictures.
    async fn messages_for_viewer(
        &self,
        viewer_id: Snowflake,
        messages: Vec<StoredMessage>,
    ) -> Result<Vec<Message>, MessageError> {
        let message_ids: Vec<Snowflake> = messages.iter().map(|message| message.id).collect();
        let mut attachments: HashMap<Snowflake, Vec<StoredAttachment>> =
            attachment_storage::attachments_of_messages(&self.pool, &message_ids).await?;
        let mut messages: Vec<Message> = messages
            .into_iter()
            .map(|message| {
                let message_attachments = attachments.remove(&message.id).unwrap_or_default();
                message_from_stored(message, None, message_attachments)
            })
            .collect();
        fill_profile_images(
            &self.pool,
            messages.iter_mut().map(|message| &mut message.author),
        )
        .await?;
        fill_reactions(&self.pool, viewer_id, &mut messages).await?;
        Ok(messages)
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
        let message = self.message_in_channel(channel.id(), message_id).await?;
        if message.author_id != user_id {
            return Err(GuildError::MissingPermissions.into());
        }
        let content = normalize_line_endings(&request.content);
        validate_message_content(&content)?;
        let audience = self.audience(&channel).await?;
        // Read before the write (attachments never change once sent), so nothing can fail
        // between the commit and the event.
        let author_images = profile_images_of_user(&self.pool, user_id).await?;
        let attachments = attachments_of(&self.pool, message.id).await?;
        let updated = message_storage::update_message_content(
            &self.pool,
            message.id,
            &content,
            now_unix_milliseconds,
        )
        .await?
        .ok_or(MessageError::MessageNotFound)?;
        let mut updated = message_from_stored(updated, None, attachments);
        updated.author.avatar = author_images.avatar;
        updated.author.banner = author_images.banner;
        publish(
            &self.guilds,
            &audience,
            &GatewayEvent::MessageUpdate(updated.clone()),
        );
        // Only the editor's answer carries reactions; a failure here leaves the edit published.
        fill_reactions(&self.pool, user_id, std::slice::from_mut(&mut updated)).await?;
        Ok(updated)
    }

    /// The author, or a member with `MANAGE_MESSAGES`, may delete; the latter is audited.
    pub(crate) async fn delete(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Result<(), MessageError> {
        let (channel, permissions) = self
            .text_channel_and_permissions_requiring(user_id, channel_id, Permissions::VIEW_CHANNEL)
            .await?;
        let message = self.message_in_channel(channel.id(), message_id).await?;
        if message.author_id != user_id && !permissions.contains(Permissions::MANAGE_MESSAGES) {
            return Err(GuildError::MissingPermissions.into());
        }
        let audience = self.audience(&channel).await?;
        // Conversations have no moderators, so only guild channels can get here for someone
        // else's message.
        let audit = channel
            .guild_id()
            .filter(|_| message.author_id != user_id)
            .map(|guild_id| {
                self.guilds.audit_entry(
                    guild_id,
                    user_id,
                    AuditAction::MessageDelete,
                    Some(message.author_id),
                    Some(channel.id()),
                )
            })
            .transpose()?;
        message_storage::delete_message(&self.pool, message.id, audit.as_ref()).await?;
        publish(
            &self.guilds,
            &audience,
            &GatewayEvent::MessageDelete {
                guild_id: channel.guild_id(),
                channel_id: channel.id(),
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
            .message_in_channel(channel.id(), request.last_read_message_id)
            .await?;
        message_storage::advance_read_state(&self.pool, user_id, channel.id(), message.id).await?;
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
        // A conversation is announced to the recipient with its first message, not before.
        if matches!(channel, MessageChannel::Direct { .. })
            && is_empty_channel(&self.pool, channel.id()).await?
        {
            return Ok(());
        }
        let audience = self.audience(&channel).await?;
        publish(
            &self.guilds,
            &audience,
            &GatewayEvent::TypingStart {
                guild_id: channel.guild_id(),
                channel_id: channel.id(),
                user_id,
            },
        );
        Ok(())
    }

    /// Only for channels the user can view now (ADR-0015).
    pub(crate) async fn list_my_read_states(
        &self,
        user_id: Snowflake,
    ) -> Result<Vec<ReadState>, MessageError> {
        let mut visible_channel_ids = self.guilds.visible_channel_ids(user_id).await?;
        visible_channel_ids.extend(
            direct_message_storage::list_user_conversations(&self.pool, user_id)
                .await
                .map_err(GuildError::from)?
                .iter()
                .map(|conversation| conversation.id),
        );
        self.list_my_read_states_in(user_id, &visible_channel_ids)
            .await
    }

    /// For callers that already know which channels the user can view (`Ready`).
    pub(crate) async fn list_my_read_states_in(
        &self,
        user_id: Snowflake,
        visible_channel_ids: &HashSet<Snowflake>,
    ) -> Result<Vec<ReadState>, MessageError> {
        Ok(message_storage::list_read_states(&self.pool, user_id)
            .await?
            .into_iter()
            .filter(|(channel_id, _)| visible_channel_ids.contains(channel_id))
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
    use crate::events::EventHub;

    struct Fixture {
        _directory: tempfile::TempDir,
        pool: SqlitePool,
        messages: Messages,
        guilds: Arc<Guilds>,
        events: Arc<EventHub>,
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
        let events = Arc::new(EventHub::default());
        let guilds = Arc::new(Guilds::new(pool.clone(), ids.clone(), events.clone()));
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
            messages: Messages::new(pool.clone(), ids, guilds.clone()),
            pool,
            guilds,
            events,
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
            attachment_ids: Vec::new(),
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

    fn event_types(
        events: &mut tokio::sync::mpsc::Receiver<crate::events::SharedEvent>,
    ) -> Vec<String> {
        let mut types = Vec::new();
        while let Ok(event) = events.try_recv() {
            let event: serde_json::Value = serde_json::from_str(&event.payload_json).unwrap();
            types.push(event["t"].as_str().unwrap().to_owned());
        }
        types
    }

    async fn create_text_channel(fixture: &Fixture, name: &str) -> mellow_protocol::Channel {
        fixture
            .guilds
            .create_channel(
                fixture.owner,
                fixture.guild.id,
                mellow_protocol::CreateChannelRequest {
                    kind: mellow_protocol::ChannelKind::Text,
                    name: name.into(),
                    topic: String::new(),
                    parent_id: None,
                },
                NOW,
            )
            .await
            .unwrap()
    }

    /// Live gateway sessions of the owner and the member, with nothing queued yet.
    fn live_sessions(
        fixture: &Fixture,
    ) -> [tokio::sync::mpsc::Receiver<crate::events::SharedEvent>; 2] {
        let mut sessions =
            [(fixture.owner, [1; 32]), (fixture.member, [2; 32])].map(|(user_id, token)| {
                let (key, events) = fixture.events.register_session(user_id, token, 1).unwrap();
                fixture.events.activate_session(key, &[fixture.guild.id]);
                events
            });
        // Drained once both are online, so the member's arrival is not left in the owner's queue.
        for events in &mut sessions {
            event_types(events);
        }
        sessions
    }

    #[tokio::test]
    async fn a_hidden_channel_is_not_found_and_its_events_reach_only_its_viewers() {
        use mellow_protocol::{OverwriteKind, SetPermissionOverwriteRequest};

        let fixture = fixture().await;
        let staff = create_text_channel(&fixture, "staff").await;
        let [mut owner_events, mut member_events] = live_sessions(&fixture);
        fixture
            .guilds
            .set_permission_overwrite(
                fixture.owner,
                staff.id,
                fixture.guild.id,
                SetPermissionOverwriteRequest {
                    kind: OverwriteKind::Role,
                    allow: Permissions::empty(),
                    deny: Permissions::VIEW_CHANNEL,
                },
            )
            .await
            .unwrap();
        let member_after_hiding = event_types(&mut member_events);
        event_types(&mut owner_events);

        fixture
            .messages
            .send(fixture.owner, staff.id, text("staff only"), NOW)
            .await
            .unwrap();
        let member_sends = fixture
            .messages
            .send(fixture.member, staff.id, text("let me in"), NOW)
            .await;
        let member_reads = fixture
            .messages
            .list_messages(fixture.member, staff.id, MessageHistoryQuery::default())
            .await;
        let member_typing = fixture
            .messages
            .start_typing(fixture.member, staff.id)
            .await;
        let member_snapshot = fixture
            .guilds
            .get_guild(fixture.member, fixture.guild.id)
            .await
            .unwrap();
        let owner_snapshot = fixture
            .guilds
            .get_guild(fixture.owner, fixture.guild.id)
            .await
            .unwrap();
        fixture
            .messages
            .send(fixture.owner, fixture.general, text("public"), NOW)
            .await
            .unwrap();
        fixture.pool.close().await;

        assert_eq!(member_after_hiding, ["GUILD_CREATE"], "a fresh snapshot");
        assert_eq!(
            event_types(&mut owner_events),
            ["MESSAGE_CREATE", "MESSAGE_CREATE"]
        );
        assert_eq!(
            event_types(&mut member_events),
            ["MESSAGE_CREATE"],
            "only the public message"
        );
        for refused in [
            member_sends.map(|_| ()),
            member_reads.map(|_| ()),
            member_typing,
        ] {
            assert!(
                matches!(refused, Err(MessageError::Guild(GuildError::NotFound))),
                "{refused:?}"
            );
        }
        assert!(
            member_snapshot
                .channels
                .iter()
                .all(|channel| channel.id != staff.id)
        );
        assert!(
            owner_snapshot
                .channels
                .iter()
                .any(|channel| channel.id == staff.id)
        );
    }

    #[tokio::test]
    async fn only_moderators_deleting_others_messages_are_audited() {
        use mellow_protocol::{AuditAction, AuditLogQuery};

        let fixture = fixture().await;
        let own = fixture
            .messages
            .send(fixture.member, fixture.general, text("mine"), NOW)
            .await
            .unwrap();
        let moderated = fixture
            .messages
            .send(fixture.member, fixture.general, text("rule breaking"), NOW)
            .await
            .unwrap();
        fixture
            .messages
            .delete(fixture.member, fixture.general, own.id)
            .await
            .unwrap();
        fixture
            .messages
            .delete(fixture.owner, fixture.general, moderated.id)
            .await
            .unwrap();
        let entries = fixture
            .guilds
            .list_audit_log(fixture.owner, fixture.guild.id, AuditLogQuery::default())
            .await
            .unwrap();
        fixture.pool.close().await;
        let deletes: Vec<_> = entries
            .iter()
            .filter(|entry| entry.action == AuditAction::MessageDelete)
            .collect();
        assert_eq!(
            deletes.len(),
            1,
            "an author deleting their own message is not moderation"
        );
        assert_eq!(
            (
                deletes[0].actor_id,
                deletes[0].target_id,
                deletes[0].context_id
            ),
            (
                Some(fixture.owner),
                Some(fixture.member),
                Some(fixture.general)
            )
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_sends_commit_and_reach_gateways_in_id_order() {
        const CONCURRENT_SENDS: u64 = 32;
        let fixture = fixture().await;
        let (session, mut events) = fixture
            .events
            .register_session(fixture.owner, [0; 32], 1)
            .unwrap();
        fixture
            .events
            .activate_session(session, &[fixture.guild.id]);
        while events.try_recv().is_ok() {}

        let sends = (0..CONCURRENT_SENDS).map(|index| {
            fixture
                .messages
                .send(fixture.member, fixture.general, text("race"), NOW + index)
        });
        for sent in futures_util::future::join_all(sends).await {
            sent.unwrap();
        }
        let history = fixture
            .messages
            .list_messages(
                fixture.owner,
                fixture.general,
                MessageHistoryQuery {
                    limit: Some(100),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        fixture.pool.close().await;

        let mut published_ids = Vec::new();
        while let Ok(event) = events.try_recv() {
            let event: serde_json::Value = serde_json::from_str(&event.payload_json).unwrap();
            published_ids.push(event["data"]["id"].as_str().unwrap().to_owned());
        }
        let stored_ids: Vec<String> = history
            .iter()
            .map(|message| message.id.to_string())
            .collect();
        assert_eq!(published_ids, stored_ids);
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
                    attachment_ids: Vec::new(),
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
                    attachment_ids: Vec::new(),
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
    async fn a_repeated_nonce_returns_the_first_message_without_a_new_event() {
        let fixture = fixture().await;
        let (session, mut events) = fixture
            .events
            .register_session(fixture.owner, [0; 32], 1)
            .unwrap();
        fixture
            .events
            .activate_session(session, &[fixture.guild.id]);
        while events.try_recv().is_ok() {}
        let with_nonce = |content: &str| SendMessageRequest {
            content: content.into(),
            reply_to_id: None,
            nonce: Some("outbox-1".into()),
            attachment_ids: Vec::new(),
        };

        let first = fixture
            .messages
            .send(fixture.member, fixture.general, with_nonce("hello"), NOW)
            .await
            .unwrap();
        let retry = fixture
            .messages
            .send(
                fixture.member,
                fixture.general,
                with_nonce("changed"),
                NOW + 1,
            )
            .await
            .unwrap();
        let other_author = fixture
            .messages
            .send(fixture.owner, fixture.general, with_nonce("hello"), NOW + 2)
            .await
            .unwrap();
        let mut published_events = 0;
        while events.try_recv().is_ok() {
            published_events += 1;
        }
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

        assert_eq!(retry, first);
        assert_eq!(retry.content, "hello");
        assert_eq!(retry.nonce.as_deref(), Some("outbox-1"));
        assert_ne!(
            other_author.id, first.id,
            "nonces are scoped to their author"
        );
        assert_eq!(published_events, 2, "a retry publishes nothing");
        assert_eq!(history.len(), 2);
        assert!(
            history.iter().all(|message| message.nonce.is_none()),
            "history never carries nonces"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_retries_with_one_nonce_create_one_message() {
        let fixture = fixture().await;
        let retry = || {
            fixture.messages.send(
                fixture.member,
                fixture.general,
                SendMessageRequest {
                    content: "once".into(),
                    reply_to_id: None,
                    nonce: Some("outbox-2".into()),
                    attachment_ids: Vec::new(),
                },
                NOW,
            )
        };
        let (first, second) = futures_util::future::join(retry(), retry()).await;
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

        assert_eq!(first.unwrap().id, second.unwrap().id);
        assert_eq!(history.len(), 1);
    }

    #[tokio::test]
    async fn a_nonce_is_free_again_once_its_message_is_deleted() {
        let fixture = fixture().await;
        let with_nonce = || SendMessageRequest {
            content: "hello".into(),
            reply_to_id: None,
            nonce: Some("outbox-3".into()),
            attachment_ids: Vec::new(),
        };
        let first = fixture
            .messages
            .send(fixture.member, fixture.general, with_nonce(), NOW)
            .await
            .unwrap();
        fixture
            .messages
            .delete(fixture.member, fixture.general, first.id)
            .await
            .unwrap();
        let after_deletion = fixture
            .messages
            .send(fixture.member, fixture.general, with_nonce(), NOW + 1)
            .await
            .unwrap();
        fixture.pool.close().await;

        assert_ne!(after_deletion.id, first.id);
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

    fn open_with(recipient_id: Snowflake) -> mellow_protocol::OpenDirectMessageRequest {
        mellow_protocol::OpenDirectMessageRequest { recipient_id }
    }

    #[tokio::test]
    async fn conversations_open_once_per_pair_and_only_with_someone_from_a_shared_guild() {
        let fixture = fixture().await;
        let opened = fixture
            .messages
            .open_direct_message(fixture.member, open_with(fixture.owner), NOW)
            .await
            .unwrap();
        let reopened_by_the_other = fixture
            .messages
            .open_direct_message(fixture.owner, open_with(fixture.member), NOW + 1)
            .await
            .unwrap();
        let with_a_stranger = fixture
            .messages
            .open_direct_message(fixture.member, open_with(fixture.outsider), NOW)
            .await;
        let with_oneself = fixture
            .messages
            .open_direct_message(fixture.member, open_with(fixture.member), NOW)
            .await;
        fixture.pool.close().await;

        assert_eq!(opened.id, reopened_by_the_other.id);
        assert_eq!(opened.recipients[0].id, fixture.owner);
        assert_eq!(reopened_by_the_other.recipients[0].id, fixture.member);
        assert!(matches!(
            with_a_stranger,
            Err(MessageError::Guild(GuildError::NotFound))
        ));
        assert!(matches!(
            with_oneself,
            Err(MessageError::Guild(GuildError::InvalidInput(_)))
        ));
    }

    #[tokio::test]
    async fn only_participants_read_and_write_a_conversation() {
        let fixture = fixture().await;
        let conversation = fixture
            .messages
            .open_direct_message(fixture.member, open_with(fixture.owner), NOW)
            .await
            .unwrap();
        let sent = fixture
            .messages
            .send(fixture.member, conversation.id, text("hello"), NOW)
            .await
            .unwrap();
        let read_by_recipient = fixture
            .messages
            .list_messages(
                fixture.owner,
                conversation.id,
                MessageHistoryQuery::default(),
            )
            .await
            .unwrap();
        let read_by_outsider = fixture
            .messages
            .list_messages(
                fixture.outsider,
                conversation.id,
                MessageHistoryQuery::default(),
            )
            .await;
        let sent_by_outsider = fixture
            .messages
            .send(fixture.outsider, conversation.id, text("hi"), NOW)
            .await;
        let deleted_by_recipient = fixture
            .messages
            .delete(fixture.owner, conversation.id, sent.id)
            .await;
        fixture.pool.close().await;

        assert_eq!(read_by_recipient.len(), 1);
        assert!(matches!(
            read_by_outsider,
            Err(MessageError::Guild(GuildError::NotFound))
        ));
        assert!(matches!(
            sent_by_outsider,
            Err(MessageError::Guild(GuildError::NotFound))
        ));
        assert!(
            matches!(
                deleted_by_recipient,
                Err(MessageError::Guild(GuildError::MissingPermissions))
            ),
            "no one moderates a conversation"
        );
    }

    #[tokio::test]
    async fn the_recipient_hears_of_a_conversation_with_its_first_message_only() {
        let fixture = fixture().await;
        let conversation = fixture
            .messages
            .open_direct_message(fixture.member, open_with(fixture.owner), NOW)
            .await
            .unwrap();
        let listed_while_empty = fixture
            .messages
            .list_direct_message_channels(fixture.owner)
            .await
            .unwrap();
        let (key, mut owner_events) = fixture
            .events
            .register_session(fixture.owner, [0; 32], 1)
            .unwrap();
        fixture.events.activate_session(key, &[fixture.guild.id]);
        while owner_events.try_recv().is_ok() {}
        for index in 0..2 {
            fixture
                .messages
                .send(fixture.member, conversation.id, text("hello"), NOW + index)
                .await
                .unwrap();
        }
        let listed_after_messages = fixture
            .messages
            .list_direct_message_channels(fixture.owner)
            .await
            .unwrap();
        let read_states = fixture
            .messages
            .list_my_read_states(fixture.owner)
            .await
            .unwrap();
        fixture.pool.close().await;

        assert_eq!(listed_while_empty, Vec::new());
        assert_eq!(
            event_types(&mut owner_events),
            [
                "DIRECT_MESSAGE_CHANNEL_CREATE",
                "MESSAGE_CREATE",
                "MESSAGE_CREATE"
            ]
        );
        assert_eq!(listed_after_messages.len(), 1);
        assert_eq!(listed_after_messages[0].recipients[0].id, fixture.member);
        assert!(listed_after_messages[0].last_message_id.is_some());
        assert!(read_states.is_empty(), "nothing acknowledged yet");
    }

    #[tokio::test]
    async fn a_participant_who_shares_no_guild_any_more_can_read_but_not_write() {
        let fixture = fixture().await;
        let conversation = fixture
            .messages
            .open_direct_message(fixture.member, open_with(fixture.owner), NOW)
            .await
            .unwrap();
        let own = fixture
            .messages
            .send(fixture.member, conversation.id, text("hello"), NOW)
            .await
            .unwrap();
        fixture
            .guilds
            .leave_guild(fixture.member, fixture.guild.id)
            .await
            .unwrap();
        let send = fixture
            .messages
            .send(
                fixture.member,
                conversation.id,
                text("still there?"),
                NOW + 1,
            )
            .await;
        let edit = fixture
            .messages
            .edit(
                fixture.member,
                conversation.id,
                own.id,
                EditMessageRequest {
                    content: "edited".into(),
                },
                NOW + 1,
            )
            .await;
        let typing = fixture
            .messages
            .start_typing(fixture.member, conversation.id)
            .await;
        let read = fixture
            .messages
            .list_messages(
                fixture.member,
                conversation.id,
                MessageHistoryQuery::default(),
            )
            .await;
        let delete_own = fixture
            .messages
            .delete(fixture.member, conversation.id, own.id)
            .await;
        fixture.pool.close().await;

        for refused in [send.map(|_| ()), edit.map(|_| ()), typing] {
            assert!(matches!(
                refused,
                Err(MessageError::Guild(GuildError::MissingPermissions))
            ));
        }
        assert_eq!(read.unwrap().len(), 1);
        assert!(delete_own.is_ok());
    }

    #[tokio::test]
    async fn outsiders_and_guild_routes_never_reach_a_conversation() {
        let fixture = fixture().await;
        let conversation = fixture
            .messages
            .open_direct_message(fixture.member, open_with(fixture.owner), NOW)
            .await
            .unwrap();
        let sent = fixture
            .messages
            .send(fixture.member, conversation.id, text("hello"), NOW)
            .await
            .unwrap();
        let outsider = fixture.outsider;
        let attempts = [
            fixture
                .messages
                .edit(
                    outsider,
                    conversation.id,
                    sent.id,
                    EditMessageRequest {
                        content: "x".into(),
                    },
                    NOW,
                )
                .await
                .map(|_| ()),
            fixture
                .messages
                .start_typing(outsider, conversation.id)
                .await,
            fixture
                .messages
                .acknowledge_read_state(
                    outsider,
                    conversation.id,
                    AcknowledgeRequest {
                        last_read_message_id: sent.id,
                    },
                )
                .await,
            fixture
                .messages
                .delete(outsider, conversation.id, sent.id)
                .await,
        ];
        let renamed_as_a_guild_channel = fixture
            .guilds
            .update_channel(
                fixture.owner,
                conversation.id,
                mellow_protocol::UpdateChannelRequest::default(),
            )
            .await;
        let deleted_as_a_guild_channel = fixture
            .guilds
            .delete_channel(fixture.owner, conversation.id)
            .await;
        fixture.pool.close().await;

        for attempt in attempts {
            assert!(matches!(
                attempt,
                Err(MessageError::Guild(GuildError::NotFound))
            ));
        }
        assert!(matches!(
            renamed_as_a_guild_channel,
            Err(GuildError::NotFound)
        ));
        assert!(matches!(
            deleted_as_a_guild_channel,
            Err(GuildError::NotFound)
        ));
    }

    #[tokio::test]
    async fn typing_in_an_empty_conversation_reaches_no_one() {
        let fixture = fixture().await;
        let conversation = fixture
            .messages
            .open_direct_message(fixture.member, open_with(fixture.owner), NOW)
            .await
            .unwrap();
        let (key, mut owner_events) = fixture
            .events
            .register_session(fixture.owner, [0; 32], 1)
            .unwrap();
        fixture.events.activate_session(key, &[fixture.guild.id]);
        while owner_events.try_recv().is_ok() {}
        fixture
            .messages
            .start_typing(fixture.member, conversation.id)
            .await
            .unwrap();
        fixture.pool.close().await;

        assert_eq!(event_types(&mut owner_events), Vec::<String>::new());
    }

    fn summaries(message: &Message) -> Vec<(String, u32, bool)> {
        message
            .reactions
            .iter()
            .map(|reaction| (reaction.emoji.clone(), reaction.count, reaction.me))
            .collect()
    }

    async fn history(fixture: &Fixture, viewer: Snowflake, channel: Snowflake) -> Vec<Message> {
        fixture
            .messages
            .list_messages(viewer, channel, MessageHistoryQuery::default())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn reactions_are_idempotent_counted_per_emoji_and_seen_with_me() {
        use mellow_protocol::ReactionUsersQuery;

        let fixture = fixture().await;
        let message = fixture
            .messages
            .send(fixture.owner, fixture.general, text("vote"), NOW)
            .await
            .unwrap();
        let [mut owner_events, mut member_events] = live_sessions(&fixture);
        let react = |user, emoji: &'static str| {
            fixture
                .messages
                .add_reaction(user, fixture.general, message.id, emoji)
        };
        react(fixture.member, "👍").await.unwrap();
        react(fixture.member, "👍").await.unwrap();
        react(fixture.owner, "👍").await.unwrap();
        react(fixture.owner, "❤️").await.unwrap();
        let seen_by_member = history(&fixture, fixture.member, fixture.general).await;
        let seen_by_owner = history(&fixture, fixture.owner, fixture.general).await;
        let users = fixture
            .messages
            .list_reacting_users(
                fixture.member,
                fixture.general,
                message.id,
                "👍",
                ReactionUsersQuery::default(),
            )
            .await
            .unwrap();
        let second_page = fixture
            .messages
            .list_reacting_users(
                fixture.member,
                fixture.general,
                message.id,
                "👍",
                ReactionUsersQuery {
                    after: Some(users[0].id),
                    limit: Some(1),
                },
            )
            .await
            .unwrap();
        let events_after_adding = event_types(&mut member_events);
        fixture
            .messages
            .remove_own_reaction(fixture.member, fixture.general, message.id, "👍")
            .await
            .unwrap();
        fixture
            .messages
            .remove_own_reaction(fixture.member, fixture.general, message.id, "👍")
            .await
            .unwrap();
        let after_removal = history(&fixture, fixture.member, fixture.general).await;
        fixture.pool.close().await;

        assert_eq!(
            events_after_adding,
            [
                "MESSAGE_REACTION_ADD",
                "MESSAGE_REACTION_ADD",
                "MESSAGE_REACTION_ADD"
            ],
            "reacting twice with one emoji sends one event"
        );
        assert_eq!(
            summaries(&seen_by_member[0]),
            [("👍".to_owned(), 2, true), ("❤".to_owned(), 1, false)]
        );
        assert_eq!(
            summaries(&seen_by_owner[0]),
            [("👍".to_owned(), 2, true), ("❤".to_owned(), 1, true)]
        );
        let mut both = vec![fixture.owner, fixture.member];
        both.sort();
        assert_eq!(
            users.iter().map(|user| user.id).collect::<Vec<_>>(),
            both,
            "by ascending id"
        );
        assert_eq!(second_page.len(), 1);
        assert_eq!(second_page[0].id, both[1]);
        assert_eq!(
            summaries(&after_removal[0]),
            [("👍".to_owned(), 1, false), ("❤".to_owned(), 1, false)]
        );
        assert_eq!(
            event_types(&mut owner_events),
            [
                "MESSAGE_REACTION_ADD",
                "MESSAGE_REACTION_ADD",
                "MESSAGE_REACTION_ADD",
                "MESSAGE_REACTION_REMOVE"
            ],
            "removing a missing reaction sends nothing"
        );
    }

    #[tokio::test]
    async fn edits_answer_with_reactions_but_publish_none_and_text_is_no_emoji() {
        let fixture = fixture().await;
        let message = fixture
            .messages
            .send(fixture.owner, fixture.general, text("vote"), NOW)
            .await
            .unwrap();
        fixture
            .messages
            .add_reaction(fixture.member, fixture.general, message.id, "👍")
            .await
            .unwrap();
        let [_owner_events, mut member_events] = live_sessions(&fixture);
        let edited = fixture
            .messages
            .edit(
                fixture.owner,
                fixture.general,
                message.id,
                EditMessageRequest {
                    content: "vote!".into(),
                },
                NOW + 1,
            )
            .await
            .unwrap();
        let invalid = fixture
            .messages
            .add_reaction(fixture.member, fixture.general, message.id, "lol")
            .await;
        fixture.pool.close().await;

        let event: serde_json::Value =
            serde_json::from_str(&member_events.try_recv().unwrap().payload_json).unwrap();
        assert_eq!(event["t"], "MESSAGE_UPDATE");
        assert!(event["data"].get("reactions").is_none(), "{event}");
        assert_eq!(summaries(&edited), [("👍".to_owned(), 1, false)]);
        assert!(matches!(
            invalid,
            Err(MessageError::InvalidInput(
                MessageRuleViolation::ReactionEmoji
            ))
        ));
    }

    #[tokio::test]
    async fn a_message_holds_at_most_twenty_distinct_emoji() {
        use crate::domain::message_rules::MAXIMUM_DISTINCT_EMOJI_PER_MESSAGE;

        let fixture = fixture().await;
        let message = fixture
            .messages
            .send(fixture.owner, fixture.general, text("react"), NOW)
            .await
            .unwrap();
        let emoji: Vec<String> = (0..=MAXIMUM_DISTINCT_EMOJI_PER_MESSAGE)
            .map(|index| char::from_u32(0x1F600 + index).unwrap().to_string())
            .collect();
        let mut outcomes = Vec::new();
        for one in &emoji {
            outcomes.push(
                fixture
                    .messages
                    .add_reaction(fixture.owner, fixture.general, message.id, one)
                    .await,
            );
        }
        let joining = fixture
            .messages
            .add_reaction(fixture.member, fixture.general, message.id, &emoji[0])
            .await;
        fixture.pool.close().await;

        let (last, allowed) = outcomes.split_last().unwrap();
        assert!(allowed.iter().all(Result::is_ok));
        assert!(
            matches!(last, Err(MessageError::Guild(GuildError::LimitReached(_)))),
            "{last:?}"
        );
        assert!(
            joining.is_ok(),
            "joining an existing emoji is still possible"
        );
    }

    #[tokio::test]
    async fn without_add_reactions_members_only_join_existing_ones() {
        use mellow_protocol::{OverwriteKind, SetPermissionOverwriteRequest};

        let fixture = fixture().await;
        fixture
            .guilds
            .set_permission_overwrite(
                fixture.owner,
                fixture.general,
                fixture.guild.id,
                SetPermissionOverwriteRequest {
                    kind: OverwriteKind::Role,
                    allow: Permissions::empty(),
                    deny: Permissions::ADD_REACTIONS,
                },
            )
            .await
            .unwrap();
        let message = fixture
            .messages
            .send(fixture.owner, fixture.general, text("hi"), NOW)
            .await
            .unwrap();
        let new_emoji = fixture
            .messages
            .add_reaction(fixture.member, fixture.general, message.id, "👍")
            .await;
        fixture
            .messages
            .add_reaction(fixture.owner, fixture.general, message.id, "👍")
            .await
            .unwrap();
        let joined = fixture
            .messages
            .add_reaction(fixture.member, fixture.general, message.id, "👍")
            .await;
        let removed = fixture
            .messages
            .remove_own_reaction(fixture.member, fixture.general, message.id, "👍")
            .await;
        let outsider = fixture
            .messages
            .add_reaction(fixture.outsider, fixture.general, message.id, "👍")
            .await;
        let unknown_message = fixture
            .messages
            .add_reaction(
                fixture.member,
                fixture.general,
                Snowflake::from_raw(1),
                "👍",
            )
            .await;
        fixture.pool.close().await;

        assert!(
            matches!(
                new_emoji,
                Err(MessageError::Guild(GuildError::MissingPermissions))
            ),
            "{new_emoji:?}"
        );
        assert!(joined.is_ok() && removed.is_ok());
        assert!(matches!(
            outsider,
            Err(MessageError::Guild(GuildError::NotFound))
        ));
        assert!(matches!(
            unknown_message,
            Err(MessageError::MessageNotFound)
        ));
    }

    #[tokio::test]
    async fn reactions_in_a_hidden_channel_are_not_found_and_reach_only_its_viewers() {
        use mellow_protocol::{OverwriteKind, SetPermissionOverwriteRequest};

        let fixture = fixture().await;
        let staff = create_text_channel(&fixture, "staff").await;
        fixture
            .guilds
            .set_permission_overwrite(
                fixture.owner,
                staff.id,
                fixture.guild.id,
                SetPermissionOverwriteRequest {
                    kind: OverwriteKind::Role,
                    allow: Permissions::empty(),
                    deny: Permissions::VIEW_CHANNEL,
                },
            )
            .await
            .unwrap();
        let secret = fixture
            .messages
            .send(fixture.owner, staff.id, text("staff only"), NOW)
            .await
            .unwrap();
        let [mut owner_events, mut member_events] = live_sessions(&fixture);
        fixture
            .messages
            .add_reaction(fixture.owner, staff.id, secret.id, "👀")
            .await
            .unwrap();
        let member_reacts = fixture
            .messages
            .add_reaction(fixture.member, staff.id, secret.id, "👀")
            .await;
        let member_lists = fixture
            .messages
            .list_reacting_users(
                fixture.member,
                staff.id,
                secret.id,
                "👀",
                mellow_protocol::ReactionUsersQuery::default(),
            )
            .await;
        let member_pins = fixture.messages.list_pins(fixture.member, staff.id).await;
        fixture.pool.close().await;

        assert_eq!(event_types(&mut owner_events), ["MESSAGE_REACTION_ADD"]);
        assert_eq!(event_types(&mut member_events), Vec::<String>::new());
        for refused in [
            member_reacts,
            member_lists.map(|_| ()),
            member_pins.map(|_| ()),
        ] {
            assert!(
                matches!(refused, Err(MessageError::Guild(GuildError::NotFound))),
                "{refused:?}"
            );
        }
    }

    #[tokio::test]
    async fn moderators_pin_newest_first_and_each_change_is_audited() {
        use mellow_protocol::{AuditAction, AuditLogQuery};

        let fixture = fixture().await;
        let mut sent = Vec::new();
        for index in 0..3 {
            sent.push(
                fixture
                    .messages
                    .send(fixture.member, fixture.general, text("note"), NOW + index)
                    .await
                    .unwrap(),
            );
        }
        let [_owner_events, mut member_events] = live_sessions(&fixture);
        for (offset, message) in (10..).zip(&sent) {
            fixture
                .messages
                .pin(fixture.owner, fixture.general, message.id, NOW + offset)
                .await
                .unwrap();
        }
        // Pinning again and unpinning what is not pinned change nothing.
        fixture
            .messages
            .pin(fixture.owner, fixture.general, sent[0].id, NOW + 20)
            .await
            .unwrap();
        fixture
            .messages
            .unpin(fixture.owner, fixture.general, sent[1].id)
            .await
            .unwrap();
        fixture
            .messages
            .unpin(fixture.owner, fixture.general, sent[1].id)
            .await
            .unwrap();
        let pins = fixture
            .messages
            .list_pins(fixture.member, fixture.general)
            .await
            .unwrap();
        let in_history = history(&fixture, fixture.member, fixture.general).await;
        let entries = fixture
            .guilds
            .list_audit_log(fixture.owner, fixture.guild.id, AuditLogQuery::default())
            .await
            .unwrap();
        let member_received: Vec<_> = std::iter::from_fn(|| member_events.try_recv().ok())
            .map(|event| serde_json::from_str::<serde_json::Value>(&event.payload_json).unwrap())
            .map(|event| format!("{} {}", event["t"], event["data"]["pinned"]))
            .collect();
        fixture.pool.close().await;

        assert_eq!(
            pins.iter().map(|message| message.id).collect::<Vec<_>>(),
            [sent[2].id, sent[0].id],
            "the most recently pinned first"
        );
        assert!(pins.iter().all(|message| message.pinned));
        assert_eq!(
            in_history
                .iter()
                .map(|message| message.pinned)
                .collect::<Vec<_>>(),
            [true, false, true]
        );
        let (pinned, unpinned) = ("\"MESSAGE_UPDATE\" true", "\"MESSAGE_UPDATE\" false");
        assert_eq!(member_received, [pinned, pinned, pinned, unpinned]);
        let pin_entries: Vec<_> = entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.action,
                    AuditAction::MessagePin | AuditAction::MessageUnpin
                )
            })
            .map(|entry| {
                (
                    entry.action,
                    entry.actor_id,
                    entry.target_id,
                    entry.context_id,
                )
            })
            .collect();
        let owner = Some(fixture.owner);
        let general = Some(fixture.general);
        assert_eq!(
            pin_entries,
            [
                (AuditAction::MessageUnpin, owner, Some(sent[1].id), general),
                (AuditAction::MessagePin, owner, Some(sent[2].id), general),
                (AuditAction::MessagePin, owner, Some(sent[1].id), general),
                (AuditAction::MessagePin, owner, Some(sent[0].id), general),
            ],
            "newest first, one entry per change"
        );
    }

    #[tokio::test]
    async fn members_without_manage_messages_neither_pin_nor_unpin() {
        let fixture = fixture().await;
        let message = fixture
            .messages
            .send(fixture.member, fixture.general, text("mine"), NOW)
            .await
            .unwrap();
        let pinned = fixture
            .messages
            .pin(fixture.member, fixture.general, message.id, NOW)
            .await;
        fixture
            .messages
            .pin(fixture.owner, fixture.general, message.id, NOW)
            .await
            .unwrap();
        let unpinned = fixture
            .messages
            .unpin(fixture.member, fixture.general, message.id)
            .await;
        fixture.pool.close().await;

        for refused in [pinned, unpinned] {
            assert!(
                matches!(
                    refused,
                    Err(MessageError::Guild(GuildError::MissingPermissions))
                ),
                "even for their own message: {refused:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_channel_holds_at_most_fifty_pins() {
        use crate::domain::message_rules::MAXIMUM_PINS_PER_CHANNEL;

        let fixture = fixture().await;
        let mut outcomes = Vec::new();
        for index in 0..=u64::from(MAXIMUM_PINS_PER_CHANNEL) {
            let message = fixture
                .messages
                .send(fixture.owner, fixture.general, text("pin me"), NOW + index)
                .await
                .unwrap();
            outcomes.push(
                fixture
                    .messages
                    .pin(fixture.owner, fixture.general, message.id, NOW + index)
                    .await,
            );
        }
        let pins = fixture
            .messages
            .list_pins(fixture.owner, fixture.general)
            .await
            .unwrap();
        fixture.pool.close().await;

        let (last, allowed) = outcomes.split_last().unwrap();
        assert!(allowed.iter().all(Result::is_ok));
        assert!(
            matches!(last, Err(MessageError::Guild(GuildError::LimitReached(_)))),
            "{last:?}"
        );
        assert_eq!(
            pins.len(),
            usize::try_from(MAXIMUM_PINS_PER_CHANNEL).unwrap()
        );
    }

    #[tokio::test]
    async fn participants_react_and_pin_in_a_conversation_without_audit() {
        use mellow_protocol::{AuditAction, AuditLogQuery};

        let fixture = fixture().await;
        let conversation = fixture
            .messages
            .open_direct_message(fixture.member, open_with(fixture.owner), NOW)
            .await
            .unwrap();
        let message = fixture
            .messages
            .send(fixture.member, conversation.id, text("hello"), NOW)
            .await
            .unwrap();
        let [mut owner_events, _member_events] = live_sessions(&fixture);
        fixture
            .messages
            .add_reaction(fixture.owner, conversation.id, message.id, "👋")
            .await
            .unwrap();
        fixture
            .messages
            .pin(fixture.owner, conversation.id, message.id, NOW)
            .await
            .unwrap();
        let outsider_reacts = fixture
            .messages
            .add_reaction(fixture.outsider, conversation.id, message.id, "👋")
            .await;
        let outsider_pins = fixture
            .messages
            .pin(fixture.outsider, conversation.id, message.id, NOW)
            .await;
        let pins = fixture
            .messages
            .list_pins(fixture.member, conversation.id)
            .await
            .unwrap();
        let entries = fixture
            .guilds
            .list_audit_log(fixture.owner, fixture.guild.id, AuditLogQuery::default())
            .await
            .unwrap();
        let mut reaction_event = None;
        while let Ok(event) = owner_events.try_recv() {
            let event: serde_json::Value = serde_json::from_str(&event.payload_json).unwrap();
            if event["t"] == "MESSAGE_REACTION_ADD" {
                reaction_event = Some(event["data"].clone());
            }
        }
        fixture.pool.close().await;

        assert_eq!(
            reaction_event.unwrap(),
            serde_json::json!({
                "channel_id": conversation.id.to_string(),
                "message_id": message.id.to_string(),
                "user_id": fixture.owner.to_string(),
                "emoji": "👋",
            }),
            "no guild_id in a conversation"
        );
        for refused in [outsider_reacts, outsider_pins] {
            assert!(
                matches!(refused, Err(MessageError::Guild(GuildError::NotFound))),
                "{refused:?}"
            );
        }
        assert_eq!(pins.len(), 1);
        assert_eq!(summaries(&pins[0]), [("👋".to_owned(), 1, false)]);
        assert!(
            entries
                .iter()
                .all(|entry| entry.action != AuditAction::MessagePin),
            "conversations have no audit log"
        );
    }

    #[tokio::test]
    async fn an_emoji_with_or_without_its_variation_selector_is_one_reaction() {
        let fixture = fixture().await;
        let message = fixture
            .messages
            .send(fixture.owner, fixture.general, text("love"), NOW)
            .await
            .unwrap();
        let [_owner_events, mut member_events] = live_sessions(&fixture);
        fixture
            .messages
            .add_reaction(fixture.owner, fixture.general, message.id, "❤️")
            .await
            .unwrap();
        fixture
            .messages
            .add_reaction(fixture.member, fixture.general, message.id, "❤")
            .await
            .unwrap();
        fixture
            .messages
            .remove_own_reaction(fixture.owner, fixture.general, message.id, "❤")
            .await
            .unwrap();
        let seen = history(&fixture, fixture.member, fixture.general).await;
        let emoji_in_events: Vec<String> = std::iter::from_fn(|| member_events.try_recv().ok())
            .map(|event| serde_json::from_str::<serde_json::Value>(&event.payload_json).unwrap())
            .map(|event| event["data"]["emoji"].as_str().unwrap().to_owned())
            .collect();
        fixture.pool.close().await;

        assert_eq!(summaries(&seen[0]), [("❤".to_owned(), 1, true)]);
        assert_eq!(
            emoji_in_events,
            ["❤", "❤", "❤"],
            "events carry the stored key"
        );
    }

    #[tokio::test]
    async fn without_the_history_a_message_and_a_missing_one_answer_alike() {
        use mellow_protocol::{OverwriteKind, SetPermissionOverwriteRequest};

        let fixture = fixture().await;
        let message = fixture
            .messages
            .send(fixture.owner, fixture.general, text("before you came"), NOW)
            .await
            .unwrap();
        fixture
            .messages
            .add_reaction(fixture.member, fixture.general, message.id, "👍")
            .await
            .unwrap();
        fixture
            .guilds
            .set_permission_overwrite(
                fixture.owner,
                fixture.general,
                fixture.member,
                SetPermissionOverwriteRequest {
                    kind: OverwriteKind::Member,
                    allow: Permissions::empty(),
                    deny: Permissions::READ_MESSAGE_HISTORY,
                },
            )
            .await
            .unwrap();
        let missing = Snowflake::from_raw(1);
        let mut attempts = Vec::new();
        for message_id in [message.id, missing] {
            attempts.push(
                fixture
                    .messages
                    .add_reaction(fixture.member, fixture.general, message_id, "👍")
                    .await,
            );
            attempts.push(
                fixture
                    .messages
                    .list_reacting_users(
                        fixture.member,
                        fixture.general,
                        message_id,
                        "👍",
                        mellow_protocol::ReactionUsersQuery::default(),
                    )
                    .await
                    .map(|_| ()),
            );
        }
        let removed_from_missing = fixture
            .messages
            .remove_own_reaction(fixture.member, fixture.general, missing, "👍")
            .await;
        let removed_own = fixture
            .messages
            .remove_own_reaction(fixture.member, fixture.general, message.id, "👍")
            .await;
        let seen = history(&fixture, fixture.owner, fixture.general).await;
        fixture.pool.close().await;

        for attempt in attempts {
            assert!(
                matches!(attempt, Err(MessageError::MessageNotFound)),
                "{attempt:?}"
            );
        }
        assert!(removed_from_missing.is_ok(), "{removed_from_missing:?}");
        assert!(
            removed_own.is_ok(),
            "taking one's own back needs no history"
        );
        assert_eq!(summaries(&seen[0]), Vec::new());
    }

    #[tokio::test]
    async fn moderators_remove_others_reactions_by_user_emoji_or_all_and_are_audited() {
        use mellow_protocol::{AuditAction, AuditLogQuery};

        let fixture = fixture().await;
        let message = fixture
            .messages
            .send(fixture.member, fixture.general, text("spam magnet"), NOW)
            .await
            .unwrap();
        for (user, emoji) in [
            (fixture.member, "👍"),
            (fixture.member, "🍕"),
            (fixture.owner, "🍕"),
            (fixture.member, "🎉"),
        ] {
            fixture
                .messages
                .add_reaction(user, fixture.general, message.id, emoji)
                .await
                .unwrap();
        }
        let [_owner_events, mut member_events] = live_sessions(&fixture);
        let remove = |actor, removal| {
            fixture
                .messages
                .remove_reactions(actor, fixture.general, message.id, removal)
        };
        let member_refused = remove(fixture.member, ReactionRemoval::Emoji("🍕".into())).await;
        remove(
            fixture.owner,
            ReactionRemoval::User {
                user_id: fixture.member,
                emoji: "👍".into(),
            },
        )
        .await
        .unwrap();
        remove(fixture.owner, ReactionRemoval::Emoji("🍕".into()))
            .await
            .unwrap();
        let after_emoji = history(&fixture, fixture.owner, fixture.general).await;
        remove(fixture.owner, ReactionRemoval::All).await.unwrap();
        // Nothing left: no event and no audit entry.
        remove(fixture.owner, ReactionRemoval::All).await.unwrap();
        let after_all = history(&fixture, fixture.owner, fixture.general).await;
        let entries = fixture
            .guilds
            .list_audit_log(fixture.owner, fixture.guild.id, AuditLogQuery::default())
            .await
            .unwrap();
        fixture.pool.close().await;

        assert!(
            matches!(
                member_refused,
                Err(MessageError::Guild(GuildError::MissingPermissions))
            ),
            "{member_refused:?}"
        );
        assert_eq!(summaries(&after_emoji[0]), [("🎉".to_owned(), 1, false)]);
        assert_eq!(summaries(&after_all[0]), Vec::new());
        assert_eq!(
            event_types(&mut member_events),
            [
                "MESSAGE_REACTION_REMOVE",
                "MESSAGE_REACTION_REMOVE_EMOJI",
                "MESSAGE_REACTION_REMOVE_ALL"
            ]
        );
        let removals: Vec<_> = entries
            .iter()
            .filter(|entry| entry.action == AuditAction::MessageReactionRemove)
            .map(|entry| (entry.actor_id, entry.target_id, entry.context_id))
            .collect();
        assert_eq!(
            removals,
            [(Some(fixture.owner), Some(message.id), Some(fixture.general)); 3]
        );
    }

    #[tokio::test]
    async fn no_one_removes_others_reactions_in_a_conversation() {
        let fixture = fixture().await;
        let conversation = fixture
            .messages
            .open_direct_message(fixture.member, open_with(fixture.owner), NOW)
            .await
            .unwrap();
        let message = fixture
            .messages
            .send(fixture.member, conversation.id, text("hi"), NOW)
            .await
            .unwrap();
        fixture
            .messages
            .add_reaction(fixture.member, conversation.id, message.id, "👋")
            .await
            .unwrap();
        let removed_by_other = fixture
            .messages
            .remove_reactions(
                fixture.owner,
                conversation.id,
                message.id,
                ReactionRemoval::All,
            )
            .await;
        let removed_by_self = fixture
            .messages
            .remove_reactions(
                fixture.member,
                conversation.id,
                message.id,
                ReactionRemoval::User {
                    user_id: fixture.member,
                    emoji: "👋".into(),
                },
            )
            .await;
        let seen = history(&fixture, fixture.owner, conversation.id).await;
        fixture.pool.close().await;

        assert!(
            matches!(
                removed_by_other,
                Err(MessageError::Guild(GuildError::MissingPermissions))
            ),
            "{removed_by_other:?}"
        );
        assert!(removed_by_self.is_ok(), "one's own reaction, by user id");
        assert_eq!(summaries(&seen[0]), Vec::new());
    }

    #[tokio::test]
    async fn a_pin_event_carries_the_message_as_its_author_left_it() {
        let fixture = fixture().await;
        let message = fixture
            .messages
            .send(fixture.member, fixture.general, text("pin me"), NOW)
            .await
            .unwrap();
        let [_owner_events, mut member_events] = live_sessions(&fixture);
        fixture
            .messages
            .pin(fixture.owner, fixture.general, message.id, NOW + 1)
            .await
            .unwrap();
        fixture.pool.close().await;

        let event: serde_json::Value =
            serde_json::from_str(&member_events.try_recv().unwrap().payload_json).unwrap();
        let mut expected = serde_json::to_value(&message).unwrap();
        expected["pinned"] = serde_json::json!(true);
        assert_eq!(event["t"], "MESSAGE_UPDATE");
        assert_eq!(
            event["data"], expected,
            "author, attachments and content read before the write"
        );
    }
}
