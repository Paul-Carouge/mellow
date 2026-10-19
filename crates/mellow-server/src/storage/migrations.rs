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
        assert_eq!(first_run.applied_versions, [1]);
        assert_eq!(first_run.backup_path, None);

        let second_run = migrate_with_backup(&pool, &backups).await.unwrap();
        assert_eq!(second_run.applied_versions, Vec::<i64>::new());
        pool.close().await;
        assert!(!backups.exists());
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
