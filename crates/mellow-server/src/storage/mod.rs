//! SQLite storage, encrypted at rest with SQLCipher (ADR-0002, ADR-0006).

pub(crate) mod accounts;
pub(crate) mod attachment_files;
pub(crate) mod attachments;
pub(crate) mod audit_log;
mod database;
pub(crate) mod direct_messages;
pub(crate) mod guild_emojis;
pub(crate) mod guild_images;
pub(crate) mod guilds;
mod master_key;
pub(crate) mod messages;
mod migrations;
pub(crate) mod profile_images;
pub(crate) mod reactions;
pub(crate) mod relationships;
pub(crate) mod search;

pub use database::{Database, DatabaseKey, StorageError, open_encrypted_database};
pub use master_key::{MasterKey, MasterKeyError};
pub use migrations::{MigrationError, MigrationOutcome, migrate_with_backup};

/// Owner only: other local users must not even copy the encrypted database or key (least privilege).
#[cfg(unix)]
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;

/// Creates `path` and missing parents, readable only by the service user on Unix.
///
/// # Errors
/// Fails on any I/O error. An existing directory is left as it is.
pub fn create_private_directory(path: &std::path::Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, PRIVATE_DIRECTORY_MODE);
    builder.create(path)
}
