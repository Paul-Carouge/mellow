//! Guild settings, channel order, invites, custom emoji, soundboard, role icons and guild search.
//! Picture and sound uploads are in `files`.

use mellow_protocol::{
    ApiErrorCode, Channel, ChannelPlacement, CustomEmoji, Guild, GuildInvite, MessageSearchQuery,
    MessageSearchResults, ReorderChannelsRequest, Role, SetJoinSoundRequest, SetRoleEmojiRequest,
    Snowflake, SoundboardSound, UpdateEmojiRequest, UpdateGuildRequest, UpdateSoundRequest,
};
use reqwest::{Method, StatusCode};

use super::RestClient;
use crate::error::ClientError;

impl RestClient {
    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_GUILD`.
    pub async fn rename_guild(
        &self,
        guild_id: Snowflake,
        name: &str,
    ) -> Result<Guild, ClientError> {
        self.call_with_json(
            Method::PATCH,
            &format!("api/v1/guilds/{guild_id}"),
            &UpdateGuildRequest {
                name: Some(name.to_owned()),
            },
        )
        .await
    }

    /// The owner only.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` for anyone else.
    pub async fn delete_guild(&self, guild_id: Snowflake) -> Result<(), ClientError> {
        self.call_without_answer(Method::DELETE, &format!("api/v1/guilds/{guild_id}"))
            .await
    }

    /// Every channel the user can view, once each, in their new order; answers the new list.
    ///
    /// # Errors
    /// Network or API errors; `ValidationFailed` when the list is not the guild's channels,
    /// `Conflict` when another reordering came first.
    pub async fn reorder_channels(
        &self,
        guild_id: Snowflake,
        placements: &[ChannelPlacement],
    ) -> Result<Vec<Channel>, ClientError> {
        self.call_with_json(
            Method::PUT,
            &format!("api/v1/guilds/{guild_id}/channels/order"),
            &ReorderChannelsRequest {
                channels: placements.to_vec(),
            },
        )
        .await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_GUILD`.
    pub async fn list_invites(&self, guild_id: Snowflake) -> Result<Vec<GuildInvite>, ClientError> {
        self.call(Method::GET, &format!("api/v1/guilds/{guild_id}/invites"))
            .await
    }

    /// # Errors
    /// Network or API errors; `NotFound` for an unknown code.
    pub async fn revoke_invite(&self, code: &str) -> Result<(), ClientError> {
        // The code becomes a path segment: refuse anything that is not a plain code.
        if code.is_empty()
            || !code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(ClientError::Api {
                status: StatusCode::NOT_FOUND.as_u16(),
                code: ApiErrorCode::NotFound,
                message: "this invite is unknown".into(),
            });
        }
        self.call_without_answer(Method::DELETE, &format!("api/v1/invites/{code}"))
            .await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn list_emojis(&self, guild_id: Snowflake) -> Result<Vec<CustomEmoji>, ClientError> {
        self.call(Method::GET, &format!("api/v1/guilds/{guild_id}/emojis"))
            .await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_EMOJIS`.
    pub async fn rename_emoji(
        &self,
        guild_id: Snowflake,
        emoji_id: Snowflake,
        name: &str,
    ) -> Result<CustomEmoji, ClientError> {
        self.call_with_json(
            Method::PATCH,
            &format!("api/v1/guilds/{guild_id}/emojis/{emoji_id}"),
            &UpdateEmojiRequest {
                name: name.to_owned(),
            },
        )
        .await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_EMOJIS`.
    pub async fn delete_emoji(
        &self,
        guild_id: Snowflake,
        emoji_id: Snowflake,
    ) -> Result<(), ClientError> {
        self.call_without_answer(
            Method::DELETE,
            &format!("api/v1/guilds/{guild_id}/emojis/{emoji_id}"),
        )
        .await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn list_sounds(
        &self,
        guild_id: Snowflake,
    ) -> Result<Vec<SoundboardSound>, ClientError> {
        self.call(Method::GET, &format!("api/v1/guilds/{guild_id}/sounds"))
            .await
    }

    /// Replaces both fields: `emoji: None` removes the sound's emoji.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` without the right to manage sounds.
    pub async fn update_sound(
        &self,
        guild_id: Snowflake,
        sound_id: Snowflake,
        request: &UpdateSoundRequest,
    ) -> Result<SoundboardSound, ClientError> {
        self.call_with_json(
            Method::PATCH,
            &format!("api/v1/guilds/{guild_id}/sounds/{sound_id}"),
            request,
        )
        .await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without the right to manage sounds.
    pub async fn delete_sound(
        &self,
        guild_id: Snowflake,
        sound_id: Snowflake,
    ) -> Result<(), ClientError> {
        self.call_without_answer(
            Method::DELETE,
            &format!("api/v1/guilds/{guild_id}/sounds/{sound_id}"),
        )
        .await
    }

    /// The sound played when the user joins a voice channel of this guild; `None` clears it.
    ///
    /// # Errors
    /// Network or API errors; `NotFound` for a sound of another guild.
    pub async fn set_join_sound(
        &self,
        guild_id: Snowflake,
        sound_id: Option<Snowflake>,
    ) -> Result<(), ClientError> {
        self.call_with_json_without_answer(
            Method::PATCH,
            &format!("api/v1/guilds/{guild_id}/members/@me/join-sound"),
            &SetJoinSoundRequest { sound_id },
        )
        .await
    }

    /// The role's icon becomes this Unicode emoji, in place of any picture (ADR-0050).
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` for a role out of the caller's reach.
    pub async fn set_role_icon_emoji(
        &self,
        guild_id: Snowflake,
        role_id: Snowflake,
        unicode_emoji: &str,
    ) -> Result<Role, ClientError> {
        self.call_with_json(
            Method::PUT,
            &format!("api/v1/guilds/{guild_id}/roles/{role_id}/icon/emoji"),
            &SetRoleEmojiRequest {
                unicode_emoji: unicode_emoji.to_owned(),
            },
        )
        .await
    }

    /// Removes a role's picture or emoji icon.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` for a role out of the caller's reach.
    pub async fn remove_role_icon(
        &self,
        guild_id: Snowflake,
        role_id: Snowflake,
    ) -> Result<Role, ClientError> {
        self.call(
            Method::DELETE,
            &format!("api/v1/guilds/{guild_id}/roles/{role_id}/icon"),
        )
        .await
    }

    /// Searches the guild's channels the user can read. A `POST`, so the query text never shows
    /// in URLs and access logs.
    ///
    /// # Errors
    /// Network or API errors; `ValidationFailed` for a query out of bounds.
    pub async fn search_guild_messages(
        &self,
        guild_id: Snowflake,
        query: &MessageSearchQuery,
    ) -> Result<MessageSearchResults, ClientError> {
        self.call_with_json(
            Method::POST,
            &format!("api/v1/guilds/{guild_id}/messages/search"),
            query,
        )
        .await
    }
}
