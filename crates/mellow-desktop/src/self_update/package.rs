//! Where this copy of Mellow is installed, which release package replaces it, and the blocking
//! work of unpacking, checking and swapping that package in (ADR-0057). Nothing here runs on the
//! UI thread: the self-update calls it from a background task.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use mellow_update::{Release, UpdateClient, UpdateError};
use semver::Version;

use super::UpdateProblem;

/// A universal app zipped, or a Linux build in a tarball, is tens of megabytes; this bounds what
/// a broken or hostile release host can make the app hold in memory.
const MAXIMUM_PACKAGE_BYTES: usize = 512 * 1024 * 1024;
/// Every staging directory, in the app's folder (macOS) or the user's cache (Linux), starts with
/// this, so one a quit or a crash left behind is recognised and removed.
const STAGING_PREFIX: &str = ".mellow-update-";
/// Held locked by the process that staged the update, so another instance's start leaves it alone.
const STAGING_LOCK_FILE: &str = "lock";
/// `<user cache>/mellow/updates` on Linux: not a tmpfs that `systemd-tmpfiles` empties after ten
/// days, and often on the same file system as `~/.local/bin`.
#[cfg(target_os = "linux")]
const LINUX_STAGING_DIRECTORIES: [&str; 2] = ["mellow", "updates"];
/// macOS runs an app opened from a quarantined download from a read-only random folder.
#[cfg(any(target_os = "macos", test))]
const APP_TRANSLOCATION_FOLDER: &str = "AppTranslocation";
/// The app's bundle identifier, which an update's code signature must carry (ADR-0055).
#[cfg(any(target_os = "macos", test))]
const BUNDLE_IDENTIFIER_REQUIREMENT: &str = "identifier \"org.mellow.desktop\"";
/// FFmpeg N ships `libavcodec.so.(N + 54)` (ADR-0055).
#[cfg(any(target_os = "linux", test))]
const LIBAVCODEC_SONAME_OFFSET: u32 = 54;
#[cfg(any(target_os = "linux", test))]
const LIBAVCODEC_SONAME_PREFIX: &str = "libavcodec.so.";
/// Where distributions put FFmpeg's libraries: Arch, Fedora, Debian and Ubuntu, and by hand.
#[cfg(target_os = "linux")]
const LIBRARY_DIRECTORIES: [&str; 6] = [
    "/usr/lib",
    "/usr/lib64",
    "/usr/lib/x86_64-linux-gnu",
    "/lib/x86_64-linux-gnu",
    "/lib64",
    "/usr/local/lib",
];
/// Folders only a package manager (or an image) writes to.
#[cfg(any(target_os = "linux", test))]
const PACKAGE_MANAGER_PREFIXES: [&str; 6] = ["/usr", "/opt", "/nix", "/snap", "/app", "/gnu"];

/// The copy of Mellow that is running, as far as updating it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installation {
    /// A `Mellow.app` signed with the release certificate; an update must satisfy the same
    /// designated requirement, so macOS keeps its permissions (ADR-0055).
    MacosBundle {
        bundle: PathBuf,
        designated_requirement: String,
    },
    /// The Linux package's `install.sh` copy in `~/.local/bin`; `ffmpeg_majors` are the FFmpeg
    /// versions the system has, newest first.
    LinuxUserInstall {
        executable: PathBuf,
        home: PathBuf,
        ffmpeg_majors: Vec<u32>,
        /// Where updates are unpacked before `install.sh` runs.
        staging_folder: PathBuf,
    },
}

/// The device and inode of a file: they change when the installed copy is replaced.
type FileIdentity = (u64, u64);

/// A package downloaded, verified and unpacked, waiting to be swapped in. Its private staging
/// directory is removed when it is dropped.
#[derive(Debug)]
pub struct StagedUpdate {
    pub version: Version,
    /// Dropped before `staging`: the lock only matters while the directory exists.
    _lock: std::fs::File,
    staging: tempfile::TempDir,
    kind: StagedKind,
    /// The installed copy's identity when the update was staged; a copy reinstalled by hand
    /// since then is newer or chosen, and is not replaced.
    installed_identity: Option<FileIdentity>,
}

