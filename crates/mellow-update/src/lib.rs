//! Finds, verifies and installs signed Mellow releases (ADR-0003, ADR-0016). Shared by
//! `mellow-server update` and the desktop app's updater; no UI and no service management here.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::Deserialize;

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/Paul-Carouge/mellow/releases/latest";
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
const MAXIMUM_EXECUTABLE_BYTES: usize = 512 * 1024 * 1024;

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
    #[error("cannot install the new executable: {0}")]
    Install(#[source] std::io::Error),
    #[error("there is no previous executable to roll back to")]
    NoPreviousExecutable,
}

/// The newest published release.
#[derive(Debug, Clone)]
pub struct Release {
    pub version: Version,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
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
    /// Network errors, or an answer that does not describe a release.
    pub async fn latest_release(&self) -> Result<Release, UpdateError> {
        let body = self
            .bounded_body(LATEST_RELEASE_URL, METADATA_TIMEOUT, MAXIMUM_METADATA_BYTES)
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
        let url_of = |wanted: &str| {
            release
                .assets
                .iter()
                .find(|asset| asset.name == wanted)
                .map(|asset| asset.browser_download_url.clone())
                .ok_or_else(|| UpdateError::MissingAsset {
                    version: release.version.clone(),
                    asset_name: wanted.to_owned(),
                })
        };
        let signature = self
            .bounded_body(
                &url_of(&signature_name)?,
                METADATA_TIMEOUT,
                MAXIMUM_SIGNATURE_BYTES,
            )
            .await?;
        let executable = self
            .bounded_body(&url_of(&name)?, DOWNLOAD_TIMEOUT, MAXIMUM_EXECUTABLE_BYTES)
            .await?;
        let signature = String::from_utf8(signature)
            .map_err(|_| UpdateError::UnexpectedResponse("signature is not text".into()))?;
        verify_signed_file(&executable, &signature, &name, RELEASE_PUBLIC_KEY)?;
        Ok(executable)
    }

    async fn bounded_body(
        &self,
        url: &str,
        timeout: Duration,
        maximum_bytes: usize,
    ) -> Result<Vec<u8>, UpdateError> {
        let mut response = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .timeout(timeout)
            .send()
            .await?
            .error_for_status()?;
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
        assets: release.assets,
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
    let mut name = executable.as_os_str().to_owned();
    name.push(".previous");
    PathBuf::from(name)
}

/// Replaces `executable` with `contents` by renames in its own directory, keeping the replaced
/// file as `<executable>.previous`. A running executable may be replaced this way on every
/// platform; the new one runs from the next start.
///
/// # Errors
/// `Install` when the directory is not writable or a rename fails; the original stays in place.
pub fn install_executable(contents: &[u8], executable: &Path) -> Result<(), UpdateError> {
    let directory = executable
        .parent()
        .ok_or_else(|| UpdateError::Install(std::io::Error::other("no parent directory")))?;
    let mut new_executable = tempfile::Builder::new()
        .prefix(".mellow-update-")
        .tempfile_in(directory)
        .map_err(UpdateError::Install)?;
    new_executable
        .write_all(contents)
        .and_then(|()| new_executable.as_file().sync_all())
        .map_err(UpdateError::Install)?;
    let permissions = std::fs::metadata(executable)
        .map_err(UpdateError::Install)?
        .permissions();
    std::fs::set_permissions(new_executable.path(), permissions).map_err(UpdateError::Install)?;
    let previous = previous_executable_path(executable);
    std::fs::rename(executable, &previous).map_err(UpdateError::Install)?;
    if let Err(error) = new_executable.persist(executable) {
        // Put the original back so the program still starts.
        let _ = std::fs::rename(&previous, executable);
        return Err(UpdateError::Install(error.error));
    }
    Ok(())
}

/// Swaps `<executable>.previous` back in; the rolled-back file is kept as
/// `<executable>.rolled-back`.
///
/// # Errors
/// `NoPreviousExecutable`, or `Install` when a rename fails.
pub fn roll_back_executable(executable: &Path) -> Result<(), UpdateError> {
    let previous = previous_executable_path(executable);
    if !previous.is_file() {
        return Err(UpdateError::NoPreviousExecutable);
    }
    let mut rolled_back = executable.as_os_str().to_owned();
    rolled_back.push(".rolled-back");
    std::fs::rename(executable, &rolled_back).map_err(UpdateError::Install)?;
    std::fs::rename(&previous, executable).map_err(UpdateError::Install)
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
        let mut forged_comment =
            PAYLOAD_SIGNATURE.replace(SIGNED_NAME, "mellow-server-9.9.9-x86_64-unknown-linux-gnu");
        forged_comment.push('\n');
        assert!(matches!(
            verify_signed_file(
                PAYLOAD,
                &forged_comment,
                "mellow-server-9.9.9-x86_64-unknown-linux-gnu",
                TEST_PUBLIC_KEY
            ),
            Err(UpdateError::InvalidSignature)
        ));
    }

    #[test]
    fn the_release_key_is_a_minisign_public_key() {
        PublicKey::decode(RELEASE_PUBLIC_KEY).unwrap();
    }

    #[test]
    fn a_release_is_read_from_its_tag_and_assets() {
        let release = parse_release(
            br#"{"tag_name":"v0.2.0","draft":false,"assets":[
                {"name":"a","browser_download_url":"https://example.org/a","size":1}]}"#,
        )
        .unwrap();
        assert_eq!(release.version, Version::new(0, 2, 0));
        assert_eq!(release.assets.len(), 1);
        assert!(parse_release(br#"{"tag_name":"latest","assets":[]}"#).is_err());
        assert!(parse_release(b"<html>").is_err());
        let older = Release {
            version: Version::new(0, 0, 1),
            assets: Vec::new(),
        };
        assert!(!is_newer_than_current(&older));
    }

    #[test]
    fn installing_keeps_the_previous_executable_for_a_roll_back() {
        let directory = tempfile::tempdir().unwrap();
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
        assert_eq!(std::fs::read(&executable).unwrap(), b"old");
        assert!(!previous_executable_path(&executable).exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_new_executable_keeps_the_permissions_of_the_old_one() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("mellow-server");
        std::fs::write(&executable, b"old").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        install_executable(b"new", &executable).unwrap();
        let mode = std::fs::metadata(&executable).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }
}
