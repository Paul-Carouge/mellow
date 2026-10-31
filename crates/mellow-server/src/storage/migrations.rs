//! Embedded, forward-only schema migrations, preceded by an encrypted snapshot whenever an
//! existing database is about to change (ARCHITECTURE.md §4.3, ADR-0003).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sqlx::SqlitePool;

use super::Database;
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
    database: &Database,
    backup_directory: &Path,
) -> Result<MigrationOutcome, MigrationError> {
    let pool = database.writer();
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
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        let backups = directory.path().join("backups");

        let first_run = migrate_with_backup(&database, &backups).await.unwrap();
        let every_version: Vec<i64> = MIGRATOR.iter().map(|migration| migration.version).collect();
        assert_eq!(first_run.applied_versions, every_version);
        assert_eq!(first_run.backup_path, None);

        let second_run = migrate_with_backup(&database, &backups).await.unwrap();
        assert_eq!(second_run.applied_versions, Vec::<i64>::new());
        database.close().await;
        assert!(!backups.exists());
    }

    #[tokio::test]
    async fn channel_deletions_find_read_states_and_attachments_by_index() {
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let mut plans = Vec::new();
        // What the cascades of a channel deletion look up.
        for lookup in [
            "EXPLAIN QUERY PLAN SELECT 1 FROM read_states WHERE channel_id = 1",
            "EXPLAIN QUERY PLAN SELECT 1 FROM attachments WHERE channel_id = 1",
        ] {
            let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(lookup)
                .fetch_all(database.writer())
                .await
                .unwrap();
            plans.push(
                rows.into_iter()
                    .map(|row| row.3)
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        database.close().await;

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
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        sqlx::query("CREATE TABLE legacy (value TEXT)")
            .execute(database.writer())
            .await
            .unwrap();
        sqlx::query("INSERT INTO legacy VALUES ('kept')")
            .execute(database.writer())
            .await
            .unwrap();

        let outcome = migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        database.close().await;

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
            .fetch_one(backup.writer())
            .await
            .unwrap();
        backup.close().await;
        assert_eq!(value, "kept");
    }

    #[tokio::test]
    async fn messages_written_before_nonces_were_stored_survive_the_migration() {
        const LAST_VERSION_WITHOUT_NONCES: i64 = 3;
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_NONCES, database.writer())
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
        .execute(database.writer())
        .await
        .unwrap();

        let outcome = migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let messages: Vec<(i64, String, Option<String>)> =
            sqlx::query_as("SELECT id, content, nonce FROM messages ORDER BY id")
                .fetch_all(database.writer())
                .await
                .unwrap();
        database.close().await;

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
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_MEMBER_ROLES, database.writer())
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
        .execute(database.writer())
        .await
        .unwrap();

        let outcome = migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let everyone: (String, i64, i64, i64) =
            sqlx::query_as("SELECT name, permissions, position, color FROM roles WHERE id = 10")
                .fetch_one(database.writer())
                .await
                .unwrap();
        sqlx::raw_sql(
            "INSERT INTO roles (id, guild_id, name, permissions, position) \
             VALUES (11, 10, 'Moderator', 0, 1); \
             INSERT INTO member_roles (guild_id, user_id, role_id) VALUES (10, 2, 11); \
             DELETE FROM guild_members WHERE guild_id = 10 AND user_id = 2;",
        )
        .execute(database.writer())
        .await
        .unwrap();
        let assignments_after_leaving: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM member_roles")
                .fetch_one(database.writer())
                .await
                .unwrap();
        let assignment_of_non_member =
            sqlx::query("INSERT INTO member_roles (guild_id, user_id, role_id) VALUES (10, 2, 11)")
                .execute(database.writer())
                .await;
        database.close().await;

        assert_eq!(
            outcome.applied_versions.first(),
            Some(&(LAST_VERSION_WITHOUT_MEMBER_ROLES + 1))
        );
        // Migrations 0012 and 0022 add ADD_REACTIONS (1 << 16), CONNECT and SPEAK (1 << 18 and
        // 1 << 19) to every @everyone role.
        assert_eq!(
            everyone,
            ("@everyone".to_owned(), 0x064a | 0x1_0000 | 0xC_0000, 0, 0)
        );
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
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_OVERWRITES, database.writer())
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
        .execute(database.writer())
        .await
        .unwrap();

        let outcome = migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO channel_permission_overwrites (channel_id, target_id, target_kind, allow, deny) \
             VALUES (20, 10, 'role', 0, 2)",
        )
        .execute(database.writer())
        .await
        .unwrap();
        let both_allowed_and_denied = sqlx::query(
            "INSERT INTO channel_permission_overwrites (channel_id, target_id, target_kind, allow, deny) \
             VALUES (20, 1, 'member', 2, 2)",
        )
        .execute(database.writer())
        .await;
        sqlx::query("DELETE FROM channels WHERE id = 20")
            .execute(database.writer())
            .await
            .unwrap();
        let overwrites_left: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM channel_permission_overwrites")
                .fetch_one(database.writer())
                .await
                .unwrap();
        database.close().await;

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
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_BANS, database.writer())
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0), \
             (2, 'bob', 'Bob', 'bob@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0), (11, 'Other', 1, 0);",
        )
        .execute(database.writer())
        .await
        .unwrap();

        let outcome = migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO bans (guild_id, user_id, reason, banned_by, created_at_unix_milliseconds) \
             VALUES (10, 2, 'spam', 1, 0), (11, 2, '', 1, 0); \
             DELETE FROM guilds WHERE id = 11;",
        )
        .execute(database.writer())
        .await
        .unwrap();
        let bans: Vec<(i64, String)> =
            sqlx::query_as("SELECT guild_id, reason FROM bans ORDER BY guild_id")
                .fetch_all(database.writer())
                .await
                .unwrap();
        database.close().await;

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
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_AUDIT_LOG, database.writer())
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0), \
             (2, 'bob', 'Bob', 'bob@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0), (11, 'Other', 1, 0);",
        )
        .execute(database.writer())
        .await
        .unwrap();

        let outcome = migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO audit_log (id, guild_id, actor_id, action, target_id) \
             VALUES (100, 10, 2, 'member_kick', 1), (101, 11, 1, 'role_create', 5); \
             DELETE FROM guilds WHERE id = 11; \
             DELETE FROM users WHERE id = 2;",
        )
        .execute(database.writer())
        .await
        .unwrap();
        let entries: Vec<(i64, Option<i64>)> =
            sqlx::query_as("SELECT id, actor_id FROM audit_log ORDER BY id")
                .fetch_all(database.writer())
                .await
                .unwrap();
        database.close().await;

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
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_CONVERSATIONS, database.writer())
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
        .execute(database.writer())
        .await
        .unwrap();

        let outcome = migrate_with_backup(&database, &directory.path().join("backups"))
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
        .execute(database.writer())
        .await
        .unwrap();
        let guild_messages: Vec<(i64,)> =
            sqlx::query_as("SELECT id FROM messages WHERE channel_id = 20")
                .fetch_all(database.writer())
                .await
                .unwrap();
        let participants: Vec<(i64,)> =
            sqlx::query_as("SELECT user_id FROM direct_message_participants")
                .fetch_all(database.writer())
                .await
                .unwrap();
        let pairs: Vec<(i64,)> = sqlx::query_as("SELECT channel_id FROM direct_message_pairs")
            .fetch_all(database.writer())
            .await
            .unwrap();
        database.close().await;

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
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_ICONS, database.writer())
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
        .execute(database.writer())
        .await
        .unwrap();

        let outcome = migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let guilds: Vec<(Option<i64>, i64)> =
            sqlx::query_as("SELECT icon_id, icon_is_animated FROM guilds ORDER BY id")
                .fetch_all(database.writer())
                .await
                .unwrap();
        let channels: Vec<(Option<i64>,)> =
            sqlx::query_as("SELECT icon_id FROM channels ORDER BY id")
                .fetch_all(database.writer())
                .await
                .unwrap();
        sqlx::raw_sql(
            "INSERT INTO guild_images (id, guild_id, channel_id, still_id, size_bytes, \
             created_at_unix_milliseconds) VALUES (50, 10, NULL, 51, 9, 0), \
             (52, 10, 20, NULL, 9, 0), (53, 10, 21, NULL, 9, 0), (54, 11, NULL, NULL, 9, 0); \
             DELETE FROM channels WHERE id = 20; \
             DELETE FROM guilds WHERE id = 11;",
        )
        .execute(database.writer())
        .await
        .unwrap();
        let images: Vec<(i64,)> = sqlx::query_as("SELECT id FROM guild_images ORDER BY id")
            .fetch_all(database.writer())
            .await
            .unwrap();
        database.close().await;

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
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_PROFILE_IMAGES, database.writer())
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0), \
             (2, 'bob', 'Bob', 'bob@example.org', 'x', 0);",
        )
        .execute(database.writer())
        .await
        .unwrap();

        let outcome = migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let before: Vec<(Option<i64>, i64, Option<i64>, i64)> = sqlx::query_as(
            "SELECT avatar_id, avatar_is_animated, banner_id, banner_is_animated FROM users",
        )
        .fetch_all(database.writer())
        .await
        .unwrap();
        sqlx::raw_sql(
            "INSERT INTO profile_images (id, user_id, still_id, size_bytes, \
             created_at_unix_milliseconds) VALUES (50, 1, 51, 9, 0), (52, 2, NULL, 9, 0); \
             UPDATE users SET avatar_id = 50, avatar_is_animated = 1 WHERE id = 1; \
             DELETE FROM users WHERE id = 1;",
        )
        .execute(database.writer())
        .await
        .unwrap();
        let images: Vec<(i64,)> = sqlx::query_as("SELECT id FROM profile_images")
            .fetch_all(database.writer())
            .await
            .unwrap();
        database.close().await;

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
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_ATTACHMENTS, database.writer())
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
        .execute(database.writer())
        .await
        .unwrap();

        let outcome = migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO attachments (id, channel_id, uploader_id, message_id, file_name, \
             content_type, size_bytes, created_at_unix_milliseconds) \
             VALUES (40, 20, 1, 31, 'a.png', 'image/png', 3, 0), \
             (41, 20, 1, NULL, 'b.txt', 'application/octet-stream', 3, 0); \
             DELETE FROM messages WHERE id = 31;",
        )
        .execute(database.writer())
        .await
        .unwrap();
        let attachments: Vec<(i64,)> = sqlx::query_as("SELECT id FROM attachments ORDER BY id")
            .fetch_all(database.writer())
            .await
            .unwrap();
        let messages: Vec<(i64,)> = sqlx::query_as("SELECT id FROM messages")
            .fetch_all(database.writer())
            .await
            .unwrap();
        database.close().await;

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
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_REACTIONS, database.writer())
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
        .execute(database.writer())
        .await
        .unwrap();

        let outcome = migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let permissions: Vec<(i64, i64)> =
            sqlx::query_as("SELECT id, permissions FROM roles ORDER BY id")
                .fetch_all(database.writer())
                .await
                .unwrap();
        let overwrites: Vec<(i64, i64, i64, i64)> = sqlx::query_as(
            "SELECT channel_id, target_id, allow, deny FROM channel_permission_overwrites \
             ORDER BY channel_id, target_id",
        )
        .fetch_all(database.writer())
        .await
        .unwrap();
        let pinned: Vec<(i64, Option<i64>)> =
            sqlx::query_as("SELECT id, pinned_at_unix_milliseconds FROM messages ORDER BY id")
                .fetch_all(database.writer())
                .await
                .unwrap();
        sqlx::raw_sql(
            "INSERT INTO message_reactions (message_id, user_id, emoji) \
             VALUES (30, 1, '👍'), (31, 1, '👍'); \
             DELETE FROM messages WHERE id = 31;",
        )
        .execute(database.writer())
        .await
        .unwrap();
        let reactions: Vec<(i64,)> = sqlx::query_as("SELECT message_id FROM message_reactions")
            .fetch_all(database.writer())
            .await
            .unwrap();
        let duplicate = sqlx::query(
            "INSERT INTO message_reactions (message_id, user_id, emoji) VALUES (30, 1, '👍')",
        )
        .execute(database.writer())
        .await;
        database.close().await;

        assert_eq!(
            outcome.applied_versions.first(),
            Some(&(LAST_VERSION_WITHOUT_REACTIONS + 1))
        );
        assert_eq!(
            permissions,
            // Migration 0022 adds CONNECT and SPEAK to @everyone too.
            [(10, 0x043e | 0x1_0000 | 0xC_0000), (11, 0)],
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
    async fn a_group_goes_with_its_channel_and_its_owner_account_cannot_vanish_under_it() {
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let writer = database.writer();
        for statement in [
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'a', 'A', 'a@example.org', 'h', 0), \
             (2, 'b', 'B', 'b@example.org', 'h', 0)",
            "INSERT INTO channels (id, guild_id, kind, name, topic, parent_id, position) \
             VALUES (30, NULL, 'direct_message', 'Trip', '', NULL, 0)",
            "INSERT INTO group_conversations (channel_id, owner_id) VALUES (30, 1)",
            "INSERT INTO direct_message_participants (channel_id, user_id) VALUES (30, 1), (30, 2)",
        ] {
            sqlx::query(statement).execute(writer).await.unwrap();
        }
        let owner_deleted = sqlx::query("DELETE FROM users WHERE id = 1")
            .execute(writer)
            .await;
        sqlx::query("DELETE FROM channels WHERE id = 30")
            .execute(writer)
            .await
            .unwrap();
        let groups_left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM group_conversations")
            .fetch_one(writer)
            .await
            .unwrap();
        database.close().await;

        assert!(
            owner_deleted.is_err(),
            "no account deletion path exists; one must hand groups over first"
        );
        assert_eq!(groups_left, 0);
    }

    #[tokio::test]
    async fn relationships_go_with_either_account_and_refuse_self_and_unknown_kinds() {
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let writer = database.writer();
        for statement in [
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'a', 'A', 'a@example.org', 'h', 0), \
             (2, 'b', 'B', 'b@example.org', 'h', 0), (3, 'c', 'C', 'c@example.org', 'h', 0)",
            "INSERT INTO relationships (user_id, other_user_id, kind, since_unix_milliseconds) \
             VALUES (1, 2, 'friend', 0), (2, 1, 'friend', 0), (3, 2, 'blocked', 0)",
        ] {
            sqlx::query(statement).execute(writer).await.unwrap();
        }
        let to_self = sqlx::query(
            "INSERT INTO relationships (user_id, other_user_id, kind, since_unix_milliseconds) \
             VALUES (1, 1, 'friend', 0)",
        )
        .execute(writer)
        .await;
        let unknown_kind = sqlx::query(
            "INSERT INTO relationships (user_id, other_user_id, kind, since_unix_milliseconds) \
             VALUES (1, 3, 'enemy', 0)",
        )
        .execute(writer)
        .await;
        sqlx::query("DELETE FROM users WHERE id = 2")
            .execute(writer)
            .await
            .unwrap();
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM relationships")
            .fetch_one(writer)
            .await
            .unwrap();
        database.close().await;

        assert!(to_self.is_err() && unknown_kind.is_err());
        assert_eq!(
            remaining, 0,
            "both sides and blocks of a deleted account go"
        );
    }

    #[tokio::test]
    async fn mentions_go_with_their_message_and_their_channel() {
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let writer = database.writer();
        for statement in [
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'owner', 'Owner', 'o@example.org', 'h', 0)",
            "INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Guild', 1, 0)",
            "INSERT INTO channels (id, guild_id, kind, name, topic, parent_id, position) \
             VALUES (20, 10, 'text', 'general', '', NULL, 0)",
            "INSERT INTO messages (id, channel_id, author_id, content) \
             VALUES (30, 20, 1, 'a'), (31, 20, 1, 'b')",
            "INSERT INTO message_mentions (user_id, channel_id, message_id) \
             VALUES (1, 20, 30), (1, 20, 31)",
        ] {
            sqlx::query(statement).execute(writer).await.unwrap();
        }
        let remaining = || async {
            sqlx::query_scalar::<_, i64>("SELECT message_id FROM message_mentions ORDER BY 1")
                .fetch_all(writer)
                .await
                .unwrap()
        };

        sqlx::query("DELETE FROM messages WHERE id = 30")
            .execute(writer)
            .await
            .unwrap();
        let after_message_deletion = remaining().await;
        sqlx::query("DELETE FROM channels WHERE id = 20")
            .execute(writer)
            .await
            .unwrap();
        let after_channel_deletion = remaining().await;
        database.close().await;

        assert_eq!(after_message_deletion, [31]);
        assert_eq!(after_channel_deletion, Vec::<i64>::new());
    }

    #[tokio::test]
    async fn every_change_to_who_sees_what_bumps_the_guild_permission_version() {
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let writer = database.writer();
        for statement in [
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'owner', 'Owner', 'o@example.org', 'h', 0), \
             (2, 'member', 'Member', 'm@example.org', 'h', 0), \
             (3, 'heir', 'Heir', 'h@example.org', 'h', 0)",
            "INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Guild', 1, 0), (20, 'Other guild', 1, 0)",
        ] {
            sqlx::query(statement).execute(writer).await.unwrap();
        }
        let version = || async {
            sqlx::query_scalar::<_, i64>("SELECT permission_version FROM guilds WHERE id = 10")
                .fetch_one(writer)
                .await
                .unwrap()
        };

        // Each statement must bump the version; the second list must not.
        let bumping = [
            "INSERT INTO guild_members (guild_id, user_id, joined_at_unix_milliseconds) \
             VALUES (10, 1, 0), (10, 2, 0), (10, 3, 0)",
            "INSERT INTO roles (id, guild_id, name, permissions, position) \
             VALUES (10, 10, '@everyone', 0, 0), (11, 10, 'Staff', 0, 1)",
            "INSERT INTO member_roles (guild_id, user_id, role_id) VALUES (10, 2, 11)",
            "UPDATE roles SET permissions = 1024 WHERE id = 11",
            "DELETE FROM member_roles WHERE user_id = 2",
            "INSERT INTO member_roles (guild_id, user_id, role_id) VALUES (10, 2, 11)",
            "DELETE FROM roles WHERE id = 11",
            "UPDATE guilds SET owner_id = 3 WHERE id = 10",
            "DELETE FROM guild_members WHERE guild_id = 10 AND user_id = 3",
            "DELETE FROM users WHERE id = 2",
        ];
        let mut bumped = Vec::new();
        for statement in bumping {
            let before = version().await;
            sqlx::query(statement).execute(writer).await.unwrap();
            bumped.push((statement, version().await > before));
        }
        let neutral = [
            "UPDATE guilds SET name = 'Renamed' WHERE id = 10",
            "INSERT INTO guild_members (guild_id, user_id, joined_at_unix_milliseconds) \
             VALUES (20, 1, 0)",
        ];
        let mut unchanged = Vec::new();
        for statement in neutral {
            let before = version().await;
            sqlx::query(statement).execute(writer).await.unwrap();
            unchanged.push((statement, version().await == before));
        }
        database.close().await;

        for (statement, did_bump) in bumped {
            assert!(did_bump, "no bump after: {statement}");
        }
        for (statement, did_not_bump) in unchanged {
            assert!(did_not_bump, "unexpected bump after: {statement}");
        }
    }

    #[tokio::test]
    async fn existing_users_are_online_without_custom_status_and_unknown_statuses_are_refused() {
        const LAST_VERSION_WITHOUT_PRESENCE: i64 = 16;
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_PRESENCE, database.writer())
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0), \
             (2, 'bob', 'Bob', 'bob@example.org', 'y', 0);",
        )
        .execute(database.writer())
        .await
        .unwrap();

        migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let users: Vec<(i64, String, Option<String>)> =
            sqlx::query_as("SELECT id, presence_status, custom_status FROM users ORDER BY id")
                .fetch_all(database.writer())
                .await
                .unwrap();
        let unknown_status = sqlx::query("UPDATE users SET presence_status = 'away' WHERE id = 1")
            .execute(database.writer())
            .await;
        database.close().await;

        assert_eq!(
            users,
            [
                (1, "online".to_owned(), None),
                (2, "online".to_owned(), None)
            ]
        );
        assert!(unknown_status.is_err());
    }

    #[tokio::test]
    async fn icons_and_reactions_stay_and_emoji_names_are_unique_per_guild_and_go_with_it() {
        const LAST_VERSION_WITHOUT_EMOJIS: i64 = 19;
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_EMOJIS, database.writer())
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0), (11, 'Other', 1, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (20, 10, 'text', 'general', 0); \
             INSERT INTO messages (id, channel_id, author_id, content) \
             VALUES (30, 20, 1, 'hello'); \
             INSERT INTO message_reactions (message_id, user_id, emoji) VALUES (30, 1, '👍'); \
             INSERT INTO guild_images (id, guild_id, channel_id, still_id, size_bytes, \
             created_at_unix_milliseconds) VALUES (50, 10, NULL, NULL, 9, 0); \
             UPDATE guilds SET icon_id = 50 WHERE id = 10;",
        )
        .execute(database.writer())
        .await
        .unwrap();

        let outcome = migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO guild_images (id, guild_id, channel_id, still_id, size_bytes, \
             created_at_unix_milliseconds) VALUES (60, 10, NULL, 61, 9, 0), \
             (62, 10, NULL, NULL, 9, 0), (63, 11, NULL, NULL, 9, 0), (64, 11, NULL, NULL, 9, 0); \
             INSERT INTO guild_emojis (id, guild_id, name) \
             VALUES (60, 10, 'party'), (62, 10, 'cake'), (63, 11, 'Party'), (64, 11, 'cake');",
        )
        .execute(database.writer())
        .await
        .unwrap();
        let same_name_other_case =
            sqlx::query("UPDATE guild_emojis SET name = 'PARTY' WHERE id = 62")
                .execute(database.writer())
                .await;
        let without_picture =
            sqlx::query("INSERT INTO guild_emojis (id, guild_id, name) VALUES (70, 10, 'ghost')")
                .execute(database.writer())
                .await;
        sqlx::raw_sql("DELETE FROM guild_images WHERE id = 62; DELETE FROM guilds WHERE id = 11;")
            .execute(database.writer())
            .await
            .unwrap();
        let emojis: Vec<(i64, String)> =
            sqlx::query_as("SELECT id, name FROM guild_emojis ORDER BY id")
                .fetch_all(database.writer())
                .await
                .unwrap();
        let kept: (Option<i64>, String) = sqlx::query_as(
            "SELECT guilds.icon_id, message_reactions.emoji FROM guilds, message_reactions \
             WHERE guilds.id = 10",
        )
        .fetch_one(database.writer())
        .await
        .unwrap();
        database.close().await;

        assert_eq!(
            outcome.applied_versions.first(),
            Some(&(LAST_VERSION_WITHOUT_EMOJIS + 1))
        );
        assert!(
            same_name_other_case.is_err(),
            "names are unique whatever the case"
        );
        assert!(without_picture.is_err(), "an emoji is a guild image");
        assert_eq!(
            emojis,
            [(60, "party".to_owned())],
            "emoji go with their picture or their guild"
        );
        assert_eq!(kept, (Some(50), "👍".to_owned()));
    }

    #[tokio::test]
    async fn existing_messages_are_indexed_for_search_and_later_writes_follow() {
        const LAST_VERSION_WITHOUT_SEARCH: i64 = 20;
        const REALISTIC_MESSAGE_COUNT: usize = 3_000;
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_SEARCH, database.writer())
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0), \
             (2, 'bob', 'Bob', 'bob@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0); \
             INSERT INTO channels (id, guild_id, kind, name, position) \
             VALUES (20, 10, 'text', 'general', 0), (21, 10, 'text', 'random', 1); \
             INSERT INTO messages (id, channel_id, author_id, content, edited_at_unix_milliseconds, \
             pinned_at_unix_milliseconds) VALUES (5, 20, 1, 'Crème BRÛLÉE au café', 9, 9);",
        )
        .execute(database.writer())
        .await
        .unwrap();
        crate::storage::search::realistic_messages::insert_messages(
            database.writer(),
            100,
            REALISTIC_MESSAGE_COUNT,
            &[20, 21],
            &[1, 2],
        )
        .await;

        migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let writer = database.writer();
        let matches = |expression: &'static str| async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH ?",
            )
            .bind(expression)
            .fetch_one(writer)
            .await
            .unwrap()
        };
        let unaccented = matches("creme AND brulee AND cafe").await;
        let indexed_rare_word = matches("xylophone").await;
        let stored_rare_word: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM messages WHERE ' ' || content || ' ' LIKE '% xylophone %'",
        )
        .fetch_one(writer)
        .await
        .unwrap();
        sqlx::raw_sql(
            "INSERT INTO messages (id, channel_id, author_id, content) VALUES (6, 20, 2, 'zebra'); \
             UPDATE messages SET content = 'okapi' WHERE id = 5; \
             UPDATE messages SET pinned_at_unix_milliseconds = NULL WHERE id = 6;",
        )
        .execute(writer)
        .await
        .unwrap();
        let after_writes = (
            matches("zebra").await,
            matches("okapi").await,
            matches("creme AND brulee AND cafe").await,
        );
        let integrity = sqlx::query(
            "INSERT INTO messages_fts (messages_fts, rank) VALUES ('integrity-check', 1)",
        )
        .execute(writer)
        .await;
        database.close().await;

        assert_eq!(unaccented, 1);
        assert!(indexed_rare_word > 0);
        assert_eq!(indexed_rare_word, stored_rare_word);
        assert_eq!(after_writes, (1, 1, 0));
        assert!(integrity.is_ok(), "{integrity:?}");
    }

    #[tokio::test]
    async fn the_latest_schema_keeps_the_search_index_in_step() {
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let triggers: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_schema WHERE type = 'trigger' AND tbl_name = 'messages' \
             AND name LIKE 'messages_fts_%' ORDER BY name",
        )
        .fetch_all(database.writer())
        .await
        .unwrap();
        let integrity = sqlx::query(
            "INSERT INTO messages_fts (messages_fts, rank) VALUES ('integrity-check', 1)",
        )
        .execute(database.writer())
        .await;
        database.close().await;

        assert_eq!(
            triggers,
            [
                "messages_fts_after_content_update",
                "messages_fts_after_delete",
                "messages_fts_after_insert"
            ]
        );
        assert!(integrity.is_ok(), "{integrity:?}");
    }

    #[tokio::test]
    async fn existing_everyone_roles_may_use_voice_and_channels_keep_no_user_limit() {
        const LAST_VERSION_WITHOUT_VOICE: i64 = 21;
        const CONNECT_AND_SPEAK: i64 = (1 << 18) | (1 << 19);
        // VIEW_CHANNEL, SEND_MESSAGES, READ_MESSAGE_HISTORY, ATTACH_FILES, CREATE_INVITE,
        // CHANGE_NICKNAME and ADD_REACTIONS: the defaults before voice.
        const EVERYONE_BEFORE_VOICE: i64 = 0x1_241e;
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        MIGRATOR
            .run_to(LAST_VERSION_WITHOUT_VOICE, database.writer())
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id, username, display_name, email, password_hash, \
             created_at_unix_milliseconds) VALUES (1, 'ada', 'Ada', 'ada@example.org', 'x', 0), \
             (2, 'bob', 'Bob', 'bob@example.org', 'x', 0); \
             INSERT INTO guilds (id, name, owner_id, created_at_unix_milliseconds) \
             VALUES (10, 'Club', 1, 0), (50, 'Locked', 2, 0); \
             INSERT INTO roles (id, guild_id, name, permissions, position, color) \
             VALUES (10, 10, '@everyone', 74782, 0, 0), (11, 10, 'Muted', 0, 1, 0), \
             (50, 50, '@everyone', 0, 0, 0); \
             INSERT INTO guild_members (guild_id, user_id, joined_at_unix_milliseconds) \
             VALUES (10, 1, 0), (10, 2, 0), (50, 2, 0); \
             INSERT INTO channels (id, guild_id, kind, name, topic, position) \
             VALUES (20, 10, 'text', 'general', 'Hello', 0); \
             INSERT INTO channels (id, guild_id, kind, name, topic, position) \
             VALUES (21, 10, 'category', 'Talk', '', 1); \
             INSERT INTO messages (id, channel_id, author_id, content) \
             VALUES (30, 20, 1, 'hello');",
        )
        .execute(database.writer())
        .await
        .unwrap();

        let outcome = migrate_with_backup(&database, &directory.path().join("backups"))
            .await
            .unwrap();
        let permissions: Vec<(i64, i64)> =
            sqlx::query_as("SELECT id, permissions FROM roles ORDER BY id")
                .fetch_all(database.writer())
                .await
                .unwrap();
        let channels: Vec<(i64, String, Option<i64>)> =
            sqlx::query_as("SELECT id, topic, user_limit FROM channels ORDER BY id")
                .fetch_all(database.writer())
                .await
                .unwrap();
        let messages: Vec<(i64,)> = sqlx::query_as("SELECT id FROM messages")
            .fetch_all(database.writer())
            .await
            .unwrap();
        let voice_channel = sqlx::query(
            "INSERT INTO channels (id, guild_id, kind, name, topic, position, user_limit) \
             VALUES (22, 10, 'voice', 'Lounge', '', 2, 25)",
        )
        .execute(database.writer())
        .await;
        let out_of_range = sqlx::query(
            "INSERT INTO channels (id, guild_id, kind, name, topic, position, user_limit) \
             VALUES (23, 10, 'voice', 'Crowd', '', 3, 26)",
        )
        .execute(database.writer())
        .await;
        database.close().await;

        assert_eq!(
            outcome.applied_versions.first(),
            Some(&(LAST_VERSION_WITHOUT_VOICE + 1))
        );
        assert_eq!(
            permissions,
            [
                (10, EVERYONE_BEFORE_VOICE | CONNECT_AND_SPEAK),
                (11, 0),
                (50, CONNECT_AND_SPEAK)
            ],
            "only @everyone roles gain CONNECT and SPEAK"
        );
        assert_eq!(
            channels,
            [(20, "Hello".to_owned(), None), (21, String::new(), None)]
        );
        assert_eq!(messages, [(30,)]);
        assert!(voice_channel.is_ok(), "{voice_channel:?}");
        assert!(out_of_range.is_err(), "user limits stop at 25");
    }

    #[tokio::test]
    async fn a_database_from_a_newer_binary_is_refused_even_with_nothing_pending() {
        let directory = tempfile::tempdir().unwrap();
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        let backups = directory.path().join("backups");
        migrate_with_backup(&database, &backups).await.unwrap();
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
             VALUES (999, 'from a newer binary', 1, x'00', 0)",
        )
        .execute(database.writer())
        .await
        .unwrap();

        let result = migrate_with_backup(&database, &backups).await;
        database.close().await;
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
        let database = open_encrypted_database(&directory.path().join("mellow.db"), &test_key())
            .await
            .unwrap();
        sqlx::query("CREATE TABLE legacy (value TEXT)")
            .execute(database.writer())
            .await
            .unwrap();
        let backups = directory.path().join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::write(backups.join("pre-migration-1.db"), b"older backup").unwrap();

        let result = migrate_with_backup(&database, &backups).await;
        let has_users_table: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE name = 'users')")
                .fetch_one(database.writer())
                .await
                .unwrap();
        database.close().await;

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