#[derive(Debug)]
enum StagedKind {
    MacosBundle {
        new_bundle: PathBuf,
        installed_bundle: PathBuf,
    },
    LinuxPackage {
        install_script: PathBuf,
        executable: PathBuf,
        home: PathBuf,
    },
}

/// Finds this copy's installation, or why it cannot update itself. Blocking: it runs `codesign`
/// on macOS and reads library folders on Linux.
///
/// # Errors
/// The [`UpdateProblem`] that keeps this copy from updating itself.
#[cfg(target_os = "macos")]
pub fn detect_installation() -> Result<Installation, UpdateProblem> {
    let executable = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|_| UpdateProblem::NotPackaged)?;
    let bundle = bundle_of_executable(&executable).ok_or(UpdateProblem::NotPackaged)?;
    if is_translocated(&bundle) {
        return Err(UpdateProblem::Translocated);
    }
    let output = Command::new("/usr/bin/codesign")
        .args(["--display", "--requirements", "-"])
        .arg(&bundle)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| UpdateProblem::NotPackaged)?;
    let text = String::from_utf8_lossy(&output.stdout);
    let designated_requirement = designated_requirement_of(&text)
        .filter(|requirement| is_release_certificate_requirement(requirement))
        .ok_or(UpdateProblem::NotPackaged)?
        .to_owned();
    // Before any download: the update is staged beside the bundle, and a standard account cannot
    // write to an administrator's /Applications.
    let folder = bundle.parent().ok_or(UpdateProblem::NotWritable)?;
    tempfile::tempfile_in(folder).map_err(|_| UpdateProblem::NotWritable)?;
    Ok(Installation::MacosBundle {
        bundle,
        designated_requirement,
    })
}

/// # Errors
/// The [`UpdateProblem`] that keeps this copy from updating itself.
#[cfg(target_os = "linux")]
pub fn detect_installation() -> Result<Installation, UpdateProblem> {
    if !cfg!(target_arch = "x86_64") {
        return Err(UpdateProblem::UnsupportedSystem);
    }
    let executable = std::env::current_exe().map_err(|_| UpdateProblem::InstalledElsewhere)?;
    let home = dirs::home_dir().ok_or(UpdateProblem::InstalledElsewhere)?;
    check_linux_install_location(&executable, &home)?;
    let staging_folder = staging_folder_of_this_copy().ok_or(UpdateProblem::NotWritable)?;
    let directories: Vec<&Path> = LIBRARY_DIRECTORIES.iter().map(Path::new).collect();
    Ok(Installation::LinuxUserInstall {
        executable,
        home,
        ffmpeg_majors: ffmpeg_majors_in(&directories),
        staging_folder,
    })
}

/// # Errors
/// Always: Windows packages come with the Windows work (ADR-0055).
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn detect_installation() -> Result<Installation, UpdateProblem> {
    Err(UpdateProblem::UnsupportedSystem)
}

