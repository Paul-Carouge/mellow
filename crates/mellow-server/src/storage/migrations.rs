//! Embedded, forward-only schema migrations, preceded by an encrypted snapshot whenever an
//! existing database is about to change (ARCHITECTURE.md §4.3, ADR-0003).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sqlx::SqlitePool;
use sqlx::migrate::Migrator;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("cannot create the backup directory {path}")]
    CannotCreateBackupDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("a pre-migration backup already exists at {path}; move it away before retrying")]
    BackupAlreadyExists { path: PathBuf },
    #[error("the pre-migration backup to {path} failed; no migration was applied")]
    BackupFailed {
        path: PathBuf,
        #[source]
        source: sqlx::Error,
    },
    #[error("the backup path {path} is not valid UTF-8, which SQLite cannot use")]
    NonUtf8BackupPath { path: PathBuf },
    #[error("database error while checking migrations")]
    Database(#[from] sqlx::Error),
    #[error("migration failed")]
    Migration(#[from] sqlx::migrate::MigrateError),
}

/// What happened, for the startup log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationOutcome {
    pub applied_versions: Vec<i64>,
    pub backup_path: Option<PathBuf>,
}

/// Applies pending migrations. If the database already holds data, first writes a consistent,
/// encrypted copy to `backup_directory/pre-migration-<first pending version>.db`.
///
/// Always runs sqlx's own checks, even with nothing pending, so a database written by a newer
/// binary (after a rollback) or an edited migration is refused instead of served.
///
/// # Errors
/// Nothing is migrated if the database is in a state sqlx would reject or the backup fails.
pub async fn migrate_with_backup(
    pool: &SqlitePool,
    backup_directory: &Path,
) -> Result<MigrationOutcome, MigrationError> {
    let applied_versions = applied_migration_versions(pool).await?;
    reject_unknown_or_failed_migrations(&applied_versions)?;
    let applied_version_numbers: BTreeSet<i64> = applied_versions
        .iter()
        .map(|applied| applied.version)
        .collect();
    let pending_versions: Vec<i64> = MIGRATOR
        .iter()
        .map(|migration| migration.version)
        .filter(|version| !applied_version_numbers.contains(version))
        .collect();

    if !pending_versions.is_empty() {
        // Warn level, so it shows even with quiet logs: an older server refuses the database
        // from here on, and whoever updates must know before a failure (ADR-0018).
        tracing::warn!(pending_versions = ?pending_versions, "database migration starting");
    }
    let backup_path = match pending_versions.first() {
        Some(&first_pending_version) if database_has_user_tables(pool).await? => {
            Some(write_backup(pool, backup_directory, first_pending_version).await?)
        }
        _ => None,
    };
    MIGRATOR.run(pool).await?;
    Ok(MigrationOutcome {
        applied_versions: pending_versions,
        backup_path,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AppliedMigration {
    version: i64,
    succeeded: bool,
}

async fn applied_migration_versions(
    pool: &SqlitePool,
) -> Result<Vec<AppliedMigration>, sqlx::Error> {
    let has_migration_table: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = '_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await?;
    if !has_migration_table {
        return Ok(Vec::new());
    }
    let rows: Vec<(i64, bool)> = sqlx::query_as("SELECT version, success FROM _sqlx_migrations")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(version, succeeded)| AppliedMigration { version, succeeded })
        .collect())
}

/// The same checks `Migrator::run` makes, done before the backup so a doomed run writes nothing.
fn reject_unknown_or_failed_migrations(
    applied_versions: &[AppliedMigration],
) -> Result<(), sqlx::migrate::MigrateError> {
    for applied in applied_versions {
        if !applied.succeeded {
            return Err(sqlx::migrate::MigrateError::Dirty(applied.version));
        }
        if !MIGRATOR
            .iter()
            .any(|migration| migration.version == applied.version)
        {
            return Err(sqlx::migrate::MigrateError::VersionMissing(applied.version));
        }
    }
    Ok(())
}

async fn database_has_user_tables(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%')",
    )
    .fetch_one(pool)
    .await
}

