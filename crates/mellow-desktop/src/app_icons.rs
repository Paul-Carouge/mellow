//! Melli as the app icon of an install made by `install.sh` (DESIGN.md §2.10).
//!
//! The icons are embedded in this binary, so they are exactly as trusted as the signed binary
//! itself: no unsigned file is ever downloaded. Writing them at every start, when they differ,
//! also refreshes them after an in-app update, which only replaces the executable.

/// The `.desktop` entry, the menu icon and the window's app id share this name, so Linux docks
/// match the running window to its menu entry and icon.
pub const LINUX_APP_ID: &str = "mellow";

#[cfg(target_os = "macos")]
const MACOS_ICON: &[u8] = include_bytes!("../../../release/icons/Mellow.icns");
#[cfg(any(target_os = "macos", test))]
const MACOS_ICON_FILE_KEY: &str = "<key>CFBundleIconFile</key>";
#[cfg(target_os = "linux")]
const LINUX_ICON: &[u8] = include_bytes!("../../../release/icons/mellow-256.png");
#[cfg(target_os = "linux")]
const LINUX_ICON_PATH_IN_DATA_HOME: &str = "icons/hicolor/256x256/apps/mellow.png";
#[cfg(target_os = "linux")]
const LINUX_MENU_ENTRY_PATH_IN_DATA_HOME: &str = "applications/mellow.desktop";
/// `Info.plist` and the menu entry are a few hundred bytes; anything far larger is not ours.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const MAXIMUM_EDITED_FILE_LENGTH: u64 = 64 * 1024;
/// Readable by the Dock, Finder and the desktop environment, writable by the user only.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const WRITTEN_FILE_MODE: u32 = 0o644;

/// Off the UI thread: a few file reads and, when something changed, writes.
pub fn install_in_background() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let spawned = std::thread::Builder::new()
            .name("app-icons".into())
            .spawn(|| {
                if let Err(error) = install() {
                    tracing::warn!(%error, "cannot install the app icon");
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "cannot start the app icon installer");
        }
    }
}

/// Inside `Mellow.app` only: a binary run from elsewhere (`cargo run`) has no bundle to dress.
#[cfg(target_os = "macos")]
fn install() -> std::io::Result<()> {
    use std::path::Path;

    let executable = std::env::current_exe()?;
    let Some(contents) = executable
        .parent()
        .filter(|directory| directory.ends_with("Contents/MacOS"))
        .and_then(Path::parent)
    else {
        return Ok(());
    };
    let Some(bundle) = contents.parent().filter(|bundle| {
        bundle
            .extension()
            .is_some_and(|extension| extension == "app")
    }) else {
        return Ok(());
    };
    let mut has_changed = write_if_different(&contents.join("Resources/Mellow.icns"), MACOS_ICON)?;
    let info_path = contents.join("Info.plist");
    let info_plist =
        read_small_regular_file(&info_path)?.and_then(|bytes| String::from_utf8(bytes).ok());
    if let Some(updated) = info_plist.as_deref().and_then(info_plist_with_icon) {
        replace_atomically(&info_path, updated.as_bytes())?;
        has_changed = true;
    }
    if has_changed {
        // Finder and the Dock reread a bundle's icon once its modification date changes.
        std::fs::File::open(bundle)?.set_modified(std::time::SystemTime::now())?;
        tracing::info!("installed the app icon");
    }
    Ok(())
}

/// Only next to the menu entry `install.sh` writes: without it, nothing would show the icon.
#[cfg(target_os = "linux")]
fn install() -> std::io::Result<()> {
    use std::path::PathBuf;

    // The XDG base directory spec ignores relative paths.
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    let Some(data_home) = data_home else {
        return Ok(());
    };
    let menu_entry_path = data_home.join(LINUX_MENU_ENTRY_PATH_IN_DATA_HOME);
    let Some(menu_entry) =
        read_small_regular_file(&menu_entry_path)?.and_then(|bytes| String::from_utf8(bytes).ok())
    else {
        return Ok(());
    };
    if write_if_different(&data_home.join(LINUX_ICON_PATH_IN_DATA_HOME), LINUX_ICON)? {
        tracing::info!("installed the app icon");
    }
    if let Some(updated) = desktop_entry_with_icon(&menu_entry) {
        replace_atomically(&menu_entry_path, updated.as_bytes())?;
    }
    Ok(())
}

/// What is at a path. Links, directories, FIFOs and devices are `Other`: never read nor replaced,
/// so a file planted there cannot make the app block, follow it elsewhere or read without limit.
#[cfg(any(target_os = "macos", target_os = "linux"))]
enum ExistingEntry {
    Missing,
    RegularFile { length: u64 },
    Other,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn existing_entry(path: &std::path::Path) -> std::io::Result<ExistingEntry> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(ExistingEntry::RegularFile {
            length: metadata.len(),
        }),
        Ok(_) => Ok(ExistingEntry::Other),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ExistingEntry::Missing),
        Err(error) => Err(error),
    }
}

