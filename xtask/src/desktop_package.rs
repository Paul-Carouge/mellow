//! The desktop app's release packages (ADR-0055): the macOS `Mellow.app` bundle and the Linux
//! package tree that the release workflow archives. Icons come from the generated favicon, so
//! they follow the tokens like every other app icon.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use resvg::usvg;

use crate::app_icons::{render_png, render_png_with_margin};

/// The workspace version every package is named and labelled with.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const VERSION_PLACEHOLDER: &str = "@VERSION@";

const PACKAGING_DIRECTORY: &str = "crates/mellow-desktop/packaging";
/// Committed and checked by `cargo xtask tokens --check`.
const FAVICON_PATH: &str = "web/public/favicon.svg";
const EXECUTABLE_NAME: &str = "mellow";

pub const MACOS_APP_NAME: &str = "Mellow.app";
const MACOS_ICON_FILE_NAME: &str = "Mellow.icns";
const MACOS_LOCALISATIONS: [&str; 2] = ["en", "fr"];
/// Apple's icon grid: the tile is 824 of 1024 points, so a 100-point margin on each side.
const MACOS_ICON_MARGIN_FRACTION: f32 = 100.0 / 1024.0;
/// Each `icns` entry's type and its PNG's size in pixels (Retina variants included).
const MACOS_ICON_ENTRIES: [(&[u8; 4], u32); 10] = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic11", 32),
    (b"icp6", 64),
    (b"ic12", 64),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic13", 256),
    (b"ic09", 512),
    (b"ic14", 512),
];
/// The 1024-pixel entry, added apart since it is the only size above 512.
const MACOS_LARGEST_ICON: (&[u8; 4], u32) = (b"ic10", 1024);

const LINUX_ICON_SIZES: [u32; 8] = [16, 24, 32, 48, 64, 128, 256, 512];
const LINUX_APP_ID: &str = "mellow";

/// The bundle's `Info.plist`, labelled with `version`.
pub fn info_plist(template: &str, version: &str) -> Result<String> {
    if !template.contains(VERSION_PLACEHOLDER) {
        bail!("the Info.plist template has no {VERSION_PLACEHOLDER}");
    }
    Ok(template.replace(VERSION_PLACEHOLDER, version))
}

/// An Apple icon file holding the given PNG pictures, each under its entry type.
pub fn icns(entries: &[([u8; 4], Vec<u8>)]) -> Result<Vec<u8>> {
    const HEADER_LENGTH: usize = 8;
    let total_length = HEADER_LENGTH
        + entries
            .iter()
            .map(|(_, png)| HEADER_LENGTH + png.len())
            .sum::<usize>();
    let mut file = Vec::with_capacity(total_length);
    file.extend_from_slice(b"icns");
    file.extend_from_slice(&u32::try_from(total_length)?.to_be_bytes());
    for (kind, png) in entries {
        file.extend_from_slice(kind);
        file.extend_from_slice(&u32::try_from(HEADER_LENGTH + png.len())?.to_be_bytes());
        file.extend_from_slice(png);
    }
    Ok(file)
}

fn macos_icon(favicon: &usvg::Tree) -> Result<Vec<u8>> {
    let mut entries = Vec::new();
    for (kind, size) in MACOS_ICON_ENTRIES.into_iter().chain([MACOS_LARGEST_ICON]) {
        entries.push((
            *kind,
            render_png_with_margin(favicon, size, MACOS_ICON_MARGIN_FRACTION)?,
        ));
    }
    icns(&entries)
}

fn read_favicon(root: &Path) -> Result<(String, usvg::Tree)> {
    let path = root.join(FAVICON_PATH);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let tree = usvg::Tree::from_str(&text, &usvg::Options::default())
        .context("the favicon is not valid SVG")?;
    Ok((text, tree))
}

fn write_file(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("cannot write {}", path.display()))
}

fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    let contents =
        std::fs::read(source).with_context(|| format!("cannot read {}", source.display()))?;
    write_file(destination, contents)
}

