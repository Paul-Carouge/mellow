//! Attachment files sealed on disk (ADR-0020): AES-256-GCM under a key derived from the master
//! key, a random nonce per file, the attachment id as associated data so a file cannot be swapped
//! for another one's. Layout: `<directory>/<two hex digits>/<id>`, each file `nonce ‖ sealed`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use mellow_protocol::Snowflake;
use zeroize::Zeroizing;

const SUFFIX_OF_TEMPORARY_FILES: &str = ".partial";

#[derive(Debug, thiserror::Error)]
pub(crate) enum AttachmentFileError {
    #[error("attachment file input/output failed")]
    Io(#[from] std::io::Error),
    #[error("cannot seal or open an attachment file")]
    Crypto,
    #[error("background task failed")]
    Task(#[from] tokio::task::JoinError),
}

#[derive(Clone)]
pub(crate) struct AttachmentFiles {
    directory: PathBuf,
    key: Arc<LessSafeKey>,
}

/// The key is never printed.
impl std::fmt::Debug for AttachmentFiles {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AttachmentFiles")
            .field("directory", &self.directory)
            .field("key", &"<redacted>")
            .finish()
    }
}

impl AttachmentFiles {
    /// # Errors
    /// `Crypto` if the key is not 32 bytes (never with `MasterKey::derive_attachment_key`).
    pub(crate) fn new(
        directory: PathBuf,
        key: &Zeroizing<[u8; 32]>,
    ) -> Result<Self, AttachmentFileError> {
        let key =
            UnboundKey::new(&AES_256_GCM, key.as_ref()).map_err(|_| AttachmentFileError::Crypto)?;
        Ok(Self {
            directory,
            key: Arc::new(LessSafeKey::new(key)),
        })
    }

    fn path_of(&self, id: Snowflake) -> PathBuf {
        let name = id.raw().to_string();
        let shard = format!("{:02x}", id.raw() % 256);
        self.directory.join(shard).join(name)
    }

    /// Seals and writes the file, through a temporary name renamed once complete, so a crash never
    /// leaves a truncated file under the final name.
    pub(crate) async fn write(
        &self,
        id: Snowflake,
        plaintext: Vec<u8>,
    ) -> Result<(), AttachmentFileError> {
        let path = self.path_of(id);
        let key = self.key.clone();
        tokio::task::spawn_blocking(move || {
            let mut nonce_bytes = [0u8; NONCE_LEN];
            getrandom::fill(&mut nonce_bytes).map_err(|_| AttachmentFileError::Crypto)?;
            // Sealed in place and written in three parts: no second copy of the file in memory.
            let mut sealed = plaintext;
            let tag = key
                .seal_in_place_separate_tag(
                    Nonce::assume_unique_for_key(nonce_bytes),
                    Aad::from(id.raw().to_le_bytes()),
                    &mut sealed,
                )
                .map_err(|_| AttachmentFileError::Crypto)?;
            if let Some(parent) = path.parent() {
                crate::storage::create_private_directory(parent)?;
            }
            let temporary = with_suffix(&path, SUFFIX_OF_TEMPORARY_FILES);
            let written = write_private_file(&temporary, &[&nonce_bytes, &sealed, tag.as_ref()])
                .and_then(|()| std::fs::rename(&temporary, &path));
            if written.is_err() {
                // A full disk or a crash must not leave half a file behind.
                let _ = std::fs::remove_file(&temporary);
            }
            Ok(written?)
        })
        .await?
    }

    /// The plaintext; `Crypto` if the file was altered or belongs to another id.
    pub(crate) async fn read(&self, id: Snowflake) -> Result<Vec<u8>, AttachmentFileError> {
        let path = self.path_of(id);
        let key = self.key.clone();
        tokio::task::spawn_blocking(move || {
            // Opened in the buffer read from disk: no second copy of the file in memory.
            let mut contents = std::fs::read(&path)?;
            if contents.len() < NONCE_LEN {
                return Err(AttachmentFileError::Crypto);
            }
            let nonce = Nonce::try_assume_unique_for_key(&contents[..NONCE_LEN])
                .map_err(|_| AttachmentFileError::Crypto)?;
            let plaintext_length = key
                .open_within(
                    nonce,
                    Aad::from(id.raw().to_le_bytes()),
                    &mut contents,
                    NONCE_LEN..,
                )
                .map_err(|_| AttachmentFileError::Crypto)?
                .len();
            contents.truncate(plaintext_length);
            Ok(contents)
        })
        .await?
    }

