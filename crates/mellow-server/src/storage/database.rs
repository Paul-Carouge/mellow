use std::fmt;
use std::path::Path;
use std::time::Duration;

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePool, SqlitePoolOptions,
};
use sqlx::{ConnectOptions as _, Connection as _};

/// How long a write waits before failing: for the writer connection inside the server, or for
/// the SQLite lock ("database is locked") held by another process such as `mellow-server admin`.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// SQLite allows one writer at a time: one connection queues writes in the pool's FIFO order
/// instead of several connections polling for the lock (ARCHITECTURE §4.3).
const WRITER_CONNECTION_COUNT: u32 = 1;
const DATABASE_KEY_LENGTH_BYTES: usize = 32;
/// `SQLITE_NOTADB`: what SQLCipher reports when the key does not decrypt the file.
const SQLITE_NOT_A_DATABASE_ERROR_CODE: &str = "26";

/// The 256-bit SQLCipher key. Never printed, never logged, wiped from memory when dropped.
/// sqlx keeps its own copy of the pragma string for reconnections while the pool lives (ADR-0006).
#[derive(zeroize::ZeroizeOnDrop)]
pub struct DatabaseKey([u8; DATABASE_KEY_LENGTH_BYTES]);

impl DatabaseKey {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; DATABASE_KEY_LENGTH_BYTES]) -> Self {
        Self(bytes)
    }

    /// SQLCipher raw-key syntax, which skips its slow passphrase derivation since the key is already random.
    pub(crate) fn to_sqlcipher_pragma_value(&self) -> String {
        // A nibble is always below 16, so `from_digit` never drops one.
        let key_hex: zeroize::Zeroizing<String> = zeroize::Zeroizing::new(
            self.0
                .iter()
                .flat_map(|byte| [byte >> 4, byte & 0x0F])
                .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
                .collect(),
        );
        format!("\"x'{}'\"", key_hex.as_str())
    }
}

/// Prints nothing of the pools: their connect options hold the raw key.
impl fmt::Debug for Database {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Database { .. }")
    }
}

impl fmt::Debug for DatabaseKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DatabaseKey(<redacted>)")
    }
}

/// The instance database: one writer connection and a pool of read-only connections.
///
/// Storage functions pick the pool themselves, so the layers above never decide how SQL runs.
/// Cloning is cheap and shares both pools.
#[derive(Clone)]
pub struct Database {
    writer: SqlitePool,
    readers: SqlitePool,
}

impl Database {
    /// The only connection that writes. A transaction holds it until it ends, so never call a
    /// storage function that writes while holding a transaction: it would wait for itself.
    pub(crate) const fn writer(&self) -> &SqlitePool {
        &self.writer
    }

    /// Read-only connections (`query_only`): any write sent to them fails.
    pub(crate) const fn readers(&self) -> &SqlitePool {
        &self.readers
    }

    /// Runs `SELECT 1` on a reader, to tell whether the file still opens and decrypts.
    ///
    /// # Errors
    /// Fails if no reader connection can run the query.
    pub async fn check_that_readers_answer(&self) -> Result<(), StorageError> {
        sqlx::query("SELECT 1").execute(&self.readers).await?;
        Ok(())
    }

