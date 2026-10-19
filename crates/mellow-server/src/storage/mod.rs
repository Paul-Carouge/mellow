//! SQLite storage, encrypted at rest with SQLCipher (ADR-0002, ADR-0006).

mod database;

pub use database::{DatabaseKey, StorageError, open_encrypted_database};
