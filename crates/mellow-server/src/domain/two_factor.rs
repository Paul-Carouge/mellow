//! Two-factor sign-in use cases (ADR-0049): enrolment, the second sign-in step, turning it off,
//! new recovery codes, and the administrator's reset.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use mellow_protocol::{
    SessionCreated, Snowflake, TwoFactorChallenge, TwoFactorEnrolment, TwoFactorRecoveryCodes,
    TwoFactorStatus,
};
use zeroize::Zeroizing;

use crate::domain::account_rules::validate_password_maximum_length;
use crate::domain::accounts::{AccountError, Accounts, VerifiedLogin};
use crate::domain::password::verify_password;
use crate::domain::session_token::{
    SessionTokenSha256, generate_session_token, session_token_sha256,
};
use crate::domain::totp::{
    base32, generate_recovery_codes, generate_totp_secret, matching_time_step, normalize_totp_code,
    recovery_code_sha256,
};
use crate::storage::Database;
use crate::storage::accounts::{AccountStorageError, StoredUser, find_user_by_id};
use crate::storage::two_factor::{
    delete_two_factor, enable_two_factor, find_two_factor, replace_recovery_codes, start_enrolment,
    use_recovery_code, use_time_step,
};

/// Long enough to open an authenticator app, short enough that a stolen token goes stale.
pub(crate) const SIGN_IN_CHALLENGE_LIFETIME: Duration = Duration::from_mins(5);
/// Wrong codes one challenge takes before it is dropped; the password is then asked again.
const CODE_ATTEMPTS_PER_CHALLENGE: u8 = 5;
/// Bounds the memory pending sign-ins take; each needed a right password, which is rate limited.
const MAXIMUM_PENDING_SIGN_INS: usize = 10_000;

#[derive(Debug, thiserror::Error)]
pub(crate) enum TwoFactorError {
    #[error(transparent)]
    Account(#[from] AccountError),
    #[error("wrong, already used or expired code")]
    InvalidCode,
    #[error("two-factor sign-in is already on")]
    AlreadyEnabled,
    #[error("two-factor sign-in is off")]
    NotEnabled,
}

impl From<AccountStorageError> for TwoFactorError {
    fn from(error: AccountStorageError) -> Self {
        Self::Account(error.into())
    }
}

impl From<getrandom::Error> for TwoFactorError {
    fn from(error: getrandom::Error) -> Self {
        Self::Account(error.into())
    }
}

impl From<crate::domain::password::PasswordHashError> for TwoFactorError {
    fn from(error: crate::domain::password::PasswordHashError) -> Self {
        Self::Account(error.into())
    }
}

struct PendingSignIn {
    login: VerifiedLogin,
    expires_at: Instant,
    attempts_left: u8,
}

/// Two-factor sign-in. Pending sign-ins live in memory: a restart only asks for the password again.
pub(crate) struct TwoFactor {
    database: Database,
    pending_sign_ins: Mutex<HashMap<SessionTokenSha256, PendingSignIn>>,
}

impl std::fmt::Debug for TwoFactor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("TwoFactor").finish_non_exhaustive()
    }
}

impl TwoFactor {
    pub(crate) fn new(database: Database) -> Self {
        Self {
            database,
            pending_sign_ins: Mutex::default(),
        }
    }

    /// # Errors
    /// Database errors.
    pub(crate) async fn status(
        &self,
        user_id: Snowflake,
    ) -> Result<TwoFactorStatus, TwoFactorError> {
        let stored = find_two_factor(&self.database, user_id).await?;
        Ok(match stored {
            Some(stored) if stored.is_enabled => TwoFactorStatus {
                is_enabled: true,
                recovery_codes_remaining: stored.recovery_codes_remaining,
            },
            _ => TwoFactorStatus {
                is_enabled: false,
                recovery_codes_remaining: 0,
            },
        })
    }

    /// A new secret for the signed-in user, once their password is proven. Replaces an
    /// unconfirmed enrolment.
    ///
    /// # Errors
    /// `InvalidCredentials` for a wrong password, `AlreadyEnabled`.
    pub(crate) async fn start_enrolment(
        &self,
        user: &StoredUser,
        current_password: String,
        now_unix_milliseconds: u64,
    ) -> Result<TwoFactorEnrolment, TwoFactorError> {
        verify_current_password(user, current_password).await?;
        let secret = generate_totp_secret()?;
        if !start_enrolment(&self.database, user.id, &secret, now_unix_milliseconds).await? {
            return Err(TwoFactorError::AlreadyEnabled);
        }
        Ok(TwoFactorEnrolment {
            secret: base32(secret.as_ref()).to_string(),
        })
    }