    /// Closes every connection. Await it before the process exits (see `open_encrypted_database`).
    pub async fn close(&self) {
        self.readers.close().await;
        self.writer.close().await;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("the database key does not decrypt this file, or the file is not a Mellow database")]
    CannotDecryptDatabase(#[source] sqlx::Error),
    #[error("the SQLite library is not SQLCipher, refusing to store data unencrypted")]
    SqlcipherUnavailable,
    #[error("database error")]
    Database(#[from] sqlx::Error),
}

/// Opens (or creates) the instance database, encrypted with `key`, in WAL mode with foreign keys enforced.
///
/// # Errors
/// Fails if the linked SQLite is not SQLCipher, if the key does not decrypt an existing file,
/// or if the file cannot be opened.
///
/// Call `Database::close` before the process exits: SQLCipher frees global state at exit, and a
/// connection still closing on a worker thread at that moment crashes the process (seen on macOS).
pub async fn open_encrypted_database(
    path: &Path,
    key: &DatabaseKey,
) -> Result<Database, StorageError> {
    // sqlx applies the `key` pragma before any other statement, as SQLCipher requires.
    // Statement logging is off because the connect pragmas contain the raw key, which sqlx would
    // otherwise log at DEBUG, or at WARN when a statement is slow.
    let key_only_options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .pragma("key", key.to_sqlcipher_pragma_value())
        .disable_statement_logging();

    ensure_sqlcipher_is_linked_and_key_decrypts(&key_only_options).await?;

    let pool_options = key_only_options
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        // Deleted messages are overwritten on disk instead of lingering in free pages, which the
        // file, backups and anyone holding the key could otherwise recover.
        .pragma("secure_delete", "ON")
        .busy_timeout(BUSY_TIMEOUT);
    // The writer opens first, so the file is in WAL mode before any reader connects.
    let writer = SqlitePoolOptions::new()
        .max_connections(WRITER_CONNECTION_COUNT)
        // Fail queued writes as fast as before the single writer, not after sqlx's default 30 s.
        .acquire_timeout(BUSY_TIMEOUT)
        .connect_with(pool_options.clone())
        .await?;
    let readers = match SqlitePoolOptions::new()
        .connect_with(pool_options.pragma("query_only", "ON"))
        .await
    {
        Ok(readers) => readers,
        Err(error) => {
            writer.close().await;
            return Err(error.into());
        }
    };
    Ok(Database { writer, readers })
}

/// Runs the checks on one connection that is always closed before returning, so no connection is
/// left closing in the background (see the shutdown hazard above), and nothing is written to a
/// database that a plain SQLite or a wrong key would corrupt.
async fn ensure_sqlcipher_is_linked_and_key_decrypts(
    key_only_options: &SqliteConnectOptions,
) -> Result<(), StorageError> {
    // Only the `key` and `foreign_keys` pragmas run here; neither reads the file, so a wrong key
    // cannot fail this step and leave a half-open connection behind.
    let mut connection = key_only_options
        .connect()
        .await
        .map_err(classify_open_error)?;
    let verification = verify_sqlcipher_and_key(&mut connection).await;
    let closing = connection.close().await;
    verification?;
    Ok(closing?)
}

async fn verify_sqlcipher_and_key(connection: &mut SqliteConnection) -> Result<(), StorageError> {
    let cipher_version: Option<String> = sqlx::query_scalar("PRAGMA cipher_version")
        .fetch_optional(&mut *connection)
        .await?;
    if cipher_version.is_none_or(|version| version.is_empty()) {
        return Err(StorageError::SqlcipherUnavailable);
    }
    // The first read of the schema is where SQLCipher actually decrypts a page.
    sqlx::query("SELECT count(*) FROM sqlite_schema")
        .execute(&mut *connection)
        .await
        .map_err(classify_open_error)?;
    Ok(())
}

fn classify_open_error(error: sqlx::Error) -> StorageError {
    match &error {
        sqlx::Error::Database(database_error)
            if database_error.code().as_deref() == Some(SQLITE_NOT_A_DATABASE_ERROR_CODE) =>
        {
            StorageError::CannotDecryptDatabase(error)
        }
        _ => StorageError::Database(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQLITE_PLAINTEXT_HEADER: &[u8] = b"SQLite format 3\0";

    fn key_filled_with(byte: u8) -> DatabaseKey {
        DatabaseKey::from_bytes([byte; DATABASE_KEY_LENGTH_BYTES])
    }

    #[tokio::test]
    async fn data_round_trips_with_the_right_key_and_the_file_is_not_plaintext() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mellow.db");

        let pool = open_encrypted_database(&path, &key_filled_with(7))
            .await
            .unwrap();
        sqlx::query("CREATE TABLE notes (body TEXT NOT NULL)")
            .execute(pool.writer())
            .await
            .unwrap();
        sqlx::query("INSERT INTO notes (body) VALUES ('a secret message')")
            .execute(pool.writer())
            .await
            .unwrap();
        // Checkpoint so the row lands in the main file, not only in the WAL.
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(pool.writer())
            .await
            .unwrap();
        pool.close().await;

        let raw_file = std::fs::read(&path).unwrap();
        assert!(!raw_file.starts_with(SQLITE_PLAINTEXT_HEADER));
        assert!(
            !raw_file
                .windows(16)
                .any(|window| window == b"a secret message")
        );

        let reopened = open_encrypted_database(&path, &key_filled_with(7))
            .await
            .unwrap();
        let body: String = sqlx::query_scalar("SELECT body FROM notes")
            .fetch_one(reopened.writer())
            .await
            .unwrap();
        assert_eq!(body, "a secret message");
        reopened.close().await;
    }

    async fn integer_pragma(pool: &SqlitePool, pragma_query: &'static str) -> i64 {
        sqlx::query_scalar(pragma_query)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn every_connection_uses_wal_secure_delete_and_foreign_keys() {
        let directory = tempfile::tempdir().unwrap();
        let database =
            open_encrypted_database(&directory.path().join("mellow.db"), &key_filled_with(3))
                .await
                .unwrap();
        for pool in [database.writer(), database.readers()] {
            let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
                .fetch_one(pool)
                .await
                .unwrap();
            assert_eq!(journal_mode, "wal");
            assert_eq!(integer_pragma(pool, "PRAGMA secure_delete").await, 1);
            assert_eq!(integer_pragma(pool, "PRAGMA foreign_keys").await, 1);
        }
        database.close().await;
    }

    #[tokio::test]
    async fn readers_refuse_writes_and_the_writer_is_a_single_connection() {
        let directory = tempfile::tempdir().unwrap();
        let database =
            open_encrypted_database(&directory.path().join("mellow.db"), &key_filled_with(4))
                .await
                .unwrap();
        sqlx::query("CREATE TABLE notes (body TEXT)")
            .execute(database.writer())
            .await
            .unwrap();

        let write_on_reader = sqlx::query("INSERT INTO notes (body) VALUES ('no')")
            .execute(database.readers())
            .await;
        let writer_connection_limit = database.writer().options().get_max_connections();
        database.close().await;

        assert!(write_on_reader.is_err(), "a reader accepted a write");
        assert_eq!(writer_connection_limit, WRITER_CONNECTION_COUNT);
    }

    #[tokio::test]
    async fn debug_output_of_the_database_never_shows_the_key() {
        let directory = tempfile::tempdir().unwrap();
        let database =
            open_encrypted_database(&directory.path().join("mellow.db"), &key_filled_with(0xAB))
                .await
                .unwrap();
        let printed = format!("{database:?}");
        database.close().await;
        assert!(
            !printed
                .to_lowercase()
                .contains(&"ab".repeat(DATABASE_KEY_LENGTH_BYTES))
        );
    }

    #[tokio::test]
    async fn opening_with_a_wrong_key_reports_a_decryption_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mellow.db");
        let pool = open_encrypted_database(&path, &key_filled_with(1))
            .await
            .unwrap();
        sqlx::query("CREATE TABLE notes (body TEXT)")
            .execute(pool.writer())
            .await
            .unwrap();
        pool.close().await;

        let result = open_encrypted_database(&path, &key_filled_with(2)).await;
        assert!(
            matches!(result, Err(StorageError::CannotDecryptDatabase(_))),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn an_unopenable_path_is_not_reported_as_a_wrong_key() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing-directory").join("mellow.db");

        let result = open_encrypted_database(&path, &key_filled_with(1)).await;
        assert!(
            matches!(result, Err(StorageError::Database(_))),
            "{result:?}"
        );
    }

    #[test]
    fn key_is_never_printed() {
        assert_eq!(
            format!("{:?}", key_filled_with(0xAB)),
            "DatabaseKey(<redacted>)"
        );
    }

    #[test]
    fn pragma_value_uses_raw_key_syntax() {
        let value = key_filled_with(0x0F).to_sqlcipher_pragma_value();
        assert_eq!(
            value,
            format!("\"x'{}'\"", "0f".repeat(DATABASE_KEY_LENGTH_BYTES))
        );
    }
}
