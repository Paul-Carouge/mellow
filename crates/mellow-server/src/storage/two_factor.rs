//! Two-factor secrets, recovery codes and administrator resets (migration 0042, ADR-0049).

use mellow_protocol::Snowflake;

use super::Database;
use crate::domain::session_token::SessionTokenSha256;
use crate::domain::totp::{RecoveryCodeSha256, TOTP_SECRET_BYTES, TotpSecret};
use crate::storage::accounts::AccountStorageError;
use crate::storage::sqlite_integers::{convert_integer, to_sqlite_integer};

/// A user's two-factor row: an enrolment waiting for its first code, or an enabled one.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct StoredTwoFactor {
    pub(crate) secret: TotpSecret,
    pub(crate) is_enabled: bool,
    pub(crate) recovery_codes_remaining: u32,
    /// App codes are refused until then, after too many wrong codes in a row.
    pub(crate) codes_locked_until_unix_milliseconds: u64,
    pub(crate) last_failed_code_at_unix_milliseconds: Option<u64>,
}

/// The secret is never printed (logs must not contain secrets).
impl std::fmt::Debug for StoredTwoFactor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredTwoFactor")
            .field("secret", &"<redacted>")
            .field("is_enabled", &self.is_enabled)
            .field("recovery_codes_remaining", &self.recovery_codes_remaining)
            .field(
                "codes_locked_until_unix_milliseconds",
                &self.codes_locked_until_unix_milliseconds,
            )
            .field(
                "last_failed_code_at_unix_milliseconds",
                &self.last_failed_code_at_unix_milliseconds,
            )
            .finish()
    }
}

/// What `reset_two_factor_by_username` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TwoFactorReset {
    UnknownUser,
    /// Nothing to reset: two-factor sign-in was off (an unconfirmed enrolment is dropped anyway).
    WasNotEnabled,
    Reset,
}

/// Secret, enabled, recovery codes left, codes locked until, last wrong code at.
type TwoFactorRow = (Vec<u8>, bool, i64, i64, Option<i64>);

/// # Errors
/// Fails on database errors; a user without any row is `Ok(None)`.
pub(crate) async fn find_two_factor(
    database: &Database,
    user_id: Snowflake,
) -> Result<Option<StoredTwoFactor>, AccountStorageError> {
    let row: Option<TwoFactorRow> = sqlx::query_as(
        "SELECT secret, enabled_at_unix_milliseconds IS NOT NULL, \
         (SELECT COUNT(*) FROM two_factor_recovery_codes WHERE user_id = two_factor_secrets.user_id), \
         codes_locked_until_unix_milliseconds, last_failed_code_at_unix_milliseconds \
         FROM two_factor_secrets WHERE user_id = ?",
    )
    .bind(to_sqlite_integer(user_id.raw())?)
    .fetch_optional(database.readers())
    .await?;
    row.map(
        |(secret_bytes, is_enabled, remaining, locked_until, last_failed_at)| {
            let secret_bytes = zeroize::Zeroizing::new(secret_bytes);
            let mut secret = TotpSecret::new([0; TOTP_SECRET_BYTES]);
            // The column's CHECK keeps it 20 bytes; a mismatch is a corrupt row, refused below.
            if secret_bytes.len() != TOTP_SECRET_BYTES {
                return Err(AccountStorageError::Database(sqlx::Error::Decode(
                    "a two-factor secret has the wrong length".into(),
                )));
            }
            secret.copy_from_slice(&secret_bytes);
            Ok(StoredTwoFactor {
                secret,
                is_enabled,
                recovery_codes_remaining: convert_integer(remaining)?,
                codes_locked_until_unix_milliseconds: convert_integer(locked_until)?,
                last_failed_code_at_unix_milliseconds: last_failed_at
                    .map(convert_integer)
                    .transpose()?,
            })
        },
    )
    .transpose()
}

/// Whether sign-in must ask this user for a second factor.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn is_two_factor_enabled(
    database: &Database,
    user_id: Snowflake,
) -> Result<bool, AccountStorageError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM two_factor_secrets \
         WHERE user_id = ? AND enabled_at_unix_milliseconds IS NOT NULL)",
    )
    .bind(to_sqlite_integer(user_id.raw())?)
    .fetch_one(database.readers())
    .await?)
}