/// Copies the executable and marks it executable, whatever the source's mode was.
fn copy_executable(source: &Path, destination: &Path) -> Result<()> {
    copy_file(source, destination)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(destination, std::fs::Permissions::from_mode(0o755))
            .with_context(|| format!("cannot make {} executable", destination.display()))?;
    }
    Ok(())
}

/// Builds `<output_directory>/Mellow.app` around `binary` and signs it with `signing_identity`
/// (`-` for an ad hoc signature, as local builds use).
pub fn build_macos_app(
    root: &Path,
    binary: &Path,
    output_directory: &Path,
    signing_identity: &str,
) -> Result<PathBuf> {
    let packaging = root.join(PACKAGING_DIRECTORY).join("macos");
    let app = output_directory.join(MACOS_APP_NAME);
    if app.exists() {
        std::fs::remove_dir_all(&app)
            .with_context(|| format!("cannot replace {}", app.display()))?;
    }
    let contents = app.join("Contents");
    let template_path = packaging.join("Info.plist");
    let template = std::fs::read_to_string(&template_path)
        .with_context(|| format!("cannot read {}", template_path.display()))?;
    write_file(
        &contents.join("Info.plist"),
        info_plist(&template, VERSION)?,
    )?;
    copy_executable(binary, &contents.join("MacOS").join(EXECUTABLE_NAME))?;
    let resources = contents.join("Resources");
    let (_, favicon) = read_favicon(root)?;
    write_file(&resources.join(MACOS_ICON_FILE_NAME), macos_icon(&favicon)?)?;
    for language in MACOS_LOCALISATIONS {
        let strings = Path::new(&format!("{language}.lproj")).join("InfoPlist.strings");
        copy_file(&packaging.join(&strings), &resources.join(&strings))?;
    }
    sign_macos_app(
        &app,
        &packaging.join("entitlements.plist"),
        signing_identity,
    )?;
    Ok(app)
}

fn sign_macos_app(app: &Path, entitlements: &Path, signing_identity: &str) -> Result<()> {
    run(Command::new("codesign")
        .args(["--force", "--options", "runtime", "--timestamp=none"])
        .arg("--entitlements")
        .arg(entitlements)
        .args(["--sign", signing_identity])
        .arg(app))?;
    run(Command::new("codesign")
        .args(["--verify", "--strict"])
        .arg(app))
}

fn run(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("cannot start {command:?}"))?;
    if !status.success() {
        bail!("{command:?} failed: {status}");
    }
    Ok(())
}

