//! The instance's health, the signed-in account (presence, two-factor sign-in, profile, GIF
//! favourites) and its relationships.

use mellow_protocol::{
    ChosenStatus, CompleteTwoFactorSignInRequest, CurrentUser, EnableTwoFactorRequest, GifSettings,
    GuildSummary, HealthResponse, NameAvailability, ProveTwoFactorRequest, ReadState, Relationship,
    SendFriendRequest, SessionCreated, SetRelationshipKind, SetRelationshipRequest, Snowflake,
    StartTwoFactorEnrolmentRequest, TwoFactorEnrolment, TwoFactorRecoveryCodes, TwoFactorStatus,
    UpdatePresenceRequest, UpdateUserProfileRequest, UserProfile,
};
use reqwest::{Method, StatusCode};
use zeroize::Zeroizing;

use super::{RestClient, percent_encode};
use crate::error::ClientError;

impl RestClient {
    /// Whether the address is a Mellow instance, which version it runs and whether its database
    /// answers. Needs no session.
    ///
    /// # Errors
    /// Network errors; `UnexpectedResponse` when something other than a Mellow instance answers.
    pub async fn health(&self) -> Result<HealthResponse, ClientError> {
        let builder = self.request(Method::GET, "api/v1/health", false)?;
        let response = builder.send().await.map_err(ClientError::Network)?;
        // A `503` still carries the health body: it says the database is unavailable.
        if !matches!(
            response.status(),
            StatusCode::OK | StatusCode::SERVICE_UNAVAILABLE
        ) {
            return Err(ClientError::UnexpectedResponse(format!(
                "HTTP {} from the health probe",
                response.status().as_u16()
            )));
        }
        Self::decode_json(response).await
    }

    /// The second sign-in step (ADR-0049): the challenge's token from
    /// [`super::SignIn::TwoFactorRequired`] with an authenticator code or a recovery code. The token
    /// is kept for later calls.
    ///
    /// # Errors
    /// `Api` with `InvalidTwoFactorCode`, `Unauthorized` for an expired or used challenge,
    /// `RateLimited`…
    pub async fn complete_two_factor_sign_in(
        &mut self,
        request: &CompleteTwoFactorSignInRequest,
    ) -> Result<SessionCreated, ClientError> {
        let session: SessionCreated = self
            .post_json("api/v1/auth/login/two-factor", request, false)
            .await?;
        self.token = Some(Zeroizing::new(session.token.clone()));
        Ok(session)
    }

    /// # Errors
    /// Network or API errors.
    pub async fn two_factor_status(&self) -> Result<TwoFactorStatus, ClientError> {
        self.call(Method::GET, "api/v1/users/@me/two-factor").await
    }

    /// A new secret; two-factor sign-in is on only once [`Self::enable_two_factor`] confirms a
    /// code made from it.
    ///
    /// # Errors
    /// Network or API errors; `InvalidCredentials` for a wrong password.
    pub async fn start_two_factor_enrolment(
        &self,
        request: &StartTwoFactorEnrolmentRequest,
    ) -> Result<TwoFactorEnrolment, ClientError> {
        self.call_with_json(
            Method::POST,
            "api/v1/users/@me/two-factor/enrolment",
            request,
        )
        .await
    }

    /// Answers the ten recovery codes, shown once.
    ///
    /// # Errors
    /// Network or API errors; `InvalidTwoFactorCode` for a wrong code.
    pub async fn enable_two_factor(
        &self,
        request: &EnableTwoFactorRequest,
    ) -> Result<TwoFactorRecoveryCodes, ClientError> {
        self.call_with_json(Method::POST, "api/v1/users/@me/two-factor/enable", request)
            .await
    }

    /// # Errors
    /// Network or API errors; `InvalidCredentials` or `InvalidTwoFactorCode`.
    pub async fn disable_two_factor(
        &self,
        request: &ProveTwoFactorRequest,
    ) -> Result<(), ClientError> {
        self.call_with_json_without_answer(
            Method::POST,
            "api/v1/users/@me/two-factor/disable",
            request,
        )
        .await
    }

    /// Replaces the recovery codes; the old ones stop working.
    ///
    /// # Errors
    /// Network or API errors; `InvalidCredentials` or `InvalidTwoFactorCode`.
    pub async fn regenerate_recovery_codes(
        &self,
        request: &ProveTwoFactorRequest,
    ) -> Result<TwoFactorRecoveryCodes, ClientError> {
        self.call_with_json(
            Method::POST,
            "api/v1/users/@me/two-factor/recovery-codes",
            request,
        )
        .await
    }

