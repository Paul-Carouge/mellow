//! The instance master key: 256 random bits in a file outside the data directory, from which
//! the SQLCipher key (and later the attachment keys) are derived (ADR-0006).

use std::fmt;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::{Zeroize as _, ZeroizeOnDrop, Zeroizing};

use crate::storage::DatabaseKey;

const MASTER_KEY_LENGTH_BYTES: usize = 32;
const MASTER_KEY_HEX_LENGTH: usize = MASTER_KEY_LENGTH_BYTES * 2;
/// The hex digits plus a line ending (`\n` or `\r\n`); anything longer is not a key file.
const MAXIMUM_MASTER_KEY_FILE_LENGTH_BYTES: u64 = MASTER_KEY_HEX_LENGTH as u64 + 2;
/// Versioned so a future key schedule can coexist with this one.
const DATABASE_KEY_DERIVATION_INFO: &[u8] = b"mellow/sqlcipher-database-key/v1";
const ATTACHMENT_KEY_DERIVATION_INFO: &[u8] = b"mellow/attachments-aes-256-gcm/v1";
/// Owner read/write only.
#[cfg(unix)]
const MASTER_KEY_FILE_MODE: u32 = 0o600;
/// Any permission bit for group or others.
#[cfg(unix)]
const GROUP_OR_OTHER_PERMISSION_BITS: u32 = 0o077;

/// Wiped from memory when dropped.
#[derive(ZeroizeOnDrop)]
pub struct MasterKey([u8; MASTER_KEY_LENGTH_BYTES]);

#[derive(Debug, thiserror::Error)]
pub enum MasterKeyError {
    #[error("the operating system random generator failed")]
    RandomGeneratorFailed(#[source] getrandom::Error),
    #[error(
        "a master key already exists at {path}; refusing to overwrite it (that would make the database unreadable)"
    )]
    AlreadyExists { path: PathBuf },
    #[error("cannot write the master key file {path}")]
    CannotWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot read the master key file {path}")]
    CannotRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "the master key file {path} must contain exactly {MASTER_KEY_HEX_LENGTH} hexadecimal characters"
    )]
    Malformed { path: PathBuf },
    #[error("the master key file {path} is readable by other users; run `chmod 600` on it")]
    TooPermissive { path: PathBuf },
    #[error(
        "this platform cannot yet restrict the master key file to the service user; Mellow runs on Linux and macOS for now"
    )]
    UnsupportedPlatform,
}

impl MasterKey {
    /// # Errors
    /// Fails only if the operating system cannot provide randomness.
    pub fn generate() -> Result<Self, MasterKeyError> {
        let mut bytes = [0u8; MASTER_KEY_LENGTH_BYTES];
        getrandom::fill(&mut bytes).map_err(MasterKeyError::RandomGeneratorFailed)?;
        let key = Self(bytes);
        bytes.zeroize();
        Ok(key)
    }

