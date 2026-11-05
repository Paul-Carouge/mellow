//! Polls in messages (ADR-0043): creating one with a message, votes, the early end and voters.
//! Who may vote is whoever reads the channel; creating one needs `SEND_POLLS` (`permissions`).

use mellow_protocol::{
    GatewayEvent, Message, NewPoll, Permissions, Poll, PollAnswer, PollVote, PublicUser,
    ReactionUsersQuery, SendMessageRequest, Snowflake,
};

use super::{MessageError, Messages, attachments_of, message_from_stored, publish};
use crate::domain::guilds::GuildError;
use crate::domain::message_rules::{
    MessageRuleViolation, reacting_users_page_size, validated_poll,
};
use crate::domain::profile_images::{fill_profile_images, profile_images_of_user};
use crate::storage::Database;
use crate::storage::polls::{self as storage, NewStoredPoll, StoredPoll, VoteOutcome, VoteRefusal};

const MILLISECONDS_PER_HOUR: u64 = 60 * 60 * 1_000;

/// A poll request checked against ADR-0043, waiting for its send time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CheckedPoll {
    question: String,
    answers: Vec<String>,
    allows_multiple_answers: bool,
    duration_hours: u32,
}

impl CheckedPoll {
    pub(super) fn into_stored(self, now_unix_milliseconds: u64) -> NewStoredPoll {
        NewStoredPoll {
            question: self.question,
            answers: self.answers,
            allows_multiple_answers: self.allows_multiple_answers,
            expires_at_unix_milliseconds: now_unix_milliseconds
                .saturating_add(u64::from(self.duration_hours) * MILLISECONDS_PER_HOUR),
        }
    }
}

/// The request's poll once valid: a poll message carries nothing else of its own.
pub(super) fn checked_poll(
    request: &SendMessageRequest,
) -> Result<Option<CheckedPoll>, MessageError> {
    let Some(poll) = &request.poll else {
        return Ok(None);
    };
    let carries_something_else = !request.content.trim().is_empty()
        || !request.attachment_ids.is_empty()
        || request.reply_to_id.is_some()
        || request.forwarded_from.is_some();
    if carries_something_else {
        return Err(MessageRuleViolation::Poll.into());
    }
    let NewPoll {
        allows_multiple_answers,
        duration_hours,
        ..
    } = *poll;
    let (question, answers) = validated_poll(poll)?;
    Ok(Some(CheckedPoll {
        question,
        answers,
        allows_multiple_answers,
        duration_hours,
    }))
}

/// A poll just created, as its first event shows it.
pub(super) fn poll_without_votes(poll: &NewStoredPoll) -> Poll {
    Poll {
        question: poll.question.clone(),
        answers: (1..)
            .zip(&poll.answers)
            .map(|(id, text)| PollAnswer {
                id,
                text: text.clone(),
                vote_count: 0,
                me_voted: false,
            })
            .collect(),
        allows_multiple_answers: poll.allows_multiple_answers,
        expires_at_unix_milliseconds: poll.expires_at_unix_milliseconds,
        ended_at_unix_milliseconds: None,
    }
}

pub(super) fn poll_from_stored(poll: StoredPoll) -> Poll {
    Poll {
        question: poll.question,
        answers: poll
            .answers
            .into_iter()
            .map(|answer| PollAnswer {
                id: answer.id,
                text: answer.text,
                vote_count: answer.vote_count,
                me_voted: answer.has_viewer_voted,
            })
            .collect(),
        allows_multiple_answers: poll.allows_multiple_answers,
        expires_at_unix_milliseconds: poll.expires_at_unix_milliseconds,
        ended_at_unix_milliseconds: poll.ended_at_unix_milliseconds,
    }
}

/// Fills `poll` of these messages, with `viewer_id`'s votes when given (REST), in two queries.
pub(super) async fn fill_polls(
    database: &Database,
    viewer_id: Option<Snowflake>,
    messages: &mut [Message],
) -> Result<(), MessageError> {
    let ids: Vec<Snowflake> = messages.iter().map(|message| message.id).collect();
    let mut polls = storage::polls_of_messages(database, &ids, viewer_id).await?;
    for message in messages {
        message.poll = polls.remove(&message.id).map(poll_from_stored);
    }
    Ok(())
}

/// One message's poll without votes of a reader, read before a write whose event must carry it.
pub(super) async fn poll_of(
    database: &Database,
    message_id: Snowflake,
) -> Result<Option<Poll>, MessageError> {
    Ok(storage::polls_of_messages(database, &[message_id], None)
        .await?
        .remove(&message_id)
        .map(poll_from_stored))
}