/// A regular file's bytes, when it is at most `maximum_length` long; `None` otherwise.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn read_regular_file(
    path: &std::path::Path,
    maximum_length: u64,
) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read as _;

    match existing_entry(path)? {
        ExistingEntry::RegularFile { length } if length <= maximum_length => {
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(maximum_length)
                .read_to_end(&mut bytes)?;
            Ok(Some(bytes))
        }
        _ => Ok(None),
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn read_small_regular_file(path: &std::path::Path) -> std::io::Result<Option<Vec<u8>>> {
    read_regular_file(path, MAXIMUM_EDITED_FILE_LENGTH)
}

/// Whether it wrote: an icon already in place is left alone, and so is anything that is not a
/// regular file.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn write_if_different(path: &std::path::Path, contents: &[u8]) -> std::io::Result<bool> {
    let expected_length = u64::try_from(contents.len()).unwrap_or(u64::MAX);
    match existing_entry(path)? {
        ExistingEntry::Other => {
            tracing::warn!(path = %path.display(), "not a regular file; the app icon is left as it is");
            return Ok(false);
        }
        ExistingEntry::RegularFile { length } if length == expected_length => {
            if read_regular_file(path, length)?.is_some_and(|current| current == contents) {
                return Ok(false);
            }
        }
        ExistingEntry::RegularFile { .. } | ExistingEntry::Missing => {}
    }
    replace_atomically(path, contents)?;
    Ok(true)
}

/// Writes a temporary file next to `path`, then renames it over `path`: readers never see half a
/// file, and a link at `path` is replaced, not followed.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn replace_atomically(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;

    let directory = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path without a directory")
    })?;
    std::fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(contents)?;
    file.as_file()
        .set_permissions(std::fs::Permissions::from_mode(WRITTEN_FILE_MODE))?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Installs made before Mellow had an icon lack `CFBundleIconFile`; `None` when nothing changes.
#[cfg(any(target_os = "macos", test))]
fn info_plist_with_icon(info_plist: &str) -> Option<String> {
    if info_plist.contains(MACOS_ICON_FILE_KEY) {
        return None;
    }
    let end_of_dictionary = info_plist.rfind("</dict>")?;
    let mut updated = info_plist.to_owned();
    updated.insert_str(
        end_of_dictionary,
        &format!("  {MACOS_ICON_FILE_KEY}<string>Mellow</string>\n"),
    );
    Some(updated)
}

/// Menu entries written before Mellow had an icon lack `Icon=`; `None` when nothing changes.
/// `install.sh` writes a single `[Desktop Entry]` group, so the key goes at the end.
#[cfg(any(target_os = "linux", test))]
fn desktop_entry_with_icon(desktop_entry: &str) -> Option<String> {
    if desktop_entry.lines().any(|line| line.starts_with("Icon=")) {
        return None;
    }
    let separator = if desktop_entry.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    Some(format!("{desktop_entry}{separator}Icon={LINUX_APP_ID}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn icons_are_written_once_and_never_through_a_link_or_into_a_fifo() {
        let directory = tempfile::tempdir().unwrap();
        let icon_path = directory.path().join("icons/mellow.png");
        assert!(write_if_different(&icon_path, b"icon").unwrap());
        assert!(!write_if_different(&icon_path, b"icon").unwrap());
        assert!(write_if_different(&icon_path, b"newer").unwrap());
        assert_eq!(std::fs::read(&icon_path).unwrap(), b"newer");

        let target = directory.path().join("elsewhere");
        std::fs::write(&target, b"keep").unwrap();
        let link = directory.path().join("link.png");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(!write_if_different(&link, b"icon").unwrap());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        assert_eq!(read_small_regular_file(&link).unwrap(), None);

        // A FIFO would block a plain read forever.
        let fifo = directory.path().join("fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        assert!(!write_if_different(&fifo, b"icon").unwrap());
        assert_eq!(read_small_regular_file(&fifo).unwrap(), None);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn files_too_large_to_be_ours_are_not_read() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Info.plist");
        let too_large = usize::try_from(MAXIMUM_EDITED_FILE_LENGTH).unwrap() + 1;
        std::fs::write(&path, vec![b'x'; too_large]).unwrap();
        assert_eq!(read_small_regular_file(&path).unwrap(), None);
    }

    #[test]
    fn an_info_plist_without_an_icon_gets_one_inside_its_dictionary() {
        let info_plist = "<plist version=\"1.0\">\n<dict>\n  <key>CFBundleName</key><string>Mellow</string>\n</dict>\n</plist>\n";
        let updated = info_plist_with_icon(info_plist).unwrap();
        assert_eq!(
            updated,
            "<plist version=\"1.0\">\n<dict>\n  <key>CFBundleName</key><string>Mellow</string>\n  <key>CFBundleIconFile</key><string>Mellow</string>\n</dict>\n</plist>\n"
        );
        assert_eq!(info_plist_with_icon(&updated), None);
        assert_eq!(info_plist_with_icon("not a property list"), None);
    }

    #[test]
    fn a_menu_entry_without_an_icon_gets_mellows() {
        let updated = desktop_entry_with_icon("[Desktop Entry]\nName=Mellow\n").unwrap();
        assert_eq!(updated, "[Desktop Entry]\nName=Mellow\nIcon=mellow\n");
        assert_eq!(desktop_entry_with_icon(&updated), None);
        assert_eq!(
            desktop_entry_with_icon("[Desktop Entry]\nName=Mellow").unwrap(),
            "[Desktop Entry]\nName=Mellow\nIcon=mellow\n"
        );
    }
}
