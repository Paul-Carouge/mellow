//! Finds, verifies and installs signed Mellow releases (ADR-0003, ADR-0016, ADR-0018). Shared by
//! `mellow-server update` and the desktop app's updater; no UI and no service management here.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::Deserialize;

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/Paul-Carouge/mellow/releases/latest";
/// Assets are fetched from here, never from a URL the release API hands out.
const RELEASE_DOWNLOAD_BASE_URL: &str = "https://github.com/Paul-Carouge/mellow/releases/download";
/// The key every release is signed with (ADR-0016); rotated only by a release signed with it.
const RELEASE_PUBLIC_KEY: &str = include_str!("../../../release/minisign.pub");
/// The target triple this binary was built for, which names its release asset.
pub const CURRENT_TARGET: &str = env!("MELLOW_UPDATE_TARGET");

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const METADATA_TIMEOUT: Duration = Duration::from_secs(30);
/// A release binary is tens of megabytes; slow links still finish within this.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(15);
/// Bounds what a hostile or broken release host can make us buffer.
const MAXIMUM_METADATA_BYTES: usize = 1024 * 1024;
const MAXIMUM_SIGNATURE_BYTES: usize = 16 * 1024;
const MAXIMUM_EXECUTABLE_BYTES: usize = 256 * 1024 * 1024;

/// Which Mellow program an update is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Program {
    Server,
    Desktop,
}

