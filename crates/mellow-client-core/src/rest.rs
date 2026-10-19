//! Typed calls to the REST API (`/api/v1`).

use std::time::Duration;

use mellow_protocol::{
    AcknowledgeRequest, ApiErrorBody, Channel, CreateChannelRequest, CreateGuildRequest,
    CreateInviteRequest, CurrentUser, Guild, Invite, LoginRequest, Member, Message,
    MessageHistoryQuery, RegisterRequest, SendMessageRequest, SessionCreated, Snowflake,
};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use zeroize::Zeroizing;

use crate::error::ClientError;
use crate::instance::InstanceAddress;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct RestClient {
    http: reqwest::Client,
    instance: InstanceAddress,
    token: Option<Zeroizing<String>>,
}

impl RestClient {
    /// # Errors
    /// Fails only if the TLS stack cannot initialise.
    pub fn new(instance: InstanceAddress) -> Result<Self, ClientError> {
        let http = reqwest::Client::builder()
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

    async fn send<T: DeserializeOwned>(builder: RequestBuilder) -> Result<T, ClientError> {
        let response = Self::checked(builder).await?;
        response
            .json()
            .await
            .map_err(|error| ClientError::UnexpectedResponse(error.to_string()))
    }

    async fn send_without_body(builder: RequestBuilder) -> Result<(), ClientError> {
        Self::checked(builder).await.map(|_| ())
    }

    async fn checked(builder: RequestBuilder) -> Result<reqwest::Response, ClientError> {
        let response = builder.send().await.map_err(ClientError::Network)?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        match response.json::<ApiErrorBody>().await {
            Ok(body) => Err(ClientError::Api {
                status: status.as_u16(),
                code: body.code,
                message: body.message,
            }),
            Err(_) => Err(ClientError::UnexpectedResponse(format!(
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
        Self::send(self.request(Method::POST, path, authenticated)?.json(body)).await
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
            Self::send_without_body(self.request(Method::POST, "api/v1/auth/logout", true)?).await;
        self.token = None;
        result
    }

    /// # Errors
    /// Network or API errors.
    pub async fn current_user(&self) -> Result<CurrentUser, ClientError> {
        Self::send(self.request(Method::GET, "api/v1/users/@me", true)?).await
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
        Self::send(self.request(Method::POST, &format!("api/v1/invites/{code}"), true)?).await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn list_members(&self, guild_id: Snowflake) -> Result<Vec<Member>, ClientError> {
        Self::send(self.request(
            Method::GET,
            &format!("api/v1/guilds/{guild_id}/members"),
            true,
        )?)
        .await
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
        Self::send(builder).await
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
        Self::send_without_body(builder).await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn start_typing(&self, channel_id: Snowflake) -> Result<(), ClientError> {
        Self::send_without_body(self.request(
            Method::POST,
            &format!("api/v1/channels/{channel_id}/typing"),
            true,
        )?)
        .await
    }
}
