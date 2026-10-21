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
    if let Some(updated) = info_plist_with_icon(&std::fs::read_to_string(&info_path)?) {
        std::fs::write(&info_path, updated)?;
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
    let menu_entry = match std::fs::read_to_string(&menu_entry_path) {
        Ok(menu_entry) => menu_entry,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if write_if_different(&data_home.join(LINUX_ICON_PATH_IN_DATA_HOME), LINUX_ICON)? {
        tracing::info!("installed the app icon");
    }
    if let Some(updated) = desktop_entry_with_icon(&menu_entry) {
        std::fs::write(&menu_entry_path, updated)?;
    }
    Ok(())
}

/// Whether it wrote: an icon already in place is left alone.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn write_if_different(path: &std::path::Path, contents: &[u8]) -> std::io::Result<bool> {
    if std::fs::read(path).is_ok_and(|current| current == contents) {
        return Ok(false);
    }
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    std::fs::write(path, contents)?;
    Ok(true)
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
