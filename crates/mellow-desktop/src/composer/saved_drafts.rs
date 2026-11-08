//! Unsent text kept across launches (DESIGN.md §5.5 "Draft"): one small file per account and
//! instance in the user's data directory, private to the user (0700 directory, 0600 file),
//! written whole and atomically, deleted with every account's drafts at sign-out. Files in the
//! tray are not kept: they were uploads to the instance, gone with the app.
//!
//! The file holds people's words: nothing here logs its content or its path. Reads and writes
//! block: run them off the UI thread.

use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use gpui::{App, AppContext as _};

use mellow_client_core::InstanceAddress;
use mellow_client_core::shared_logic::message_length::MAXIMUM_MESSAGE_LENGTH;
use mellow_protocol::Snowflake;

use crate::pictures::disk_cache::{
    APPLICATION_DIRECTORY, create_private_directory, instance_directory_name,
};

const DRAFTS_DIRECTORY: &str = "drafts";
const FILE_HEADER: &str = "mellow-drafts 1";
/// The channels with the latest drafts only (the preferences' `MAXIMUM_REMEMBERED_PLACES`), so
/// the file stays small whatever was typed where.
pub const MAXIMUM_SAVED_DRAFTS: usize = 200;
/// The longest message that can be sent, at four bytes a character: anything longer could not
/// be sent anyway and is cut at a character boundary.
pub const MAXIMUM_SAVED_DRAFT_BYTES: usize = 4 * MAXIMUM_MESSAGE_LENGTH;
/// Every draft escaped at worst twice its length, with its id; larger files were not written here.
const MAXIMUM_FILE_BYTES: u64 =
    (MAXIMUM_SAVED_DRAFTS * (2 * MAXIMUM_SAVED_DRAFT_BYTES + 32)) as u64;

/// Bumped at sign-out, at once: writes asked for before it never land after the deletion.
static SESSION_EPOCH: AtomicU64 = AtomicU64::new(0);
/// Writes and the deletion hold it, so the deletion waits for a write under way.
static FILE_ACCESS: Mutex<()> = Mutex::new(());

/// A channel's unsent text, most recent first in a list.
pub type SavedDraft = (Snowflake, String);

/// One account's drafts file, in the session it was opened in. Clones share the order of writes.
#[derive(Debug, Clone)]
pub struct SavedDraftsFile {
    path: PathBuf,
    epoch: u64,
    /// The latest write asked for; an older one that starts after it skips its write.
    latest_write: Arc<AtomicU64>,
}

impl PartialEq for SavedDraftsFile {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl SavedDraftsFile {
    /// `<user data>/mellow/drafts/<instance>-<user id>`, in the current session.
    #[must_use]
    pub fn for_account(instance: &InstanceAddress, user_id: Snowflake) -> Option<Self> {
        let file_name = format!("{}-{user_id}", instance_directory_name(instance));
        Some(Self::at(drafts_directory()?.join(file_name)))
    }