impl Messages {
    /// The channel and message of a poll the user may vote on or list: they read the channel.
    async fn poll_target(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Result<(super::MessageChannel, super::StoredMessage), MessageError> {
        let (channel, _) = self
            .text_channel_and_permissions_requiring(
                user_id,
                channel_id,
                Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY,
            )
            .await?;
        let message = self.message_in_channel(channel.id(), message_id).await?;
        if !storage::has_poll(&self.database, message.id).await? {
            return Err(MessageError::MessageNotFound);
        }
        Ok((channel, message))
    }

    /// Votes for `answer_id` while the poll is open; again is a no-op. A single-choice poll moves
    /// the vote: clients get a removal for the old answer, then the addition.
    pub(crate) async fn add_poll_vote(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        answer_id: u32,
        now_unix_milliseconds: u64,
    ) -> Result<(), MessageError> {
        let (channel, message) = self.poll_target(user_id, channel_id, message_id).await?;
        let audience = self.audience(&channel).await?;
        let vote = |answer_id| PollVote {
            guild_id: channel.guild_id(),
            channel_id: channel.id(),
            message_id: message.id,
            answer_id,
            user_id,
        };
        match storage::add_vote(
            &self.database,
            message.id,
            answer_id,
            user_id,
            now_unix_milliseconds,
        )
        .await?
        {
            VoteOutcome::Added { moved_from } => {
                for old_answer in moved_from {
                    publish(
                        &self.guilds,
                        &audience,
                        &GatewayEvent::MessagePollVoteRemove(vote(old_answer)),
                    );
                }
                publish(
                    &self.guilds,
                    &audience,
                    &GatewayEvent::MessagePollVoteAdd(vote(answer_id)),
                );
                Ok(())
            }
            VoteOutcome::AlreadyCounted => Ok(()),
            VoteOutcome::Refused(VoteRefusal::Ended) => Err(MessageRuleViolation::PollEnded.into()),
            VoteOutcome::Refused(VoteRefusal::NoSuchAnswer) => Err(MessageError::MessageNotFound),
        }
    }

    /// Takes the user's vote for `answer_id` back while the poll is open; none is a no-op.
    pub(crate) async fn remove_poll_vote(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        answer_id: u32,
        now_unix_milliseconds: u64,
    ) -> Result<(), MessageError> {
        let (channel, message) = self.poll_target(user_id, channel_id, message_id).await?;
        let audience = self.audience(&channel).await?;
        match storage::remove_vote(
            &self.database,
            message.id,
            answer_id,
            user_id,
            now_unix_milliseconds,
        )
        .await?
        {
            Ok(true) => {
                publish(
                    &self.guilds,
                    &audience,
                    &GatewayEvent::MessagePollVoteRemove(PollVote {
                        guild_id: channel.guild_id(),
                        channel_id: channel.id(),
                        message_id: message.id,
                        answer_id,
                        user_id,
                    }),
                );
                Ok(())
            }
            Ok(false) => Ok(()),
            Err(_) => Err(MessageRuleViolation::PollEnded.into()),
        }
    }

    /// Only the poll's author ends it early; clients get the message with `ended_at`.
    pub(crate) async fn end_poll(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        now_unix_milliseconds: u64,
    ) -> Result<(), MessageError> {
        let (channel, message) = self.poll_target(user_id, channel_id, message_id).await?;
        if message.author_id != user_id {
            return Err(GuildError::MissingPermissions.into());
        }
        let audience = self.audience(&channel).await?;
        let author_images = profile_images_of_user(&self.database, message.author_id).await?;
        let attachments = attachments_of(&self.database, message.id).await?;
        let previews = super::link_previews::previews_of(&self.database, message.id).await?;
        // Read before the write, so nothing can fail between the commit and the event.
        let poll = poll_of(&self.database, message.id).await?;
        if !storage::end_poll(&self.database, message.id, now_unix_milliseconds).await? {
            return Err(MessageRuleViolation::PollEnded.into());
        }
        let mut ended = message_from_stored(message, None, attachments);
        ended.author.avatar = author_images.avatar;
        ended.author.banner = author_images.banner;
        ended.link_previews = previews;
        ended.poll = poll.map(|poll| Poll {
            ended_at_unix_milliseconds: Some(now_unix_milliseconds),
            ..poll
        });
        publish(&self.guilds, &audience, &GatewayEvent::MessageUpdate(ended));
        Ok(())
    }

    /// Who voted for `answer_id`, like `list_reacting_users`.
    pub(crate) async fn list_poll_voters(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
        message_id: Snowflake,
        answer_id: u32,
        query: ReactionUsersQuery,
    ) -> Result<Vec<PublicUser>, MessageError> {
        let limit = reacting_users_page_size(query.limit)?;
        let (_, message) = self.poll_target(user_id, channel_id, message_id).await?;
        let mut users: Vec<PublicUser> =
            storage::list_voters(&self.database, message.id, answer_id, query.after, limit)
                .await?
                .into_iter()
                .map(|user| PublicUser {
                    id: user.id,
                    username: user.username,
                    display_name: user.display_name,
                    avatar: None,
                    banner: None,
                })
                .collect();
        fill_profile_images(&self.database, users.iter_mut()).await?;
        Ok(users)
    }
}
