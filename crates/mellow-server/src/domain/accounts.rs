//! Account use cases: register, create by the administrator, log in, authenticate, log out,
//! rename, change the password, pick a status.

use std::time::Duration;

use crate::storage::Database;
use mellow_protocol::{
    ChangePasswordRequest, ChosenStatus, CurrentUser, GatewayEvent, LoginRequest, RegisterRequest,
    SessionCreated, Snowflake, UpdateCurrentUserRequest, UpdatePresenceRequest,
};
use zeroize::Zeroizing;

use crate::domain::account_names::{
    account_name_key, format_friend_code, generate_friend_code, normalize_account_name,
};
use crate::domain::account_rules::{
    AccountRuleViolation, RegistrationMode, normalize_custom_status, normalize_device_name,
    normalize_email, normalize_login, validate_password_length, validate_password_maximum_length,
};
use crate::domain::clock::now_unix_milliseconds;
use crate::domain::password::{PasswordHashError, hash_password, verify_password};
use crate::domain::session_token::{
    SessionTokenSha256, generate_session_token, session_token_sha256,
};
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::events::ChosenPresence;
use crate::storage::account_names::{is_name_key_taken_by_another_account, rename_account};
use crate::storage::accounts::{
    AccountStorageError, NewSession, NewUser, StoredUser, delete_expired_sessions, delete_session,
    find_session_owner, find_user_by_login, find_user_presence, insert_session, insert_user,
    record_session_use, replace_password_and_delete_other_sessions, update_user_presence,
};
use crate::storage::guilds::{GuildStorageError, list_user_guilds};
use crate::storage::two_factor::is_two_factor_enabled;

/// A session lasts 90 days, then the user signs in again.
const SESSION_LIFETIME: Duration = Duration::from_hours(90 * 24);
const EXPIRED_SESSION_PURGE_INTERVAL: Duration = Duration::from_hours(1);
/// Sessions are deleted this long after they expire, so a system clock that jumped forward
/// (refused sessions until it is fixed) does not also wipe every session for good.
const EXPIRED_SESSION_PURGE_GRACE: Duration = Duration::from_hours(24);
/// `sessions.last_seen_at` is precise to this much, for a future list of signed-in devices.
const SESSION_USE_RECORD_INTERVAL: Duration = Duration::from_hours(1);
/// Random 40-bit codes almost never collide; a few draws make a failure practically impossible.
const FRIEND_CODE_DRAWS_PER_NEW_ACCOUNT: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error(transparent)]
    InvalidInput(#[from] AccountRuleViolation),
    #[error("this name is already taken")]
    UsernameTaken,
    #[error("this email address is already used")]
    EmailTaken,
    #[error("registration is closed on this instance")]
    RegistrationClosed,
    #[error("wrong login or password")]
    InvalidCredentials,
    #[error("this account is disabled")]
    AccountDisabled,
    #[error("missing, invalid or expired session")]
    Unauthorized,
    #[error("the server is busy; try again shortly")]
    ServerBusy,
    /// The password was right, but the account has two-factor sign-in on (ADR-0049): no session
    /// until a code is checked. Refused like a wrong login by any caller that does not handle it.
    #[error("a second factor is required")]
    SecondFactorRequired(Box<VerifiedLogin>),
    #[error("internal error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl From<AccountStorageError> for AccountError {
    fn from(error: AccountStorageError) -> Self {
        match error {
            AccountStorageError::UsernameTaken => Self::UsernameTaken,
            AccountStorageError::EmailTaken => Self::EmailTaken,
            other => Self::Internal(Box::new(other)),
        }
    }
}

impl From<PasswordHashError> for AccountError {
    fn from(error: PasswordHashError) -> Self {
        match error {
            PasswordHashError::Busy => Self::ServerBusy,
            other => Self::Internal(Box::new(other)),
        }
    }
}

impl From<GuildStorageError> for AccountError {
    fn from(error: GuildStorageError) -> Self {
        Self::Internal(Box::new(error))
    }
}

impl From<mellow_protocol::SnowflakeError> for AccountError {
    fn from(error: mellow_protocol::SnowflakeError) -> Self {
        Self::Internal(Box::new(error))
    }
}

impl From<getrandom::Error> for AccountError {
    fn from(error: getrandom::Error) -> Self {
        Self::Internal(Box::new(error))
    }
}

/// A sign-in whose password was checked, waiting for its second factor.
#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedLogin {
    pub(crate) user_id: Snowflake,
    /// The session opens only while the account still has this password.
    pub(crate) verified_password_hash: String,
    pub(crate) device_name: String,
}

/// The password hash is never printed.
impl std::fmt::Debug for VerifiedLogin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedLogin")
            .field("user_id", &self.user_id)
            .field("device_name", &self.device_name)
            .finish_non_exhaustive()
    }
}

/// Fields of a new account, validated by `Accounts::create_account`.
#[derive(Clone, PartialEq, Eq)]
pub struct NewAccount {
    pub username: String,
    pub display_name: Option<String>,
    pub email: String,
    pub password: Zeroizing<String>,
    pub is_instance_administrator: bool,
}

