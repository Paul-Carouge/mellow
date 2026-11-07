//! Per-device settings (ADR-0051 "Session"): one small `key=value` file in the OS configuration
//! directory, the desktop's counterpart of the web client's `localStorage`. A missing or unreadable
//! file gives the defaults, unknown lines are ignored and every value is validated, since anything
//! may have written the file. Reads and writes block: run them off the UI thread.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use gpui::App;
use mellow_client_core::InstanceAddress;

const FILE_NAME: &str = "preferences";
/// Ours stays far below; anything larger was not written by Mellow and is not read.
const MAXIMUM_FILE_LENGTH: u64 = 64 * 1024;
const INSTANCE_ADDRESS_KEY: &str = "instance-address";

/// The latest save asked for; an older save that starts after it skips its write.
static LATEST_SAVE_REQUEST: AtomicU64 = AtomicU64::new(0);
/// One write at a time, so the file is never written by two threads at once.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preferences {
    /// The instance of the last sign-in, offered again on the instance address screen.
    pub last_instance_address: Option<InstanceAddress>,
}

impl Preferences {
    /// The stored preferences; the defaults when there are none or they cannot be read. Blocks.
    #[must_use]
    pub fn load() -> Self {
        preferences_file_path().map_or_else(Self::default, |path| Self::load_from(&path))
    }

    /// Only a regular file of a size Mellow writes is read: a FIFO would block, and anything
    /// larger was not written by Mellow.
    fn load_from(path: &Path) -> Self {
        let read = std::fs::metadata(path).and_then(|metadata| {
            if !metadata.is_file() {
                return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
            }
            let mut text = String::new();
            std::fs::File::open(path)?
                .take(MAXIMUM_FILE_LENGTH + 1)
                .read_to_string(&mut text)?;
            if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAXIMUM_FILE_LENGTH {
                return Err(std::io::Error::from(std::io::ErrorKind::FileTooLarge));
            }
            Ok(text)
        });
        match read {
            Ok(text) => Self::from_file_text(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                tracing::warn!(error = %error.kind(), "cannot read the preferences; using the defaults");
                Self::default()
            }
        }
    }

    fn from_file_text(text: &str) -> Self {
        let mut preferences = Self::default();
        for (key, value) in text.lines().filter_map(|line| line.split_once('=')) {
            if key.trim() == INSTANCE_ADDRESS_KEY {
                // The same https rule as a typed address: a tampered file cannot downgrade it.
                preferences.last_instance_address = InstanceAddress::parse(value.trim()).ok();
            }
        }
        preferences
    }

    fn to_file_text(&self) -> String {
        let instance_address = self
            .last_instance_address
            .as_ref()
            .map_or("", InstanceAddress::as_str);
        format!("{INSTANCE_ADDRESS_KEY}={instance_address}\n")
    }

    /// Writes the preferences on GPUI's background executor; of several quick saves, the latest
    /// wins.
    pub fn save_in_background(&self, cx: &App) {
        let Some(path) = preferences_file_path() else {
            return;
        };
        let text = self.to_file_text();
        let request = LATEST_SAVE_REQUEST.fetch_add(1, Ordering::SeqCst) + 1;
        cx.background_executor()
            .spawn(async move {
                let _write = WRITE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
                if LATEST_SAVE_REQUEST.load(Ordering::SeqCst) != request {
                    return;
                }
                if let Err(error) = write_atomically(&path, &text) {
                    tracing::warn!(error = %error.kind(), "cannot save the preferences");
                }
            })
            .detach();
    }
}

/// A crash or a full disk mid-write leaves the previous file, never half of the new one.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    std::fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(text.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// `~/Library/Application Support/Mellow` on macOS, `%APPDATA%\Mellow` on Windows,
/// `$XDG_CONFIG_HOME/mellow` (or `~/.config/mellow`) elsewhere.
fn preferences_file_path() -> Option<PathBuf> {
    let directory = if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/Mellow")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?).join("Mellow")
    } else {
        // The XDG base directory spec ignores relative paths.
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?
            .join("mellow")
    };
    Some(directory.join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_instance(address: &str) -> Preferences {
        Preferences {
            last_instance_address: Some(InstanceAddress::parse(address).unwrap()),
        }
    }

    #[test]
    fn preferences_survive_a_round_trip_through_an_atomic_write() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested").join(FILE_NAME);
        let preferences = with_instance("chat.example.org");

        write_atomically(&path, &preferences.to_file_text()).unwrap();
        assert_eq!(Preferences::load_from(&path), preferences);

        write_atomically(&path, &Preferences::default().to_file_text()).unwrap();
        assert_eq!(Preferences::load_from(&path), Preferences::default());
        // Only the file itself: no temporary file is left behind.
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn only_a_regular_file_of_a_sane_size_is_read() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE_NAME);
        let line = "instance-address=https://chat.example.org/\n";
        std::fs::write(&path, line).unwrap();
        assert_eq!(
            Preferences::load_from(&path),
            with_instance("chat.example.org")
        );

        // Truncated reading would keep the first 64 KiB of an oversized file.
        let oversized = format!(
            "{line}{}",
            "#".repeat(usize::try_from(MAXIMUM_FILE_LENGTH).unwrap())
        );
        std::fs::write(&path, oversized).unwrap();
        assert_eq!(Preferences::load_from(&path), Preferences::default());

        // A directory (or a FIFO) is never opened for reading.
        assert_eq!(
            Preferences::load_from(directory.path()),
            Preferences::default()
        );
        assert_eq!(
            Preferences::load_from(&directory.path().join("missing")),
            Preferences::default()
        );
    }

    #[test]
    fn unknown_or_invalid_lines_keep_the_defaults() {
        for text in [
            "instance-address=http://chat.example.org\n",
            "instance-address=ftp://chat.example.org\n",
            "instance-address=https://user:secret@chat.example.org\n",
            "noise\ntheme=sepia\n",
        ] {
            assert_eq!(
                Preferences::from_file_text(text),
                Preferences::default(),
                "{text}"
            );
        }
    }
}