/// Where this copy stages its updates, so the leftovers of an earlier run can be removed at the
/// start: the app's folder on macOS, the user's cache on Linux. Blocking.
#[must_use]
pub fn staging_folder_of_this_copy() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let executable = std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .ok()?;
        Some(bundle_of_executable(&executable)?.parent()?.to_owned())
    }
    #[cfg(target_os = "linux")]
    {
        Some(
            LINUX_STAGING_DIRECTORIES
                .iter()
                .fold(dirs::cache_dir()?, |folder, name| folder.join(name)),
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Whether macOS runs this bundle from its read-only App Translocation copy.
#[cfg(any(target_os = "macos", test))]
pub(super) fn is_translocated(bundle: &Path) -> bool {
    bundle
        .components()
        .any(|component| component.as_os_str() == APP_TRANSLOCATION_FOLDER)
}

/// `…/Mellow.app` for `…/Mellow.app/Contents/MacOS/mellow`.
#[cfg(any(target_os = "macos", test))]
pub(super) fn bundle_of_executable(executable: &Path) -> Option<PathBuf> {
    let macos_folder = executable.parent()?;
    let contents_folder = macos_folder.parent()?;
    let bundle = contents_folder.parent()?;
    let is_bundle = macos_folder.file_name()? == "MacOS"
        && contents_folder.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then(|| bundle.to_owned())
}

/// The requirement `codesign --display --requirements -` prints after `designated => `.
#[cfg(any(target_os = "macos", test))]
pub(super) fn designated_requirement_of(codesign_output: &str) -> Option<&str> {
    codesign_output
        .lines()
        .find_map(|line| line.strip_prefix("designated => "))
        .map(str::trim)
        .filter(|requirement| !requirement.is_empty())
}

/// A release build's requirement pins its bundle identifier and the release certificate; an ad
/// hoc or locally signed build pins its own hash or another certificate, and is not updated.
#[cfg(any(target_os = "macos", test))]
pub(super) fn is_release_certificate_requirement(requirement: &str) -> bool {
    requirement.contains(BUNDLE_IDENTIFIER_REQUIREMENT)
        && requirement.contains("certificate leaf = H\"")
        && !requirement.contains("cdhash")
}

/// Only the copy `install.sh` puts in `~/.local/bin` updates itself: one in a system folder
/// belongs to a package manager, and one elsewhere was built or placed by hand.
#[cfg(any(target_os = "linux", test))]
pub(super) fn check_linux_install_location(
    executable: &Path,
    home: &Path,
) -> Result<(), UpdateProblem> {
    let expected = home.join(".local").join("bin").join("mellow");
    // Fedora Atomic's /home is a link to /var/home, and the running executable's path is resolved.
    let expected = std::fs::canonicalize(&expected).unwrap_or(expected);
    if executable != expected {
        let is_system_folder = PACKAGE_MANAGER_PREFIXES
            .iter()
            .any(|prefix| executable.starts_with(prefix));
        return Err(if is_system_folder {
            UpdateProblem::PackageManager
        } else {
            UpdateProblem::InstalledElsewhere
        });
    }
    let folder = executable
        .parent()
        .ok_or(UpdateProblem::InstalledElsewhere)?;
    // The real test: what `access(2)` says can differ from the file modes (ACLs, read-only mounts).
    tempfile::tempfile_in(folder).map_err(|_| UpdateProblem::NotWritable)?;
    Ok(())
}

/// The FFmpeg majors whose `libavcodec` is in `directories`, newest first.
#[cfg(any(target_os = "linux", test))]
pub(super) fn ffmpeg_majors_in(directories: &[&Path]) -> Vec<u32> {
    let mut majors: Vec<u32> = directories
        .iter()
        .filter_map(|directory| std::fs::read_dir(directory).ok())
        .flatten()
        .filter_map(|entry| {
            let name = entry.ok()?.file_name();
            let soname_major = name.to_str()?.strip_prefix(LIBAVCODEC_SONAME_PREFIX)?;
            let is_plain_number =
                !soname_major.is_empty() && soname_major.bytes().all(|byte| byte.is_ascii_digit());
            let soname_major: u32 = is_plain_number.then(|| soname_major.parse().ok())??;
            soname_major.checked_sub(LIBAVCODEC_SONAME_OFFSET)
        })
        .filter(|major| *major > 0)
        .collect();
    majors.sort_unstable_by(|left, right| right.cmp(left));
    majors.dedup();
    majors
}

/// The release asset that replaces `installation` (ADR-0055's names): on Linux, the package for
/// the newest FFmpeg the system has that the release carries.
///
/// # Errors
/// `NoFfmpeg` or `NoPackage`.
pub fn asset_name_for(
    installation: &Installation,
    release: &Release,
) -> Result<String, UpdateProblem> {
    let version = &release.version;
    match installation {
        Installation::MacosBundle { .. } => {
            let name = format!("mellow-desktop-{version}-universal-apple-darwin.zip");
            if release.has_signed_asset(&name) {
                Ok(name)
            } else {
                Err(UpdateProblem::NoPackage)
            }
        }
        Installation::LinuxUserInstall { ffmpeg_majors, .. } => {
            if ffmpeg_majors.is_empty() {
                return Err(UpdateProblem::NoFfmpeg);
            }
            ffmpeg_majors
                .iter()
                .map(|major| {
                    format!(
                        "mellow-desktop-{version}-x86_64-unknown-linux-gnu-ffmpeg{major}.tar.gz"
                    )
                })
                .find(|name| release.has_signed_asset(name))
                .ok_or(UpdateProblem::NoPackage)
        }
    }
}

/// The newest release if it is newer than `current`.
///
/// # Errors
/// `Network` when the release host cannot be read.
pub async fn find_newer_release(
    client: &UpdateClient,
    current: &Version,
) -> Result<Option<Release>, UpdateProblem> {
    match client.fetch_latest_release().await {
        Ok(release) => Ok((release.version > *current).then_some(release)),
        Err(UpdateError::NoReleasePublished) => Ok(None),
        Err(error) => Err(problem_of(&error)),
    }
}

/// Downloads `asset_name` of `release` and returns it once its signature verifies (ADR-0016).
///
/// # Errors
/// `Network`, `NoPackage` or `BadSignature`.
pub async fn download_verified_package(
    client: &UpdateClient,
    release: &Release,
    asset_name: &str,
) -> Result<Vec<u8>, UpdateProblem> {
    client
        .download_verified_asset(release, asset_name, MAXIMUM_PACKAGE_BYTES)
        .await
        .map_err(|error| problem_of(&error))
}

fn problem_of(error: &UpdateError) -> UpdateProblem {
    match error {
        UpdateError::MissingAsset { .. } => UpdateProblem::NoPackage,
        UpdateError::InvalidSignature | UpdateError::WrongSignedFile { .. } => {
            UpdateProblem::BadSignature
        }
        UpdateError::Network(_)
        | UpdateError::NoReleasePublished
        | UpdateError::UnexpectedResponse(_) => UpdateProblem::Network,
        UpdateError::UnsafeInstallLocation { .. }
        | UpdateError::ReplaceExecutable(_)
        | UpdateError::ExecutableLeftAside { .. }
        | UpdateError::NoPreviousExecutable => UpdateProblem::InstallFailed,
    }
}

/// Writes the verified `package` to a private staging directory, unpacks it and checks it is
/// what `installation` needs: on macOS a bundle signed by the running app's certificate, on Linux
/// the package's installer and binary. Blocking.
///
/// # Errors
/// `NotWritable`, `BadPackage` or `WrongCertificate`.
pub fn stage_package(
    installation: &Installation,
    version: Version,
    asset_name: &str,
    package: &[u8],
) -> Result<StagedUpdate, UpdateProblem> {
    // Beside the bundle on macOS, so the swap is a rename on one file system; tempfile makes the
    // folder readable by its owner only.
    let (staging_folder, installed_file) = match installation {
        Installation::MacosBundle { bundle, .. } => (
            bundle
                .parent()
                .ok_or(UpdateProblem::NotWritable)?
                .to_owned(),
            bundle.join("Contents").join("Info.plist"),
        ),
        Installation::LinuxUserInstall {
            executable,
            staging_folder,
            ..
        } => {
            std::fs::create_dir_all(staging_folder).map_err(|_| UpdateProblem::NotWritable)?;
            (staging_folder.clone(), executable.clone())
        }
    };
    let installed_identity = file_identity(&installed_file);
    let staging = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .tempdir_in(&staging_folder)
        .map_err(|_| UpdateProblem::NotWritable)?;
    let lock = std::fs::File::create(staging.path().join(STAGING_LOCK_FILE))
        .map_err(|_| UpdateProblem::NotWritable)?;
    lock.try_lock().map_err(|_| UpdateProblem::NotWritable)?;
    let archive = staging.path().join("package");
    let unpacked = staging.path().join("unpacked");
    std::fs::write(&archive, package)
        .and_then(|()| std::fs::create_dir(&unpacked))
        .map_err(|_| UpdateProblem::NotWritable)?;
    let kind = match installation {
        Installation::MacosBundle {
            bundle,
            designated_requirement,
        } => {
            // `ditto` made the zip and keeps the signature's extended attributes (ADR-0055).
            run_quietly(
                Command::new("/usr/bin/ditto")
                    .args(["-x", "-k"])
                    .arg(&archive)
                    .arg(&unpacked),
            )
            .map_err(|()| UpdateProblem::BadPackage)?;
            let new_bundle = unpacked.join("Mellow.app");
            if !is_real_directory(&new_bundle) {
                return Err(UpdateProblem::BadPackage);
            }
            check_code_signature(&new_bundle, designated_requirement)?;
            StagedKind::MacosBundle {
                new_bundle,
                installed_bundle: bundle.clone(),
            }
        }
        Installation::LinuxUserInstall {
            executable, home, ..
        } => {
            run_quietly(
                Command::new("tar")
                    .args(["-x", "-z", "--no-same-owner", "-f"])
                    .arg(&archive)
                    .arg("-C")
                    .arg(&unpacked),
            )
            .map_err(|()| UpdateProblem::BadPackage)?;
            let package_name = asset_name
                .strip_suffix(".tar.gz")
                .ok_or(UpdateProblem::BadPackage)?;
            let package_directory = unpacked.join(package_name);
            let install_script = package_directory.join("install.sh");
            let is_complete = is_real_directory(&package_directory)
                && is_regular_file(&install_script)
                && is_regular_file(&package_directory.join("bin").join("mellow"));
            if !is_complete {
                return Err(UpdateProblem::BadPackage);
            }
            StagedKind::LinuxPackage {
                install_script,
                executable: executable.clone(),
                home: home.clone(),
            }
        }
    };
    // The archive is no longer needed once unpacked; failing to remove it only costs space until
    // the staging directory goes.
    let _ = std::fs::remove_file(&archive);
    Ok(StagedUpdate {
        version,
        _lock: lock,
        staging,
        kind,
        installed_identity,
    })
}

/// macOS only checks a signature against a requirement; elsewhere a bundle is never staged.
fn check_code_signature(bundle: &Path, designated_requirement: &str) -> Result<(), UpdateProblem> {
    run_quietly(
        Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict", "-R"])
            .arg(format!("={designated_requirement}"))
            .arg(bundle),
    )
    .map_err(|()| UpdateProblem::WrongCertificate)
}

/// Swaps the staged package in and returns what to start instead of this process. The call
/// checks come before this: it is never run during a voice call (ADR-0057). A copy reinstalled
/// by hand since the update was staged is left alone (replacing it could be a downgrade) and is
/// what the returned path starts. Blocking.
///
/// # Errors
/// `InstallFailed`, with the running version left in place.
pub fn apply_staged_update(staged: &mut StagedUpdate) -> Result<PathBuf, UpdateProblem> {
    let (installed_file, restart_path) = match &staged.kind {
        StagedKind::MacosBundle {
            installed_bundle, ..
        } => (
            installed_bundle.join("Contents").join("Info.plist"),
            installed_bundle.clone(),
        ),
        StagedKind::LinuxPackage { executable, .. } => (executable.clone(), executable.clone()),
    };
    if file_identity(&installed_file) != staged.installed_identity {
        tracing::info!("Mellow was reinstalled since the update was staged; it is not replaced");
        return Ok(restart_path);
    }
    match &staged.kind {
        StagedKind::MacosBundle {
            new_bundle,
            installed_bundle,
        } => {
            // The replaced bundle goes into the staging directory, removed at the next start.
            let previous = staged.staging.path().join("previous.app");
            match swap_in(new_bundle, installed_bundle, &previous) {
                Ok(()) => {}
                Err(SwapFailure::NothingMoved) => return Err(UpdateProblem::InstallFailed),
                Err(SwapFailure::InstalledCopyLeftAside) => {
                    // The only copy of the app is in the staging directory now: keep it.
                    staged.staging.disable_cleanup(true);
                    tracing::warn!("the installed app could not be put back after a failed update");
                    return Err(UpdateProblem::InstallFailed);
                }
            }
        }
        StagedKind::LinuxPackage {
            install_script,
            executable,
            home,
        } => run_install_script(install_script, executable, home)?,
    }
    Ok(restart_path)
}

#[cfg(unix)]
fn file_identity(path: &Path) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata(path)
        .ok()
        .map(|metadata| (metadata.dev(), metadata.ino()))
}

/// Windows stages no update (ADR-0055).
#[cfg(not(unix))]
fn file_identity(_path: &Path) -> Option<FileIdentity> {
    None
}

/// Why [`swap_in`] failed.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SwapFailure {
    /// `installed` is untouched.
    NothingMoved,
    /// Neither copy is at `installed`: the installed one is at `previous`.
    InstalledCopyLeftAside,
}