/// The password is never printed (logs must not contain secrets).
impl std::fmt::Debug for NewAccount {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NewAccount")
            .field("username", &self.username)
            .field("display_name", &self.display_name)
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .field("is_instance_administrator", &self.is_instance_administrator)
            .finish()
    }
}

#[derive(Debug)]
pub struct Accounts {
    database: Database,
    /// Shared with the other use cases so ids are unique across the whole process.
    ids: std::sync::Arc<SnowflakeGenerator>,
    registration_mode: RegistrationMode,
    /// Logout ends the token's live gateway sessions at once.
    events: std::sync::Arc<crate::events::EventHub>,
}

impl Accounts {
    #[must_use]
    pub(crate) fn new(
        database: Database,
        registration_mode: RegistrationMode,
        ids: std::sync::Arc<SnowflakeGenerator>,
        events: std::sync::Arc<crate::events::EventHub>,
    ) -> Self {
        Self {
            database,
            ids,
            registration_mode,
            events,
        }
    }

    /// Self-service sign-up, only when registration is open. Returns a signed-in session.
    ///
    /// # Errors
    /// `RegistrationClosed`, invalid input, or a taken username or email.
    pub async fn register(
        &self,
        request: RegisterRequest,
        now_unix_milliseconds: u64,
    ) -> Result<SessionCreated, AccountError> {
        if self.registration_mode != RegistrationMode::Open {
            return Err(AccountError::RegistrationClosed);
        }
        let device_name = normalize_device_name(&request.device_name)?;
        let user = self
            .insert_new_account(
                NewAccount {
                    username: request.username,
                    display_name: request.display_name,
                    email: request.email,
                    password: Zeroizing::new(request.password),
                    is_instance_administrator: false,
                },
                now_unix_milliseconds,
            )
            .await?;
        self.open_session(user, device_name, now_unix_milliseconds)
            .await
    }

    /// Creates an account regardless of the registration mode (`mellow-server admin create-user`).
    ///
    /// # Errors
    /// Invalid input, or a taken username or email.
    pub async fn create_account(
        &self,
        account: NewAccount,
        now_unix_milliseconds: u64,
    ) -> Result<CurrentUser, AccountError> {
        Ok(current_user(
            self.insert_new_account(account, now_unix_milliseconds)
                .await?,
        ))
    }

    async fn insert_new_account(
        &self,
        account: NewAccount,
        now_unix_milliseconds: u64,
    ) -> Result<StoredUser, AccountError> {
        let name =
            normalize_account_name(account.display_name.as_deref().unwrap_or(&account.username))?;
        let email = normalize_email(&account.email)?;
        validate_password_length(&account.password)?;
        let password_hash = hash_password(account.password).await?;
        let mut new_user = NewUser {
            id: self.ids.next_id(now_unix_milliseconds)?,
            name_key: account_name_key(&name),
            name,
            friend_code: generate_friend_code()?,
            email,
            password_hash,
            is_instance_administrator: account.is_instance_administrator,
            created_at_unix_milliseconds: now_unix_milliseconds,
        };
        let mut draws_left = FRIEND_CODE_DRAWS_PER_NEW_ACCOUNT;
        loop {
            match insert_user(&self.database, &new_user).await {
                Err(AccountStorageError::FriendCodeTaken) if draws_left > 1 => {
                    draws_left -= 1;
                    new_user.friend_code = generate_friend_code()?;
                }
                result => break result?,
            }
        }
        Ok(StoredUser {
            id: new_user.id,
            username: new_user.name.clone(),
            display_name: new_user.name,
            email: new_user.email,
            password_hash: new_user.password_hash,
            is_instance_administrator: new_user.is_instance_administrator,
            is_disabled: false,
            avatar: None,
            banner: None,
            status: ChosenStatus::Online,
            custom_status: None,
            friend_code: Some(new_user.friend_code),
        })
    }

    /// # Errors
    /// `InvalidCredentials` for an unknown login or a wrong password (indistinguishable, and
    /// equally slow); `AccountDisabled` only once the password is proven right;
    /// `SecondFactorRequired` when the account has two-factor sign-in on.
    pub async fn login(
        &self,
        request: LoginRequest,
        now_unix_milliseconds: u64,
    ) -> Result<SessionCreated, AccountError> {
        let device_name = normalize_device_name(&request.device_name)?;
        let login = normalize_login(&request.login)?;
        let password = Zeroizing::new(request.password);
        // Bounds Argon2 input; only the maximum, so older passwords keep working.
        validate_password_maximum_length(&password)?;
        let user = find_user_by_login(&self.database, &account_name_key(&login), &login).await?;
        let stored_hash = user.as_ref().map(|user| user.password_hash.clone());
        let is_password_correct = verify_password(password, stored_hash).await?;
        let user = match user {
            Some(user) if is_password_correct => user,
            _ => return Err(AccountError::InvalidCredentials),
        };
        if user.is_disabled {
            return Err(AccountError::AccountDisabled);
        }
        if is_two_factor_enabled(&self.database, user.id).await? {
            return Err(AccountError::SecondFactorRequired(Box::new(
                VerifiedLogin {
                    user_id: user.id,
                    verified_password_hash: user.password_hash,
                    device_name,
                },
            )));
        }
        self.open_session(user, device_name, now_unix_milliseconds)
            .await
    }

