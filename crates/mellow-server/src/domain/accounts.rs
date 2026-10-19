//! Account use cases: register, create by the administrator, log in, authenticate, log out.

use std::time::Duration;

use mellow_protocol::{CurrentUser, LoginRequest, RegisterRequest, SessionCreated};
use sqlx::SqlitePool;
use zeroize::Zeroizing;

use crate::domain::account_rules::{
    AccountRuleViolation, RegistrationMode, normalize_device_name, normalize_display_name,
    normalize_email, normalize_login, validate_password_length, validate_password_maximum_length,
    validate_username,
};
use crate::domain::password::{PasswordHashError, hash_password, verify_password};
use crate::domain::session_token::{generate_session_token, session_token_sha256};
use crate::domain::snowflake_generator::SnowflakeGenerator;
use crate::storage::accounts::{
    AccountStorageError, NewSession, NewUser, StoredUser, delete_session, find_user_by_login,
    find_user_by_session, insert_session, insert_user,
};

/// A session lasts 90 days, then the user signs in again.
const SESSION_LIFETIME: Duration = Duration::from_hours(90 * 24);

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error(transparent)]
    InvalidInput(#[from] AccountRuleViolation),
    #[error("this username is already taken")]
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
    pool: SqlitePool,
    /// Shared with the other use cases so ids are unique across the whole process.
    ids: std::sync::Arc<SnowflakeGenerator>,
    registration_mode: RegistrationMode,
}