/// Stores the secret of a new enrolment, replacing an unconfirmed one. Returns `false`, changing
/// nothing, when two-factor sign-in is already on.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn start_enrolment(
    database: &Database,
    user_id: Snowflake,
    secret: &TotpSecret,
    now_unix_milliseconds: u64,
) -> Result<bool, AccountStorageError> {
    let result = sqlx::query(
        "INSERT INTO two_factor_secrets (user_id, secret, created_at_unix_milliseconds) \
         VALUES (?1, ?2, ?3) \
         ON CONFLICT (user_id) DO UPDATE SET secret = excluded.secret, last_used_time_step = -1, \
         created_at_unix_milliseconds = excluded.created_at_unix_milliseconds \
         WHERE two_factor_secrets.enabled_at_unix_milliseconds IS NULL",
    )
    .bind(to_sqlite_integer(user_id.raw())?)
    .bind(secret.as_slice())
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .execute(database.writer())
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Turns two-factor sign-in on with the recovery codes' hashes, once a code for `accepted_step` of
/// `verified_secret` was checked, and signs out every session of the user but `kept_session`: one
/// opened by someone who had only the password must not outlive the new protection. Returns
/// `false`, changing nothing, when the enrolment changed meanwhile (another secret, already
/// enabled) or that step was already used.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn enable_two_factor(
    database: &Database,
    user_id: Snowflake,
    verified_secret: &TotpSecret,
    accepted_step: u64,
    recovery_codes: &[RecoveryCodeSha256],
    kept_session: &SessionTokenSha256,
    now_unix_milliseconds: u64,
) -> Result<bool, AccountStorageError> {
    let user_id = to_sqlite_integer(user_id.raw())?;
    let mut transaction = database.writer().begin().await?;
    let updated = sqlx::query(
        "UPDATE two_factor_secrets SET enabled_at_unix_milliseconds = ?1, last_used_time_step = ?2 \
         WHERE user_id = ?3 AND secret = ?4 AND enabled_at_unix_milliseconds IS NULL \
         AND last_used_time_step < ?2",
    )
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .bind(to_sqlite_integer(accepted_step)?)
    .bind(user_id)
    .bind(verified_secret.as_slice())
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() == 0 {
        return Ok(false);
    }
    insert_recovery_codes(&mut transaction, user_id, recovery_codes).await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_sha256 != ?")
        .bind(user_id)
        .bind(kept_session.as_slice())
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(true)
}

async fn insert_recovery_codes(
    transaction: &mut sqlx::SqliteConnection,
    user_id: i64,
    recovery_codes: &[RecoveryCodeSha256],
) -> Result<(), AccountStorageError> {
    sqlx::query("DELETE FROM two_factor_recovery_codes WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    for code in recovery_codes {
        sqlx::query("INSERT INTO two_factor_recovery_codes (user_id, code_sha256) VALUES (?, ?)")
            .bind(user_id)
            .bind(code.as_slice())
            .execute(&mut *transaction)
            .await?;
    }
    Ok(())
}

/// Records that the code of `step` was used, so it never works again, and forgives the wrong codes
/// before it. Returns `false` when that step (or a later one) was already used, app codes are
/// locked, two-factor sign-in is off, or the secret changed.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn use_time_step(
    database: &Database,
    user_id: Snowflake,
    verified_secret: &TotpSecret,
    step: u64,
    now_unix_milliseconds: u64,
) -> Result<bool, AccountStorageError> {
    let result = sqlx::query(
        "UPDATE two_factor_secrets SET last_used_time_step = ?1, failed_code_streak = 0 \
         WHERE user_id = ?2 AND secret = ?3 AND enabled_at_unix_milliseconds IS NOT NULL \
         AND last_used_time_step < ?1 AND codes_locked_until_unix_milliseconds <= ?4",
    )
    .bind(to_sqlite_integer(step)?)
    .bind(to_sqlite_integer(user_id.raw())?)
    .bind(verified_secret.as_slice())
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .execute(database.writer())
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Spends a recovery code: returns whether it was one of the user's unused codes while two-factor
/// sign-in is on. It works while app codes are locked, and lifts the lock: only the owner has it.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn use_recovery_code(
    database: &Database,
    user_id: Snowflake,
    code: &RecoveryCodeSha256,
) -> Result<bool, AccountStorageError> {
    let user_id = to_sqlite_integer(user_id.raw())?;
    let mut transaction = database.writer().begin().await?;
    let result = sqlx::query(
        "DELETE FROM two_factor_recovery_codes WHERE user_id = ?1 AND code_sha256 = ?2 \
         AND EXISTS (SELECT 1 FROM two_factor_secrets WHERE user_id = ?1 \
         AND enabled_at_unix_milliseconds IS NOT NULL)",
    )
    .bind(user_id)
    .bind(code.as_slice())
    .execute(&mut *transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Ok(false);
    }
    sqlx::query(
        "UPDATE two_factor_secrets SET failed_code_streak = 0, \
         codes_locked_until_unix_milliseconds = 0 WHERE user_id = ?",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(true)
}

/// Counts a wrong code and returns how many came in a row, or `None` when two-factor sign-in is
/// off.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn record_wrong_code(
    database: &Database,
    user_id: Snowflake,
    now_unix_milliseconds: u64,
) -> Result<Option<u32>, AccountStorageError> {
    let streak: Option<i64> = sqlx::query_scalar(
        "UPDATE two_factor_secrets SET failed_code_streak = failed_code_streak + 1, \
         last_failed_code_at_unix_milliseconds = ? \
         WHERE user_id = ? AND enabled_at_unix_milliseconds IS NOT NULL \
         RETURNING failed_code_streak",
    )
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .bind(to_sqlite_integer(user_id.raw())?)
    .fetch_optional(database.writer())
    .await?;
    Ok(streak.map(convert_integer).transpose()?)
}