    /// Turns two-factor sign-in on with a code from the enrolment's secret, and returns the
    /// recovery codes (shown once).
    ///
    /// # Errors
    /// `InvalidCode`, `AlreadyEnabled`, `NotEnabled` without an enrolment.
    pub(crate) async fn enable(
        &self,
        user_id: Snowflake,
        code: &str,
        now_unix_milliseconds: u64,
    ) -> Result<TwoFactorRecoveryCodes, TwoFactorError> {
        let stored = find_two_factor(&self.database, user_id)
            .await?
            .ok_or(TwoFactorError::NotEnabled)?;
        if stored.is_enabled {
            return Err(TwoFactorError::AlreadyEnabled);
        }
        let step = normalize_totp_code(code)
            .and_then(|code| {
                matching_time_step(stored.secret.as_ref(), *code, now_unix_milliseconds)
            })
            .ok_or(TwoFactorError::InvalidCode)?;
        let (codes, hashes) = new_recovery_codes()?;
        if !enable_two_factor(
            &self.database,
            user_id,
            &stored.secret,
            step,
            &hashes,
            now_unix_milliseconds,
        )
        .await?
        {
            return Err(TwoFactorError::InvalidCode);
        }
        Ok(codes)
    }

    /// Turns two-factor sign-in off, with the password and a code.
    ///
    /// # Errors
    /// `InvalidCredentials` for a wrong password, `InvalidCode`, `NotEnabled`.
    pub(crate) async fn disable(
        &self,
        user: &StoredUser,
        current_password: String,
        code: &str,
        now_unix_milliseconds: u64,
    ) -> Result<(), TwoFactorError> {
        verify_current_password(user, current_password).await?;
        self.check_code(user.id, code, now_unix_milliseconds)
            .await?;
        delete_two_factor(&self.database, user.id).await?;
        Ok(())
    }

    /// Replaces the recovery codes, with the password and a code.
    ///
    /// # Errors
    /// `InvalidCredentials` for a wrong password, `InvalidCode`, `NotEnabled`.
    pub(crate) async fn regenerate_recovery_codes(
        &self,
        user: &StoredUser,
        current_password: String,
        code: &str,
        now_unix_milliseconds: u64,
    ) -> Result<TwoFactorRecoveryCodes, TwoFactorError> {
        verify_current_password(user, current_password).await?;
        self.check_code(user.id, code, now_unix_milliseconds)
            .await?;
        let (codes, hashes) = new_recovery_codes()?;
        if !replace_recovery_codes(&self.database, user.id, &hashes).await? {
            return Err(TwoFactorError::NotEnabled);
        }
        Ok(codes)
    }

    /// Accepts a current TOTP code (once) or spends a recovery code.
    async fn check_code(
        &self,
        user_id: Snowflake,
        code: &str,
        now_unix_milliseconds: u64,
    ) -> Result<(), TwoFactorError> {
        let stored = find_two_factor(&self.database, user_id)
            .await?
            .filter(|stored| stored.is_enabled)
            .ok_or(TwoFactorError::NotEnabled)?;
        let is_accepted = if let Some(totp_code) = normalize_totp_code(code) {
            match matching_time_step(stored.secret.as_ref(), *totp_code, now_unix_milliseconds) {
                Some(step) => use_time_step(&self.database, user_id, &stored.secret, step).await?,
                None => false,
            }
        } else if let Some(recovery_code) = recovery_code_sha256(code) {
            use_recovery_code(&self.database, user_id, &recovery_code).await?
        } else {
            false
        };
        if is_accepted {
            Ok(())
        } else {
            Err(TwoFactorError::InvalidCode)
        }
    }