    /// Resolves a bearer token to its signed-in user.
    ///
    /// # Errors
    /// `Unauthorized` for a malformed, unknown or expired token; `AccountDisabled` for a disabled account.
    pub async fn authenticate(
        &self,
        token: &str,
        now_unix_milliseconds: u64,
    ) -> Result<CurrentUser, AccountError> {
        let token_sha256 = session_token_sha256(token).ok_or(AccountError::Unauthorized)?;
        let owner = find_session_owner(&self.database, &token_sha256, now_unix_milliseconds)
            .await?
            .ok_or(AccountError::Unauthorized)?;
        if owner.user.is_disabled {
            return Err(AccountError::AccountDisabled);
        }
        // Best effort: a figure precise to the hour must never deny access, e.g. on a full disk.
        if should_record_session_use(owner.last_seen_at_unix_milliseconds, now_unix_milliseconds)
            && let Err(error) =
                record_session_use(&self.database, &token_sha256, now_unix_milliseconds).await
        {
            tracing::warn!(%error, "cannot record session use");
        }
        Ok(current_user(owner.user))
    }

    /// Ends the session the token belongs to. Idempotent.
    ///
    /// # Errors
    /// `Unauthorized` for a malformed token; database errors.
    pub async fn logout(&self, token: &str) -> Result<(), AccountError> {
        let token_sha256 = session_token_sha256(token).ok_or(AccountError::Unauthorized)?;
        delete_session(&self.database, &token_sha256).await?;
        self.events.disconnect_token(&token_sha256);
        Ok(())
    }

    /// Whether `user_id` could take `name` now: their own name, in another case, stays available.
    /// Never says who holds a taken name.
    ///
    /// # Errors
    /// Invalid input for a name breaking the rules; database errors.
    pub async fn is_name_available(
        &self,
        user_id: Snowflake,
        name: &str,
    ) -> Result<bool, AccountError> {
        let name = normalize_account_name(name)?;
        Ok(
            !is_name_key_taken_by_another_account(
                &self.database,
                &account_name_key(&name),
                user_id,
            )
            .await?,
        )
    }

    /// Renames the signed-in user and tells everyone who can see them (ADR-0048). The old name is
    /// free for anyone at once; the friend code keeps pointing to the account.
    ///
    /// # Errors
    /// `Unauthorized` for an invalid token, invalid input, or a name another account holds.
    pub async fn update_current_user(
        &self,
        token: &str,
        request: UpdateCurrentUserRequest,
        now_unix_milliseconds: u64,
    ) -> Result<CurrentUser, AccountError> {
        let name = request
            .display_name
            .or(request.username)
            .as_deref()
            .map(normalize_account_name)
            .transpose()?;
        let user = self
            .verified_session_owner(token, now_unix_milliseconds)
            .await?;
        let Some(name) = name.filter(|name| *name != user.display_name) else {
            return Ok(current_user(user));
        };
        if !rename_account(&self.database, user.id, &name, &account_name_key(&name)).await? {
            return Err(AccountError::Unauthorized);
        }
        let updated = current_user(StoredUser {
            username: name.clone(),
            display_name: name,
            ..user
        });
        let guild_ids: Vec<_> = list_user_guilds(&self.database, updated.id)
            .await?
            .into_iter()
            .map(|guild| guild.id)
            .collect();
        self.events.publish_to_user_and_guilds(
            updated.id,
            &guild_ids,
            &GatewayEvent::UserUpdate(crate::domain::profile_images::public_user(&updated)),
        );
        Ok(updated)
    }

    /// Stores the signed-in user's chosen status and custom status, and tells their guilds what
    /// changed for them.
    ///
    /// # Errors
    /// `Unauthorized` for an invalid token, invalid input for a custom status breaking the rules.
    pub async fn update_presence(
        &self,
        token: &str,
        request: UpdatePresenceRequest,
        now_unix_milliseconds: u64,
    ) -> Result<CurrentUser, AccountError> {
        let custom_status = request
            .custom_status
            .as_deref()
            .map(normalize_custom_status)
            .transpose()?
            .flatten();
        let user = self
            .verified_session_owner(token, now_unix_milliseconds)
            .await?;
        let version = update_user_presence(
            &self.database,
            user.id,
            request.status,
            custom_status.as_deref(),
        )
        .await?
        .ok_or(AccountError::Unauthorized)?;
        self.events.set_presence(
            user.id,
            ChosenPresence {
                status: request.status,
                custom_status: custom_status.clone(),
                version,
            },
        );
        Ok(current_user(StoredUser {
            status: request.status,
            custom_status,
            ..user
        }))
    }