    /// Writes the key as hex to a new file readable only by its owner.
    ///
    /// # Errors
    /// Refuses to overwrite an existing file; fails on any I/O error, and on platforms where the
    /// file cannot be restricted to its owner.
    pub fn write_new_file(&self, path: &Path) -> Result<(), MasterKeyError> {
        let cannot_write = |source| MasterKeyError::CannotWrite {
            path: path.to_path_buf(),
            source,
        };
        if let Some(parent) = path.parent() {
            crate::storage::create_private_directory(parent).map_err(cannot_write)?;
        }
        let mut file = open_new_owner_only_file(path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists {
                MasterKeyError::AlreadyExists {
                    path: path.to_path_buf(),
                }
            } else {
                cannot_write(source)
            }
        })?;
        let hex_digits = Zeroizing::new(hex(&self.0));
        writeln!(file, "{}", hex_digits.as_str()).map_err(cannot_write)?;
        file.sync_all().map_err(cannot_write)
    }

    /// # Errors
    /// Fails if the file is missing, malformed, too large, or readable by group or others.
    pub fn read_file(path: &Path) -> Result<Self, MasterKeyError> {
        let cannot_read = |source| MasterKeyError::CannotRead {
            path: path.to_path_buf(),
            source,
        };
        // One open, then checks on the handle: the file cannot be swapped between check and read.
        let file = std::fs::File::open(path).map_err(cannot_read)?;
        ensure_owner_only(&file, path)?;
        let mut text = Zeroizing::new(String::new());
        file.take(MAXIMUM_MASTER_KEY_FILE_LENGTH_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(cannot_read)?;
        parse_hex_key(text.trim()).ok_or_else(|| MasterKeyError::Malformed {
            path: path.to_path_buf(),
        })
    }

    /// HKDF-SHA256 of the master key, so the database key never equals the stored secret and each
    /// future use (attachments) gets an independent key.
    ///
    /// # Panics
    /// Never: HKDF-SHA256 can output up to 8160 bytes and 32 are requested.
    #[must_use]
    pub fn derive_database_key(&self) -> DatabaseKey {
        let mut database_key = Zeroizing::new([0u8; MASTER_KEY_LENGTH_BYTES]);
        let derivation = Hkdf::<Sha256>::new(None, &self.0);
        // Invariant: HKDF-SHA256 can produce up to 255 × 32 bytes, and we ask for 32.
        #[allow(clippy::expect_used)]
        derivation
            .expand(DATABASE_KEY_DERIVATION_INFO, database_key.as_mut())
            .expect("32 bytes is within HKDF-SHA256's output limit");
        DatabaseKey::from_bytes(*database_key)
    }

    /// The key sealing attachment files (ADR-0020), independent of the database key.
    ///
    /// # Panics
    /// Never: HKDF-SHA256 can output up to 8160 bytes and 32 are requested.
    #[must_use]
    pub fn derive_attachment_key(&self) -> Zeroizing<[u8; MASTER_KEY_LENGTH_BYTES]> {
        let mut attachment_key = Zeroizing::new([0u8; MASTER_KEY_LENGTH_BYTES]);
        let derivation = Hkdf::<Sha256>::new(None, &self.0);
        // Invariant: HKDF-SHA256 can produce up to 255 × 32 bytes, and we ask for 32.
        #[allow(clippy::expect_used)]
        derivation
            .expand(ATTACHMENT_KEY_DERIVATION_INFO, attachment_key.as_mut())
            .expect("32 bytes is within HKDF-SHA256's output limit");
        attachment_key
    }
}

impl fmt::Debug for MasterKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MasterKey(<redacted>)")
    }
}

#[cfg(unix)]
fn open_new_owner_only_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(MASTER_KEY_FILE_MODE)
        .open(path)
}

// shortcut: Windows needs a protected DACL (owner + SYSTEM) before Mellow can keep a key there;
// until the Windows service is packaged, refuse rather than leave the key readable by all users.
#[cfg(not(unix))]
fn open_new_owner_only_file(_path: &Path) -> std::io::Result<std::fs::File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        MasterKeyError::UnsupportedPlatform.to_string(),
    ))
}

