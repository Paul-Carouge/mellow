//! Per-device preferences (DESIGN.md §5.9): the theme, reduced motion and which servers keep their
//! member sheet open. Stored as a small `key=value` file in the OS configuration directory; a
//! missing or unreadable file gives the defaults, and unknown lines are ignored.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use slint::ComponentHandle as _;

use crate::generated_ui::{AppWindow, MotionPreference, ThemeChoice};

const FILE_NAME: &str = "preferences";
/// Ours stays far below; anything larger was not written by Mellow and is not read.
const MAXIMUM_FILE_LENGTH: u64 = 64 * 1024;
const THEME_KEY: &str = "theme";
const REDUCED_MOTION_KEY: &str = "reduced-motion";
const MEMBER_SHEET_OPEN_KEY: &str = "member-sheet-open";

/// The latest save asked for; an older save that starts after it skips its write.
static LATEST_SAVE_REQUEST: AtomicU64 = AtomicU64::new(0);
/// One write at a time, so the file is never written by two threads at once.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq)]
pub struct Preferences {
    pub theme: ThemeChoice,
    pub is_motion_reduced: bool,
    /// Server ids whose member sheet was left open.
    member_sheet_open_server_ids: BTreeSet<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            is_motion_reduced: false,
            member_sheet_open_server_ids: BTreeSet::new(),
        }
    }
}

impl Preferences {
    /// Reads the stored preferences; the defaults when there are none or they cannot be read.
    pub fn load() -> Self {
        let Some(path) = file_path() else {
            return Self::default();
        };
        let read = std::fs::File::open(&path).and_then(|file| {
            use std::io::Read as _;
            let mut text = String::new();
            file.take(MAXIMUM_FILE_LENGTH).read_to_string(&mut text)?;
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

    pub fn is_member_sheet_open(&self, server_id: &str) -> bool {
        self.member_sheet_open_server_ids.contains(server_id)
    }

    pub fn set_member_sheet_open(&mut self, server_id: &str, is_open: bool) {
        if is_open {
            self.member_sheet_open_server_ids
                .insert(server_id.to_owned());
        } else {
            self.member_sheet_open_server_ids.remove(server_id);
        }
    }

    fn from_file_text(text: &str) -> Self {
        let mut preferences = Self::default();
        for (key, value) in text.lines().filter_map(|line| line.split_once('=')) {
            match (key.trim(), value.trim()) {
                (THEME_KEY, "system") => preferences.theme = ThemeChoice::System,
                (THEME_KEY, "dark") => preferences.theme = ThemeChoice::Dark,
                (THEME_KEY, "light") => preferences.theme = ThemeChoice::Light,
                (REDUCED_MOTION_KEY, value) => preferences.is_motion_reduced = value == "true",
                (MEMBER_SHEET_OPEN_KEY, ids) => {
                    preferences.member_sheet_open_server_ids = ids
                        .split(',')
                        .map(str::trim)
                        .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
                        .map(str::to_owned)
                        .collect();
                }
                _ => {}
            }
        }
        preferences
    }

    fn to_file_text(&self) -> String {
        let theme = match self.theme {
            ThemeChoice::Dark => "dark",
            ThemeChoice::Light => "light",
            ThemeChoice::System => "system",
        };
        let server_ids: Vec<&str> = self
            .member_sheet_open_server_ids
            .iter()
            .map(String::as_str)
            .collect();
        format!(
            "{THEME_KEY}={theme}\n{REDUCED_MOTION_KEY}={}\n{MEMBER_SHEET_OPEN_KEY}={}\n",
            self.is_motion_reduced,
            server_ids.join(",")
        )
    }

    /// Writes the preferences off the UI thread; of several quick saves, the latest wins.
    pub fn save_in_background(&self) {
        let Some(path) = file_path() else {
            return;
        };
        let text = self.to_file_text();
        let request = LATEST_SAVE_REQUEST.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::spawn(move || {
            let _guard = WRITE_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if LATEST_SAVE_REQUEST.load(Ordering::SeqCst) != request {
                return;
            }
            if let Err(error) = write_atomically(&path, &text) {
                tracing::warn!(error = %error.kind(), "cannot save the preferences");
            }
        });
    }

    /// Shows the theme and motion choices in the window.
    pub fn apply_appearance(&self, window: &AppWindow) {
        window.set_theme_choice(self.theme);
        window
            .global::<MotionPreference>()
            .set_is_reduced(self.is_motion_reduced);
    }

    /// Reads back the choices made in Appearance.
    pub fn read_appearance(&mut self, window: &AppWindow) {
        self.theme = window.get_theme_choice();
        self.is_motion_reduced = window.global::<MotionPreference>().get_is_reduced();
    }
}

fn write_atomically(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let directory = path
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    std::fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(text.as_bytes())?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// `~/Library/Application Support/Mellow` on macOS, `%APPDATA%\Mellow` on Windows,
/// `$XDG_CONFIG_HOME/mellow` (or `~/.config/mellow`) elsewhere.
fn file_path() -> Option<PathBuf> {
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

    #[test]
    fn preferences_survive_a_round_trip_through_the_file() {
        let mut preferences = Preferences {
            theme: ThemeChoice::Light,
            is_motion_reduced: true,
            ..Preferences::default()
        };
        preferences.set_member_sheet_open("42", true);
        preferences.set_member_sheet_open("7", true);
        preferences.set_member_sheet_open("7", false);

        let read_back = Preferences::from_file_text(&preferences.to_file_text());

        assert_eq!(read_back, preferences);
        assert!(read_back.is_member_sheet_open("42"));
        assert!(!read_back.is_member_sheet_open("7"));
    }

    #[test]
    fn unknown_or_malformed_lines_keep_the_defaults() {
        let text = "theme=sepia\nreduced-motion=maybe\nmember-sheet-open=12,../etc, ,34\nnoise\n";

        let preferences = Preferences::from_file_text(text);

        assert_eq!(preferences.theme, ThemeChoice::System);
        assert!(!preferences.is_motion_reduced);
        let ids: Vec<_> = preferences.member_sheet_open_server_ids.iter().collect();
        assert_eq!(ids, ["12", "34"]);
    }
}