    /// A missing file is not an error: deleting twice is harmless.
    pub(crate) async fn delete(&self, id: Snowflake) -> Result<(), AttachmentFileError> {
        match tokio::fs::remove_file(self.path_of(id)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Temporary files left by a crash mid-write, created before the cutoff (from their id).
    pub(crate) async fn remove_partial_files_before(
        &self,
        cutoff_unix_milliseconds: u64,
    ) -> Result<usize, AttachmentFileError> {
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || {
            let mut removed = 0;
            let Ok(shards) = std::fs::read_dir(&directory) else {
                return Ok(removed);
            };
            for shard in shards {
                let shard = shard?;
                if !shard.file_type()?.is_dir() {
                    continue;
                }
                for file in std::fs::read_dir(shard.path())? {
                    let file = file?;
                    let name = file.file_name();
                    let is_stale = name
                        .to_str()
                        .and_then(|name| name.strip_suffix(SUFFIX_OF_TEMPORARY_FILES))
                        .and_then(|id| id.parse::<u64>().ok())
                        .is_some_and(|id| {
                            Snowflake::from_raw(id).unix_milliseconds() < cutoff_unix_milliseconds
                        });
                    if is_stale {
                        std::fs::remove_file(file.path())?;
                        removed += 1;
                    }
                }
            }
            Ok(removed)
        })
        .await?
    }

    /// Ids of every stored file, for the sweep of files whose row is gone.
    pub(crate) async fn stored_ids(&self) -> Result<Vec<Snowflake>, AttachmentFileError> {
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || {
            let mut ids = Vec::new();
            let Ok(shards) = std::fs::read_dir(&directory) else {
                return Ok(ids);
            };
            for shard in shards {
                let shard = shard?;
                if !shard.file_type()?.is_dir() {
                    continue;
                }
                for file in std::fs::read_dir(shard.path())? {
                    let name = file?.file_name();
                    if let Some(id) = name.to_str().and_then(|name| name.parse::<u64>().ok()) {
                        ids.push(Snowflake::from_raw(id));
                    }
                }
            }
            Ok(ids)
        })
        .await?
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Readable by the service account only, like the database.
fn write_private_file(path: &Path, parts: &[&[u8]]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    for part in parts {
        file.write_all(part)?;
    }
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(directory: &Path) -> AttachmentFiles {
        AttachmentFiles::new(directory.to_path_buf(), &Zeroizing::new([7u8; 32])).unwrap()
    }

    #[tokio::test]
    async fn files_round_trip_sealed_and_cannot_pass_for_another_id() {
        let directory = tempfile::tempdir().unwrap();
        let store = files(directory.path());
        let id = Snowflake::from_raw(4242);
        store.write(id, b"hello attachment".to_vec()).await.unwrap();

        let on_disk = std::fs::read(store.path_of(id)).unwrap();
        assert!(
            !on_disk.windows(5).any(|window| window == b"hello"),
            "sealed on disk"
        );
        assert_eq!(store.read(id).await.unwrap(), b"hello attachment");

        let other = Snowflake::from_raw(4243);
        std::fs::create_dir_all(store.path_of(other).parent().unwrap()).unwrap();
        std::fs::copy(store.path_of(id), store.path_of(other)).unwrap();
        assert!(matches!(
            store.read(other).await,
            Err(AttachmentFileError::Crypto)
        ));

        assert_eq!(store.stored_ids().await.unwrap().len(), 2);
        store.delete(id).await.unwrap();
        store.delete(id).await.unwrap();
        assert!(store.read(id).await.is_err());
    }

    #[tokio::test]
    async fn another_key_cannot_open_a_file() {
        let directory = tempfile::tempdir().unwrap();
        let id = Snowflake::from_raw(9);
        files(directory.path())
            .write(id, b"secret".to_vec())
            .await
            .unwrap();
        let other_key =
            AttachmentFiles::new(directory.path().to_path_buf(), &Zeroizing::new([8u8; 32]))
                .unwrap();
        assert!(matches!(
            other_key.read(id).await,
            Err(AttachmentFileError::Crypto)
        ));
    }
}