#[cfg(unix)]
fn ensure_owner_only(file: &std::fs::File, path: &Path) -> Result<(), MasterKeyError> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = file
        .metadata()
        .map_err(|source| MasterKeyError::CannotRead {
            path: path.to_path_buf(),
            source,
        })?
        .permissions()
        .mode();
    if mode & GROUP_OR_OTHER_PERMISSION_BITS != 0 {
        return Err(MasterKeyError::TooPermissive {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_owner_only(_file: &std::fs::File, _path: &Path) -> Result<(), MasterKeyError> {
    Err(MasterKeyError::UnsupportedPlatform)
}

fn parse_hex_key(hex_digits: &str) -> Option<MasterKey> {
    if hex_digits.len() != MASTER_KEY_HEX_LENGTH
        || !hex_digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    let mut bytes = [0u8; MASTER_KEY_LENGTH_BYTES];
    let (hex_pairs, _) = hex_digits.as_bytes().as_chunks::<2>();
    for (byte, pair) in bytes.iter_mut().zip(hex_pairs) {
        let pair = std::str::from_utf8(pair).ok()?;
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    let key = MasterKey(bytes);
    bytes.zeroize();
    Some(key)
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0F])
        // A nibble is always below 16, so `from_digit` never drops one.
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing_accepts_only_exactly_64_hex_digits() {
        let valid = "0f".repeat(MASTER_KEY_LENGTH_BYTES);
        assert_eq!(
            parse_hex_key(&valid).unwrap().0,
            [0x0F; MASTER_KEY_LENGTH_BYTES]
        );
        let with_plus_signs = format!("+f{}", "0f".repeat(MASTER_KEY_LENGTH_BYTES - 1));
        for text in [
            "",
            "abcd",
            &"zz".repeat(32),
            &with_plus_signs,
            &format!("{valid}0"),
        ] {
            assert!(parse_hex_key(text).is_none(), "{text:?}");
        }
    }

    #[test]
    fn derived_database_key_differs_between_master_keys() {
        let first = MasterKey([1; MASTER_KEY_LENGTH_BYTES]).derive_database_key();
        let second = MasterKey([2; MASTER_KEY_LENGTH_BYTES]).derive_database_key();
        assert_ne!(
            first.to_sqlcipher_pragma_value(),
            second.to_sqlcipher_pragma_value()
        );
    }

    #[test]
    fn key_is_never_printed() {
        assert_eq!(
            format!("{:?}", MasterKey([7; MASTER_KEY_LENGTH_BYTES])),
            "MasterKey(<redacted>)"
        );
    }

    #[cfg(unix)]
    mod on_unix {
        use std::os::unix::fs::PermissionsExt as _;

        use super::*;

        #[test]
        fn written_key_reads_back_identically_and_derives_the_same_database_key() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("keys").join("master.key");
            let key = MasterKey::generate().unwrap();
            key.write_new_file(&path).unwrap();

            let read_back = MasterKey::read_file(&path).unwrap();
            assert_eq!(read_back.0, key.0);
            assert_eq!(
                read_back.derive_database_key().to_sqlcipher_pragma_value(),
                key.derive_database_key().to_sqlcipher_pragma_value()
            );
        }

        #[test]
        fn existing_key_file_is_never_overwritten() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("master.key");
            MasterKey::generate()
                .unwrap()
                .write_new_file(&path)
                .unwrap();
            let original = std::fs::read(&path).unwrap();

            let result = MasterKey::generate().unwrap().write_new_file(&path);
            assert!(matches!(result, Err(MasterKeyError::AlreadyExists { .. })));
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }

        #[test]
        fn malformed_and_oversized_key_files_are_rejected() {
            let directory = tempfile::tempdir().unwrap();
            let oversized = "0".repeat(1_000_000);
            for (name, content) in [("short", "abcd"), ("oversized", oversized.as_str())] {
                let path = directory.path().join(name);
                MasterKey([0; MASTER_KEY_LENGTH_BYTES])
                    .write_new_file(&path)
                    .unwrap();
                std::fs::write(&path, content).unwrap();
                assert!(
                    matches!(
                        MasterKey::read_file(&path),
                        Err(MasterKeyError::Malformed { .. })
                    ),
                    "{name}"
                );
            }
        }

        #[test]
        fn key_file_and_its_directory_are_owner_only_and_permissive_files_are_refused() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("keys").join("master.key");
            MasterKey::generate()
                .unwrap()
                .write_new_file(&path)
                .unwrap();
            let file_mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(file_mode & 0o777, MASTER_KEY_FILE_MODE);
            let directory_mode = std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(directory_mode & GROUP_OR_OTHER_PERMISSION_BITS, 0);

            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(matches!(
                MasterKey::read_file(&path),
                Err(MasterKeyError::TooPermissive { .. })
            ));
        }
    }
}
