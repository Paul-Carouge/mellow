//! Users and sessions tables (migration 0001).

use mellow_protocol::{ProfileImage, Snowflake};

use super::Database;

use crate::domain::session_token::SessionTokenSha256;

/// `SQLITE_CONSTRAINT_UNIQUE` extended result code.
const SQLITE_UNIQUE_CONSTRAINT_ERROR_CODE: &str = "2067";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredUser {
    pub id: Snowflake,
    pub username: String,
    pub display_name: String,
    pub email: String,
    pub password_hash: String,
    pub is_instance_administrator: bool,
    pub is_disabled: bool,
    pub avatar: Option<ProfileImage>,
    pub banner: Option<ProfileImage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewUser {
    pub id: Snowflake,
    pub username: String,
    pub display_name: String,
    pub email: String,
    pub password_hash: String,
    pub is_instance_administrator: bool,
    pub created_at_unix_milliseconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSession {
    pub id: Snowflake,
    pub user_id: Snowflake,
    pub token_sha256: SessionTokenSha256,
    pub device_name: String,
    pub created_at_unix_milliseconds: u64,
    pub expires_at_unix_milliseconds: u64,
}

/// The user behind an unexpired session, with when that session was last recorded as used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionOwner {
    pub user: StoredUser,
    pub last_seen_at_unix_milliseconds: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum AccountStorageError {
    #[error("this username is already taken")]
    UsernameTaken,
    #[error("this email address is already used")]
    EmailTaken,
    #[error("value does not fit SQLite's signed 64-bit integer")]
    IntegerOutOfRange,
    #[error("database error")]
    Database(#[from] sqlx::Error),
}

type StoredUserRow = (
    i64,
    String,
    String,
    String,
    String,
    bool,
    Option<i64>,
    Option<i64>,
    bool,
    Option<i64>,
    bool,
);

/// A macro, not a `const`, so queries can be built with `concat!` and stay `&'static str`
/// (sqlx 0.9 refuses runtime-built SQL as an injection guard).
macro_rules! select_user_columns {
    () => {
        "users.id, users.username, users.display_name, users.email, users.password_hash, \
         users.is_instance_administrator, users.disabled_at_unix_milliseconds, users.avatar_id, \
         users.avatar_is_animated, users.banner_id, users.banner_is_animated"
    };
}

/// SQLite integers are signed; Snowflakes and Unix milliseconds fit until about 2095.
fn to_sqlite_integer(value: u64) -> Result<i64, AccountStorageError> {
    i64::try_from(value).map_err(|_| AccountStorageError::IntegerOutOfRange)
}

fn stored_user_from_row(row: StoredUserRow) -> Result<StoredUser, AccountStorageError> {
    let (
        id,
        username,
        display_name,
        email,
        password_hash,
        is_instance_administrator,
        disabled_at,
        avatar_id,
        avatar_is_animated,
        banner_id,
        banner_is_animated,
    ) = row;
    let profile_image = |id: Option<i64>, is_animated: bool| {
        id.map(|id| {
            Ok::<_, AccountStorageError>(ProfileImage {
                id: Snowflake::from_raw(
                    u64::try_from(id).map_err(|_| AccountStorageError::IntegerOutOfRange)?,
                ),
                is_animated,
            })
        })
        .transpose()
    };
    Ok(StoredUser {
        id: Snowflake::from_raw(
            u64::try_from(id).map_err(|_| AccountStorageError::IntegerOutOfRange)?,
        ),
        username,
        display_name,
        email,
        password_hash,
        is_instance_administrator,
        is_disabled: disabled_at.is_some(),
        avatar: profile_image(avatar_id, avatar_is_animated)?,
        banner: profile_image(banner_id, banner_is_animated)?,
    })
}

/// # Errors
/// `UsernameTaken` / `EmailTaken` when a unique constraint rejects the row (case-insensitive).
pub async fn insert_user(database: &Database, user: &NewUser) -> Result<(), AccountStorageError> {
    let result = sqlx::query(
        "INSERT INTO users (id, username, display_name, email, password_hash, \
         is_instance_administrator, created_at_unix_milliseconds) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(to_sqlite_integer(user.id.raw())?)
    .bind(&user.username)
    .bind(&user.display_name)
    .bind(&user.email)
    .bind(&user.password_hash)
    .bind(user.is_instance_administrator)
    .bind(to_sqlite_integer(user.created_at_unix_milliseconds)?)
    .execute(database.writer())
    .await;
    result.map(|_| ()).map_err(unique_violation_as_taken)
}

fn unique_violation_as_taken(error: sqlx::Error) -> AccountStorageError {
    match error {
        sqlx::Error::Database(database_error)
            if database_error.code().as_deref() == Some(SQLITE_UNIQUE_CONSTRAINT_ERROR_CODE) =>
        {
            // SQLite names the violated column: "UNIQUE constraint failed: users.email".
            if database_error.message().contains("users.email") {
                AccountStorageError::EmailTaken
            } else {
                AccountStorageError::UsernameTaken
            }
        }
        error => error.into(),
    }
}

/// Applies the given names (`None` keeps the stored one) in one statement, so concurrent changes of
/// different fields both land, and returns the stored `(username, display_name)`; `None` for an
/// unknown user.
///
/// # Errors
/// `UsernameTaken` when another account has this username (case-insensitive); database errors.
pub async fn update_user_names(
    database: &Database,
    user_id: Snowflake,
    username: Option<&str>,
    display_name: Option<&str>,
) -> Result<Option<(String, String)>, AccountStorageError> {
    sqlx::query_as(
        "UPDATE users SET username = COALESCE(?, username), \
         display_name = COALESCE(?, display_name) WHERE id = ? RETURNING username, display_name",
    )
    .bind(username)
    .bind(display_name)
    .bind(to_sqlite_integer(user_id.raw())?)
    .fetch_optional(database.writer())
    .await
    .map_err(unique_violation_as_taken)
}

/// Replaces `verified_password_hash` with `new_password_hash` and deletes every other session of
/// the user, in one transaction, so no session opened with the old password outlives the change.
/// Changes nothing and returns `false` when the stored hash is no longer the verified one (another
/// change won the race), the account was disabled, or the kept session expired or was revoked.
///
/// # Errors
/// Fails on database errors.
pub async fn replace_password_and_delete_other_sessions(
    database: &Database,
    user_id: Snowflake,
    verified_password_hash: &str,
    new_password_hash: &str,
    kept_session_token_sha256: &SessionTokenSha256,
    now_unix_milliseconds: u64,
) -> Result<bool, AccountStorageError> {
    let user_id = to_sqlite_integer(user_id.raw())?;
    let mut transaction = database.writer().begin().await?;
    let updated = sqlx::query(
        "UPDATE users SET password_hash = ? \
         WHERE id = ? AND password_hash = ? AND disabled_at_unix_milliseconds IS NULL \
         AND EXISTS (SELECT 1 FROM sessions WHERE sessions.user_id = users.id \
         AND sessions.token_sha256 = ? AND sessions.expires_at_unix_milliseconds > ?)",
    )
    .bind(new_password_hash)
    .bind(user_id)
    .bind(verified_password_hash)
    .bind(kept_session_token_sha256.as_slice())
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() == 0 {
        return Ok(false);
    }
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_sha256 != ?")
        .bind(user_id)
        .bind(kept_session_token_sha256.as_slice())
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(true)
}

/// Looks up by username or email, both case-insensitive (`COLLATE NOCASE`).
///
/// # Errors
/// Fails on database errors only; an unknown id is `Ok(None)`.
pub async fn find_user_by_id(
    database: &Database,
    user_id: Snowflake,
) -> Result<Option<StoredUser>, AccountStorageError> {
    let row: Option<StoredUserRow> = sqlx::query_as(concat!(
        "SELECT ",
        select_user_columns!(),
        " FROM users WHERE users.id = ?"
    ))
    .bind(to_sqlite_integer(user_id.raw())?)
    .fetch_optional(database.readers())
    .await?;
    row.map(stored_user_from_row).transpose()
}

/// # Errors
/// Fails on database errors only; an unknown login is `Ok(None)`.
pub async fn find_user_by_login(
    database: &Database,
    login: &str,
) -> Result<Option<StoredUser>, AccountStorageError> {
    let row: Option<StoredUserRow> = sqlx::query_as(concat!(
        "SELECT ",
        select_user_columns!(),
        " FROM users WHERE users.username = ?1 OR users.email = ?1"
    ))
    .bind(login)
    .fetch_optional(database.readers())
    .await?;
    row.map(stored_user_from_row).transpose()
}

/// Inserts the session only while the user is enabled and still has `verified_password_hash`, so a
/// login verified just before a password change cannot outlive it. Returns whether it was inserted.
///
/// # Errors
/// Fails on database errors.
pub async fn insert_session(
    database: &Database,
    session: &NewSession,
    verified_password_hash: &str,
) -> Result<bool, AccountStorageError> {
    let result = sqlx::query(
        "INSERT INTO sessions (id, user_id, token_sha256, device_name, created_at_unix_milliseconds, \
         last_seen_at_unix_milliseconds, expires_at_unix_milliseconds) \
         SELECT ?, id, ?, ?, ?, ?, ? FROM users \
         WHERE id = ? AND password_hash = ? AND disabled_at_unix_milliseconds IS NULL",
    )
    .bind(to_sqlite_integer(session.id.raw())?)
    .bind(session.token_sha256.as_slice())
    .bind(&session.device_name)
    .bind(to_sqlite_integer(session.created_at_unix_milliseconds)?)
    .bind(to_sqlite_integer(session.created_at_unix_milliseconds)?)
    .bind(to_sqlite_integer(session.expires_at_unix_milliseconds)?)
    .bind(to_sqlite_integer(session.user_id.raw())?)
    .bind(verified_password_hash)
    .execute(database.writer())
    .await?;
    Ok(result.rows_affected() == 1)
}

/// The user owning an unexpired session, whether or not the account is disabled (the caller decides).
///
/// # Errors
/// Fails on database errors only.
pub async fn find_session_owner(
    database: &Database,
    token_sha256: &SessionTokenSha256,
    now_unix_milliseconds: u64,
) -> Result<Option<SessionOwner>, AccountStorageError> {
    let row: Option<sqlx::sqlite::SqliteRow> = sqlx::query(concat!(
        "SELECT ",
        select_user_columns!(),
        ", sessions.last_seen_at_unix_milliseconds",
        " FROM sessions JOIN users ON users.id = sessions.user_id",
        " WHERE sessions.token_sha256 = ? AND sessions.expires_at_unix_milliseconds > ?"
    ))
    .bind(token_sha256.as_slice())
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .fetch_optional(database.readers())
    .await?;
    row.map(|row| {
        use sqlx::{FromRow as _, Row as _};
        let last_seen: i64 = row.try_get("last_seen_at_unix_milliseconds")?;
        Ok(SessionOwner {
            user: stored_user_from_row(StoredUserRow::from_row(&row)?)?,
            last_seen_at_unix_milliseconds: u64::try_from(last_seen)
                .map_err(|_| AccountStorageError::IntegerOutOfRange)?,
        })
    })
    .transpose()
}

/// Records that a session was used at `now_unix_milliseconds`; never moves it backwards.
///
/// # Errors
/// Fails on database errors.
pub async fn record_session_use(
    database: &Database,
    token_sha256: &SessionTokenSha256,
    now_unix_milliseconds: u64,
) -> Result<(), AccountStorageError> {
    sqlx::query(
        "UPDATE sessions SET last_seen_at_unix_milliseconds = ?1 \
         WHERE token_sha256 = ?2 AND last_seen_at_unix_milliseconds < ?1",
    )
    .bind(to_sqlite_integer(now_unix_milliseconds)?)
    .bind(token_sha256.as_slice())
    .execute(database.writer())
    .await?;
    Ok(())
}

/// Deletes the sessions expired at `expired_before_unix_milliseconds` and returns how many there were.
///
/// # Errors
/// Fails on database errors.
pub async fn delete_expired_sessions(
    database: &Database,
    expired_before_unix_milliseconds: u64,
) -> Result<u64, AccountStorageError> {
    let result = sqlx::query("DELETE FROM sessions WHERE expires_at_unix_milliseconds <= ?")
        .bind(to_sqlite_integer(expired_before_unix_milliseconds)?)
        .execute(database.writer())
        .await?;
    Ok(result.rows_affected())
}

/// Idempotent: deleting an unknown session is not an error.
///
/// # Errors
/// Fails on database errors.
pub async fn delete_session(
    database: &Database,
    token_sha256: &SessionTokenSha256,
) -> Result<(), AccountStorageError> {
    sqlx::query("DELETE FROM sessions WHERE token_sha256 = ?")
        .bind(token_sha256.as_slice())
        .execute(database.writer())
        .await?;
    Ok(())
}
