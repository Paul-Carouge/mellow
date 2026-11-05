//! SQLite storage, encrypted at rest with SQLCipher (ADR-0002, ADR-0006).

pub(crate) mod account_names;
pub(crate) mod accounts;
pub(crate) mod attachment_files;
pub(crate) mod attachments;
pub(crate) mod audit_log;
pub(crate) mod backups;
pub(crate) mod checklists;
mod database;
pub(crate) mod direct_messages;
pub(crate) mod gif_favourites;
pub(crate) mod guild_channels;
pub(crate) mod guild_emojis;
pub(crate) mod guild_images;
pub(crate) mod guild_invites;
pub(crate) mod guild_members;
pub(crate) mod guild_roles;
pub(crate) mod guild_sounds;
pub(crate) mod guilds;
pub(crate) mod image_recompression;
pub(crate) mod link_previews;
mod master_key;
pub(crate) mod messages;
mod migrations;
pub(crate) mod polls;
pub(crate) mod profile_images;
pub(crate) mod reactions;
pub(crate) mod relationships;
pub(crate) mod search;
mod sqlite_integers;
pub(crate) mod two_factor;
pub(crate) mod user_profiles;

pub use database::{Database, DatabaseKey, StorageError, open_encrypted_database};
pub use master_key::{MasterKey, MasterKeyError};
pub use migrations::{
    MigrationError, MigrationOutcome, ensure_no_pending_migration, migrate_with_backup,
};

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
