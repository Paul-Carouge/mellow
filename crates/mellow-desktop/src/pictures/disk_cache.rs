//! Profile pictures, icons and custom emoji kept on disk between launches, in the user's cache
//! directory: immutable files (a new picture gets a new id), so they are never revalidated.
//!
//! The directory is the user's own (0700, files 0600), as a browser's cache is, bounded in bytes
//! with the least recently used files leaving first, and deleted on sign-out. Conversation files
//! (attachments, previews) never come here: they stay in memory only (ADR-0051).
//!
//! Every call does blocking file I/O: run them with `spawn_blocking`.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use mellow_client_core::{DownloadableFile, InstanceAddress};

use super::lru::ByteBoundedLru;

/// About two thousand avatars and emoji at their stored sizes.
pub(crate) const DISK_CACHE_CAPACITY_BYTES: usize = 128 * 1024 * 1024;
pub(crate) const APPLICATION_DIRECTORY: &str = "mellow";
const PICTURES_DIRECTORY: &str = "pictures";
const PARTIAL_FILE_SUFFIX: &str = ".partial";

/// The file name a cached picture is stored under, or `None` for files kept in memory only.
pub(crate) fn disk_file_name(file: DownloadableFile) -> Option<String> {
    match file {
        DownloadableFile::ProfileImage(id) => Some(format!("profile-{id}")),
        DownloadableFile::ProfileImageStill(id) => Some(format!("profile-still-{id}")),
        DownloadableFile::Icon(id) => Some(format!("icon-{id}")),
        DownloadableFile::IconStill(id) => Some(format!("icon-still-{id}")),
        DownloadableFile::Attachment(_)
        | DownloadableFile::AttachmentPreview(_)
        | DownloadableFile::LinkPreviewImage { .. }
        | DownloadableFile::Sound(_)
        | DownloadableFile::LinkIcon(_) => None,
    }
}

/// `<user cache>/mellow/pictures`, removed whole on sign-out.
pub(crate) fn pictures_directory() -> Option<PathBuf> {
    Some(
        dirs::cache_dir()?
            .join(&*crate::profile::with_profile_suffix(APPLICATION_DIRECTORY))
            .join(PICTURES_DIRECTORY),
    )
}

/// One directory per instance: ids are only unique within an instance.
pub(crate) fn instance_directory_name(instance: &InstanceAddress) -> String {
    instance
        .as_str()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '.' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn create_private_directory(directory: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
        builder.mode(0o700);
        builder.create(directory)?;
        // An existing directory keeps its mode through `create`: tighten it in any case.
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    {
        builder.create(directory)
    }
}

fn write_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)
}

#[derive(Debug)]
pub(crate) struct DiskCache {
    directory: PathBuf,
    /// File names and sizes; the value carries nothing. Writes hold it, so `close` waits for them.
    index: Mutex<ByteBoundedLru<String, ()>>,
    /// Set at sign-out before the directory is deleted: no write may land in it afterwards.
    is_closed: AtomicBool,
}

impl DiskCache {
    /// Opens (or creates) `directory`, indexing the files already there oldest first by their
    /// last use, and dropping any over the capacity or left half-written.
    ///
    /// # Errors
    /// When the directory cannot be created or read.
    pub(crate) fn open(directory: PathBuf, capacity_bytes: usize) -> io::Result<Self> {
        create_private_directory(&directory)?;
        let mut files = Vec::new();
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let metadata = entry.metadata()?;
            if !metadata.is_file() {
                continue;
            }
            if name.ends_with(PARTIAL_FILE_SUFFIX) {
                let _ = fs::remove_file(entry.path());
                continue;
            }
            let last_use = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let byte_count = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
            files.push((last_use, name, byte_count));
        }
        files.sort();
        let mut index = ByteBoundedLru::new(capacity_bytes);
        for (_, name, byte_count) in files {
            for (evicted_name, ()) in index.insert(name, (), byte_count) {
                let _ = fs::remove_file(directory.join(evicted_name));
            }
        }
        Ok(Self {
            directory,
            index: Mutex::new(index),
            is_closed: AtomicBool::new(false),
        })
    }

    fn index(&self) -> std::sync::MutexGuard<'_, ByteBoundedLru<String, ()>> {
        // The index holds no invariant a panicking holder could break halfway.
        self.index.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The cached bytes, marked as just used.
    pub(crate) fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.index().get(&name.to_owned())?;
        let path = self.directory.join(name);
        let Ok(bytes) = fs::read(&path) else {
            self.index().remove(&name.to_owned());
            return None;
        };
        // The modification time records the last use for the next launch's index.
        if let Ok(file) = fs::File::options().write(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        Some(bytes)
    }

    /// Stores `bytes` under `name`, written aside first so a crash never leaves a torn file.
    ///
    /// # Errors
    /// When the file cannot be written, or the cache is closed.
    pub(crate) fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let mut index = self.index();
        if self.is_closed.load(Ordering::Acquire) {
            return Err(io::Error::other("the picture cache is closed"));
        }
        let path = self.directory.join(name);
        let partial_path = self.directory.join(format!("{name}{PARTIAL_FILE_SUFFIX}"));
        write_private_file(&partial_path, bytes)?;
        fs::rename(&partial_path, &path)?;
        let evicted = index.insert(name.to_owned(), (), bytes.len());
        for (evicted_name, ()) in evicted {
            let _ = fs::remove_file(self.directory.join(evicted_name));
        }
        Ok(())
    }

    /// Refuses every later write, once the write in progress (if any) is done: the directory can
    /// then be deleted without a file landing in it. Blocks on that write.
    pub(crate) fn close(&self) {
        let _index = self.index();
        self.is_closed.store(true, Ordering::Release);
    }

    /// Forgets a file that turned out unreadable as a picture.
    pub(crate) fn remove(&self, name: &str) {
        if self.index().remove(&name.to_owned()).is_some() {
            let _ = fs::remove_file(self.directory.join(name));
        }
    }
}

