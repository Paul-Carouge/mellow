//! Typed calls to the REST API (`/api/v1`). Routes are grouped by area in the submodules; files
//! (downloads and uploads) are in `files`.

mod accounts;
mod files;
mod guilds;
mod messages;

use std::time::Duration;

use mellow_protocol::{
    AcknowledgeRequest, ApiErrorBody, AuditLogEntry, AuditLogQuery, Ban, ChangePasswordRequest,
    Channel, CreateBanRequest, CreateChannelRequest, CreateGuildRequest, CreateInviteRequest,
    CreateRoleRequest, CurrentUser, DirectMessageChannel, EditMessageRequest, Guild, Invite,
    LoginRequest, Member, Message, MessageHistoryQuery, OpenDirectMessageRequest, RegisterRequest,
    ReorderRolesRequest, Role, SendMessageRequest, SessionCreated, SetPermissionOverwriteRequest,
    Snowflake, TwoFactorChallenge, UpdateChannelRequest, UpdateCurrentUserRequest,
    UpdateRoleRequest,
};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use zeroize::Zeroizing;

pub use self::files::{Download, DownloadableFile, DownloadedFile, UploadProgress};
use crate::error::ClientError;
use crate::instance::InstanceAddress;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Far above any honest answer (a 1 000-member list or a page of messages); bounds what a
/// hostile instance can make the client buffer.
const MAXIMUM_RESPONSE_BODY_BYTES: usize = 8 * 1024 * 1024;
/// The longest `Retry-After` honoured with one automatic retry; a longer wait is the caller's call.
const LONGEST_HONOURED_RETRY_AFTER: Duration = Duration::from_secs(10);

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
        Self::decode_json(response).await
    }

    async fn decode_json<T: DeserializeOwned>(
        response: reqwest::Response,
    ) -> Result<T, ClientError> {
        let body = Self::bounded_body(response).await?;
        serde_json::from_slice(&body)
            .map_err(|error| ClientError::UnexpectedResponse(format!("{:?}", error.classify())))
    }

    /// An authenticated call without a body, answered with JSON.
    async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
    ) -> Result<T, ClientError> {
        Self::send_and_decode_json(self.request(method, path, true)?).await
    }

    /// An authenticated call with a JSON body, answered with JSON.
    async fn call_with_json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T, ClientError> {
        Self::send_and_decode_json(self.request(method, path, true)?.json(body)).await
    }

    /// An authenticated call without a body, answered with `204 No Content`.
    async fn call_without_answer(&self, method: Method, path: &str) -> Result<(), ClientError> {
        Self::send_and_discard_body(self.request(method, path, true)?).await
    }

    /// An authenticated call with a JSON body, answered with `204 No Content`.
    async fn call_with_json_without_answer(
        &self,
        method: Method,
        path: &str,
        body: &impl Serialize,
    ) -> Result<(), ClientError> {
        Self::send_and_discard_body(self.request(method, path, true)?.json(body)).await
    }

    async fn send_and_discard_body(builder: RequestBuilder) -> Result<(), ClientError> {
        Self::response_or_api_error(builder).await.map(|_| ())
    }

    async fn bounded_body(response: reqwest::Response) -> Result<Vec<u8>, ClientError> {
        Self::body_within(response, MAXIMUM_RESPONSE_BODY_BYTES)
            .await?
            .ok_or_else(|| ClientError::UnexpectedResponse("response body too large".into()))
    }

    /// The whole body, or `None` as soon as it is known to exceed `maximum_bytes`.
    async fn body_within(
        mut response: reqwest::Response,
        maximum_bytes: usize,
    ) -> Result<Option<Vec<u8>>, ClientError> {
        if response
            .content_length()
            .is_some_and(|length| length > maximum_bytes as u64)
        {
            return Ok(None);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(ClientError::Network)? {
            if body.len() + chunk.len() > maximum_bytes {
                return Ok(None);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Some(body))
    }

    /// Sends the request; a `429` or `503` that asks to come back within
    /// `LONGEST_HONOURED_RETRY_AFTER` is retried once after that delay. Such refusals come before
    /// the instance acts, so even a `POST` is safe to repeat. Streamed bodies (uploads) cannot be
    /// replayed and are never retried.
    async fn send_retrying_when_asked(
        builder: RequestBuilder,
    ) -> Result<reqwest::Response, ClientError> {
        let replay = builder.try_clone();
        let response = builder.send().await.map_err(ClientError::Network)?;
        match (replay, honoured_retry_delay(&response)) {
            (Some(replay), Some(delay)) => {
                drop(response);
                tokio::time::sleep(delay).await;
                replay.send().await.map_err(ClientError::Network)
            }
            _ => Ok(response),
        }
    }

    async fn response_or_api_error(
        builder: RequestBuilder,
    ) -> Result<reqwest::Response, ClientError> {
        let response = Self::send_retrying_when_asked(builder).await?;
        Self::api_error_unless_success(response).await
    }

    async fn api_error_unless_success(
        response: reqwest::Response,
    ) -> Result<reqwest::Response, ClientError> {
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
    /// `Api` with `InvalidCredentials`, `AccountDisabled`, `RateLimited`…; `TwoFactorRequired`
    /// when the password was right but the account has two-factor sign-in on (ADR-0049): finish
    /// with [`Self::complete_two_factor_sign_in`].
    pub async fn login(&mut self, request: &LoginRequest) -> Result<SessionCreated, ClientError> {
        let builder = self
            .request(Method::POST, "api/v1/auth/login", false)?
            .json(request);
        let response = Self::response_or_api_error(builder).await?;
        if response.status() == StatusCode::ACCEPTED {
            let challenge: TwoFactorChallenge = Self::decode_json(response).await?;
            return Err(ClientError::TwoFactorRequired(challenge));
        }
        let session: SessionCreated = Self::decode_json(response).await?;
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

    /// Renames the signed-in user; absent fields stay unchanged.
    ///
    /// # Errors
    /// Network or API errors; `UsernameTaken` when another account has the username.
    pub async fn update_current_user(
        &self,
        request: &UpdateCurrentUserRequest,
    ) -> Result<CurrentUser, ClientError> {
        let builder = self
            .request(Method::PATCH, "api/v1/users/@me", true)?
            .json(request);
        Self::send_and_decode_json(builder).await
    }

    /// Signs out every other session of the user; this one stays signed in.
    ///
    /// # Errors
    /// Network or API errors; `InvalidCredentials` for a wrong current password.
    pub async fn change_password(
        &self,
        request: &ChangePasswordRequest,
    ) -> Result<(), ClientError> {
        let builder = self
            .request(Method::POST, "api/v1/users/@me/password", true)?
            .json(request);
        Self::send_and_discard_body(builder).await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn create_guild(&self, name: &str) -> Result<Guild, ClientError> {
        self.post_json(
            "api/v1/guilds",
            &CreateGuildRequest {
                name: name.to_owned(),
                initial_categories: None,
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
        if code.is_empty()
            || !code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
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

    /// The one-to-one conversation with that user, opened if needed.
    ///
    /// # Errors
    /// Network or API errors; `NotFound` unless the user shares a guild with us.
    pub async fn open_direct_message(
        &self,
        recipient_id: Snowflake,
    ) -> Result<DirectMessageChannel, ClientError> {
        self.post_json(
            "api/v1/users/@me/channels",
            &OpenDirectMessageRequest { recipient_id },
            true,
        )
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

/// The delay a `429 Too Many Requests` or `503 Service Unavailable` asks for in whole seconds,
/// when it is short enough to wait for.
fn honoured_retry_delay(response: &reqwest::Response) -> Option<Duration> {
    if !matches!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
    ) {
        return None;
    }
    let seconds: u64 = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let delay = Duration::from_secs(seconds);
    (delay <= LONGEST_HONOURED_RETRY_AFTER).then_some(delay)
}

/// Percent-encodes everything but unreserved ASCII (RFC 3986), as `encodeURIComponent` does.
fn percent_encode(text: &str) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            // Writing to a String cannot fail.
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
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

    /// A loopback server answering each connection with the next scripted answer; returns its
    /// address and the number of requests it has read.
    async fn scripted_server(
        answers: Vec<&'static str>,
    ) -> (
        InstanceAddress,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address =
            InstanceAddress::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let request_count = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&request_count);
        tokio::spawn(async move {
            for answer in answers {
                let (mut connection, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = connection.read(&mut buffer).await.unwrap();
                    request.extend_from_slice(&buffer[..read]);
                }
                counter.fetch_add(1, Ordering::SeqCst);
                connection.write_all(answer.as_bytes()).await.unwrap();
                connection.shutdown().await.unwrap();
            }
        });
        (address, request_count)
    }

    const HEALTHY: &str = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{\"status\":\"ok\",\"version\":\"0.48.0\"}";

    #[tokio::test]
    async fn a_busy_answer_with_a_short_retry_after_is_retried_once() {
        let (address, request_count) = scripted_server(vec![
            "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 0\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            HEALTHY,
        ])
        .await;
        let health = RestClient::new(address).unwrap().health().await.unwrap();
        assert_eq!(health.version, "0.48.0");
        assert_eq!(request_count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_refusal_without_retry_after_or_with_a_long_one_is_not_retried() {
        for refusal in [
            "HTTP/1.1 429 Too Many Requests\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 3600\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        ] {
            let (address, request_count) = scripted_server(vec![refusal, HEALTHY]).await;
            let error = RestClient::new(address)
                .unwrap()
                .health()
                .await
                .unwrap_err();
            assert!(
                matches!(error, ClientError::UnexpectedResponse(_)),
                "{error:?}"
            );
            assert_eq!(request_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        }
    }
}