    /// Gives the hub the user's stored presence, for a session just registered there. Read after
    /// the registration, so a pick made meanwhile is either in this read or applied by its own
    /// request, and the versions keep the newer one.
    ///
    /// # Errors
    /// `Unauthorized` for an unknown user; database errors.
    pub async fn load_presence_into_events(&self, user_id: Snowflake) -> Result<(), AccountError> {
        let stored = find_user_presence(&self.database, user_id)
            .await?
            .ok_or(AccountError::Unauthorized)?;
        self.events.set_presence(
            user_id,
            ChosenPresence {
                status: stored.status,
                custom_status: stored.custom_status,
                version: stored.version,
            },
        );
        Ok(())
    }

    /// Replaces the password once the current one is proven, and signs out every other session of
    /// the user; the session behind `token` stays valid.
    ///
    /// # Errors
    /// `Unauthorized` for an invalid token, `InvalidCredentials` for a wrong current password (or
    /// one changed meanwhile), invalid input for a new password of the wrong length.
    pub async fn change_password(
        &self,
        token: &str,
        request: ChangePasswordRequest,
        now_unix_milliseconds: u64,
    ) -> Result<(), AccountError> {
        let current_password = Zeroizing::new(request.current_password);
        let new_password = Zeroizing::new(request.new_password);
        validate_password_maximum_length(&current_password)?;
        validate_password_length(&new_password)?;
        let token_sha256 = session_token_sha256(token).ok_or(AccountError::Unauthorized)?;
        let user = self
            .verified_session_owner(token, now_unix_milliseconds)
            .await?;
        if !verify_password(current_password, Some(user.password_hash.clone())).await? {
            return Err(AccountError::InvalidCredentials);
        }
        let new_password_hash = hash_password(new_password).await?;
        if !replace_password_and_delete_other_sessions(
            &self.database,
            user.id,
            &user.password_hash,
            &new_password_hash,
            &token_sha256,
            now_unix_milliseconds,
        )
        .await?
        {
            return Err(AccountError::InvalidCredentials);
        }
        self.events
            .disconnect_user_except_token(user.id, &token_sha256);
        Ok(())
    }

    /// Ends the live gateway sessions of every session token of the user but `kept`, once their
    /// sessions were deleted.
    pub(crate) fn disconnect_user_except_token(
        &self,
        user_id: Snowflake,
        kept: &SessionTokenSha256,
    ) {
        self.events.disconnect_user_except_token(user_id, kept);
    }

    /// The enabled user behind an unexpired session, with their password hash.
    pub(crate) async fn verified_session_owner(
        &self,
        token: &str,
        now_unix_milliseconds: u64,
    ) -> Result<StoredUser, AccountError> {
        let token_sha256 = session_token_sha256(token).ok_or(AccountError::Unauthorized)?;
        let owner = find_session_owner(&self.database, &token_sha256, now_unix_milliseconds)
            .await?
            .ok_or(AccountError::Unauthorized)?;
        if owner.user.is_disabled {
            return Err(AccountError::AccountDisabled);
        }
        Ok(owner.user)
    }

    /// Opens a session for a user whose credentials were checked; refused if their password
    /// changed or the account was disabled since.
    pub(crate) async fn open_session(
        &self,
        user: StoredUser,
        device_name: String,
        now_unix_milliseconds: u64,
    ) -> Result<SessionCreated, AccountError> {
        let token = generate_session_token()?;
        let token_sha256 = session_token_sha256(&token).ok_or_else(|| {
            AccountError::Internal("a generated session token failed its own format check".into())
        })?;
        let session_lifetime_milliseconds =
            u64::try_from(SESSION_LIFETIME.as_millis()).unwrap_or(u64::MAX);
        let is_inserted = insert_session(
            &self.database,
            &NewSession {
                id: self.ids.next_id(now_unix_milliseconds)?,
                user_id: user.id,
                token_sha256,
                device_name,
                created_at_unix_milliseconds: now_unix_milliseconds,
                expires_at_unix_milliseconds: now_unix_milliseconds
                    .saturating_add(session_lifetime_milliseconds),
            },
            &user.password_hash,
        )
        .await?;
        // The password changed or the account was disabled since it was verified.
        if !is_inserted {
            return Err(AccountError::InvalidCredentials);
        }
        Ok(SessionCreated {
            token: token.to_string(),
            user: current_user(user),
        })
    }
}

/// Deletes sessions expired for a day now, then every hour, until the future is dropped. Expired sessions
/// are already refused; this only keeps the table from growing with every forgotten device.
pub async fn purge_expired_sessions_periodically(database: Database) {
    let mut purge_interval = tokio::time::interval(EXPIRED_SESSION_PURGE_INTERVAL);
    purge_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        purge_interval.tick().await;
        let grace_milliseconds =
            u64::try_from(EXPIRED_SESSION_PURGE_GRACE.as_millis()).unwrap_or(u64::MAX);
        let expired_before = now_unix_milliseconds().saturating_sub(grace_milliseconds);
        match delete_expired_sessions(&database, expired_before).await {
            Ok(0) => {}
            Ok(deleted_sessions) => tracing::info!(deleted_sessions, "expired sessions purged"),
            Err(error) => tracing::warn!(%error, "cannot purge expired sessions"),
        }
    }
}