/// Deletes every instance's cached pictures: a signed-out computer keeps no one's pictures.
///
/// # Errors
/// When the directory exists and cannot be removed.
pub(crate) fn remove_pictures_directory(directory: &Path) -> io::Result<()> {
    match fs::remove_dir_all(directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::Snowflake;

    use super::*;

    fn snowflake(value: u64) -> Snowflake {
        Snowflake::from_raw(value)
    }

    #[test]
    fn only_profile_pictures_and_icons_go_to_disk() {
        assert_eq!(
            disk_file_name(DownloadableFile::ProfileImage(snowflake(7))).as_deref(),
            Some("profile-7")
        );
        assert_eq!(
            disk_file_name(DownloadableFile::IconStill(snowflake(9))).as_deref(),
            Some("icon-still-9")
        );
        assert_eq!(
            disk_file_name(DownloadableFile::Attachment(snowflake(1))),
            None
        );
        assert_eq!(
            disk_file_name(DownloadableFile::AttachmentPreview(snowflake(1))),
            None
        );
    }

    #[test]
    fn instance_directories_hold_only_safe_characters() {
        let instance = InstanceAddress::parse("https://chat.example.org:8443/mellow/").unwrap();
        assert_eq!(
            instance_directory_name(&instance),
            "https___chat.example.org_8443_mellow_"
        );
    }

    #[test]
    fn files_round_trip_and_the_oldest_leave_past_the_capacity() {
        let directory = tempfile::tempdir().unwrap();
        let cache_directory = directory.path().join("instance");
        let cache = DiskCache::open(cache_directory.clone(), 10).unwrap();
        cache.write("a", b"1234").unwrap();
        cache.write("b", b"5678").unwrap();
        assert_eq!(cache.read("a").as_deref(), Some(&b"1234"[..]));
        cache.write("c", b"9012").unwrap();
        assert_eq!(cache.read("b"), None, "b was the least recently used");
        assert!(!cache_directory.join("b").exists());
        assert_eq!(cache.read("c").as_deref(), Some(&b"9012"[..]));
    }

    #[test]
    fn a_reopened_cache_finds_its_files_and_drops_half_written_ones() {
        let directory = tempfile::tempdir().unwrap();
        let cache_directory = directory.path().join("instance");
        DiskCache::open(cache_directory.clone(), 100)
            .unwrap()
            .write("a", b"1234")
            .unwrap();
        fs::write(cache_directory.join("b.partial"), b"torn").unwrap();
        let reopened = DiskCache::open(cache_directory.clone(), 100).unwrap();
        assert_eq!(reopened.read("a").as_deref(), Some(&b"1234"[..]));
        assert!(!cache_directory.join("b.partial").exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_directory_and_files_are_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().unwrap();
        let cache_directory = directory.path().join("instance");
        fs::create_dir_all(&cache_directory).unwrap();
        fs::set_permissions(&cache_directory, fs::Permissions::from_mode(0o755)).unwrap();
        let cache = DiskCache::open(cache_directory.clone(), 100).unwrap();
        cache.write("a", b"1234").unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&cache_directory), 0o700);
        assert_eq!(mode(&cache_directory.join("a")), 0o600);
    }

    #[test]
    fn a_closed_cache_writes_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let cache_directory = directory.path().join("instance");
        let cache = DiskCache::open(cache_directory.clone(), 100).unwrap();
        cache.close();
        assert!(cache.write("a", b"1234").is_err());
        assert!(!cache_directory.join("a").exists());
        assert!(!cache_directory.join("a.partial").exists());
    }

    #[test]
    fn signing_out_removes_everything_and_tolerates_a_missing_directory() {
        let directory = tempfile::tempdir().unwrap();
        let pictures = directory.path().join("pictures");
        DiskCache::open(pictures.join("instance"), 100)
            .unwrap()
            .write("a", b"1234")
            .unwrap();
        remove_pictures_directory(&pictures).unwrap();
        assert!(!pictures.exists());
        remove_pictures_directory(&pictures).unwrap();
    }
}