async fn write_backup(
    pool: &SqlitePool,
    backup_directory: &Path,
    first_pending_version: i64,
) -> Result<PathBuf, MigrationError> {
    crate::storage::create_private_directory(backup_directory).map_err(|source| {
        MigrationError::CannotCreateBackupDirectory {
            path: backup_directory.to_path_buf(),
            source,
        }
    })?;
    let backup_path = backup_directory.join(format!("pre-migration-{first_pending_version}.db"));
    if backup_path.exists() {
        return Err(MigrationError::BackupAlreadyExists { path: backup_path });
    }
    // Written under a temporary name and renamed once complete, so an interrupted backup
    // (full disk, killed process) never looks like a valid one.
    let partial_path = backup_path.with_extension("db.partial");
    let backup_failed = |source| MigrationError::BackupFailed {
        path: backup_path.clone(),
        source,
    };
    if partial_path.exists() {
        std::fs::remove_file(&partial_path)
            .map_err(|error| backup_failed(sqlx::Error::Io(error)))?;
    }
    let partial_path_text = partial_path
        .to_str()
        .ok_or_else(|| MigrationError::NonUtf8BackupPath {
            path: partial_path.clone(),
        })?
        .to_owned();
    // `VACUUM INTO` writes a consistent copy; SQLCipher encrypts it with the same key. The path is
    // bound as a parameter, never spliced into the SQL.
    sqlx::query("VACUUM INTO ?")
        .bind(partial_path_text)
        .execute(pool)
        .await
        .map_err(backup_failed)?;
    std::fs::rename(&partial_path, &backup_path)
        .map_err(|error| backup_failed(sqlx::Error::Io(error)))?;
    Ok(backup_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{DatabaseKey, open_encrypted_database};

    const SQLITE_PLAINTEXT_HEADER: &[u8] = b"SQLite format 3\0";

    fn test_key() -> DatabaseKey {
        DatabaseKey::from_bytes([9; 32])
    }

    #[tokio::test]
    async fn fresh_database_is_migrated_without_a_backup_and_rerunning_is_a_no_op() {
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        let backups = directory.path().join("backups");

        let first_run = migrate_with_backup(&pool, &backups).await.unwrap();
        let every_version: Vec<i64> = MIGRATOR.iter().map(|migration| migration.version).collect();
        assert_eq!(first_run.applied_versions, every_version);
        assert_eq!(first_run.backup_path, None);

        let second_run = migrate_with_backup(&pool, &backups).await.unwrap();
        assert_eq!(second_run.applied_versions, Vec::<i64>::new());
        pool.close().await;
        assert!(!backups.exists());
    }

    #[tokio::test]
    async fn channel_deletions_find_read_states_and_attachments_by_index() {
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        migrate_with_backup(&pool, &directory.path().join("backups"))
            .await
            .unwrap();
        let mut plans = Vec::new();
        // What the cascades of a channel deletion look up.
        for lookup in [
            "EXPLAIN QUERY PLAN SELECT 1 FROM read_states WHERE channel_id = 1",
            "EXPLAIN QUERY PLAN SELECT 1 FROM attachments WHERE channel_id = 1",
        ] {
            let rows: Vec<(i64, i64, i64, String)> =
                sqlx::query_as(lookup).fetch_all(&pool).await.unwrap();
            plans.push(
                rows.into_iter()
                    .map(|row| row.3)
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        pool.close().await;

        assert!(
            plans[0].contains("read_states_by_channel_id"),
            "{}",
            plans[0]
        );
        assert!(
            plans[1].contains("attachments_by_channel_id"),
            "{}",
            plans[1]
        );
    }

    #[tokio::test]
    async fn existing_data_is_backed_up_encrypted_before_migrating() {
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        sqlx::query("CREATE TABLE legacy (value TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO legacy VALUES ('kept')")
            .execute(&pool)
            .await
            .unwrap();

        let outcome = migrate_with_backup(&pool, &directory.path().join("backups"))
            .await
            .unwrap();
        pool.close().await;

        let backup_path = outcome.backup_path.unwrap();
        assert!(backup_path.ends_with("pre-migration-1.db"));
        assert!(!backup_path.with_extension("db.partial").exists());
        assert!(
            !std::fs::read(&backup_path)
                .unwrap()
                .starts_with(SQLITE_PLAINTEXT_HEADER)
        );
        let backup = open_encrypted_database(&backup_path, &test_key())
            .await
            .unwrap();
        let value: String = sqlx::query_scalar("SELECT value FROM legacy")
            .fetch_one(&backup)
            .await
            .unwrap();
        backup.close().await;
        assert_eq!(value, "kept");
    }

    #[tokio::test]
    async fn messages_written_before_nonces_were_stored_survive_the_migration() {
        const LAST_VERSION_WITHOUT_NONCES: i64 = 3;
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_NONCES, &pool)
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (2, 'Club', 1, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (3, 2, 'text', 'general', 0); \
             INSERT INTO messages (id, channel_id, author_id, content) \
             VALUES (4, 3, 1, 'hello'), (5, 3, 1, 'again');",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = migrate_with_backup(&pool, &directory.path().join("backups"))
            .await
            .unwrap();
        let messages: Vec<(i64, String, Option<String>)> =
            sqlx::query_as("SELECT id, content, nonce FROM messages ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        pool.close().await;

        assert_eq!(
            outcome.applied_versions.first(),
            Some(&(LAST_VERSION_WITHOUT_NONCES + 1))
        );
        assert_eq!(
            messages,
            [(4, "hello".to_owned(), None), (5, "again".to_owned(), None)]
        );
    }

    #[tokio::test]
    async fn guilds_created_before_named_roles_keep_their_everyone_role() {
        const LAST_VERSION_WITHOUT_MEMBER_ROLES: i64 = 4;
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_MEMBER_ROLES, &pool)
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0), \
             (2, 'bob', 'Bob', 'bob@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0); \
             INSERT INTO roles (id, guild_id, name, permissions, position) \
             VALUES (10, 10, '@everyone', 1610, 0); \
             INSERT INTO guild_members (guild_id, user_id, joined_at_unix_milliseconds) \
             VALUES (10, 1, 0), (10, 2, 0);",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = migrate_with_backup(&pool, &directory.path().join("backups"))
            .await
            .unwrap();
        let everyone: (String, i64, i64, i64) =
            sqlx::query_as("SELECT name, permissions, position, color FROM roles WHERE id = 10")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::raw_sql(
            "INSERT INTO roles (id, guild_id, name, permissions, position) \
             VALUES (11, 10, 'Moderator', 0, 1); \
             INSERT INTO member_roles (guild_id, user_id, role_id) VALUES (10, 2, 11); \
             DELETE FROM guild_members WHERE guild_id = 10 AND user_id = 2;",
        )
        .execute(&pool)
        .await
        .unwrap();
        let assignments_after_leaving: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM member_roles")
                .fetch_one(&pool)
                .await
                .unwrap();
        let assignment_of_non_member =
            sqlx::query("INSERT INTO member_roles (guild_id, user_id, role_id) VALUES (10, 2, 11)")
                .execute(&pool)
                .await;
        pool.close().await;

        assert_eq!(
            outcome.applied_versions.first(),
            Some(&(LAST_VERSION_WITHOUT_MEMBER_ROLES + 1))
        );
        // Migration 0012 adds ADD_REACTIONS (1 << 16) to every @everyone role.
        assert_eq!(everyone, ("@everyone".to_owned(), 0x064a | 0x1_0000, 0, 0));
        assert_eq!(
            assignments_after_leaving, 0,
            "leaving drops the member's roles"
        );
        assert!(assignment_of_non_member.is_err(), "only members hold roles");
    }

    #[tokio::test]
    async fn channels_created_before_overwrites_keep_their_messages_and_accept_overwrites() {
        const LAST_VERSION_WITHOUT_OVERWRITES: i64 = 5;
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_OVERWRITES, &pool)
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0); \
             INSERT INTO roles (id, guild_id, name, permissions, position) \
             VALUES (10, 10, '@everyone', 1610, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (20, 10, 'text', 'general', 0); \
             INSERT INTO messages (id, channel_id, author_id, content) VALUES (30, 20, 1, 'hello');",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = migrate_with_backup(&pool, &directory.path().join("backups"))
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO channel_permission_overwrites (channel_id, target_id, target_kind, allow, deny) \
             VALUES (20, 10, 'role', 0, 2)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let both_allowed_and_denied = sqlx::query(
            "INSERT INTO channel_permission_overwrites (channel_id, target_id, target_kind, allow, deny) \
             VALUES (20, 1, 'member', 2, 2)",
        )
        .execute(&pool)
        .await;
        sqlx::query("DELETE FROM channels WHERE id = 20")
            .execute(&pool)
            .await
            .unwrap();
        let overwrites_left: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM channel_permission_overwrites")
                .fetch_one(&pool)
                .await
                .unwrap();
        pool.close().await;

        assert_eq!(
            outcome.applied_versions.first(),
            Some(&(LAST_VERSION_WITHOUT_OVERWRITES + 1))
        );
        assert!(
            both_allowed_and_denied.is_err(),
            "allow and deny never share a bit"
        );
        assert_eq!(
            overwrites_left, 0,
            "deleting a channel drops its overwrites"
        );
    }

    #[tokio::test]
    async fn bans_are_dropped_with_their_guild() {
        const LAST_VERSION_WITHOUT_BANS: i64 = 6;
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_BANS, &pool)
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0), \
             (2, 'bob', 'Bob', 'bob@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0), (11, 'Other', 1, 0);",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = migrate_with_backup(&pool, &directory.path().join("backups"))
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO bans (guild_id, user_id, reason, banned_by, created_at_unix_milliseconds) \
             VALUES (10, 2, 'spam', 1, 0), (11, 2, '', 1, 0); \
             DELETE FROM guilds WHERE id = 11;",
        )
        .execute(&pool)
        .await
        .unwrap();
        let bans: Vec<(i64, String)> =
            sqlx::query_as("SELECT guild_id, reason FROM bans ORDER BY guild_id")
                .fetch_all(&pool)
                .await
                .unwrap();
        pool.close().await;

        assert_eq!(
            outcome.applied_versions.first(),
            Some(&(LAST_VERSION_WITHOUT_BANS + 1))
        );
        assert_eq!(
            bans,
            [(10, "spam".to_owned())],
            "a deleted guild drops its bans"
        );
    }

    #[tokio::test]
    async fn audit_entries_follow_their_guild_and_outlive_their_actor() {
        const LAST_VERSION_WITHOUT_AUDIT_LOG: i64 = 7;
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_AUDIT_LOG, &pool)
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0), \
             (2, 'bob', 'Bob', 'bob@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0), (11, 'Other', 1, 0);",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = migrate_with_backup(&pool, &directory.path().join("backups"))
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO audit_log (id, guild_id, actor_id, action, target_id) \
             VALUES (100, 10, 2, 'member_kick', 1), (101, 11, 1, 'role_create', 5); \
             DELETE FROM guilds WHERE id = 11; \
             DELETE FROM users WHERE id = 2;",
        )
        .execute(&pool)
        .await
        .unwrap();
        let entries: Vec<(i64, Option<i64>)> =
            sqlx::query_as("SELECT id, actor_id FROM audit_log ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        pool.close().await;

        assert_eq!(
            outcome.applied_versions.first(),
            Some(&(LAST_VERSION_WITHOUT_AUDIT_LOG + 1))
        );
        assert_eq!(
            entries,
            [(100, None)],
            "kept without its deleted actor; gone with its guild"
        );
    }

    #[tokio::test]
    async fn guild_channels_and_their_messages_survive_and_conversations_follow_their_users() {
        const LAST_VERSION_WITHOUT_CONVERSATIONS: i64 = 8;
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_CONVERSATIONS, &pool)
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0), \
             (2, 'bob', 'Bob', 'bob@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (20, 10, 'text', 'general', 0); \
             INSERT INTO messages (id, channel_id, author_id, content) \
             VALUES (30, 20, 1, 'hello');",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = migrate_with_backup(&pool, &directory.path().join("backups"))
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (21, NULL, 'direct_message', '', 0); \
             INSERT INTO direct_message_participants (channel_id, user_id) VALUES (21, 1), (21, 2); \
             INSERT INTO direct_message_pairs (lower_user_id, higher_user_id, channel_id) \
             VALUES (1, 2, 21); \
             DELETE FROM users WHERE id = 2;",
        )
        .execute(&pool)
        .await
        .unwrap();
        let guild_messages: Vec<(i64,)> =
            sqlx::query_as("SELECT id FROM messages WHERE channel_id = 20")
                .fetch_all(&pool)
                .await
                .unwrap();
        let participants: Vec<(i64,)> =
            sqlx::query_as("SELECT user_id FROM direct_message_participants")
                .fetch_all(&pool)
                .await
                .unwrap();
        let pairs: Vec<(i64,)> = sqlx::query_as("SELECT channel_id FROM direct_message_pairs")
            .fetch_all(&pool)
            .await
            .unwrap();
        pool.close().await;

        assert_eq!(
            outcome.applied_versions.first(),
            Some(&(LAST_VERSION_WITHOUT_CONVERSATIONS + 1))
        );
        assert_eq!(guild_messages, [(30,)]);
        assert_eq!(
            participants,
            [(1,)],
            "a deleted user leaves the conversation"
        );
        assert!(pairs.is_empty(), "the pair goes with either user");
    }

    #[tokio::test]
    async fn existing_guilds_and_channels_have_no_icon_and_theirs_go_with_them() {
        const LAST_VERSION_WITHOUT_ICONS: i64 = 13;
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_ICONS, &pool)
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0), (11, 'Other', 1, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (20, 10, 'text', 'general', 0), (21, 10, 'text', 'random', 1);",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = migrate_with_backup(&pool, &directory.path().join("backups"))
            .await
            .unwrap();
        let guilds: Vec<(Option<i64>, i64)> =
            sqlx::query_as("SELECT icon_id, icon_is_animated FROM guilds ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        let channels: Vec<(Option<i64>,)> =
            sqlx::query_as("SELECT icon_id FROM channels ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        sqlx::raw_sql(
            "INSERT INTO guild_images (id, guild_id, channel_id, still_id, size_bytes, \
             created_at_unix_milliseconds) VALUES (50, 10, NULL, 51, 9, 0), \
             (52, 10, 20, NULL, 9, 0), (53, 10, 21, NULL, 9, 0), (54, 11, NULL, NULL, 9, 0); \
             DELETE FROM channels WHERE id = 20; \
             DELETE FROM guilds WHERE id = 11;",
        )
        .execute(&pool)
        .await
        .unwrap();
        let images: Vec<(i64,)> = sqlx::query_as("SELECT id FROM guild_images ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        pool.close().await;

        assert_eq!(
            outcome.applied_versions.first(),
            Some(&(LAST_VERSION_WITHOUT_ICONS + 1))
        );
        assert_eq!(guilds, [(None, 0), (None, 0)]);
        assert_eq!(channels, [(None,), (None,)]);
        assert_eq!(
            images,
            [(50,), (53,)],
            "icons go with their channel or guild"
        );
    }

    #[tokio::test]
    async fn existing_users_have_no_profile_images_and_theirs_go_with_them() {
        const LAST_VERSION_WITHOUT_PROFILE_IMAGES: i64 = 10;
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_PROFILE_IMAGES, &pool)
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0), \
             (2, 'bob', 'Bob', 'bob@example.org', 'x', 0);",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = migrate_with_backup(&pool, &directory.path().join("backups"))
            .await
            .unwrap();
        let before: Vec<(Option<i64>, i64, Option<i64>, i64)> = sqlx::query_as(
            "SELECT avatar_id, avatar_is_animated, banner_id, banner_is_animated FROM users",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(
            "INSERT INTO profile_images (id, user_id, still_id, size_bytes, \
             created_at_unix_milliseconds) VALUES (50, 1, 51, 9, 0), (52, 2, NULL, 9, 0); \
             UPDATE users SET avatar_id = 50, avatar_is_animated = 1 WHERE id = 1; \
             DELETE FROM users WHERE id = 1;",
        )
        .execute(&pool)
        .await
        .unwrap();
        let images: Vec<(i64,)> = sqlx::query_as("SELECT id FROM profile_images")
            .fetch_all(&pool)
            .await
            .unwrap();
        pool.close().await;

        assert_eq!(
            outcome.applied_versions.first(),
            Some(&(LAST_VERSION_WITHOUT_PROFILE_IMAGES + 1))
        );
        assert_eq!(before, [(None, 0, None, 0), (None, 0, None, 0)]);
        assert_eq!(images, [(52,)], "a deleted account takes its pictures");
    }

    #[tokio::test]
    async fn attachments_go_with_their_message_and_keep_their_channel_messages() {
        const LAST_VERSION_WITHOUT_ATTACHMENTS: i64 = 9;
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_ATTACHMENTS, &pool)
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (20, 10, 'text', 'general', 0); \
             INSERT INTO messages (id, channel_id, author_id, content) \
             VALUES (30, 20, 1, 'hello'), (31, 20, 1, 'files');",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = migrate_with_backup(&pool, &directory.path().join("backups"))
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO attachments (id, channel_id, uploader_id, message_id, file_name, \
             content_type, size_bytes, created_at_unix_milliseconds) \
             VALUES (40, 20, 1, 31, 'a.png', 'image/png', 3, 0), \
             (41, 20, 1, NULL, 'b.txt', 'application/octet-stream', 3, 0); \
             DELETE FROM messages WHERE id = 31;",
        )
        .execute(&pool)
        .await
        .unwrap();
        let attachments: Vec<(i64,)> = sqlx::query_as("SELECT id FROM attachments ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        let messages: Vec<(i64,)> = sqlx::query_as("SELECT id FROM messages")
            .fetch_all(&pool)
            .await
            .unwrap();
        pool.close().await;

        assert_eq!(
            outcome.applied_versions.first(),
            Some(&(LAST_VERSION_WITHOUT_ATTACHMENTS + 1))
        );
        assert_eq!(
            attachments,
            [(41,)],
            "a deleted message takes its attachments"
        );
        assert_eq!(messages, [(30,)]);
    }

    #[tokio::test]
    async fn existing_everyone_roles_may_react_and_reactions_go_with_their_message() {
        const LAST_VERSION_WITHOUT_REACTIONS: i64 = 11;
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_REACTIONS, &pool)
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0); \
             INSERT INTO roles (id, guild_id, name, permissions, position, color) \
             VALUES (10, 10, '@everyone', 1086, 0, 0), (11, 10, 'Muted', 0, 1, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (20, 10, 'text', 'general', 0), (21, 10, 'text', 'announcements', 1); \
             INSERT INTO channel_permission_overwrites \
             (channel_id, target_id, target_kind, allow, deny) \
             VALUES (21, 10, 'role', 0, 4), (21, 11, 'role', 4, 0), (20, 1, 'member', 0, 6), \
             (20, 11, 'role', 0, 2); \
             INSERT INTO messages (id, channel_id, author_id, content) \
             VALUES (30, 20, 1, 'hello'), (31, 20, 1, 'again');",
        )
        .execute(&pool)
        .await
        .unwrap();

        let outcome = migrate_with_backup(&pool, &directory.path().join("backups"))
            .await
            .unwrap();
        let permissions: Vec<(i64, i64)> =
            sqlx::query_as("SELECT id, permissions FROM roles ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        let overwrites: Vec<(i64, i64, i64, i64)> = sqlx::query_as(
            "SELECT channel_id, target_id, allow, deny FROM channel_permission_overwrites \
             ORDER BY channel_id, target_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let pinned: Vec<(i64, Option<i64>)> =
            sqlx::query_as("SELECT id, pinned_at_unix_milliseconds FROM messages ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        sqlx::raw_sql(
            "INSERT INTO message_reactions (message_id, user_id, emoji) \
             VALUES (30, 1, '👍'), (31, 1, '👍'); \
             DELETE FROM messages WHERE id = 31;",
        )
        .execute(&pool)
        .await
        .unwrap();
        let reactions: Vec<(i64,)> = sqlx::query_as("SELECT message_id FROM message_reactions")
            .fetch_all(&pool)
            .await
            .unwrap();
        let duplicate = sqlx::query(
            "INSERT INTO message_reactions (message_id, user_id, emoji) VALUES (30, 1, '👍')",
        )
        .execute(&pool)
        .await;
        pool.close().await;

        assert_eq!(
            outcome.applied_versions.first(),
            Some(&(LAST_VERSION_WITHOUT_REACTIONS + 1))
        );
        assert_eq!(
            permissions,
            [(10, 0x043e | 0x1_0000), (11, 0)],
            "only @everyone gains ADD_REACTIONS"
        );
        assert_eq!(
            overwrites,
            [
                (20, 1, 0, 0x6 | 0x1_0000),
                (20, 11, 0, 0x2),
                (21, 10, 0, 0x4 | 0x1_0000),
                (21, 11, 0x4, 0),
            ],
            "channels closed to sending stay closed to new reactions"
        );
        assert_eq!(pinned, [(30, None), (31, None)]);
        assert_eq!(reactions, [(30,)], "a deleted message takes its reactions");
        assert!(duplicate.is_err(), "one reaction per user and emoji");
    }

    #[tokio::test]
    async fn a_database_from_a_newer_binary_is_refused_even_with_nothing_pending() {
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        let backups = directory.path().join("backups");
        migrate_with_backup(&pool, &backups).await.unwrap();
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
             VALUES (999, 'from a newer binary', 1, x'00', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let result = migrate_with_backup(&pool, &backups).await;
        pool.close().await;
        assert!(
            matches!(
                result,
                Err(MigrationError::Migration(
                    sqlx::migrate::MigrateError::VersionMissing(999)
                ))
            ),
            "{result:?}"
        );
        assert!(!backups.exists(), "a refused run must not write a backup");
    }

    #[tokio::test]
    async fn an_existing_backup_is_never_overwritten_and_blocks_the_migration() {
        let directory = tempfile::tempdir().unwrap();
        let pool = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        sqlx::query("CREATE TABLE legacy (value TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        let backups = directory.path().join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::write(backups.join("pre-migration-1.db"), b"older backup").unwrap();

        let result = migrate_with_backup(&pool, &backups).await;
        let has_users_table: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE name = 'users')")
                .fetch_one(&pool)
                .await
                .unwrap();
        pool.close().await;

        assert!(matches!(
            result,
            Err(MigrationError::BackupAlreadyExists { .. })
        ));
        assert!(!has_users_table, "no migration may run without a backup");
        assert_eq!(
            std::fs::read(backups.join("pre-migration-1.db")).unwrap(),
            b"older backup"
        );
    }
}