    /// Whether the signed-in user could take `name` (ADR-0048).
    ///
    /// # Errors
    /// Network or API errors; `ValidationFailed` when the name breaks the naming rules.
    pub async fn name_availability(&self, name: &str) -> Result<NameAvailability, ClientError> {
        let builder = self
            .request(Method::GET, "api/v1/users/name-availability", true)?
            .query(&[("name", name)]);
        Self::send_and_decode_json(builder).await
    }

    /// Replaces both; `None` (or a blank text) clears the custom status.
    ///
    /// # Errors
    /// Network or API errors.
    pub async fn update_presence(
        &self,
        status: ChosenStatus,
        custom_status: Option<String>,
    ) -> Result<CurrentUser, ClientError> {
        self.call_with_json(
            Method::PUT,
            "api/v1/users/@me/presence",
            &UpdatePresenceRequest {
                status,
                custom_status,
            },
        )
        .await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn list_my_guilds(&self) -> Result<Vec<GuildSummary>, ClientError> {
        self.call(Method::GET, "api/v1/users/@me/guilds").await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn list_my_read_states(&self) -> Result<Vec<ReadState>, ClientError> {
        self.call(Method::GET, "api/v1/users/@me/read-states").await
    }

    /// What this app needs to call Klipy itself (ADR-0042).
    ///
    /// # Errors
    /// Network or API errors; `NotFound` when the instance offers no GIFs.
    pub async fn gif_settings(&self) -> Result<GifSettings, ClientError> {
        self.call(Method::GET, "api/v1/gifs/settings").await
    }

    /// Klipy slugs, newest first.
    ///
    /// # Errors
    /// Network or API errors.
    pub async fn list_gif_favourites(&self) -> Result<Vec<String>, ClientError> {
        self.call(Method::GET, "api/v1/users/@me/gif-favourites")
            .await
    }

    /// # Errors
    /// Network or API errors; `Conflict` past `MAXIMUM_GIF_FAVOURITES`.
    pub async fn add_gif_favourite(&self, slug: &str) -> Result<(), ClientError> {
        self.call_without_answer(
            Method::PUT,
            &format!("api/v1/users/@me/gif-favourites/{}", percent_encode(slug)),
        )
        .await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn remove_gif_favourite(&self, slug: &str) -> Result<(), ClientError> {
        self.call_without_answer(
            Method::DELETE,
            &format!("api/v1/users/@me/gif-favourites/{}", percent_encode(slug)),
        )
        .await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn my_profile(&self) -> Result<UserProfile, ClientError> {
        self.call(Method::GET, "api/v1/users/@me/profile").await
    }

    /// "About me" and links (ADR-0050).
    ///
    /// # Errors
    /// Network or API errors; `NotFound` for someone the reader shares nothing with.
    pub async fn user_profile(&self, user_id: Snowflake) -> Result<UserProfile, ClientError> {
        self.call(Method::GET, &format!("api/v1/users/{user_id}/profile"))
            .await
    }

    /// Replaces both the text and the links.
    ///
    /// # Errors
    /// Network or API errors; `ValidationFailed` for a text or link out of bounds.
    pub async fn update_my_profile(
        &self,
        request: &UpdateUserProfileRequest,
    ) -> Result<UserProfile, ClientError> {
        self.call_with_json(Method::PUT, "api/v1/users/@me/profile", request)
            .await
    }

    /// # Errors
    /// Network or API errors.
    pub async fn list_relationships(&self) -> Result<Vec<Relationship>, ClientError> {
        self.call(Method::GET, "api/v1/users/@me/relationships")
            .await
    }

    /// Asks someone to be friends, by name or friend code (ADR-0048), or accepts their request.
    ///
    /// # Errors
    /// Network or API errors; `NotFound` for a missing account or a block.
    pub async fn send_friend_request(
        &self,
        request: &SendFriendRequest,
    ) -> Result<Relationship, ClientError> {
        self.call_with_json(Method::POST, "api/v1/users/@me/relationships", request)
            .await
    }

    /// `Friend` accepts their request; `Blocked` blocks them.
    ///
    /// # Errors
    /// Network or API errors.
    pub async fn set_relationship(
        &self,
        user_id: Snowflake,
        kind: SetRelationshipKind,
    ) -> Result<Relationship, ClientError> {
        self.call_with_json(
            Method::PUT,
            &format!("api/v1/users/@me/relationships/{user_id}"),
            &SetRelationshipRequest { kind },
        )
        .await
    }

    /// Removes a friend, refuses or cancels a request, or unblocks.
    ///
    /// # Errors
    /// Network or API errors.
    pub async fn remove_relationship(&self, user_id: Snowflake) -> Result<(), ClientError> {
        self.call_without_answer(
            Method::DELETE,
            &format!("api/v1/users/@me/relationships/{user_id}"),
        )
        .await
    }
}
