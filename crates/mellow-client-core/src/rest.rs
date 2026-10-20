//! Typed calls to the REST API (`/api/v1`).

use std::time::Duration;

use mellow_protocol::{
    AcknowledgeRequest, ApiErrorBody, AuditLogEntry, AuditLogQuery, Ban, Channel, CreateBanRequest,
    CreateChannelRequest, CreateGuildRequest, CreateInviteRequest, CreateRoleRequest, CurrentUser,
    EditMessageRequest, Guild, Invite, LoginRequest, Member, Message, MessageHistoryQuery,
    RegisterRequest, ReorderRolesRequest, Role, SendMessageRequest, SessionCreated,
    SetPermissionOverwriteRequest, Snowflake, UpdateChannelRequest, UpdateRoleRequest,
};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use zeroize::Zeroizing;

use crate::error::ClientError;
use crate::instance::InstanceAddress;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Far above any honest answer (a 1 000-member list or a page of messages); bounds what a
/// hostile instance can make the client buffer.
const MAXIMUM_RESPONSE_BODY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct RestClient {
    http: reqwest::Client,
    instance: InstanceAddress,
    token: Option<Zeroizing<String>>,
}

/// The bearer token is never printed (`Zeroizing`'s own `Debug` would show it).
impl std::fmt::Debug for RestClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RestClient")
            .field("instance", &self.instance)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish_non_exhaustive()
    }
}

impl RestClient {
    /// # Errors
    /// Fails only if the TLS stack cannot initialise.
    pub fn new(instance: InstanceAddress) -> Result<Self, ClientError> {
        let mut builder = reqwest::Client::builder();
        if instance.is_loopback() {
            // Plain http is only allowed on loopback; an environment proxy would carry the token
            // in clear off this machine.
            builder = builder.no_proxy();
        }
        let http = builder
            .user_agent(concat!("mellow-client/", env!("CARGO_PKG_VERSION")))
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            // Redirects could forward the bearer token elsewhere; an instance has no reason to redirect.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(ClientError::Network)?;
        Ok(Self {
            http,
            instance,
            token: None,
        })
    }

    #[must_use]
    pub fn instance(&self) -> &InstanceAddress {
        &self.instance
    }

    pub fn set_token(&mut self, token: Zeroizing<String>) {
        self.token = Some(token);
    }

    #[must_use]
    pub fn token(&self) -> Option<&str> {
        self.token.as_ref().map(|token| token.as_str())
    }

    fn request(
        &self,
        method: Method,
        path: &str,
        authenticated: bool,
    ) -> Result<RequestBuilder, ClientError> {
        let builder = self.http.request(method, self.instance.api_url(path)?);
        if !authenticated {
            return Ok(builder);
        }
        let token = self.token.as_ref().ok_or(ClientError::NotSignedIn)?;
        Ok(builder.bearer_auth(token.as_str()))
    }

    async fn send_and_decode_json<T: DeserializeOwned>(
        builder: RequestBuilder,
    ) -> Result<T, ClientError> {
        let response = Self::response_or_api_error(builder).await?;
        let body = Self::bounded_body(response).await?;
        serde_json::from_slice(&body)
            .map_err(|error| ClientError::UnexpectedResponse(format!("{:?}", error.classify())))
    }

    async fn send_and_discard_body(builder: RequestBuilder) -> Result<(), ClientError> {
        Self::response_or_api_error(builder).await.map(|_| ())
    }

