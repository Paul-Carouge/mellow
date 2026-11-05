//! Attachment files sealed on disk (ADR-0020): AES-256-GCM under a key derived from the master
//! key, a random nonce per file, the attachment id as associated data so a file cannot be swapped
//! for another one's. Layout: `<directory>/<shard>/<id>`, each file `nonce ‖ sealed`, the shard
//! being the two hex digits of the top byte of the id mixed by `SplitMix64`'s finaliser (ADR-0034).
//! Before it, the shard was the id's low byte, zero for nearly every generated id: such files are
//! still found where they are, and `move_files_to_their_shards` moves them at startup.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use mellow_protocol::Snowflake;
use zeroize::Zeroizing;

pub(crate) const SUFFIX_OF_TEMPORARY_FILES: &str = ".partial";
/// Written in the attachment directory once a move left every file in its shard, so later starts
/// skip the listing (ADR-0034). Not a directory: `backup` does not copy it, so a restored backup
/// is walked again once.
pub(crate) const NAME_OF_MARKER_OF_FINISHED_MOVE: &str = ".moved-to-shards";

/// Files a sweep deleted, and files it could not delete and left for the next sweep.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SweptFiles {
    pub(crate) deleted: usize,
    pub(crate) failed: usize,
}

/// Files a startup move put in their shard, and files it could not move and left where they are,
/// still found there.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MovedFiles {
    pub(crate) moved: usize,
    pub(crate) failed: usize,
}

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

    pub(crate) fn path_of(&self, id: Snowflake) -> PathBuf {
        self.directory.join(shard_of(id)).join(id.raw().to_string())
    }

    /// Where the file was stored before ADR-0034, if it was never moved.
    pub(crate) fn legacy_path_of(&self, id: Snowflake) -> PathBuf {
        self.directory
            .join(format!("{:02x}", id.raw() % 256))
            .join(id.raw().to_string())
    }

    /// The file's path if it exists, in its shard or where it was stored before ADR-0034.
    pub(crate) fn existing_path_of(&self, id: Snowflake) -> Option<PathBuf> {
        [self.path_of(id), self.legacy_path_of(id)]
            .into_iter()
            .find(|path| path.is_file())
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
        let legacy_path = self.legacy_path_of(id);
        let key = self.key.clone();
        tokio::task::spawn_blocking(move || {
            // Opened in the buffer read from disk: no second copy of the file in memory.
            let mut contents = match std::fs::read(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::read(&legacy_path)?
                }
                read => read?,
            };
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
        for path in [self.path_of(id), self.legacy_path_of(id)] {
            match tokio::fs::remove_file(path).await {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.into());
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Moves files stored before ADR-0034 to their shard, each with one rename within the
    /// directory: atomic, so a crash leaves every file in one place or the other, and the next
    /// start goes on. Temporary files stay where they are, for the sweep. Only shard directories
    /// are listed. A file or a directory that cannot be moved or listed is counted and left, still
    /// found where it is: reads fall back to the old place, so this never stops the start. A move
    /// without failure leaves a marker, and a start that finds it lists nothing.
    pub(crate) async fn move_files_to_their_shards(
        &self,
    ) -> Result<MovedFiles, AttachmentFileError> {
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut moved_files = MovedFiles::default();
            let marker = store.directory.join(NAME_OF_MARKER_OF_FINISHED_MOVE);
            if marker.is_file() {
                return Ok(moved_files);
            }
            let Ok(shards) = std::fs::read_dir(&store.directory) else {
                return Ok(moved_files);
            };
            for shard in shards {
                let listed_files = shard.and_then(|shard| {
                    let is_shard = shard.file_type()?.is_dir()
                        && shard.file_name().to_str().is_some_and(is_shard_name);
                    Ok(if is_shard {
                        Some((shard.file_name(), std::fs::read_dir(shard.path())?))
                    } else {
                        None
                    })
                });
                let (shard_name, files) = match listed_files {
                    Ok(Some(listed)) => listed,
                    Ok(None) => continue,
                    Err(error) => {
                        tracing::warn!(%error, "cannot list an attachment shard directory");
                        moved_files.failed += 1;
                        continue;
                    }
                };
                for file in files {
                    let file = match file {
                        Ok(file) => file,
                        Err(error) => {
                            tracing::warn!(%error, "cannot list an attachment shard directory");
                            moved_files.failed += 1;
                            continue;
                        }
                    };
                    let Some(id) = file.file_name().to_str().and_then(|name| name.parse().ok())
                    else {
                        continue;
                    };
                    let id = Snowflake::from_raw(id);
                    if shard_name.to_str() == Some(shard_of(id).as_str()) {
                        continue;
                    }
                    let destination = store.path_of(id);
                    let renamed = destination
                        .parent()
                        .map_or(Ok(()), crate::storage::create_private_directory)
                        .and_then(|()| std::fs::rename(file.path(), &destination));
                    match renamed {
                        Ok(()) => moved_files.moved += 1,
                        Err(error) => {
                            // The file name is an attachment id: left out of the log.
                            tracing::warn!(%error, "cannot move an attachment file to its shard");
                            moved_files.failed += 1;
                        }
                    }
                }
            }
            if moved_files.failed == 0
                && let Err(error) = write_private_file(&marker, &[])
            {
                // Only costs a listing at the next start, which tries again.
                tracing::warn!(%error, "cannot record that attachment files are in their shards");
            }
            Ok(moved_files)
        })
        .await?
    }

    /// Temporary files left by a crash mid-write, created before the cutoff (from their id). A
    /// file that cannot be deleted is counted and skipped: it must not stop the others.
    pub(crate) async fn remove_partial_files_before(
        &self,
        cutoff_unix_milliseconds: u64,
    ) -> Result<SweptFiles, AttachmentFileError> {
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || {
            let mut swept = SweptFiles::default();
            let Ok(shards) = std::fs::read_dir(&directory) else {
                return Ok(swept);
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
                    if !is_stale {
                        continue;
                    }
                    match std::fs::remove_file(file.path()) {
                        Ok(()) => swept.deleted += 1,
                        Err(error) => {
                            // The file name is an attachment id: left out of the log.
                            tracing::warn!(%error, "cannot delete a temporary attachment file");
                            swept.failed += 1;
                        }
                    }
                }
            }
            Ok(swept)
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

/// The shard directory of a file: generated ids differ mostly in their timestamp bits, so they are
/// mixed by `SplitMix64`'s finaliser (every input bit flips each output bit half the time) and the
/// top byte is kept. It places files on disk: never change it without moving them.
fn shard_of(id: Snowflake) -> String {
    let mut mixed = id.raw();
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    mixed ^= mixed >> 31;
    format!("{:02x}", mixed >> 56)
}

/// A name `shard_of` or the rule before ADR-0034 can give: two lowercase hexadecimal digits.
fn is_shard_name(name: &str) -> bool {
    name.len() == 2
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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

    /// Ids an instance generates: one upload every 37 ms, from the server's generator.
    fn generated_ids(count: usize) -> Vec<Snowflake> {
        let generator = crate::domain::snowflake_generator::SnowflakeGenerator::default();
        (0..count)
            .map(|index| {
                let now = mellow_protocol::MELLOW_EPOCH_UNIX_MILLISECONDS
                    + 86_400_000
                    + 37 * u64::try_from(index).unwrap();
                generator.next_id(now).unwrap()
            })
            .collect()
    }

    /// Writes the file where it was stored before ADR-0034.
    async fn write_in_legacy_place(store: &AttachmentFiles, id: Snowflake) {
        store
            .write(id, id.raw().to_le_bytes().to_vec())
            .await
            .unwrap();
        let legacy = store.legacy_path_of(id);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::rename(store.path_of(id), legacy).unwrap();
    }

    #[test]
    fn generated_ids_spread_over_every_shard() {
        let shard_count = 256;
        let files_per_shard = 100;
        let mut counts = std::collections::HashMap::<String, usize>::new();
        for id in generated_ids(shard_count * files_per_shard) {
            assert_eq!(id.raw() % 256, 0, "the old rule put them all in 00");
            *counts.entry(shard_of(id)).or_default() += 1;
        }

        assert_eq!(counts.len(), shard_count);
        assert!(
            counts
                .values()
                .all(|count| (files_per_shard / 2..=files_per_shard * 2).contains(count)),
            "{counts:?}"
        );
    }

    #[tokio::test]
    async fn a_file_stored_before_the_new_shards_is_read_and_deleted_where_it_is() {
        let directory = tempfile::tempdir().unwrap();
        let store = files(directory.path());
        let id = generated_ids(1)[0];
        write_in_legacy_place(&store, id).await;

        assert_eq!(store.existing_path_of(id), Some(store.legacy_path_of(id)));
        assert_eq!(store.read(id).await.unwrap(), id.raw().to_le_bytes());
        assert_eq!(store.stored_ids().await.unwrap(), vec![id]);
        store.delete(id).await.unwrap();
        assert_eq!(store.existing_path_of(id), None);
        assert_eq!(store.stored_ids().await.unwrap(), Vec::new());
    }

    #[tokio::test]
    async fn a_move_stopped_halfway_goes_on_at_the_next_start() {
        let directory = tempfile::tempdir().unwrap();
        let store = files(directory.path());
        let ids = generated_ids(40);
        for id in &ids {
            write_in_legacy_place(&store, *id).await;
        }
        // As if the process had stopped after moving the first half.
        for id in &ids[..20] {
            let path = store.path_of(*id);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::rename(store.legacy_path_of(*id), path).unwrap();
        }
        let partial = with_suffix(&store.legacy_path_of(ids[0]), SUFFIX_OF_TEMPORARY_FILES);
        std::fs::write(&partial, b"half a file").unwrap();
        // The few whose shard is still the old one have nothing to move.
        let left_to_move = ids[20..]
            .iter()
            .filter(|id| store.path_of(**id) != store.legacy_path_of(**id))
            .count();

        let resumed = store.move_files_to_their_shards().await.unwrap();
        let again = store.move_files_to_their_shards().await.unwrap();

        assert_eq!(
            resumed,
            MovedFiles {
                moved: left_to_move,
                failed: 0
            }
        );
        assert_eq!(again, MovedFiles::default());
        for id in &ids {
            assert_eq!(store.existing_path_of(*id), Some(store.path_of(*id)));
            assert_eq!(store.read(*id).await.unwrap(), id.raw().to_le_bytes());
        }
        assert!(partial.exists(), "temporary files are left to the sweep");
        let mut stored = store.stored_ids().await.unwrap();
        stored.sort_unstable();
        assert_eq!(stored, ids);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_unreadable_directory_does_not_stop_the_move() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let store = files(directory.path());
        let id = generated_ids(1)[0];
        write_in_legacy_place(&store, id).await;
        // As `lost+found` when `attachments/` is its own mount point: skipped, never listed.
        let foreign = directory.path().join("lost+found");
        // A shard the service account cannot list: counted, the others still moved.
        let locked_shard = directory.path().join("ab");
        for locked in [&foreign, &locked_shard] {
            std::fs::create_dir(locked).unwrap();
            std::fs::set_permissions(locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        }

        let moved = store.move_files_to_their_shards().await;
        for locked in [&foreign, &locked_shard] {
            std::fs::set_permissions(locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        }

        assert_eq!(
            moved.unwrap(),
            MovedFiles {
                moved: 1,
                failed: 1
            }
        );
        assert_eq!(store.existing_path_of(id), Some(store.path_of(id)));
        assert!(
            !directory
                .path()
                .join(NAME_OF_MARKER_OF_FINISHED_MOVE)
                .exists(),
            "a move with a failure is tried again at the next start"
        );
    }

    #[tokio::test]
    async fn a_finished_move_is_not_tried_again() {
        let directory = tempfile::tempdir().unwrap();
        let store = files(directory.path());
        let [first, second] = generated_ids(2)[..] else {
            unreachable!()
        };
        write_in_legacy_place(&store, first).await;
        let first_start = store.move_files_to_their_shards().await.unwrap();
        // Put back in the old place after the move: only a new listing would move it.
        write_in_legacy_place(&store, second).await;

        let second_start = store.move_files_to_their_shards().await.unwrap();

        assert_eq!(
            first_start,
            MovedFiles {
                moved: 1,
                failed: 0
            }
        );
        assert_eq!(second_start, MovedFiles::default());
        assert_eq!(
            store.existing_path_of(second),
            Some(store.legacy_path_of(second)),
            "not listed again"
        );
        assert_eq!(
            store.read(second).await.unwrap(),
            second.raw().to_le_bytes()
        );
    }
}