/// Once per interval: every request writing would queue behind message sends on SQLite's lock.
fn should_record_session_use(
    last_seen_at_unix_milliseconds: u64,
    now_unix_milliseconds: u64,
) -> bool {
    let interval_milliseconds =
        u64::try_from(SESSION_USE_RECORD_INTERVAL.as_millis()).unwrap_or(u64::MAX);
    now_unix_milliseconds.saturating_sub(last_seen_at_unix_milliseconds) >= interval_milliseconds
}

pub(crate) fn current_user(user: StoredUser) -> CurrentUser {
    CurrentUser {
        id: user.id,
        username: user.username,
        display_name: user.display_name,
        email: user.email,
        is_instance_administrator: user.is_instance_administrator,
        avatar: user.avatar,
        banner: user.banner,
        status: user.status,
        custom_status: user.custom_status,
        friend_code: user.friend_code.map(format_friend_code),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use mellow_protocol::{MELLOW_EPOCH_UNIX_MILLISECONDS, PublicUser, Snowflake};

    use super::*;
    use crate::storage::{DatabaseKey, migrate_with_backup, open_encrypted_database};

    pub(crate) const NOW: u64 = MELLOW_EPOCH_UNIX_MILLISECONDS + 86_400_000;
    pub(crate) const PASSWORD: &str = "correct horse battery";

    /// A migrated database in a temporary directory; keep the directory alive while using the database.
    pub(crate) async fn migrated_database() -> (tempfile::TempDir, Database) {
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(
            &directory.path().join("mellow.db"),
            &DatabaseKey::from_bytes([3; 32]),
        )
        .await
        .unwrap();
        migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        (directory, database)
    }

    pub(crate) fn register_request(username: &str, email: &str) -> RegisterRequest {
        RegisterRequest {
            username: username.into(),
            display_name: None,
            email: email.into(),
            password: PASSWORD.into(),
            device_name: "Test device".into(),
        }
    }

    fn login_request(login: &str, password: &str) -> LoginRequest {
        LoginRequest {
            login: login.into(),
            password: password.into(),
            device_name: "Test device".into(),
        }
    }

    #[tokio::test]
    async fn register_then_login_by_username_or_email_and_authenticate() {
        let (_directory, database) = migrated_database().await;
        let accounts = Accounts::new(
            database.clone(),
            RegistrationMode::Open,
            std::sync::Arc::default(),
            std::sync::Arc::default(),
        );

        let registered = accounts
            .register(register_request("aurore", "Aurore@Example.org"), NOW)
            .await
            .unwrap();
        assert_eq!(registered.user.display_name, "aurore");
        assert_eq!(registered.user.email, "Aurore@example.org");
        assert_eq!(
            accounts.authenticate(&registered.token, NOW).await.unwrap(),
            registered.user
        );

        for login in ["aurore", "AURORE", "aurore@EXAMPLE.org"] {
            let session = accounts
                .login(login_request(login, PASSWORD), NOW)
                .await
                .unwrap();
            assert_eq!(session.user.id, registered.user.id, "{login}");
        }
        database.close().await;
    }

    #[tokio::test]
    async fn wrong_password_and_unknown_user_get_the_same_error() {
        let (_directory, database) = migrated_database().await;
        let accounts = Accounts::new(
            database.clone(),
            RegistrationMode::Open,
            std::sync::Arc::default(),
            std::sync::Arc::default(),
        );
        accounts
            .register(register_request("aurore", "aurore@example.org"), NOW)
            .await
            .unwrap();

        let wrong_password = accounts
            .login(login_request("aurore", "not the password"), NOW)
            .await;
        let unknown_user = accounts.login(login_request("nobody", PASSWORD), NOW).await;
        database.close().await;
        assert!(matches!(
            wrong_password,
            Err(AccountError::InvalidCredentials)
        ));
        assert!(matches!(
            unknown_user,
            Err(AccountError::InvalidCredentials)
        ));
    }

    #[tokio::test]
    async fn duplicates_closed_registration_and_invalid_input_are_rejected() {
        let (_directory, database) = migrated_database().await;
        let open = Accounts::new(
            database.clone(),
            RegistrationMode::Open,
            std::sync::Arc::default(),
            std::sync::Arc::default(),
        );
        open.register(register_request("aurore", "aurore@example.org"), NOW)
            .await
            .unwrap();

        let same_email = open
            .register(register_request("basile", "AURORE@example.org"), NOW)
            .await;
        let same_username = open
            .register(register_request("aurore", "other@example.org"), NOW)
            .await;
        let bad_username = open
            .register(register_request("Aurore!", "x@example.org"), NOW)
            .await;
        let closed = Accounts::new(
            database.clone(),
            RegistrationMode::Closed,
            std::sync::Arc::default(),
            std::sync::Arc::default(),
        )
        .register(register_request("chloe", "chloe@example.org"), NOW)
        .await;
        database.close().await;

        assert!(matches!(same_email, Err(AccountError::EmailTaken)));
        assert!(matches!(same_username, Err(AccountError::UsernameTaken)));
        assert!(matches!(
            bad_username,
            Err(AccountError::InvalidInput(
                AccountRuleViolation::InvalidAccountName
            ))
        ));
        assert!(matches!(closed, Err(AccountError::RegistrationClosed)));
    }

    #[tokio::test]
    async fn sessions_expire_and_logout_revokes_them() {
        let (_directory, database) = migrated_database().await;
        let accounts = Accounts::new(
            database.clone(),
            RegistrationMode::Open,
            std::sync::Arc::default(),
            std::sync::Arc::default(),
        );
        let session = accounts
            .register(register_request("aurore", "aurore@example.org"), NOW)
            .await
            .unwrap();
        let lifetime = u64::try_from(SESSION_LIFETIME.as_millis()).unwrap();

        let just_before_expiry = accounts
            .authenticate(&session.token, NOW + lifetime - 1)
            .await;
        let at_expiry = accounts.authenticate(&session.token, NOW + lifetime).await;
        accounts.logout(&session.token).await.unwrap();
        let after_logout = accounts.authenticate(&session.token, NOW).await;
        let malformed = accounts.authenticate("Bearer nonsense", NOW).await;
        database.close().await;

        assert!(just_before_expiry.is_ok());
        assert!(matches!(at_expiry, Err(AccountError::Unauthorized)));
        assert!(matches!(after_logout, Err(AccountError::Unauthorized)));
        assert!(matches!(malformed, Err(AccountError::Unauthorized)));
    }

    #[tokio::test]
    async fn session_use_is_recorded_at_most_once_per_interval_and_expired_ones_are_purged() {
        let (_directory, database) = migrated_database().await;
        let accounts = Accounts::new(
            database.clone(),
            RegistrationMode::Open,
            std::sync::Arc::default(),
            std::sync::Arc::default(),
        );
        let session = accounts
            .register(register_request("aurore", "aurore@example.org"), NOW)
            .await
            .unwrap();
        let interval = u64::try_from(SESSION_USE_RECORD_INTERVAL.as_millis()).unwrap();
        let last_seen = || async {
            let (last_seen,): (i64,) =
                sqlx::query_as("SELECT last_seen_at_unix_milliseconds FROM sessions")
                    .fetch_one(database.writer())
                    .await
                    .unwrap();
            u64::try_from(last_seen).unwrap()
        };

        accounts
            .authenticate(&session.token, NOW + interval - 1)
            .await
            .unwrap();
        let before_interval = last_seen().await;
        accounts
            .authenticate(&session.token, NOW + interval)
            .await
            .unwrap();
        let after_interval = last_seen().await;

        // Use is recorded only forward, and a failed record never refuses the session.
        accounts.authenticate(&session.token, NOW).await.unwrap();
        let after_clock_went_back = last_seen().await;
        let token_sha256 = session_token_sha256(&session.token).unwrap();
        record_session_use(&database, &token_sha256, NOW)
            .await
            .unwrap();
        let recorded_backwards = last_seen().await;
        sqlx::query(
            "CREATE TRIGGER refuse_session_updates BEFORE UPDATE ON sessions \
             BEGIN SELECT RAISE(ABORT, 'disk full'); END",
        )
        .execute(database.writer())
        .await
        .unwrap();
        let despite_failed_record = accounts
            .authenticate(&session.token, NOW + 2 * interval)
            .await;

        let lifetime = u64::try_from(SESSION_LIFETIME.as_millis()).unwrap();
        let still_valid = delete_expired_sessions(&database, NOW + lifetime - 1)
            .await
            .unwrap();
        let expired = delete_expired_sessions(&database, NOW + lifetime)
            .await
            .unwrap();
        database.close().await;

        assert_eq!(before_interval, NOW);
        assert_eq!(after_interval, NOW + interval);
        assert_eq!(after_clock_went_back, NOW + interval);
        assert_eq!(recorded_backwards, NOW + interval);
        assert!(despite_failed_record.is_ok());
        assert_eq!((still_valid, expired), (0, 1));
    }

    #[tokio::test]
    async fn the_purge_task_deletes_expired_sessions_as_soon_as_it_starts() {
        let (_directory, database) = migrated_database().await;
        let accounts = Accounts::new(
            database.clone(),
            RegistrationMode::Open,
            std::sync::Arc::default(),
            std::sync::Arc::default(),
        );
        // `NOW` is in early 2026: by the real clock this session expired long ago.
        accounts
            .register(register_request("aurore", "aurore@example.org"), NOW)
            .await
            .unwrap();
        let purge = tokio::spawn(purge_expired_sessions_periodically(database.clone()));
        let mut remaining_sessions = 1;
        for _ in 0..100 {
            let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions")
                .fetch_one(database.writer())
                .await
                .unwrap();
            remaining_sessions = count;
            if remaining_sessions == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        purge.abort();
        let _cancelled = purge.await;
        database.close().await;
        assert_eq!(remaining_sessions, 0);
    }

    #[tokio::test]
    async fn disabled_accounts_cannot_log_in_or_use_existing_sessions() {
        let (_directory, database) = migrated_database().await;
        let accounts = Accounts::new(
            database.clone(),
            RegistrationMode::Open,
            std::sync::Arc::default(),
            std::sync::Arc::default(),
        );
        let session = accounts
            .register(register_request("aurore", "aurore@example.org"), NOW)
            .await
            .unwrap();
        sqlx::query("UPDATE users SET disabled_at_unix_milliseconds = ?")
            .bind(i64::try_from(NOW).unwrap())
            .execute(database.writer())
            .await
            .unwrap();

        let existing_session = accounts.authenticate(&session.token, NOW).await;
        let new_login = accounts.login(login_request("aurore", PASSWORD), NOW).await;
        database.close().await;
        assert!(matches!(
            existing_session,
            Err(AccountError::AccountDisabled)
        ));
        assert!(matches!(new_login, Err(AccountError::AccountDisabled)));
    }

    fn rename(
        username: Option<&str>,
        display_name: Option<&str>,
        current_password: Option<&str>,
    ) -> UpdateCurrentUserRequest {
        UpdateCurrentUserRequest {
            username: username.map(Into::into),
            display_name: display_name.map(Into::into),
            current_password: current_password.map(Into::into),
        }
    }

    #[tokio::test]
    async fn renaming_checks_the_name_frees_the_old_one_and_tells_the_user_sessions() {
        let (_directory, database) = migrated_database().await;
        let events = std::sync::Arc::new(crate::events::EventHub::default());
        let accounts = Accounts::new(
            database.clone(),
            RegistrationMode::Open,
            std::sync::Arc::default(),
            events.clone(),
        );
        let aurore = accounts
            .register(register_request("aurore", "aurore@example.org"), NOW)
            .await
            .unwrap();
        let basile = accounts
            .register(register_request("Basile", "basile@example.org"), NOW)
            .await
            .unwrap();
        let (session_key, mut session_events) = events
            .register_session(aurore.user.id, [9; 32], 10)
            .unwrap();
        events.activate_session(session_key, &[]);
        while session_events.try_recv().is_ok() {}
        let token = aurore.token.as_str();

        let unchanged = accounts
            .update_current_user(token, rename(None, Some(" aurore "), None), NOW)
            .await
            .unwrap();
        let nothing_published = session_events.try_recv().is_err();
        let name_taken = accounts
            .update_current_user(token, rename(None, Some("BASILÉ"), None), NOW)
            .await;
        let taken_by_alias = accounts
            .update_current_user(token, rename(Some("basile"), None, None), NOW)
            .await;
        let invalid = accounts
            .update_current_user(token, rename(None, Some("   "), None), NOW)
            .await;
        let own_name_available = accounts
            .is_name_available(aurore.user.id, "AURORE")
            .await
            .unwrap();
        let other_name_available = accounts
            .is_name_available(aurore.user.id, "basile")
            .await
            .unwrap();
        let renamed = accounts
            .update_current_user(token, rename(None, Some("  Aurore   B  "), None), NOW)
            .await
            .unwrap();
        let event = session_events.try_recv().unwrap().payload_json.to_string();
        let old_name_taken_by_basile = accounts
            .update_current_user(&basile.token, rename(Some("Aurore"), None, None), NOW)
            .await
            .unwrap();
        let stored = accounts.authenticate(token, NOW).await.unwrap();
        let signed_in_by_new_name = accounts
            .login(login_request("aurore b", PASSWORD), NOW)
            .await
            .unwrap();
        database.close().await;

        assert_eq!(unchanged, aurore.user);
        assert!(nothing_published);
        assert!(matches!(name_taken, Err(AccountError::UsernameTaken)));
        assert!(matches!(taken_by_alias, Err(AccountError::UsernameTaken)));
        assert!(matches!(
            invalid,
            Err(AccountError::InvalidInput(
                AccountRuleViolation::InvalidAccountName
            ))
        ));
        assert!(own_name_available && !other_name_available);
        assert_eq!(
            (renamed.username.as_str(), renamed.display_name.as_str()),
            ("Aurore B", "Aurore B"),
            "the deprecated username follows the name"
        );
        assert_eq!(renamed.friend_code, aurore.user.friend_code, "permanent");
        assert_eq!(stored, renamed);
        assert_eq!(old_name_taken_by_basile.display_name, "Aurore");
        assert_eq!(signed_in_by_new_name.user.id, aurore.user.id);
        assert_eq!(
            serde_json::from_str::<GatewayEvent>(&event).unwrap(),
            GatewayEvent::UserUpdate(PublicUser {
                id: renamed.id,
                username: "Aurore B".into(),
                display_name: "Aurore B".into(),
                avatar: None,
                banner: None,
            })
        );
    }

    #[tokio::test]
    async fn names_are_unique_whatever_their_case_and_accents_and_sign_in_by_any_case() {
        let (_directory, database) = migrated_database().await;
        let accounts = Accounts::new(
            database.clone(),
            RegistrationMode::Open,
            std::sync::Arc::default(),
            std::sync::Arc::default(),
        );
        let mut elodie = register_request("elodie", "elodie@example.org");
        elodie.display_name = Some("Élodie".into());
        let elodie = accounts.register(elodie, NOW).await.unwrap();
        let mut twin = register_request("x", "twin@example.org");
        twin.display_name = Some("ELODIE".into());
        let twin = accounts.register(twin, NOW).await;
        let other = accounts
            .register(register_request("paul", "paul@example.org"), NOW)
            .await
            .unwrap();
        let by_folded_name = accounts
            .login(login_request("  eLoDiE ", PASSWORD), NOW)
            .await
            .unwrap();
        let by_email = accounts
            .login(login_request("ELODIE@example.org", PASSWORD), NOW)
            .await
            .unwrap();
        database.close().await;

        assert_eq!(
            elodie.user.display_name, "Élodie",
            "capitals and accents kept"
        );
        assert_eq!(elodie.user.username, "Élodie");
        assert!(matches!(twin, Err(AccountError::UsernameTaken)));
        assert_eq!(by_folded_name.user.id, elodie.user.id);
        assert_eq!(by_email.user.id, elodie.user.id);
        let code = elodie.user.friend_code.clone().unwrap();
        assert_eq!(code.len(), 9);
        assert_eq!(&code[4..5], "-");
        assert_ne!(elodie.user.friend_code, other.user.friend_code);
    }
    /// A login or a password change verified against the old hash must not land after another
    /// password change committed meanwhile.
    #[tokio::test]
    async fn writes_verified_against_a_replaced_password_are_refused() {
        let (_directory, database) = migrated_database().await;
        let accounts = Accounts::new(
            database.clone(),
            RegistrationMode::Open,
            std::sync::Arc::default(),
            std::sync::Arc::default(),
        );
        let session = accounts
            .register(register_request("aurore", "aurore@example.org"), NOW)
            .await
            .unwrap();
        let old_hash = find_user_by_login(&database, "aurore", "aurore")
            .await
            .unwrap()
            .unwrap()
            .password_hash;
        let token_sha256 = session_token_sha256(&session.token).unwrap();
        let replaced = replace_password_and_delete_other_sessions(
            &database,
            session.user.id,
            &old_hash,
            "new-hash",
            &token_sha256,
            NOW,
        )
        .await
        .unwrap();

        let late_change = replace_password_and_delete_other_sessions(
            &database,
            session.user.id,
            &old_hash,
            "attacker-hash",
            &token_sha256,
            NOW,
        )
        .await
        .unwrap();
        let late_login = insert_session(
            &database,
            &NewSession {
                id: Snowflake::from_raw(999),
                user_id: session.user.id,
                token_sha256: [7; 32],
                device_name: "late".into(),
                created_at_unix_milliseconds: NOW,
                expires_at_unix_milliseconds: NOW + 1,
            },
            &old_hash,
        )
        .await
        .unwrap();
        let stored_hash = find_user_by_login(&database, "aurore", "aurore")
            .await
            .unwrap()
            .unwrap()
            .password_hash;
        database.close().await;

        assert!(replaced);
        assert!(!late_change);
        assert!(!late_login);
        assert_eq!(stored_hash, "new-hash");
    }

    #[tokio::test]
    async fn changing_the_password_needs_the_current_one_and_signs_out_the_other_sessions() {
        let (_directory, database) = migrated_database().await;
        let accounts = Accounts::new(
            database.clone(),
            RegistrationMode::Open,
            std::sync::Arc::default(),
            std::sync::Arc::default(),
        );
        let this_device = accounts
            .register(register_request("aurore", "aurore@example.org"), NOW)
            .await
            .unwrap();
        let other_device = accounts
            .login(login_request("aurore", PASSWORD), NOW)
            .await
            .unwrap();
        let change = |current: &str, new: &str| ChangePasswordRequest {
            current_password: current.into(),
            new_password: new.into(),
        };

        let wrong_current = accounts
            .change_password(
                &this_device.token,
                change("not the password", "a brand new password"),
                NOW,
            )
            .await;
        let too_short = accounts
            .change_password(&this_device.token, change(PASSWORD, "short"), NOW)
            .await;
        let other_still_valid = accounts.authenticate(&other_device.token, NOW).await;
        accounts
            .change_password(
                &this_device.token,
                change(PASSWORD, "a brand new password"),
                NOW,
            )
            .await
            .unwrap();
        let this_after = accounts.authenticate(&this_device.token, NOW).await;
        let other_after = accounts.authenticate(&other_device.token, NOW).await;
        let old_password = accounts.login(login_request("aurore", PASSWORD), NOW).await;
        let new_password = accounts
            .login(login_request("aurore", "a brand new password"), NOW)
            .await;
        database.close().await;

        assert!(matches!(
            wrong_current,
            Err(AccountError::InvalidCredentials)
        ));
        assert!(matches!(
            too_short,
            Err(AccountError::InvalidInput(
                AccountRuleViolation::InvalidPasswordLength
            ))
        ));
        assert!(other_still_valid.is_ok());
        assert!(this_after.is_ok());
        assert!(matches!(other_after, Err(AccountError::Unauthorized)));
        assert!(matches!(
            old_password,
            Err(AccountError::InvalidCredentials)
        ));
        assert!(new_password.is_ok());
    }
}