impl Accounts {
    #[must_use]
    pub fn new(
        pool: SqlitePool,
        registration_mode: RegistrationMode,
        ids: std::sync::Arc<SnowflakeGenerator>,
    ) -> Self {
        Self {
            pool,
            ids,
            registration_mode,
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
        validate_username(&account.username)?;
        let display_name =
            normalize_display_name(account.display_name.as_deref().unwrap_or(&account.username))?;
        let email = normalize_email(&account.email)?;
        validate_password_length(&account.password)?;
        let password_hash = hash_password(account.password).await?;
        let new_user = NewUser {
            id: self.ids.next_id(now_unix_milliseconds)?,
            username: account.username,
            display_name,
            email,
            password_hash,
            is_instance_administrator: account.is_instance_administrator,
            created_at_unix_milliseconds: now_unix_milliseconds,
        };
        insert_user(&self.pool, &new_user).await?;
        Ok(StoredUser {
            id: new_user.id,
            username: new_user.username,
            display_name: new_user.display_name,
            email: new_user.email,
            password_hash: new_user.password_hash,
            is_instance_administrator: new_user.is_instance_administrator,
            is_disabled: false,
        })
    }

    /// # Errors
    /// `InvalidCredentials` for an unknown login or a wrong password (indistinguishable, and
    /// equally slow); `AccountDisabled` only once the password is proven right.
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
        let user = find_user_by_login(&self.pool, &login).await?;
        let stored_hash = user.as_ref().map(|user| user.password_hash.clone());
        let is_password_correct = verify_password(password, stored_hash).await?;
        let user = match user {
            Some(user) if is_password_correct => user,
            _ => return Err(AccountError::InvalidCredentials),
        };
        if user.is_disabled {
            return Err(AccountError::AccountDisabled);
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
        let user = find_user_by_session(&self.pool, &token_sha256, now_unix_milliseconds)
            .await?
            .ok_or(AccountError::Unauthorized)?;
        if user.is_disabled {
            return Err(AccountError::AccountDisabled);
        }
        Ok(current_user(user))
    }

    /// Ends the session the token belongs to. Idempotent.
    ///
    /// # Errors
    /// `Unauthorized` for a malformed token; database errors.
    pub async fn logout(&self, token: &str) -> Result<(), AccountError> {
        let token_sha256 = session_token_sha256(token).ok_or(AccountError::Unauthorized)?;
        delete_session(&self.pool, &token_sha256).await?;
        Ok(())
    }

    async fn open_session(
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
        insert_session(
            &self.pool,
            &NewSession {
                id: self.ids.next_id(now_unix_milliseconds)?,
                user_id: user.id,
                token_sha256,
                device_name,
                created_at_unix_milliseconds: now_unix_milliseconds,
                expires_at_unix_milliseconds: now_unix_milliseconds
                    .saturating_add(session_lifetime_milliseconds),
            },
        )
        .await?;
        Ok(SessionCreated {
            token: token.to_string(),
            user: current_user(user),
        })
    }
}

fn current_user(user: StoredUser) -> CurrentUser {
    CurrentUser {
        id: user.id,
        username: user.username,
        display_name: user.display_name,
        email: user.email,
        is_instance_administrator: user.is_instance_administrator,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use mellow_protocol::MELLOW_EPOCH_UNIX_MILLISECONDS;

    use super::*;
    use crate::storage::{DatabaseKey, migrate_with_backup, open_encrypted_database};

    pub(crate) const NOW: u64 = MELLOW_EPOCH_UNIX_MILLISECONDS + 86_400_000;
    pub(crate) const PASSWORD: &str = "correct horse battery";

    /// A migrated database in a temporary directory; keep the directory alive while using the pool.
    pub(crate) async fn migrated_pool() -> (tempfile::TempDir, SqlitePool) {
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(
            &directory.path().join("mellow.db"),
            &DatabaseKey::from_bytes([3; 32]),
        )
        .await
        .unwrap();
        migrate_with_backup(&pool, &directory.path().join("backups"))
            .await
            .unwrap();
        (directory, pool)
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
        let (_directory, pool) = migrated_pool().await;
        let accounts = Accounts::new(
            pool.clone(),
            RegistrationMode::Open,
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
        pool.close().await;
    }

    #[tokio::test]
    async fn wrong_password_and_unknown_user_get_the_same_error() {
        let (_directory, pool) = migrated_pool().await;
        let accounts = Accounts::new(
            pool.clone(),
            RegistrationMode::Open,
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
        pool.close().await;
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
        let (_directory, pool) = migrated_pool().await;
        let open = Accounts::new(
            pool.clone(),
            RegistrationMode::Open,
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
            pool.clone(),
            RegistrationMode::Closed,
            std::sync::Arc::default(),
        )
        .register(register_request("chloe", "chloe@example.org"), NOW)
        .await;
        pool.close().await;

        assert!(matches!(same_email, Err(AccountError::EmailTaken)));
        assert!(matches!(same_username, Err(AccountError::UsernameTaken)));
        assert!(matches!(
            bad_username,
            Err(AccountError::InvalidInput(
                AccountRuleViolation::InvalidUsername
            ))
        ));
        assert!(matches!(closed, Err(AccountError::RegistrationClosed)));
    }

    #[tokio::test]
    async fn sessions_expire_and_logout_revokes_them() {
        let (_directory, pool) = migrated_pool().await;
        let accounts = Accounts::new(
            pool.clone(),
            RegistrationMode::Open,
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
        pool.close().await;

        assert!(just_before_expiry.is_ok());
        assert!(matches!(at_expiry, Err(AccountError::Unauthorized)));
        assert!(matches!(after_logout, Err(AccountError::Unauthorized)));
        assert!(matches!(malformed, Err(AccountError::Unauthorized)));
    }

    #[tokio::test]
    async fn disabled_accounts_cannot_log_in_or_use_existing_sessions() {
        let (_directory, pool) = migrated_pool().await;
        let accounts = Accounts::new(
            pool.clone(),
            RegistrationMode::Open,
            std::sync::Arc::default(),
        );
        let session = accounts
            .register(register_request("aurore", "aurore@example.org"), NOW)
            .await
            .unwrap();
        sqlx::query("UPDATE users SET disabled_at_unix_milliseconds = ?")
            .bind(i64::try_from(NOW).unwrap())
            .execute(&pool)
            .await
            .unwrap();

        let existing_session = accounts.authenticate(&session.token, NOW).await;
        let new_login = accounts.login(login_request("aurore", PASSWORD), NOW).await;
        pool.close().await;
        assert!(matches!(
            existing_session,
            Err(AccountError::AccountDisabled)
        ));
        assert!(matches!(new_login, Err(AccountError::AccountDisabled)));
    }
}
