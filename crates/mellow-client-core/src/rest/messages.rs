//! Reactions, polls, checklists, progress bars, pins, conversation search and group
//! conversations.

use mellow_protocol::{
    Checklist, ChecklistItemRequest, CreateGroupConversationRequest, DirectMessageChannel, Message,
    MessageSearchQuery, MessageSearchResults, NewProgressBar, ProgressBar, PublicUser,
    ReactionUsersQuery, Snowflake, UpdateGroupConversationRequest,
};
use reqwest::Method;

use super::{RestClient, percent_encode};
use crate::error::ClientError;

/// `…/messages/{m}/reactions/{emoji}`; the emoji (Unicode or `:id`) is encoded as one segment.
fn reactions_path(channel_id: Snowflake, message_id: Snowflake, emoji: &str) -> String {
    format!(
        "api/v1/channels/{channel_id}/messages/{message_id}/reactions/{}",
        percent_encode(emoji)
    )
}

fn poll_answer_path(channel_id: Snowflake, message_id: Snowflake, answer_id: u32) -> String {
    format!("api/v1/channels/{channel_id}/messages/{message_id}/poll/answers/{answer_id}")
}

fn checklist_item_path(channel_id: Snowflake, message_id: Snowflake, item_id: u32) -> String {
    format!("api/v1/channels/{channel_id}/messages/{message_id}/checklist/items/{item_id}")
}