/// Puts `new` at `installed` and the installed copy at `previous`, never leaving `installed`
/// missing: in one exchange where the file system allows it (APFS), otherwise in two renames
/// with the first undone if the second fails.
pub(super) fn swap_in(new: &Path, installed: &Path, previous: &Path) -> Result<(), SwapFailure> {
    #[cfg(target_os = "macos")]
    if exchange_paths(new, installed).is_ok() {
        // The installed copy is at `new` now, inside the staging directory either way.
        let _ = std::fs::rename(new, previous);
        return Ok(());
    }
    std::fs::rename(installed, previous).map_err(|_| SwapFailure::NothingMoved)?;
    if std::fs::rename(new, installed).is_err() {
        return match std::fs::rename(previous, installed) {
            Ok(()) => Err(SwapFailure::NothingMoved),
            Err(_) => Err(SwapFailure::InstalledCopyLeftAside),
        };
    }
    Ok(())
}

/// Exchanges two paths atomically (`renamex_np` with `RENAME_SWAP`); fails where the file system
/// cannot (`ENOTSUP` outside APFS), leaving both in place.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn exchange_paths(first: &Path, second: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt as _;
    let first = std::ffi::CString::new(first.as_os_str().as_bytes())?;
    let second = std::ffi::CString::new(second.as_os_str().as_bytes())?;
    // SAFETY: both pointers come from live `CString`s, so they are valid, NUL-terminated and
    // outlive the call, which only reads them.
    let result = unsafe { libc::renamex_np(first.as_ptr(), second.as_ptr(), libc::RENAME_SWAP) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Runs the new package's own `install.sh` (signed with it), which checks its libraries with
/// `ldd`, renames the binary into place and refreshes the desktop entry and icons (ADR-0055).
/// The running binary is kept under a hard link meanwhile and put back if the script fails.
pub(super) fn run_install_script(
    install_script: &Path,
    executable: &Path,
    home: &Path,
) -> Result<(), UpdateProblem> {
    let previous = executable.with_file_name(".mellow.previous");
    match std::fs::remove_file(&previous) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(UpdateProblem::NotWritable);
        }
        _ => {}
    }
    std::fs::hard_link(executable, &previous).map_err(|_| UpdateProblem::NotWritable)?;
    let installed = run_quietly(
        Command::new("/bin/sh")
            .arg(install_script)
            .env("HOME", home),
    );
    if installed.is_err() {
        // An atomic rename: the binary is the old one again, whatever the script got to.
        if std::fs::rename(&previous, executable).is_ok() {
            // When the script failed before replacing anything, both names are one file and
            // `rename(2)` keeps both: the extra name goes.
            let _ = std::fs::remove_file(&previous);
        } else {
            tracing::warn!("the previous Mellow binary could not be put back");
        }
        return Err(UpdateProblem::InstallFailed);
    }
    let _ = std::fs::remove_file(&previous);
    Ok(())
}

/// Runs a system tool without a terminal; only whether it succeeded matters (its output can
/// hold paths with the user's name, which are not logged).
fn run_quietly(command: &mut Command) -> Result<(), ()> {
    let succeeded = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if succeeded { Ok(()) } else { Err(()) }
}

fn is_real_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

fn is_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file())
}

/// Removes staging directories a quit or a crash left in `folder` (after a restart, one holds the
/// replaced app), but not one another running instance holds locked. Blocking.
pub fn remove_stale_staging_directories(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let is_staging = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(STAGING_PREFIX));
        // `file_type` does not follow links, so a link named like one is left alone.
        let is_directory = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if !is_staging || !is_directory || is_staging_held_by_a_running_app(&entry.path()) {
            continue;
        }
        if std::fs::remove_dir_all(entry.path()).is_err() {
            tracing::debug!("a leftover update folder could not be removed");
        }
    }
}

fn is_staging_held_by_a_running_app(staging: &Path) -> bool {
    std::fs::File::open(staging.join(STAGING_LOCK_FILE))
        .is_ok_and(|lock| matches!(lock.try_lock(), Err(std::fs::TryLockError::WouldBlock)))
}