    /// Holds a sign-in whose password was right until its code comes.
    ///
    /// # Errors
    /// `ServerBusy` when too many sign-ins are pending.
    pub(crate) fn issue_challenge(
        &self,
        login: VerifiedLogin,
        now: Instant,
    ) -> Result<TwoFactorChallenge, TwoFactorError> {
        let token = generate_session_token()?;
        let token_sha256 = session_token_sha256(&token).ok_or_else(|| {
            AccountError::Internal("a generated token failed its own format check".into())
        })?;
        let mut pending = self.lock_pending_sign_ins();
        if pending.len() >= MAXIMUM_PENDING_SIGN_INS {
            pending.retain(|_, sign_in| sign_in.expires_at > now);
            if pending.len() >= MAXIMUM_PENDING_SIGN_INS {
                return Err(AccountError::ServerBusy.into());
            }
        }
        pending.insert(
            token_sha256,
            PendingSignIn {
                login,
                expires_at: now + SIGN_IN_CHALLENGE_LIFETIME,
                attempts_left: CODE_ATTEMPTS_PER_CHALLENGE,
            },
        );
        Ok(TwoFactorChallenge {
            two_factor_token: token.to_string(),
            expires_in_seconds: u32::try_from(SIGN_IN_CHALLENGE_LIFETIME.as_secs())
                .unwrap_or(u32::MAX),
        })
    }

    /// The account a live challenge signs into, so its attempts can be counted per account.
    pub(crate) fn challenge_owner(&self, token: &str, now: Instant) -> Option<Snowflake> {
        let token_sha256 = session_token_sha256(token)?;
        self.lock_pending_sign_ins()
            .get(&token_sha256)
            .filter(|sign_in| sign_in.expires_at > now)
            .map(|sign_in| sign_in.login.user_id)
    }

    /// The second sign-in step: a TOTP code or a recovery code opens the session, and the
    /// challenge is spent. Each wrong code uses one of the challenge's attempts.
    ///
    /// # Errors
    /// `Unauthorized` for an unknown, expired or spent challenge; `InvalidCode`.
    pub(crate) async fn complete_sign_in(
        &self,
        accounts: &Accounts,
        token: &str,
        code: &str,
        now: Instant,
        now_unix_milliseconds: u64,
    ) -> Result<SessionCreated, TwoFactorError> {
        let token_sha256 = session_token_sha256(token).ok_or(AccountError::Unauthorized)?;
        let login = {
            let pending = self.lock_pending_sign_ins();
            let sign_in = pending
                .get(&token_sha256)
                .filter(|sign_in| sign_in.expires_at > now)
                .ok_or(AccountError::Unauthorized)?;
            sign_in.login.clone()
        };
        match self
            .check_code(login.user_id, code, now_unix_milliseconds)
            .await
        {
            Ok(()) => {}
            Err(TwoFactorError::InvalidCode | TwoFactorError::NotEnabled) => {
                self.spend_attempt(&token_sha256);
                return Err(TwoFactorError::InvalidCode);
            }
            Err(error) => return Err(error),
        }
        // Single use: whoever removes it first opens the session.
        if self.lock_pending_sign_ins().remove(&token_sha256).is_none() {
            return Err(AccountError::Unauthorized.into());
        }
        let user = find_user_by_id(&self.database, login.user_id)
            .await?
            .filter(|user| !user.is_disabled && user.password_hash == login.verified_password_hash)
            .ok_or(AccountError::InvalidCredentials)?;
        Ok(accounts
            .open_session(user, login.device_name, now_unix_milliseconds)
            .await?)
    }

    fn spend_attempt(&self, token_sha256: &SessionTokenSha256) {
        let mut pending = self.lock_pending_sign_ins();
        if let Some(sign_in) = pending.get_mut(token_sha256) {
            sign_in.attempts_left = sign_in.attempts_left.saturating_sub(1);
            if sign_in.attempts_left == 0 {
                pending.remove(token_sha256);
            }
        }
    }

    fn lock_pending_sign_ins(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<SessionTokenSha256, PendingSignIn>> {
        // The map stays consistent whatever panicked while it was held: every write is one call.
        self.pending_sign_ins
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

async fn verify_current_password(
    user: &StoredUser,
    current_password: String,
) -> Result<(), TwoFactorError> {
    let current_password = Zeroizing::new(current_password);
    validate_password_maximum_length(&current_password).map_err(AccountError::from)?;
    if verify_password(current_password, Some(user.password_hash.clone())).await? {
        Ok(())
    } else {
        Err(AccountError::InvalidCredentials.into())
    }
}

fn new_recovery_codes() -> Result<
    (
        TwoFactorRecoveryCodes,
        Vec<crate::domain::totp::RecoveryCodeSha256>,
    ),
    TwoFactorError,
> {
    let codes = generate_recovery_codes()?;
    let hashes = codes
        .iter()
        .filter_map(|code| recovery_code_sha256(code))
        .collect();
    Ok((
        TwoFactorRecoveryCodes {
            recovery_codes: codes.iter().map(|code| code.to_string()).collect(),
        },
        hashes,
    ))
}

#[cfg(test)]
mod tests;