/// Refuses the user's app codes until `locked_until_unix_milliseconds`.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn lock_codes(
    database: &Database,
    user_id: Snowflake,
    locked_until_unix_milliseconds: u64,
) -> Result<(), AccountStorageError> {
    sqlx::query(
        "UPDATE two_factor_secrets SET codes_locked_until_unix_milliseconds = ? WHERE user_id = ?",
    )
    .bind(to_sqlite_integer(locked_until_unix_milliseconds)?)
    .bind(to_sqlite_integer(user_id.raw())?)
    .execute(database.writer())
    .await?;
    Ok(())
}

/// Replaces every recovery code of a user with two-factor sign-in on. Returns `false` otherwise.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn replace_recovery_codes(
    database: &Database,
    user_id: Snowflake,
    recovery_codes: &[RecoveryCodeSha256],
) -> Result<bool, AccountStorageError> {
    let user_id = to_sqlite_integer(user_id.raw())?;
    let mut transaction = database.writer().begin().await?;
    let is_enabled: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM two_factor_secrets \
         WHERE user_id = ? AND enabled_at_unix_milliseconds IS NOT NULL)",
    )
    .bind(user_id)
    .fetch_one(&mut *transaction)
    .await?;
    if !is_enabled {
        return Ok(false);
    }
    insert_recovery_codes(&mut transaction, user_id, recovery_codes).await?;
    transaction.commit().await?;
    Ok(true)
}

/// Turns two-factor sign-in off: the secret and the recovery codes are deleted.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn delete_two_factor(
    database: &Database,
    user_id: Snowflake,
) -> Result<(), AccountStorageError> {
    let user_id = to_sqlite_integer(user_id.raw())?;
    let mut transaction = database.writer().begin().await?;
    delete_two_factor_rows(&mut transaction, user_id).await?;
    transaction.commit().await?;
    Ok(())
}

/// Returns whether two-factor sign-in was on.
async fn delete_two_factor_rows(
    transaction: &mut sqlx::SqliteConnection,
    user_id: i64,
) -> Result<bool, AccountStorageError> {
    sqlx::query("DELETE FROM two_factor_recovery_codes WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    let deleted = sqlx::query(
        "DELETE FROM two_factor_secrets WHERE user_id = ? \
         RETURNING enabled_at_unix_milliseconds IS NOT NULL",
    )
    .bind(user_id)
    .fetch_optional(&mut *transaction)
    .await?;
    Ok(deleted.is_some_and(|row| sqlx::Row::get::<bool, _>(&row, 0)))
}

/// `mellow-server admin reset-two-factor`: turns the user's two-factor sign-in off and records the
/// reset with `reset_id` in the same transaction.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn reset_two_factor_by_username(
    database: &Database,
    username: &str,
    reset_id: Snowflake,
    now_unix_milliseconds: u64,
) -> Result<TwoFactorReset, AccountStorageError> {
    let mut transaction = database.writer().begin().await?;
    let Some((user_id, stored_username)): Option<(i64, String)> =
        sqlx::query_as("SELECT id, username FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(&mut *transaction)
            .await?
    else {
        return Ok(TwoFactorReset::UnknownUser);
    };
    if !delete_two_factor_rows(&mut transaction, user_id).await? {
        transaction.commit().await?;
        return Ok(TwoFactorReset::WasNotEnabled);
    }
    sqlx::query(
        "INSERT INTO two_factor_resets (id, user_id, username, reset_at_unix_milliseconds) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(to_sqlite_integer(reset_id.raw())?)
    .bind(user_id)
    .bind(stored_username)
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(TwoFactorReset::Reset)
}
