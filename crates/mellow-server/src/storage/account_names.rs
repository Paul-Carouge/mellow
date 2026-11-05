//! Account names and friend codes (migration 0041, ADR-0048).

use std::collections::HashSet;

use mellow_protocol::Snowflake;

use super::Database;
use crate::domain::account_names::{
    account_name_from_old_names, account_name_key, generate_friend_code, numbered_account_name,
};
use crate::storage::accounts::{AccountStorageError, unique_violation_as_taken};
use crate::storage::sqlite_integers::{convert_integer, to_snowflake, to_sqlite_integer};

/// Gives every account still without one a unique name and a friend code, in one transaction:
/// accounts made before migration 0041, the first time this binary starts. Each keeps its display
/// name (without the characters names may no longer hold) when no older account took it, else gets
/// " 2", " 3"… after it, oldest first. Returns how many accounts changed; a no-op afterwards.
///
/// # Errors
/// Fails on database errors or when the operating system has no randomness to give.
pub(crate) async fn give_names_and_friend_codes_to_older_accounts(
    database: &Database,
) -> Result<u64, AccountStorageError> {
    let mut transaction = database.writer().begin().await?;
    let unnamed: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT id, username, display_name FROM users WHERE name_key IS NULL \
         ORDER BY created_at_unix_milliseconds, id",
    )
    .fetch_all(&mut *transaction)
    .await?;
    let uncoded: Vec<i64> = sqlx::query_scalar("SELECT id FROM users WHERE friend_code IS NULL")
        .fetch_all(&mut *transaction)
        .await?;
    if unnamed.is_empty() && uncoded.is_empty() {
        return Ok(0);
    }
    let mut taken_keys: HashSet<String> =
        sqlx::query_scalar("SELECT name_key FROM users WHERE name_key IS NOT NULL")
            .fetch_all(&mut *transaction)
            .await?
            .into_iter()
            .collect();
    // Old usernames step aside first, so a name never meets another account's old username.
    sqlx::query("UPDATE users SET username = '#' || id WHERE name_key IS NULL")
        .execute(&mut *transaction)
        .await?;
    for (id, username, display_name) in &unnamed {
        let wanted = account_name_from_old_names(display_name, username);
        let mut name = wanted.clone();
        let mut number = 2;
        while taken_keys.contains(&account_name_key(&name)) {
            name = numbered_account_name(&wanted, number);
            number += 1;
        }
        let key = account_name_key(&name);
        sqlx::query(
            "UPDATE users SET username = ?1, display_name = ?1, name_key = ?2 WHERE id = ?3",
        )
        .bind(&name)
        .bind(&key)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        taken_keys.insert(key);
    }
    let mut taken_codes: HashSet<i64> =
        sqlx::query_scalar("SELECT friend_code FROM users WHERE friend_code IS NOT NULL")
            .fetch_all(&mut *transaction)
            .await?
            .into_iter()
            .collect();
    for id in &uncoded {
        let code = loop {
            let code: i64 = convert_integer(generate_friend_code()?)?;
            if taken_codes.insert(code) {
                break code;
            }
        };
        sqlx::query("UPDATE users SET friend_code = ? WHERE id = ?")
            .bind(code)
            .bind(id)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(u64::try_from(unnamed.len().max(uncoded.len())).unwrap_or(u64::MAX))
}

/// Whether an account other than `user_id` holds a name with this key; disabled accounts keep
/// theirs.
///
/// # Errors
/// Fails on database errors.
pub(crate) async fn is_name_key_taken_by_another_account(
    database: &Database,
    name_key: &str,
    user_id: Snowflake,
) -> Result<bool, AccountStorageError> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE name_key = ? AND id != ?)")
            .bind(name_key)
            .bind(to_sqlite_integer(user_id.raw())?)
            .fetch_one(database.readers())
            .await?,
    )
}

/// Gives the account its new name (the deprecated `username` follows it). Returns whether the
/// account exists.
///
/// # Errors
/// `UsernameTaken` when another account holds a name with this key; database errors.
pub(crate) async fn rename_account(
    database: &Database,
    user_id: Snowflake,
    name: &str,
    name_key: &str,
) -> Result<bool, AccountStorageError> {
    let result = sqlx::query(
        "UPDATE users SET username = ?1, display_name = ?1, name_key = ?2 WHERE id = ?3",
    )
    .bind(name)
    .bind(name_key)
    .bind(to_sqlite_integer(user_id.raw())?)
    .execute(database.writer())
    .await
    .map_err(unique_violation_as_taken)?;
    Ok(result.rows_affected() > 0)
}

/// # Errors
/// Fails on database errors only; nobody with this key is `Ok(None)`.
pub(crate) async fn find_enabled_user_id_by_name_key(
    database: &Database,
    name_key: &str,
) -> Result<Option<Snowflake>, AccountStorageError> {
    let id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM users WHERE name_key = ? AND disabled_at_unix_milliseconds IS NULL",
    )
    .bind(name_key)
    .fetch_optional(database.readers())
    .await?;
    Ok(id.map(to_snowflake).transpose()?)
}

/// # Errors
/// Fails on database errors only; nobody with this code is `Ok(None)`.
pub(crate) async fn find_enabled_user_id_by_friend_code(
    database: &Database,
    friend_code: u64,
) -> Result<Option<Snowflake>, AccountStorageError> {
    let id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM users WHERE friend_code = ? AND disabled_at_unix_milliseconds IS NULL",
    )
    .bind(to_sqlite_integer(friend_code)?)
    .fetch_optional(database.readers())
    .await?;
    Ok(id.map(to_snowflake).transpose()?)
}