    pub(super) fn at(path: PathBuf) -> Self {
        Self {
            path,
            epoch: SESSION_EPOCH.load(Ordering::SeqCst),
            latest_write: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Takes a place among the writes asked for: only the latest one writes.
    #[must_use]
    pub fn next_write(&self) -> WriteRequest {
        WriteRequest(self.latest_write.fetch_add(1, Ordering::SeqCst) + 1)
    }

    /// The drafts kept last time, most recent first; none when the file is missing or was not
    /// written by Mellow. Blocks.
    #[must_use]
    pub fn read(&self) -> Vec<SavedDraft> {
        let read = std::fs::metadata(&self.path).and_then(|metadata| {
            // A FIFO would block, and anything larger was not written here.
            if !metadata.is_file() || metadata.len() > MAXIMUM_FILE_BYTES {
                return Err(io::Error::from(io::ErrorKind::InvalidData));
            }
            let mut text = String::new();
            std::fs::File::open(&self.path)?
                .take(MAXIMUM_FILE_BYTES)
                .read_to_string(&mut text)?;
            Ok(text)
        });
        match read {
            Ok(text) => parse_drafts(&text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                tracing::warn!(error = %error.kind(), "cannot read the saved drafts");
                Vec::new()
            }
        }
    }

    /// Writes `drafts` in place of the file (removed when there are none), unless a later write
    /// was asked for or the session ended since. Blocks.
    pub fn write(&self, drafts: &[SavedDraft], request: WriteRequest) {
        let _access = FILE_ACCESS.lock().unwrap_or_else(PoisonError::into_inner);
        if SESSION_EPOCH.load(Ordering::SeqCst) != self.epoch
            || self.latest_write.load(Ordering::SeqCst) != request.0
        {
            return;
        }
        let written = if drafts.is_empty() {
            match std::fs::remove_file(&self.path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            }
        } else {
            write_privately(&self.path, &file_text(drafts))
        };
        if let Err(error) = written {
            tracing::warn!(error = %error.kind(), "cannot save the drafts");
        }
    }
}

/// The order of a write among those asked for on one file.
#[derive(Debug, Clone, Copy)]
pub struct WriteRequest(u64);

/// `<user data>/mellow/drafts`, never in a Windows roaming profile: people's words stay on this
/// machine.
fn drafts_directory() -> Option<PathBuf> {
    Some(
        dirs::data_local_dir()?
            .join(APPLICATION_DIRECTORY)
            .join(DRAFTS_DIRECTORY),
    )
}

/// Deletes every account's drafts off the UI thread: a signed-out computer keeps no one's words.
/// Writes asked for before never land afterwards.
pub fn forget_saved_drafts(cx: &App) {
    SESSION_EPOCH.fetch_add(1, Ordering::SeqCst);
    cx.background_spawn(async { delete_saved_drafts() })
        .detach();
}

/// The same, blocking: at exit, where a detached deletion might never run.
pub fn forget_saved_drafts_now() {
    SESSION_EPOCH.fetch_add(1, Ordering::SeqCst);
    delete_saved_drafts();
}

fn delete_saved_drafts() {
    let Some(directory) = drafts_directory() else {
        return;
    };
    let _access = FILE_ACCESS.lock().unwrap_or_else(PoisonError::into_inner);
    match std::fs::remove_dir_all(&directory) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            tracing::warn!(error = %error.kind(), "cannot delete the drafts after signing out");
        }
        _ => {}
    }
}

/// A crash or a full disk mid-write leaves the previous file, never half of the new one; the
/// temporary file is created 0600 and the directory 0700.
fn write_privately(path: &Path, text: &str) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    create_private_directory(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(text.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// The file's text: a header, then one `<channel id> <escaped text>` line per draft, most recent
/// first, within the bounds.
fn file_text(drafts: &[SavedDraft]) -> String {
    let mut text = format!("{FILE_HEADER}\n");
    for (channel_id, draft) in drafts
        .iter()
        .filter(|(_, draft)| !draft.is_empty())
        .take(MAXIMUM_SAVED_DRAFTS)
    {
        text.push_str(&channel_id.to_string());
        text.push(' ');
        escape_into(within_bound(draft), &mut text);
        text.push('\n');
    }
    text
}

fn within_bound(draft: &str) -> &str {
    if draft.len() <= MAXIMUM_SAVED_DRAFT_BYTES {
        return draft;
    }
    let mut end = MAXIMUM_SAVED_DRAFT_BYTES;
    while !draft.is_char_boundary(end) {
        end -= 1;
    }
    &draft[..end]
}

/// One line per draft: backslashes, line feeds and carriage returns are escaped.
fn escape_into(draft: &str, text: &mut String) {
    for character in draft.chars() {
        match character {
            '\\' => text.push_str("\\\\"),
            '\n' => text.push_str("\\n"),
            '\r' => text.push_str("\\r"),
            other => text.push(other),
        }
    }
}

fn unescape(line: &str) -> Option<String> {
    let mut draft = String::with_capacity(line.len());
    let mut characters = line.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            draft.push(character);
            continue;
        }
        draft.push(match characters.next()? {
            '\\' => '\\',
            'n' => '\n',
            'r' => '\r',
            _ => return None,
        });
    }
    Some(draft)
}

/// Anything may have written the file: lines that are not drafts are ignored, and the bounds
/// hold whatever it says.
fn parse_drafts(text: &str) -> Vec<SavedDraft> {
    let mut lines = text.lines();
    if lines.next() != Some(FILE_HEADER) {
        return Vec::new();
    }
    let mut drafts: Vec<SavedDraft> = Vec::new();
    for line in lines {
        if drafts.len() == MAXIMUM_SAVED_DRAFTS {
            break;
        }
        let Some((channel, escaped)) = line.split_once(' ') else {
            continue;
        };
        let is_decimal = !channel.is_empty()
            && channel.len() <= 20
            && channel.bytes().all(|byte| byte.is_ascii_digit());
        let Some(channel_id) = is_decimal.then(|| channel.parse().ok()).flatten() else {
            continue;
        };
        let Some(draft) = unescape(escaped) else {
            continue;
        };
        let draft = within_bound(&draft).to_owned();
        if !draft.is_empty() && drafts.iter().all(|(id, _)| *id != channel_id) {
            drafts.push((channel_id, draft));
        }
    }
    drafts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(raw: u64) -> Snowflake {
        Snowflake::from_raw(raw)
    }

    #[test]
    fn keeps_every_character_of_a_draft_through_the_file() {
        let drafts = vec![
            (
                channel(2),
                "two lines\nwith a \\ and\r\na return".to_owned(),
            ),
            (channel(1), "café 🙂 \\n is not a line feed".to_owned()),
        ];
        assert_eq!(parse_drafts(&file_text(&drafts)), drafts);
    }

    #[test]
    fn leaves_empty_drafts_out_and_keeps_the_latest_within_the_count() {
        let mut drafts: Vec<SavedDraft> = (0..=MAXIMUM_SAVED_DRAFTS as u64)
            .map(|raw| (channel(raw + 1), format!("draft {raw}")))
            .collect();
        drafts.insert(0, (channel(9_999), String::new()));
        let kept = parse_drafts(&file_text(&drafts));
        assert_eq!(kept.len(), MAXIMUM_SAVED_DRAFTS);
        assert_eq!(kept.first(), Some(&(channel(1), "draft 0".to_owned())));
    }

    #[test]
    fn cuts_a_draft_longer_than_any_message_at_a_character_boundary() {
        let draft = "é".repeat(MAXIMUM_SAVED_DRAFT_BYTES);
        let kept = parse_drafts(&file_text(&[(channel(1), draft)]));
        let (_, text) = &kept[0];
        assert!(text.len() <= MAXIMUM_SAVED_DRAFT_BYTES);
        assert!(text.chars().all(|character| character == 'é'));
    }

    #[test]
    fn ignores_what_mellow_did_not_write() {
        assert_eq!(parse_drafts("1 hello\n"), Vec::new());
        let text = format!(
            "{FILE_HEADER}\nnot-an-id text\n-3 negative\n4 bad \\x escape\n5 kept\n5 duplicate\n6\n"
        );
        assert_eq!(parse_drafts(&text), vec![(channel(5), "kept".to_owned())]);
    }

    #[cfg(unix)]
    #[test]
    fn writes_a_file_private_to_the_user_and_removes_it_once_empty() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().expect("a temporary directory");
        let file = SavedDraftsFile::at(directory.path().join("drafts").join("instance-1"));
        let drafts = vec![(channel(1), "unsent".to_owned())];
        file.write(&drafts, file.next_write());
        assert_eq!(file.read(), drafts);
        let mode = |path: &Path| {
            std::fs::metadata(path)
                .expect("written")
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode(&file.path), 0o600);
        assert_eq!(mode(&directory.path().join("drafts")), 0o700);

        file.write(&[], file.next_write());
        assert!(!file.path.exists());
    }

    #[test]
    fn an_older_write_never_replaces_a_newer_one() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let file = SavedDraftsFile::at(directory.path().join("instance-1"));
        let older = file.next_write();
        let newer = file.next_write();
        file.write(&[(channel(1), "newer".to_owned())], newer);
        file.write(&[(channel(1), "older".to_owned())], older);
        assert_eq!(file.read(), vec![(channel(1), "newer".to_owned())]);
    }
}