    async fn bounded_body(mut response: reqwest::Response) -> Result<Vec<u8>, ClientError> {
        let too_large = || ClientError::UnexpectedResponse("response body too large".into());
        if response
            .content_length()
            .is_some_and(|length| length > MAXIMUM_RESPONSE_BODY_BYTES as u64)
        {
            return Err(too_large());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(ClientError::Network)? {
            if body.len() + chunk.len() > MAXIMUM_RESPONSE_BODY_BYTES {
                return Err(too_large());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn response_or_api_error(
        builder: RequestBuilder,
    ) -> Result<reqwest::Response, ClientError> {
        let response = builder.send().await.map_err(ClientError::Network)?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let error_body = Self::bounded_body(response)
            .await
            .ok()
            .and_then(|body| serde_json::from_slice::<ApiErrorBody>(&body).ok());
        match error_body {
            Some(body) => Err(ClientError::Api {
                status: status.as_u16(),
                code: body.code,
                message: body.message,
            }),
            None => Err(ClientError::UnexpectedResponse(format!(
                "HTTP {} without an error body",
                status.as_u16()
            ))),
        }
    }

    async fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
        authenticated: bool,
    ) -> Result<T, ClientError> {
        Self::send_and_decode_json(self.request(Method::POST, path, authenticated)?.json(body))
            .await
    }

    /// Creates an account and signs in; the token is kept for later calls.
    ///
    /// # Errors
    /// `Api` with `RegistrationClosed`, `UsernameTaken`, `EmailTaken`, `ValidationFailed`…
    pub async fn register(
        &mut self,
        request: &RegisterRequest,
    ) -> Result<SessionCreated, ClientError> {
        let session: SessionCreated = self
            .post_json("api/v1/auth/register", request, false)
            .await?;
        self.token = Some(Zeroizing::new(session.token.clone()));
        Ok(session)
    }

    /// # Errors
    /// `Api` with `InvalidCredentials`, `AccountDisabled`, `RateLimited`…
    pub async fn login(&mut self, request: &LoginRequest) -> Result<SessionCreated, ClientError> {
        let session: SessionCreated = self.post_json("api/v1/auth/login", request, false).await?;
        self.token = Some(Zeroizing::new(session.token.clone()));
        Ok(session)
    }

    /// Ends the session on the server and forgets the token.
    ///
    /// # Errors
    /// Network or API errors; the token is forgotten anyway.
    pub async fn logout(&mut self) -> Result<(), ClientError> {
        let result =
            Self::send_and_discard_body(self.request(Method::POST, "api/v1/auth/logout", true)?)
                .await;
        self.token = None;
        result
    }

    /// # Errors
    /// Network or API errors.
    pub async fn current_user(&self) -> Result<CurrentUser, ClientError> {
        Self::send_and_decode_json(self.request(Method::GET, "api/v1/users/@me", true)?).await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn create_guild(&self, name: &str) -> Result<Guild, ClientError> {
        self.post_json(
            "api/v1/guilds",
            &CreateGuildRequest {
                name: name.to_owned(),
            },
            true,
        )
        .await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn create_channel(
        &self,
        guild_id: Snowflake,
        request: &CreateChannelRequest,
    ) -> Result<Channel, ClientError> {
        self.post_json(&format!("api/v1/guilds/{guild_id}/channels"), request, true)
            .await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn create_invite(
        &self,
        guild_id: Snowflake,
        request: &CreateInviteRequest,
    ) -> Result<Invite, ClientError> {
        self.post_json(&format!("api/v1/guilds/{guild_id}/invites"), request, true)
            .await
    }

    /// # Errors
    /// `Api` with `InvalidInvite` for unknown, expired or used-up codes.
    pub async fn join_with_invite(&self, code: &str) -> Result<Guild, ClientError> {
        // The code becomes a path segment: reject anything that is not a plain code.
        if code.is_empty() || !code.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(ClientError::Api {
                status: StatusCode::NOT_FOUND.as_u16(),
                code: mellow_protocol::ApiErrorCode::InvalidInvite,
                message: "this invite is unknown, expired or used up".into(),
            });
        }
        Self::send_and_decode_json(self.request(
            Method::POST,
            &format!("api/v1/invites/{code}"),
            true,
        )?)
        .await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn list_members(&self, guild_id: Snowflake) -> Result<Vec<Member>, ClientError> {
        Self::send_and_decode_json(self.request(
            Method::GET,
            &format!("api/v1/guilds/{guild_id}/members"),
            true,
        )?)
        .await
    }

    /// # Errors
    /// Network or API errors; `NotFound` when the user is not a member.
    pub async fn guild(&self, guild_id: Snowflake) -> Result<Guild, ClientError> {
        Self::send_and_decode_json(self.request(
            Method::GET,
            &format!("api/v1/guilds/{guild_id}"),
            true,
        )?)
        .await
    }

    /// # Errors
    /// Network or API errors; the owner cannot leave their own guild.
    pub async fn leave_guild(&self, guild_id: Snowflake) -> Result<(), ClientError> {
        Self::send_and_discard_body(self.request(
            Method::DELETE,
            &format!("api/v1/guilds/{guild_id}/members/@me"),
            true,
        )?)
        .await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_CHANNELS`.
    pub async fn update_channel(
        &self,
        channel_id: Snowflake,
        request: &UpdateChannelRequest,
    ) -> Result<Channel, ClientError> {
        let builder = self
            .request(
                Method::PATCH,
                &format!("api/v1/channels/{channel_id}"),
                true,
            )?
            .json(request);
        Self::send_and_decode_json(builder).await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_CHANNELS`.
    pub async fn delete_channel(&self, channel_id: Snowflake) -> Result<(), ClientError> {
        Self::send_and_discard_body(self.request(
            Method::DELETE,
            &format!("api/v1/channels/{channel_id}"),
            true,
        )?)
        .await
    }

    /// Replaces the overwrite of `target_id` (a role or a member) on the channel; the answer is
    /// the channel with its new overwrites.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_ROLES` or for bits the caller
    /// lacks.
    pub async fn set_permission_overwrite(
        &self,
        channel_id: Snowflake,
        target_id: Snowflake,
        request: &SetPermissionOverwriteRequest,
    ) -> Result<Channel, ClientError> {
        let builder = self
            .request(
                Method::PUT,
                &format!("api/v1/channels/{channel_id}/permissions/{target_id}"),
                true,
            )?
            .json(request);
        Self::send_and_decode_json(builder).await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_ROLES`.
    pub async fn delete_permission_overwrite(
        &self,
        channel_id: Snowflake,
        target_id: Snowflake,
    ) -> Result<Channel, ClientError> {
        Self::send_and_decode_json(self.request(
            Method::DELETE,
            &format!("api/v1/channels/{channel_id}/permissions/{target_id}"),
            true,
        )?)
        .await
    }

    /// The new role starts lowest, just above `@everyone`.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` without `MANAGE_ROLES` or for bits the caller
    /// lacks.
    pub async fn create_role(
        &self,
        guild_id: Snowflake,
        request: &CreateRoleRequest,
    ) -> Result<Role, ClientError> {
        self.post_json(&format!("api/v1/guilds/{guild_id}/roles"), request, true)
            .await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` for a role at or above the caller's rank.
    pub async fn update_role(
        &self,
        guild_id: Snowflake,
        role_id: Snowflake,
        request: &UpdateRoleRequest,
    ) -> Result<Role, ClientError> {
        let builder = self
            .request(
                Method::PATCH,
                &format!("api/v1/guilds/{guild_id}/roles/{role_id}"),
                true,
            )?
            .json(request);
        Self::send_and_decode_json(builder).await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` for a role at or above the caller's rank.
    pub async fn delete_role(
        &self,
        guild_id: Snowflake,
        role_id: Snowflake,
    ) -> Result<(), ClientError> {
        Self::send_and_discard_body(self.request(
            Method::DELETE,
            &format!("api/v1/guilds/{guild_id}/roles/{role_id}"),
            true,
        )?)
        .await
    }

    /// Takes every role except `@everyone`, from lowest to highest; answers the new role list.
    ///
    /// # Errors
    /// Network or API errors; `ValidationFailed` when the list is not the guild's roles.
    pub async fn reorder_roles(
        &self,
        guild_id: Snowflake,
        request: &ReorderRolesRequest,
    ) -> Result<Vec<Role>, ClientError> {
        let builder = self
            .request(
                Method::PUT,
                &format!("api/v1/guilds/{guild_id}/roles/order"),
                true,
            )?
            .json(request);
        Self::send_and_decode_json(builder).await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` for a role out of the caller's reach.
    pub async fn add_member_role(
        &self,
        guild_id: Snowflake,
        user_id: Snowflake,
        role_id: Snowflake,
    ) -> Result<(), ClientError> {
        Self::send_and_discard_body(self.request(
            Method::PUT,
            &format!("api/v1/guilds/{guild_id}/members/{user_id}/roles/{role_id}"),
            true,
        )?)
        .await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` for a role out of the caller's reach.
    pub async fn remove_member_role(
        &self,
        guild_id: Snowflake,
        user_id: Snowflake,
        role_id: Snowflake,
    ) -> Result<(), ClientError> {
        Self::send_and_discard_body(self.request(
            Method::DELETE,
            &format!("api/v1/guilds/{guild_id}/members/{user_id}/roles/{role_id}"),
            true,
        )?)
        .await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without `KICK_MEMBERS` or for a member at or
    /// above the caller's rank.
    pub async fn kick_member(
        &self,
        guild_id: Snowflake,
        user_id: Snowflake,
    ) -> Result<(), ClientError> {
        Self::send_and_discard_body(self.request(
            Method::DELETE,
            &format!("api/v1/guilds/{guild_id}/members/{user_id}"),
            true,
        )?)
        .await
    }

    /// Bans `user_id`, a member or not, and removes them from the guild.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` without `BAN_MEMBERS` or for a member at or
    /// above the caller's rank.
    pub async fn ban_member(
        &self,
        guild_id: Snowflake,
        user_id: Snowflake,
        request: &CreateBanRequest,
    ) -> Result<(), ClientError> {
        let builder = self
            .request(
                Method::PUT,
                &format!("api/v1/guilds/{guild_id}/bans/{user_id}"),
                true,
            )?
            .json(request);
        Self::send_and_discard_body(builder).await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without `BAN_MEMBERS`.
    pub async fn unban_member(
        &self,
        guild_id: Snowflake,
        user_id: Snowflake,
    ) -> Result<(), ClientError> {
        Self::send_and_discard_body(self.request(
            Method::DELETE,
            &format!("api/v1/guilds/{guild_id}/bans/{user_id}"),
            true,
        )?)
        .await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without `BAN_MEMBERS`.
    pub async fn list_bans(&self, guild_id: Snowflake) -> Result<Vec<Ban>, ClientError> {
        Self::send_and_decode_json(self.request(
            Method::GET,
            &format!("api/v1/guilds/{guild_id}/bans"),
            true,
        )?)
        .await
    }

    /// Newest entries first.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` without `VIEW_AUDIT_LOG`.
    pub async fn list_audit_log(
        &self,
        guild_id: Snowflake,
        query: &AuditLogQuery,
    ) -> Result<Vec<AuditLogEntry>, ClientError> {
        let builder = self
            .request(
                Method::GET,
                &format!("api/v1/guilds/{guild_id}/audit-log"),
                true,
            )?
            .query(query);
        Self::send_and_decode_json(builder).await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn list_messages(
        &self,
        channel_id: Snowflake,
        query: &MessageHistoryQuery,
    ) -> Result<Vec<Message>, ClientError> {
        let builder = self
            .request(
                Method::GET,
                &format!("api/v1/channels/{channel_id}/messages"),
                true,
            )?
            .query(query);
        Self::send_and_decode_json(builder).await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn send_message(
        &self,
        channel_id: Snowflake,
        request: &SendMessageRequest,
    ) -> Result<Message, ClientError> {
        self.post_json(
            &format!("api/v1/channels/{channel_id}/messages"),
            request,
            true,
        )
        .await
    }

    /// Only the author may edit a message.
    ///
    /// # Errors
    /// Network or API errors; `MissingPermissions` for someone else's message.
    pub async fn edit_message(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
        request: &EditMessageRequest,
    ) -> Result<Message, ClientError> {
        let builder = self
            .request(
                Method::PATCH,
                &format!("api/v1/channels/{channel_id}/messages/{message_id}"),
                true,
            )?
            .json(request);
        Self::send_and_decode_json(builder).await
    }

    /// # Errors
    /// Network or API errors; `MissingPermissions` without the right to delete it.
    pub async fn delete_message(
        &self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Result<(), ClientError> {
        Self::send_and_discard_body(self.request(
            Method::DELETE,
            &format!("api/v1/channels/{channel_id}/messages/{message_id}"),
            true,
        )?)
        .await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn acknowledge(
        &self,
        channel_id: Snowflake,
        last_read_message_id: Snowflake,
    ) -> Result<(), ClientError> {
        let builder = self
            .request(
                Method::PUT,
                &format!("api/v1/channels/{channel_id}/read-state"),
                true,
            )?
            .json(&AcknowledgeRequest {
                last_read_message_id,
            });
        Self::send_and_discard_body(builder).await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn start_typing(&self, channel_id: Snowflake) -> Result<(), ClientError> {
        Self::send_and_discard_body(self.request(
            Method::POST,
            &format!("api/v1/channels/{channel_id}/typing"),
            true,
        )?)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_never_shows_the_token() {
        let mut client =
            RestClient::new(InstanceAddress::parse("https://chat.example.org").unwrap()).unwrap();
        client.set_token(Zeroizing::new("mlw_secret".into()));
        let printed = format!("{client:?}");
        assert!(!printed.contains("mlw_secret"), "{printed}");
        assert!(printed.contains("<redacted>"));
    }
}