impl RestClient {
    /// Idempotent: reacting twice with one emoji changes nothing.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` without `ADD_REACTIONS` for a new emoji.
    pub async fn add_reaction(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
    ) -> Result<(), ClientError> {
        let path = format!("{}/@me", reactions_path(channel_id, message_id, emoji));
        self.call_without_answer(Method::PUT, &path).await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn remove_own_reaction(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
    ) -> Result<(), ClientError> {
        let path = format!("{}/@me", reactions_path(channel_id, message_id, emoji));
        self.call_without_answer(Method::DELETE, &path).await
    }

    /// A moderator removes someone else's reaction.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_MESSAGES`.
    pub async fn remove_user_reaction(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
        user_id: Snowflake,
    ) -> Result<(), ClientError> {
        let path = format!(
            "{}/{user_id}",
            reactions_path(channel_id, message_id, emoji)
        );
        self.call_without_answer(Method::DELETE, &path).await
    }

    /// A moderator removes every reaction with this emoji.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_MESSAGES`.
    pub async fn remove_emoji_reactions(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
    ) -> Result<(), ClientError> {
        self.call_without_answer(
            Method::DELETE,
            &reactions_path(channel_id, message_id, emoji),
        )
        .await
    }

    /// A moderator removes every reaction of the message.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_MESSAGES`.
    pub async fn remove_all_reactions(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Result<(), ClientError> {
        self.call_without_answer(
            Method::DELETE,
            &format!("api/v1/channels/{channel_id}/messages/{message_id}/reactions"),
        )
        .await
    }

    /// Who reacted with this emoji, by ascending id.
    ///
    /// # Errors
    /// Network or API errors.
    pub async fn list_reacting_users(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
        query: &ReactionUsersQuery,
    ) -> Result<Vec<PublicUser>, ClientError> {
        let builder = self
            .request(
                Method::GET,
                &reactions_path(channel_id, message_id, emoji),
                true,
            )?
            .query(query);
        Self::send_and_decode_json(builder).await
    }

    /// Idempotent; a single-choice poll moves the vote here (ADR-0043).
    ///
    /// # Errors
    /// Network or API errors; `ValidationFailed` once the poll has ended.
    pub async fn add_poll_vote(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        answer_id: u32,
    ) -> Result<(), ClientError> {
        let path = format!(
            "{}/votes/@me",
            poll_answer_path(channel_id, message_id, answer_id)
        );
        self.call_without_answer(Method::PUT, &path).await
    }

    /// # Errors
    /// Network or API errors; `ValidationFailed` once the poll has ended.
    pub async fn remove_poll_vote(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        answer_id: u32,
    ) -> Result<(), ClientError> {
        let path = format!(
            "{}/votes/@me",
            poll_answer_path(channel_id, message_id, answer_id)
        );
        self.call_without_answer(Method::DELETE, &path).await
    }

    /// Who voted for this answer, by ascending id.
    ///
    /// # Errors
    /// Network or API errors.
    pub async fn list_poll_voters(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        answer_id: u32,
        query: &ReactionUsersQuery,
    ) -> Result<Vec<PublicUser>, ClientError> {
        let path = format!(
            "{}/voters",
            poll_answer_path(channel_id, message_id, answer_id)
        );
        let builder = self.request(Method::GET, &path, true)?.query(query);
        Self::send_and_decode_json(builder).await
    }

    /// Its author ends the poll now; `MESSAGE_UPDATE` carries the result.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` for someone else's poll.
    pub async fn end_poll(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Result<(), ClientError> {
        self.call_without_answer(
            Method::POST,
            &format!("api/v1/channels/{channel_id}/messages/{message_id}/poll/end"),
        )
        .await
    }

    /// Its author or a moderator adds an item at the end (ADR-0047).
    ///
    /// # Errors
    /// Network or API errors; `LimitReached` past `MAXIMUM_CHECKLIST_ITEMS`.
    pub async fn add_checklist_item(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        text: &str,
    ) -> Result<Checklist, ClientError> {
        self.call_with_json(
            Method::POST,
            &format!("api/v1/channels/{channel_id}/messages/{message_id}/checklist/items"),
            &ChecklistItemRequest {
                text: text.to_owned(),
            },
        )
        .await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` for someone else's checklist.
    pub async fn rename_checklist_item(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        item_id: u32,
        text: &str,
    ) -> Result<Checklist, ClientError> {
        self.call_with_json(
            Method::PATCH,
            &checklist_item_path(channel_id, message_id, item_id),
            &ChecklistItemRequest {
                text: text.to_owned(),
            },
        )
        .await
    }

    /// Never the last item: the instance refuses it.
    ///
    /// # Errors
    /// Network or API errors; `ValidationFailed` for the last item.
    pub async fn remove_checklist_item(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        item_id: u32,
    ) -> Result<Checklist, ClientError> {
        self.call(
            Method::DELETE,
            &checklist_item_path(channel_id, message_id, item_id),
        )
        .await
    }

    /// Ticks or unticks an item; idempotent. Answers the checklist as it is now.
    ///
    /// # Errors
    /// Network or API errors; `NotFound` for an unknown item.
    pub async fn set_checklist_item_ticked(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        item_id: u32,
        is_ticked: bool,
    ) -> Result<Checklist, ClientError> {
        let method = if is_ticked {
            Method::PUT
        } else {
            Method::DELETE
        };
        let path = format!(
            "{}/check",
            checklist_item_path(channel_id, message_id, item_id)
        );
        self.call(method, &path).await
    }

    /// Its author or a moderator replaces the label and value (ADR-0047).
    ///
    /// # Errors
    /// Network or API errors; `ValidationFailed` for a value above the maximum.
    pub async fn update_progress_bar(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        progress_bar: &NewProgressBar,
    ) -> Result<ProgressBar, ClientError> {
        self.call_with_json(
            Method::PATCH,
            &format!("api/v1/channels/{channel_id}/messages/{message_id}/progress-bar"),
            progress_bar,
        )
        .await
    }

    /// The most recently pinned first.
    ///
    /// # Errors
    /// Network or API errors.
    pub async fn list_pins(&self, channel_id: Snowflake) -> Result<Vec<Message>, ClientError> {
        self.call(Method::GET, &format!("api/v1/channels/{channel_id}/pins"))
            .await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_MESSAGES` in a guild,
    /// `LimitReached` past the pin limit.
    pub async fn pin_message(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Result<(), ClientError> {
        self.call_without_answer(
            Method::PUT,
            &format!("api/v1/channels/{channel_id}/pins/{message_id}"),
        )
        .await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_MESSAGES` in a guild.
    pub async fn unpin_message(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Result<(), ClientError> {
        self.call_without_answer(
            Method::DELETE,
            &format!("api/v1/channels/{channel_id}/pins/{message_id}"),
        )
        .await
    }

    /// Searches the user's direct and group conversations.
    ///
    /// # Errors
    /// Network or API errors; `ValidationFailed` for a query out of bounds.
    pub async fn search_my_conversation_messages(
        &self,
        query: &MessageSearchQuery,
    ) -> Result<MessageSearchResults, ClientError> {
        self.call_with_json(Method::POST, "api/v1/users/@me/messages/search", query)
            .await
    }

    /// A new group with friends of the caller (ADR-0026).
    ///
    /// # Errors
    /// Network or API errors; `NotFound` for someone who is not a friend, `LimitReached` past
    /// `MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS`.
    pub async fn create_group_conversation(
        &self,
        request: &CreateGroupConversationRequest,
    ) -> Result<DirectMessageChannel, ClientError> {
        self.call_with_json(Method::POST, "api/v1/group-conversations", request)
            .await
    }

    /// An empty name clears it.
    ///
    /// # Errors
    /// Network or API errors; `NotFound` unless the caller takes part.
    pub async fn rename_group_conversation(
        &self,
        channel_id: Snowflake,
        name: &str,
    ) -> Result<DirectMessageChannel, ClientError> {
        self.call_with_json(
            Method::PATCH,
            &format!("api/v1/group-conversations/{channel_id}"),
            &UpdateGroupConversationRequest {
                name: name.to_owned(),
            },
        )
        .await
    }

    /// Adds a friend of the caller.
    ///
    /// # Errors
    /// Network or API errors; `NotFound` for someone who is not a friend.
    pub async fn add_group_participant(
        &self,
        channel_id: Snowflake,
        user_id: Snowflake,
    ) -> Result<DirectMessageChannel, ClientError> {
        self.call(
            Method::PUT,
            &format!("api/v1/group-conversations/{channel_id}/recipients/{user_id}"),
        )
        .await
    }

    /// Removing oneself leaves the group; only its owner removes others.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` for someone else when not the owner.
    pub async fn remove_group_participant(
        &self,
        channel_id: Snowflake,
        user_id: Snowflake,
    ) -> Result<(), ClientError> {
        self.call_without_answer(
            Method::DELETE,
            &format!("api/v1/group-conversations/{channel_id}/recipients/{user_id}"),
        )
        .await
    }
}