/// Lays out the Linux package in `output_directory`: `bin/mellow`, `share/` as under
/// `~/.local/share`, `install.sh` and the licence.
pub fn build_linux_package(root: &Path, binary: &Path, output_directory: &Path) -> Result<()> {
    if output_directory.exists() {
        bail!(
            "{} already exists; the package starts from an empty directory",
            output_directory.display()
        );
    }
    let packaging = root.join(PACKAGING_DIRECTORY).join("linux");
    let share = output_directory.join("share");
    copy_executable(binary, &output_directory.join("bin").join(EXECUTABLE_NAME))?;
    copy_executable(
        &packaging.join("install.sh"),
        &output_directory.join("install.sh"),
    )?;
    copy_file(&root.join("LICENSE"), &output_directory.join("LICENSE"))?;
    copy_file(
        &packaging.join(format!("{LINUX_APP_ID}.desktop")),
        &share
            .join("applications")
            .join(format!("{LINUX_APP_ID}.desktop")),
    )?;
    copy_file(
        &packaging.join(format!("{LINUX_APP_ID}.metainfo.xml")),
        &share
            .join("metainfo")
            .join(format!("{LINUX_APP_ID}.metainfo.xml")),
    )?;
    let icons = share.join("icons").join("hicolor");
    let (favicon_text, favicon) = read_favicon(root)?;
    write_file(
        &icons
            .join("scalable")
            .join("apps")
            .join(format!("{LINUX_APP_ID}.svg")),
        favicon_text,
    )?;
    for size in LINUX_ICON_SIZES {
        write_file(
            &icons
                .join(format!("{size}x{size}"))
                .join("apps")
                .join(format!("{LINUX_APP_ID}.png")),
            render_png(&favicon, size)?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_root() -> PathBuf {
        crate::workspace_root().unwrap()
    }

    #[test]
    fn the_info_plist_carries_the_version_and_the_desktop_identifier() {
        let root = workspace_root();
        let template =
            std::fs::read_to_string(root.join(PACKAGING_DIRECTORY).join("macos/Info.plist"))
                .unwrap();
        let plist = info_plist(&template, "1.2.3").unwrap();
        assert!(!plist.contains(VERSION_PLACEHOLDER));
        assert_eq!(plist.matches("<string>1.2.3</string>").count(), 2);
        // Notifications are posted under the app's identifier, so the bundle must carry it.
        let desktop_lib =
            std::fs::read_to_string(root.join("crates/mellow-desktop/src/lib.rs")).unwrap();
        assert!(desktop_lib.contains(r#"APP_IDENTIFIER: &str = "org.mellow.desktop";"#));
        assert!(plist.contains("<string>org.mellow.desktop</string>"));
        assert!(info_plist("<plist/>", "1.2.3").is_err());
    }

    #[test]
    fn the_linux_desktop_entry_is_named_after_the_wayland_app_id() {
        let root = workspace_root();
        let window_chrome =
            std::fs::read_to_string(root.join("crates/mellow-desktop/src/window_chrome.rs"))
                .unwrap();
        assert!(window_chrome.contains(&format!(r#"APP_ID: &str = "{LINUX_APP_ID}";"#)));
        let entry = std::fs::read_to_string(
            root.join(PACKAGING_DIRECTORY)
                .join(format!("linux/{LINUX_APP_ID}.desktop")),
        )
        .unwrap();
        // install.sh rewrites exactly this line to the installed path.
        assert!(entry.lines().any(|line| line == "Exec=mellow"));
        assert!(entry.contains(&format!("StartupWMClass={LINUX_APP_ID}\n")));
    }

    #[test]
    fn an_icns_file_lists_each_entry_with_its_length() {
        let file = icns(&[(*b"ic07", vec![1, 2, 3]), (*b"icp4", vec![4])]).unwrap();
        assert_eq!(&file[..4], b"icns");
        assert_eq!(u32::from_be_bytes(file[4..8].try_into().unwrap()), 28);
        assert_eq!(&file[8..12], b"ic07");
        assert_eq!(u32::from_be_bytes(file[12..16].try_into().unwrap()), 11);
        assert_eq!(&file[16..19], &[1, 2, 3]);
        assert_eq!(&file[19..23], b"icp4");
        assert_eq!(file.len(), 28);
    }

    #[test]
    fn the_macos_icon_leaves_apples_margin_transparent() {
        let (_, favicon) = read_favicon(&workspace_root()).unwrap();
        let png = render_png_with_margin(&favicon, 1024, MACOS_ICON_MARGIN_FRACTION).unwrap();
        let picture = resvg::tiny_skia::Pixmap::decode_png(&png).unwrap();
        assert_eq!(picture.pixel(99, 512).unwrap().alpha(), 0);
        assert_eq!(picture.pixel(512, 512).unwrap().alpha(), u8::MAX);
        assert_eq!(picture.pixel(924, 512).unwrap().alpha(), 0);
    }

    #[test]
    fn the_linux_package_holds_the_binary_entry_icons_and_installer() {
        let scratch = tempfile::tempdir().unwrap();
        let binary = scratch.path().join("mellow-binary");
        std::fs::write(&binary, b"#!/bin/sh\n").unwrap();
        let package = scratch.path().join("package");
        build_linux_package(&workspace_root(), &binary, &package).unwrap();
        for relative in [
            "bin/mellow",
            "install.sh",
            "LICENSE",
            "share/applications/mellow.desktop",
            "share/metainfo/mellow.metainfo.xml",
            "share/icons/hicolor/scalable/apps/mellow.svg",
            "share/icons/hicolor/512x512/apps/mellow.png",
            "share/icons/hicolor/16x16/apps/mellow.png",
        ] {
            assert!(package.join(relative).is_file(), "{relative} is missing");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(package.join("bin/mellow"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755);
        }
        // A second run into the same directory would mix two packages.
        assert!(build_linux_package(&workspace_root(), &binary, &package).is_err());
    }
}