impl Program {
    fn binary_name(self) -> &'static str {
        match self {
            Self::Server => "mellow-server",
            Self::Desktop => "mellow-desktop",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("cannot reach the release host")]
    Network(#[from] reqwest::Error),
    #[error("no release is published yet")]
    NoReleasePublished,
    #[error("unexpected answer from the release host: {0}")]
    UnexpectedResponse(String),
    #[error("release {version} has no {asset_name}")]
    MissingAsset {
        version: Version,
        asset_name: String,
    },
    #[error("the release signature does not verify")]
    InvalidSignature,
    #[error("the signature covers {signed_name}, not {expected_name}")]
    WrongSignedFile {
        expected_name: String,
        signed_name: String,
    },
    /// The executable or its directory could be changed by another account, which could then
    /// run code as whoever installs the update (typically root).
    #[error("refusing to update {path}: {reason}")]
    UnsafeInstallLocation { path: PathBuf, reason: &'static str },
    #[error("cannot replace the executable")]
    ReplaceExecutable(#[source] std::io::Error),
    /// A failed replacement could not put the original back at its path.
    #[error("the executable could not be put back; the original is now {original}")]
    ExecutableLeftAside {
        original: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("there is no previous executable to roll back to")]
    NoPreviousExecutable,
}

/// The newest published release.
#[derive(Debug, Clone)]
pub struct Release {
    pub version: Version,
    asset_names: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GithubReleaseAsset {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubReleaseAsset>,
}

/// The version of this build (the workspace version, shared by every Mellow binary).
///
/// # Panics
/// Never: Cargo only builds packages whose version is valid semver.
#[must_use]
pub fn current_version() -> Version {
    // Cargo refuses a package version that is not valid semver, so this cannot fail.
    #[allow(clippy::expect_used)]
    Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo package versions are semver")
}

/// Compares with the version of the running process: a long-running caller that already
/// installed an update must remember it, or it would be offered again.
#[must_use]
pub fn is_newer_than_current(release: &Release) -> bool {
    release.version > current_version()
}

/// `mellow-server-0.2.0-x86_64-unknown-linux-gnu`, with `.exe` on Windows (ADR-0016).
#[must_use]
pub fn asset_name(program: Program, version: &Version, target: &str) -> String {
    let extension = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    format!("{}-{version}-{target}{extension}", program.binary_name())
}

#[derive(Debug, Clone)]
pub struct UpdateClient {
    http: reqwest::Client,
}

impl UpdateClient {
    /// # Errors
    /// Fails only if the TLS stack cannot initialise.
    pub fn new() -> Result<Self, UpdateError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("mellow-update/", env!("CARGO_PKG_VERSION")))
            .https_only(true)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()?;
        Ok(Self { http })
    }

    /// # Errors
    /// `NoReleasePublished`, network errors, or an answer that does not describe a release.
    pub async fn fetch_latest_release(&self) -> Result<Release, UpdateError> {
        let request = self
            .http
            .get(LATEST_RELEASE_URL)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json");
        let body = download_body_with_size_limit(request, METADATA_TIMEOUT, MAXIMUM_METADATA_BYTES)
            .await
            .map_err(|error| match error {
                UpdateError::Network(error)
                    if error.status() == Some(reqwest::StatusCode::NOT_FOUND) =>
                {
                    UpdateError::NoReleasePublished
                }
                other => other,
            })?;
        parse_release(&body)
    }

    /// Downloads this platform's executable of `program` and its signature, and returns the
    /// executable only if the signature verifies against the release key for that exact file.
    ///
    /// # Errors
    /// Network errors, a missing asset, or a signature that does not verify.
    pub async fn download_verified_executable(
        &self,
        release: &Release,
        program: Program,
    ) -> Result<Vec<u8>, UpdateError> {
        let name = asset_name(program, &release.version, CURRENT_TARGET);
        let signature_name = format!("{name}.minisig");
        for wanted in [&name, &signature_name] {
            if !release.asset_names.contains(wanted) {
                return Err(UpdateError::MissingAsset {
                    version: release.version.clone(),
                    asset_name: wanted.clone(),
                });
            }
        }
        let download = |asset: &str, timeout, maximum_bytes| {
            let url = format!("{RELEASE_DOWNLOAD_BASE_URL}/v{}/{asset}", release.version);
            download_body_with_size_limit(self.http.get(url), timeout, maximum_bytes)
        };
        let signature =
            download(&signature_name, METADATA_TIMEOUT, MAXIMUM_SIGNATURE_BYTES).await?;
        let executable = download(&name, DOWNLOAD_TIMEOUT, MAXIMUM_EXECUTABLE_BYTES).await?;
        let signature = String::from_utf8(signature)
            .map_err(|_| UpdateError::UnexpectedResponse("signature is not text".into()))?;
        verify_signed_file(&executable, &signature, &name, RELEASE_PUBLIC_KEY)?;
        Ok(executable)
    }
}

async fn download_body_with_size_limit(
    request: reqwest::RequestBuilder,
    timeout: Duration,
    maximum_bytes: usize,
) -> Result<Vec<u8>, UpdateError> {
    let mut response = request.timeout(timeout).send().await?.error_for_status()?;
    let too_large = || UpdateError::UnexpectedResponse("answer too large".into());
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes as u64)
    {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > maximum_bytes {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn parse_release(body: &[u8]) -> Result<Release, UpdateError> {
    let release: GithubRelease = serde_json::from_slice(body)
        .map_err(|error| UpdateError::UnexpectedResponse(format!("{:?}", error.classify())))?;
    let version = release
        .tag_name
        .strip_prefix('v')
        .and_then(|version| Version::parse(version).ok())
        .ok_or_else(|| UpdateError::UnexpectedResponse("release tag is not vX.Y.Z".into()))?;
    Ok(Release {
        version,
        asset_names: release.assets.into_iter().map(|asset| asset.name).collect(),
    })
}

/// Checks `contents` against a minisign signature and that the signature's trusted comment is
/// `expected_name`, so a signature of another file, platform or version is refused (ADR-0016).
///
/// # Errors
/// `InvalidSignature` or `WrongSignedFile`.
pub fn verify_signed_file(
    contents: &[u8],
    signature_text: &str,
    expected_name: &str,
    public_key_text: &str,
) -> Result<(), UpdateError> {
    let public_key =
        PublicKey::decode(public_key_text).map_err(|_| UpdateError::InvalidSignature)?;
    let signature = Signature::decode(signature_text).map_err(|_| UpdateError::InvalidSignature)?;
    // The trusted comment is covered by the signature, so it can be read once this passes.
    public_key
        .verify(contents, &signature, false)
        .map_err(|_| UpdateError::InvalidSignature)?;
    let signed_name = signature.trusted_comment();
    if signed_name != expected_name {
        return Err(UpdateError::WrongSignedFile {
            expected_name: expected_name.to_owned(),
            signed_name: signed_name.to_owned(),
        });
    }
    Ok(())
}

/// Where `install_executable` keeps the executable it replaced.
#[must_use]
pub fn previous_executable_path(executable: &Path) -> PathBuf {
    path_with_suffix(executable, ".previous")
}

fn path_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Refuses an executable, or a directory holding it, that a user other than the one updating
/// (or root) could change, or that is a symbolic link (ADR-0018).
///
/// # Errors
/// `UnsafeInstallLocation`, or `ReplaceExecutable` when the files cannot be inspected.
#[cfg(unix)]
pub fn check_install_location(executable: &Path) -> Result<(), UpdateError> {
    use std::os::unix::fs::MetadataExt as _;
    const GROUP_OR_OTHERS_WRITE_BITS: u32 = 0o022;
    let updating_user_id = rustix::process::geteuid().as_raw();
    let directory = executable.parent().unwrap_or(Path::new("/"));
    for path in [executable, directory] {
        let metadata = std::fs::symlink_metadata(path).map_err(UpdateError::ReplaceExecutable)?;
        let unsafe_because = |reason| UpdateError::UnsafeInstallLocation {
            path: path.to_owned(),
            reason,
        };
        if metadata.file_type().is_symlink() {
            return Err(unsafe_because("it is a symbolic link"));
        }
        if metadata.uid() != updating_user_id && metadata.uid() != 0 {
            return Err(unsafe_because("another user owns it"));
        }
        if metadata.mode() & GROUP_OR_OTHERS_WRITE_BITS != 0 {
            return Err(unsafe_because("other users can write to it"));
        }
    }
    Ok(())
}

/// Windows has no owner check here: the desktop app installs per user, and the server refuses
/// to run on Windows (master key file permissions).
///
/// # Errors
/// Never.
#[cfg(not(unix))]
pub fn check_install_location(_executable: &Path) -> Result<(), UpdateError> {
    Ok(())
}

/// Replaces `executable` with `contents`, keeping the replaced file as `<executable>.previous`.
/// On Unix the replacement is one atomic rename; on Windows the running executable must be
/// renamed aside first. The new file keeps the old one's permissions (and owner, on Unix).
/// Blocking file I/O: call it off any UI thread.
///
/// # Errors
/// `UnsafeInstallLocation`, `ReplaceExecutable` (the original stays in place), or
/// `ExecutableLeftAside` if a failed replacement could not put the original back.
pub fn install_executable(contents: &[u8], executable: &Path) -> Result<(), UpdateError> {
    check_install_location(executable)?;
    let directory = executable.parent().ok_or_else(|| {
        UpdateError::ReplaceExecutable(std::io::Error::other("no parent directory"))
    })?;
    let mut new_executable = tempfile::Builder::new()
        .prefix(".mellow-update-")
        .tempfile_in(directory)
        .map_err(UpdateError::ReplaceExecutable)?;
    new_executable
        .write_all(contents)
        .and_then(|()| new_executable.as_file().sync_all())
        .map_err(UpdateError::ReplaceExecutable)?;
    let metadata = std::fs::metadata(executable).map_err(UpdateError::ReplaceExecutable)?;
    // Through the open file, never the path, which another process could swap meanwhile.
    new_executable
        .as_file()
        .set_permissions(metadata.permissions())
        .map_err(UpdateError::ReplaceExecutable)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        std::os::unix::fs::fchown(
            new_executable.as_file(),
            Some(metadata.uid()),
            Some(metadata.gid()),
        )
        .map_err(UpdateError::ReplaceExecutable)?;
    }
    let previous = previous_executable_path(executable);
    replace_keeping_previous(new_executable, executable, &previous)
}

#[cfg(unix)]
fn replace_keeping_previous(
    new_executable: tempfile::NamedTempFile,
    executable: &Path,
    previous: &Path,
) -> Result<(), UpdateError> {
    match std::fs::remove_file(previous) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(UpdateError::ReplaceExecutable(error));
        }
        _ => {}
    }
    std::fs::hard_link(executable, previous).map_err(UpdateError::ReplaceExecutable)?;
    new_executable
        .persist(executable)
        .map(|_| ())
        .map_err(|error| UpdateError::ReplaceExecutable(error.error))
}

#[cfg(not(unix))]
fn replace_keeping_previous(
    new_executable: tempfile::NamedTempFile,
    executable: &Path,
    previous: &Path,
) -> Result<(), UpdateError> {
    std::fs::rename(executable, previous).map_err(UpdateError::ReplaceExecutable)?;
    let Err(error) = new_executable.persist(executable) else {
        return Ok(());
    };
    Err(match std::fs::rename(previous, executable) {
        Ok(()) => UpdateError::ReplaceExecutable(error.error),
        Err(source) => UpdateError::ExecutableLeftAside {
            original: previous.to_owned(),
            source,
        },
    })
}

/// Swaps `<executable>.previous` back in; the rolled-back file is kept as
/// `<executable>.rolled-back`.
///
/// # Errors
/// `NoPreviousExecutable`, `ReplaceExecutable`, or `ExecutableLeftAside` if the executable could
/// not be put back after a failure.
pub fn roll_back_executable(executable: &Path) -> Result<(), UpdateError> {
    check_install_location(executable)?;
    let previous = previous_executable_path(executable);
    if !previous.is_file() {
        return Err(UpdateError::NoPreviousExecutable);
    }
    let rolled_back = path_with_suffix(executable, ".rolled-back");
    std::fs::rename(executable, &rolled_back).map_err(UpdateError::ReplaceExecutable)?;
    let Err(error) = std::fs::rename(&previous, executable) else {
        return Ok(());
    };
    Err(match std::fs::rename(&rolled_back, executable) {
        Ok(()) => UpdateError::ReplaceExecutable(error),
        Err(source) => UpdateError::ExecutableLeftAside {
            original: rolled_back,
            source,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PUBLIC_KEY: &str = include_str!("../tests/fixtures/test.pub");
    const PAYLOAD: &[u8] = include_bytes!("../tests/fixtures/payload");
    const PAYLOAD_SIGNATURE: &str = include_str!("../tests/fixtures/payload.minisig");
    const PAYLOAD_SIGNED_FOR_ANOTHER_FILE: &str =
        include_str!("../tests/fixtures/payload-other-name.minisig");
    const SIGNED_NAME: &str = "mellow-server-0.2.0-x86_64-unknown-linux-gnu";

    #[test]
    fn asset_names_follow_the_release_workflow() {
        let version = Version::new(0, 2, 0);
        assert_eq!(
            asset_name(Program::Server, &version, "x86_64-unknown-linux-gnu"),
            SIGNED_NAME
        );
        assert_eq!(
            asset_name(Program::Desktop, &version, "x86_64-pc-windows-msvc"),
            "mellow-desktop-0.2.0-x86_64-pc-windows-msvc.exe"
        );
    }

    #[test]
    fn a_signature_verifies_only_for_its_file_name_contents_and_key() {
        verify_signed_file(PAYLOAD, PAYLOAD_SIGNATURE, SIGNED_NAME, TEST_PUBLIC_KEY).unwrap();
        assert!(matches!(
            verify_signed_file(b"tampered", PAYLOAD_SIGNATURE, SIGNED_NAME, TEST_PUBLIC_KEY),
            Err(UpdateError::InvalidSignature)
        ));
        assert!(matches!(
            verify_signed_file(
                PAYLOAD,
                PAYLOAD_SIGNED_FOR_ANOTHER_FILE,
                SIGNED_NAME,
                TEST_PUBLIC_KEY
            ),
            Err(UpdateError::WrongSignedFile { .. })
        ));
        assert!(matches!(
            verify_signed_file(PAYLOAD, PAYLOAD_SIGNATURE, SIGNED_NAME, RELEASE_PUBLIC_KEY),
            Err(UpdateError::InvalidSignature)
        ));
        let forged_name = "mellow-server-9.9.9-x86_64-unknown-linux-gnu";
        let forged_comment = PAYLOAD_SIGNATURE.replace(SIGNED_NAME, forged_name);
        assert!(matches!(
            verify_signed_file(PAYLOAD, &forged_comment, forged_name, TEST_PUBLIC_KEY),
            Err(UpdateError::InvalidSignature)
        ));
    }

    #[test]
    fn the_release_key_is_a_minisign_public_key() {
        PublicKey::decode(RELEASE_PUBLIC_KEY).unwrap();
    }

    #[test]
    fn a_release_is_read_from_its_tag_and_asset_names() {
        let release = parse_release(
            br#"{"tag_name":"v0.2.0","draft":false,"assets":[
                {"name":"a","browser_download_url":"https://elsewhere.example/a","size":1}]}"#,
        )
        .unwrap();
        assert_eq!(release.version, Version::new(0, 2, 0));
        assert_eq!(release.asset_names, ["a"]);
        assert!(parse_release(br#"{"tag_name":"latest","assets":[]}"#).is_err());
        assert!(parse_release(b"<html>").is_err());
        let older = Release {
            version: Version::new(0, 0, 1),
            asset_names: Vec::new(),
        };
        assert!(!is_newer_than_current(&older));
    }

    #[tokio::test]
    async fn a_release_without_this_platform_is_refused_before_any_download() {
        let version = Version::new(9, 0, 0);
        let release = Release {
            asset_names: vec![asset_name(Program::Server, &version, CURRENT_TARGET)],
            version,
        };
        let result = UpdateClient::new()
            .unwrap()
            .download_verified_executable(&release, Program::Server)
            .await;
        assert!(matches!(
            result,
            Err(UpdateError::MissingAsset { asset_name, .. }) if asset_name.ends_with(".minisig")
        ));
    }

    /// The temporary directory itself must pass `check_install_location`.
    fn private_directory() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        directory
    }

    #[test]
    fn installing_keeps_the_previous_executable_for_a_roll_back() {
        let directory = private_directory();
        let executable = directory.path().join("mellow-server");
        std::fs::write(&executable, b"old").unwrap();
        assert!(matches!(
            roll_back_executable(&executable),
            Err(UpdateError::NoPreviousExecutable)
        ));

        install_executable(b"new", &executable).unwrap();
        assert_eq!(std::fs::read(&executable).unwrap(), b"new");
        assert_eq!(
            std::fs::read(previous_executable_path(&executable)).unwrap(),
            b"old"
        );
        install_executable(b"newer", &executable).unwrap();
        assert_eq!(
            std::fs::read(previous_executable_path(&executable)).unwrap(),
            b"new",
            "a second update replaces the kept executable"
        );
        let leftovers = std::fs::read_dir(directory.path())
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".mellow-update-")
            })
            .count();
        assert_eq!(leftovers, 0, "no temporary file left behind");

        roll_back_executable(&executable).unwrap();
        assert_eq!(std::fs::read(&executable).unwrap(), b"new");
        assert!(!previous_executable_path(&executable).exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_new_executable_keeps_the_permissions_of_the_old_one() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = private_directory();
        let executable = directory.path().join("mellow-server");
        std::fs::write(&executable, b"old").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o750)).unwrap();
        install_executable(b"new", &executable).unwrap();
        let mode = std::fs::metadata(&executable).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o750);
    }

    #[cfg(unix)]
    #[test]
    fn an_install_location_others_can_change_is_refused() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = private_directory();
        let executable = directory.path().join("mellow-server");
        std::fs::write(&executable, b"old").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o775)).unwrap();
        assert!(matches!(
            install_executable(b"new", &executable),
            Err(UpdateError::UnsafeInstallLocation { .. })
        ));
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(matches!(
            install_executable(b"new", &executable),
            Err(UpdateError::UnsafeInstallLocation { .. })
        ));
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&executable, &link).unwrap();
        assert!(matches!(
            install_executable(b"new", &link),
            Err(UpdateError::UnsafeInstallLocation { .. })
        ));
        assert_eq!(std::fs::read(&executable).unwrap(), b"old");
    }
}
