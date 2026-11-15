//! The self-update against a local release host (no network): finding, choosing, verifying,
//! staging and swapping in a package, and refusing a tampered one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use mellow_update::{Release, UpdateClient};
use semver::Version;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::package::{self, Installation};
use super::{UpdateProblem, UpdateStatus, can_restart_to_update, delay_with_jitter};

const TEST_PUBLIC_KEY: &str = include_str!("../../tests/fixtures/self_update/test.pub");
const LINUX_PACKAGE_NAME: &str = "mellow-desktop-9.0.0-x86_64-unknown-linux-gnu-ffmpeg7.tar.gz";
const LINUX_PACKAGE: &[u8] = include_bytes!(
    "../../tests/fixtures/self_update/mellow-desktop-9.0.0-x86_64-unknown-linux-gnu-ffmpeg7.tar.gz"
);
const LINUX_PACKAGE_SIGNATURE: &[u8] = include_bytes!(
    "../../tests/fixtures/self_update/mellow-desktop-9.0.0-x86_64-unknown-linux-gnu-ffmpeg7.tar.gz.minisig"
);
/// What the fixture's `bin/mellow` holds.
#[cfg(unix)]
const NEW_BINARY: &[u8] = b"new mellow\n";
const OLD_BINARY: &[u8] = b"old mellow\n";

/// Serves `routes` over plain HTTP on a loopback port until the test's runtime ends; any other
/// path answers 404. Returns the base URL.
async fn serve_release_host(routes: HashMap<String, Vec<u8>>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let routes = Arc::new(routes);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let routes = Arc::clone(&routes);
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut chunk = [0_u8; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => request.extend_from_slice(&chunk[..read]),
                    }
                }
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let (status, body) = routes.get(path).map_or(("404 Not Found", &[][..]), |body| {
                    ("200 OK", body.as_slice())
                });
                let head = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(body).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://{address}")
}

fn release_json(version: &str, asset_names: &[&str]) -> Vec<u8> {
    let assets: Vec<serde_json::Value> = asset_names
        .iter()
        .map(|name| serde_json::json!({ "name": name }))
        .collect();
    serde_json::json!({ "tag_name": format!("v{version}"), "assets": assets })
        .to_string()
        .into_bytes()
}

/// A release host for v9.0.0 carrying the Linux FFmpeg 7 package, signed with the test key.
fn routes_with(package: &[u8], signature: &[u8]) -> HashMap<String, Vec<u8>> {
    let signature_name = format!("{LINUX_PACKAGE_NAME}.minisig");
    HashMap::from([
        (
            "/latest".to_owned(),
            release_json("9.0.0", &[LINUX_PACKAGE_NAME, &signature_name]),
        ),
        (
            format!("/download/v9.0.0/{LINUX_PACKAGE_NAME}"),
            package.to_vec(),
        ),
        (
            format!("/download/v9.0.0/{signature_name}"),
            signature.to_vec(),
        ),
    ])
}

/// A home whose `~/.local/bin/mellow` holds [`OLD_BINARY`].
fn home_with_installed_mellow() -> (tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let bin = home.path().join(".local").join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let executable = bin.join("mellow");
    std::fs::write(&executable, OLD_BINARY).unwrap();
    (home, executable)
}

fn linux_installation(home: &Path, executable: &Path, ffmpeg_majors: Vec<u32>) -> Installation {
    Installation::LinuxUserInstall {
        executable: executable.to_owned(),
        home: home.to_owned(),
        ffmpeg_majors,
        staging_folder: home.join(".cache").join("mellow").join("updates"),
    }
}

fn file_names_in(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

// Runs the package's `install.sh`, which needs a Unix shell.
#[cfg(unix)]
#[tokio::test]
async fn a_signed_linux_package_is_found_verified_staged_and_installed() {
    let base_url = serve_release_host(routes_with(LINUX_PACKAGE, LINUX_PACKAGE_SIGNATURE)).await;
    let client = UpdateClient::with_test_release_host(&base_url, TEST_PUBLIC_KEY).unwrap();
    let (home, executable) = home_with_installed_mellow();
    // FFmpeg 8 is the newest the system has, but the release only carries a package for 7.
    let installation = linux_installation(home.path(), &executable, vec![8, 7]);

    let release = package::find_newer_release(&client, &Version::new(1, 0, 0))
        .await
        .unwrap()
        .expect("9.0.0 is newer than 1.0.0");
    assert_eq!(release.version, Version::new(9, 0, 0));
    let asset_name = package::asset_name_for(&installation, &release).unwrap();
    assert_eq!(asset_name, LINUX_PACKAGE_NAME);
    let progress = mellow_update::DownloadProgress::default();
    let downloaded =
        package::download_verified_package(&client, &release, &asset_name, Some(&progress))
            .await
            .unwrap();
    let package_bytes = u64::try_from(LINUX_PACKAGE.len()).unwrap();
    assert_eq!(
        (progress.received_bytes(), progress.total_bytes()),
        (package_bytes, Some(package_bytes)),
        "the package's bytes are counted for the toast's progress bar, the signature's are not"
    );
    let mut staged = package::stage_package(
        &installation,
        release.version.clone(),
        &asset_name,
        &downloaded,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(&executable).unwrap(),
        OLD_BINARY,
        "nothing replaced before the swap"
    );

    let relaunch = package::apply_staged_update(&mut staged, true).unwrap();
    assert_eq!(relaunch, package::Relaunch::ThisApp(executable.clone()));
    assert_eq!(std::fs::read(&executable).unwrap(), NEW_BINARY);
    assert_eq!(
        file_names_in(executable.parent().unwrap()),
        ["mellow"],
        "no previous copy or temporary file left beside it"
    );
    let staging_folder = home.path().join(".cache").join("mellow").join("updates");
    assert_eq!(file_names_in(&staging_folder).len(), 1);
    drop(staged);
    assert!(
        file_names_in(&staging_folder).is_empty(),
        "staged in the user's cache, removed with the update"
    );
}

// Runs the package's `install.sh`, which needs a Unix shell.
#[cfg(unix)]
#[test]
fn a_copy_reinstalled_after_staging_is_not_replaced() {
    let (home, executable) = home_with_installed_mellow();
    let installation = linux_installation(home.path(), &executable, vec![7]);
    let mut staged = package::stage_package(
        &installation,
        Version::new(9, 0, 0),
        LINUX_PACKAGE_NAME,
        LINUX_PACKAGE,
    )
    .unwrap();
    // A newer version installed by hand, as `install.sh` does: a new file renamed into place.
    let manual = executable.with_file_name(".manual");
    std::fs::write(&manual, b"manual mellow\n").unwrap();
    std::fs::rename(&manual, &executable).unwrap();

    assert_eq!(
        package::apply_staged_update(&mut staged, true).unwrap(),
        package::Relaunch::ThisApp(executable.clone()),
        "the reinstalled copy is what restarts"
    );
    assert_eq!(std::fs::read(&executable).unwrap(), b"manual mellow\n");
}

#[tokio::test]
async fn the_same_or_an_older_release_is_not_offered() {
    let base_url = serve_release_host(HashMap::from([(
        "/latest".to_owned(),
        release_json("9.0.0", &[]),
    )]))
    .await;
    let client = UpdateClient::with_test_release_host(&base_url, TEST_PUBLIC_KEY).unwrap();
    for current in [Version::new(9, 0, 0), Version::new(10, 0, 0)] {
        assert!(
            package::find_newer_release(&client, &current)
                .await
                .unwrap()
                .is_none()
        );
    }
    // No release at all is not a failure either.
    let empty_host = serve_release_host(HashMap::new()).await;
    let client = UpdateClient::with_test_release_host(&empty_host, TEST_PUBLIC_KEY).unwrap();
    assert_eq!(
        package::find_newer_release(&client, &Version::new(1, 0, 0))
            .await
            .unwrap()
            .map(|release| release.version),
        None
    );
}

#[tokio::test]
async fn a_release_host_that_is_not_one_is_a_network_problem() {
    let base_url = serve_release_host(HashMap::from([(
        "/latest".to_owned(),
        br#"{"tag_name":"v9.0.0-from-elsewhere","assets":[]}"#.to_vec(),
    )]))
    .await;
    let client = UpdateClient::with_test_release_host(&base_url, TEST_PUBLIC_KEY).unwrap();
    assert!(matches!(
        package::find_newer_release(&client, &Version::new(1, 0, 0)).await,
        Err(UpdateProblem::Network)
    ));
}

#[tokio::test]
async fn a_tampered_package_or_signature_is_refused_before_anything_is_unpacked() {
    let mut tampered_package = LINUX_PACKAGE.to_vec();
    if let Some(last) = tampered_package.last_mut() {
        *last ^= 1;
    }
    let other_name_signature = String::from_utf8(LINUX_PACKAGE_SIGNATURE.to_vec())
        .unwrap()
        .replace(
            LINUX_PACKAGE_NAME,
            "mellow-desktop-9.0.0-universal-apple-darwin.zip",
        );
    let mut tampered_signature = LINUX_PACKAGE_SIGNATURE.to_vec();
    // A character of the signature line, the second of the file.
    let signature_line_start = tampered_signature
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap()
        + 1;
    tampered_signature[signature_line_start + 10] ^= 1;
    let release = Release::for_tests(
        Version::new(9, 0, 0),
        vec![
            LINUX_PACKAGE_NAME.to_owned(),
            format!("{LINUX_PACKAGE_NAME}.minisig"),
        ],
    );
    for (package, signature) in [
        (tampered_package.as_slice(), LINUX_PACKAGE_SIGNATURE),
        (LINUX_PACKAGE, tampered_signature.as_slice()),
        (LINUX_PACKAGE, other_name_signature.as_bytes()),
    ] {
        let base_url = serve_release_host(routes_with(package, signature)).await;
        let client = UpdateClient::with_test_release_host(&base_url, TEST_PUBLIC_KEY).unwrap();
        assert_eq!(
            package::download_verified_package(&client, &release, LINUX_PACKAGE_NAME, None)
                .await
                .err(),
            Some(UpdateProblem::BadSignature)
        );
    }
}

#[test]
fn the_asset_follows_the_system_and_the_release() {
    let version = Version::new(9, 0, 0);
    let names = |names: &[&str]| -> Vec<String> {
        names
            .iter()
            .flat_map(|name| [(*name).to_owned(), format!("{name}.minisig")])
            .collect()
    };
    let release = Release::for_tests(
        version.clone(),
        names(&[
            "mellow-desktop-9.0.0-universal-apple-darwin.zip",
            "mellow-desktop-9.0.0-x86_64-unknown-linux-gnu-ffmpeg6.tar.gz",
            "mellow-desktop-9.0.0-x86_64-unknown-linux-gnu-ffmpeg9.tar.gz",
        ]),
    );
    let macos = Installation::MacosBundle {
        bundle: PathBuf::from("/Applications/Mellow.app"),
        designated_requirement: String::new(),
    };
    assert_eq!(
        package::asset_name_for(&macos, &release).unwrap(),
        "mellow-desktop-9.0.0-universal-apple-darwin.zip"
    );
    let linux = |majors| {
        linux_installation(
            Path::new("/home/a"),
            Path::new("/home/a/.local/bin/mellow"),
            majors,
        )
    };
    assert_eq!(
        package::asset_name_for(&linux(vec![9, 6]), &release).unwrap(),
        "mellow-desktop-9.0.0-x86_64-unknown-linux-gnu-ffmpeg9.tar.gz",
        "the newest FFmpeg wins"
    );
    assert_eq!(
        package::asset_name_for(&linux(vec![7, 6]), &release).unwrap(),
        "mellow-desktop-9.0.0-x86_64-unknown-linux-gnu-ffmpeg6.tar.gz"
    );
    assert_eq!(
        package::asset_name_for(&linux(vec![8]), &release),
        Err(UpdateProblem::NoPackage)
    );
    assert_eq!(
        package::asset_name_for(&linux(Vec::new()), &release),
        Err(UpdateProblem::NoFfmpeg)
    );
    let windows = Installation::WindowsUserInstall {
        executable: PathBuf::from(r"C:\Users\a\AppData\Local\Programs\Mellow\mellow.exe"),
        staging_folder: PathBuf::from(r"C:\Users\a\AppData\Local\mellow\updates"),
    };
    assert_eq!(
        package::asset_name_for(&windows, &release),
        Err(UpdateProblem::NoPackage),
        "a release without the Windows installer"
    );
    let with_windows = Release::for_tests(
        Version::new(9, 0, 0),
        names(&["mellow-desktop-9.0.0-x86_64-pc-windows-msvc-setup.exe"]),
    );
    assert_eq!(
        package::asset_name_for(&windows, &with_windows).unwrap(),
        "mellow-desktop-9.0.0-x86_64-pc-windows-msvc-setup.exe"
    );
    let unsigned = Release::for_tests(
        version,
        vec!["mellow-desktop-9.0.0-universal-apple-darwin.zip".to_owned()],
    );
    assert_eq!(
        package::asset_name_for(&macos, &unsigned),
        Err(UpdateProblem::NoPackage),
        "an asset without its signature is not offered"
    );
}

#[test]
fn ffmpeg_majors_come_from_libavcodec_sonames() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    for name in [
        "libavcodec.so.61",
        "libavcodec.so.61.19.101",
        "libavcodec.so",
        "libavcodec.so.6x",
        "libavformat.so.62",
    ] {
        std::fs::write(first.path().join(name), b"").unwrap();
    }
    for name in ["libavcodec.so.63", "libavcodec.so.61", "libavcodec.so.12"] {
        std::fs::write(second.path().join(name), b"").unwrap();
    }
    let missing = first.path().join("missing");
    assert_eq!(
        package::ffmpeg_majors_in(&[first.path(), second.path(), &missing]),
        [9, 7],
        "newest first, once each, only libavcodec, nothing below FFmpeg 1"
    );
}

#[test]
fn only_the_install_script_copy_updates_itself_on_linux() {
    let (home, executable) = home_with_installed_mellow();
    let executable = std::fs::canonicalize(executable).unwrap();
    assert_eq!(
        package::check_linux_install_location(&executable, home.path()),
        Ok(())
    );
    assert_eq!(
        package::check_linux_install_location(Path::new("/usr/bin/mellow"), home.path()),
        Err(UpdateProblem::PackageManager)
    );
    assert_eq!(
        package::check_linux_install_location(Path::new("/opt/mellow/mellow"), home.path()),
        Err(UpdateProblem::PackageManager)
    );
    assert_eq!(
        package::check_linux_install_location(
            &home.path().join("src/target/release/mellow"),
            home.path()
        ),
        Err(UpdateProblem::InstalledElsewhere)
    );
}

#[test]
fn only_the_installer_copy_updates_itself_on_windows() {
    let local_app_data = tempfile::tempdir().unwrap();
    let folder = local_app_data.path().join("Programs").join("Mellow");
    std::fs::create_dir_all(&folder).unwrap();
    let executable = folder.join("mellow.exe");
    assert_eq!(
        package::check_windows_installation(&executable, local_app_data.path(), false),
        Ok(())
    );
    // Windows paths are compared without case (this file system may not find that folder).
    let other_case = local_app_data
        .path()
        .join("programs")
        .join("MELLOW")
        .join("Mellow.exe");
    assert_ne!(
        package::check_windows_installation(&other_case, local_app_data.path(), false),
        Err(UpdateProblem::NotPackaged)
    );
    for elsewhere in [
        local_app_data.path().join("src/target/release/mellow.exe"),
        local_app_data.path().join("Programs/Mellow/old/mellow.exe"),
        PathBuf::from("/Program Files/Mellow/mellow.exe"),
    ] {
        assert_eq!(
            package::check_windows_installation(&elsewhere, local_app_data.path(), false),
            Err(UpdateProblem::NotPackaged),
            "{}",
            elsewhere.display()
        );
    }
    // A folder that is missing or cannot be written to.
    let missing = tempfile::tempdir().unwrap();
    let unwritable = missing.path().join("Programs/Mellow/mellow.exe");
    assert_eq!(
        package::check_windows_installation(&unwritable, missing.path(), false),
        Err(UpdateProblem::NotWritable)
    );
}

#[test]
fn an_elevated_mellow_never_updates_itself_on_windows() {
    let local_app_data = tempfile::tempdir().unwrap();
    let folder = local_app_data.path().join("Programs").join("Mellow");
    std::fs::create_dir_all(&folder).unwrap();
    assert_eq!(
        package::check_windows_installation(
            &folder.join("mellow.exe"),
            local_app_data.path(),
            true
        ),
        Err(UpdateProblem::Elevated),
        "even the installer's own copy"
    );
}

#[test]
fn a_staged_installer_changed_after_its_check_is_refused() {
    let local_app_data = tempfile::tempdir().unwrap();
    let installation = Installation::WindowsUserInstall {
        executable: local_app_data.path().join("Programs/Mellow/mellow.exe"),
        staging_folder: local_app_data.path().join("mellow/updates"),
    };
    let verified = b"MZ verified installer";
    let staged = package::stage_package(
        &installation,
        Version::new(9, 0, 0),
        "mellow-desktop-9.0.0-x86_64-pc-windows-msvc-setup.exe",
        verified,
    )
    .unwrap();
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, verified);
    let installer = std::fs::read_dir(local_app_data.path().join("mellow/updates"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("mellow-setup.exe");
    let mut file = std::fs::File::open(&installer).unwrap();
    assert_eq!(
        package::check_staged_installer(&mut file, digest.as_ref()),
        Ok(())
    );
    std::fs::write(&installer, b"MZ another program").unwrap();
    let mut file = std::fs::File::open(&installer).unwrap();
    assert_eq!(
        package::check_staged_installer(&mut file, digest.as_ref()),
        Err(UpdateProblem::BadPackage)
    );
    drop(file);
    drop(staged);
}

#[test]
fn the_windows_installer_runs_silently_and_waits_for_the_app_to_quit() {
    assert_eq!(
        package::windows_installer_arguments(false),
        ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/UPDATE"],
        "at quit: nothing opens afterwards"
    );
    assert_eq!(
        package::windows_installer_arguments(true),
        [
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/UPDATE",
            "/LAUNCH"
        ],
        "\"Restart to update\": the installer opens the new version"
    );
}

#[test]
fn a_windows_installer_is_staged_as_an_executable_and_nothing_else() {
    let local_app_data = tempfile::tempdir().unwrap();
    let installation = Installation::WindowsUserInstall {
        executable: local_app_data.path().join("Programs/Mellow/mellow.exe"),
        staging_folder: local_app_data.path().join("mellow/updates"),
    };
    let name = "mellow-desktop-9.0.0-x86_64-pc-windows-msvc-setup.exe";
    assert_eq!(
        package::stage_package(&installation, Version::new(9, 0, 0), name, b"not a program")
            .unwrap_err(),
        UpdateProblem::BadPackage
    );
    let staged =
        package::stage_package(&installation, Version::new(9, 0, 0), name, b"MZ installer")
            .unwrap();
    let staging_folder = local_app_data.path().join("mellow/updates");
    let staging_directories = file_names_in(&staging_folder);
    assert_eq!(staging_directories.len(), 1);
    let staging = staging_folder.join(&staging_directories[0]);
    assert_eq!(
        std::fs::read(staging.join("mellow-setup.exe")).unwrap(),
        b"MZ installer"
    );
    assert!(!staging.join("package").exists());
    drop(staged);
    assert_eq!(file_names_in(&staging_folder), Vec::<String>::new());
}

#[test]
fn a_release_app_is_recognised_by_its_bundle_and_requirement() {
    assert_eq!(
        package::bundle_of_executable(Path::new("/Applications/Mellow.app/Contents/MacOS/mellow")),
        Some(PathBuf::from("/Applications/Mellow.app"))
    );
    assert_eq!(
        package::bundle_of_executable(Path::new("/Users/a/target/debug/mellow")),
        None
    );
    assert!(package::is_translocated(Path::new(
        "/private/var/folders/ab/cd/T/AppTranslocation/0123-4567/d/Mellow.app"
    )));
    assert!(!package::is_translocated(Path::new(
        "/Applications/Mellow.app"
    )));
    let output = "Executable=/Applications/Mellow.app/Contents/MacOS/mellow\n\
        designated => identifier \"org.mellow.desktop\" and certificate leaf = H\"0123abcd\"\n";
    let requirement = package::designated_requirement_of(output).unwrap();
    assert_eq!(
        requirement,
        "identifier \"org.mellow.desktop\" and certificate leaf = H\"0123abcd\""
    );
    assert!(package::is_release_certificate_requirement(requirement));
    assert!(
        !package::is_release_certificate_requirement("cdhash H\"0123abcd\""),
        "an ad hoc build is not updated"
    );
    assert!(!package::is_release_certificate_requirement(
        "identifier \"org.example\" and certificate leaf = H\"0123abcd\""
    ));
    assert_eq!(package::designated_requirement_of("nothing here"), None);
}

#[test]
fn a_failed_swap_puts_the_installed_copy_back() {
    let directory = tempfile::tempdir().unwrap();
    let installed = directory.path().join("Mellow.app");
    let previous = directory.path().join("previous.app");
    std::fs::create_dir(&installed).unwrap();
    std::fs::write(installed.join("version"), b"old").unwrap();

    let missing = directory.path().join("missing.app");
    assert_eq!(
        package::swap_in(&missing, &installed, &previous),
        Err(package::SwapFailure::NothingMoved)
    );
    assert_eq!(std::fs::read(installed.join("version")).unwrap(), b"old");
    assert!(!previous.exists());

    let new = directory.path().join("new.app");
    std::fs::create_dir(&new).unwrap();
    std::fs::write(new.join("version"), b"new").unwrap();
    assert_eq!(package::swap_in(&new, &installed, &previous), Ok(()));
    assert_eq!(std::fs::read(installed.join("version")).unwrap(), b"new");
    assert_eq!(std::fs::read(previous.join("version")).unwrap(), b"old");
    assert!(!new.exists());
}

#[test]
fn a_failing_install_script_leaves_the_running_binary_in_place() {
    let (home, executable) = home_with_installed_mellow();
    let package = tempfile::tempdir().unwrap();
    let script = package.path().join("install.sh");
    // Replaces the binary, then fails, as a script stopping halfway would.
    std::fs::write(
        &script,
        "printf broken > \"$HOME/.local/bin/.new\" && mv -f \"$HOME/.local/bin/.new\" \"$HOME/.local/bin/mellow\"\nexit 1\n",
    )
    .unwrap();
    assert_eq!(
        package::run_install_script(&script, &executable, home.path()),
        Err(UpdateProblem::InstallFailed)
    );
    assert_eq!(std::fs::read(&executable).unwrap(), OLD_BINARY);
    assert_eq!(file_names_in(executable.parent().unwrap()), ["mellow"]);
}

#[test]
fn an_install_script_failing_before_replacing_anything_leaves_no_extra_name() {
    let (home, executable) = home_with_installed_mellow();
    let package = tempfile::tempdir().unwrap();
    let script = package.path().join("install.sh");
    // As `install.sh` stopping at its `ldd` check, before it moves anything.
    std::fs::write(&script, "exit 1\n").unwrap();
    assert_eq!(
        package::run_install_script(&script, &executable, home.path()),
        Err(UpdateProblem::InstallFailed)
    );
    assert_eq!(std::fs::read(&executable).unwrap(), OLD_BINARY);
    assert_eq!(file_names_in(executable.parent().unwrap()), ["mellow"]);
}

#[test]
fn a_damaged_linux_package_is_refused() {
    let (home, executable) = home_with_installed_mellow();
    let installation = linux_installation(home.path(), &executable, vec![7]);
    assert_eq!(
        package::stage_package(
            &installation,
            Version::new(9, 0, 0),
            LINUX_PACKAGE_NAME,
            b"not a tarball"
        )
        .err(),
        Some(UpdateProblem::BadPackage)
    );
    // A signed tarball under another asset's name lacks the folder that name promises.
    assert_eq!(
        package::stage_package(
            &installation,
            Version::new(9, 0, 0),
            "mellow-desktop-9.0.0-x86_64-unknown-linux-gnu-ffmpeg8.tar.gz",
            LINUX_PACKAGE
        )
        .err(),
        Some(UpdateProblem::BadPackage)
    );
}

#[cfg(target_os = "macos")]
#[test]
fn an_app_not_signed_by_the_running_certificate_is_refused() {
    let source = tempfile::tempdir().unwrap();
    let bundle = source.path().join("Mellow.app");
    std::fs::create_dir_all(bundle.join("Contents").join("MacOS")).unwrap();
    std::fs::write(
        bundle.join("Contents").join("MacOS").join("mellow"),
        b"unsigned",
    )
    .unwrap();
    let zip = source.path().join("Mellow.zip");
    let zipped = std::process::Command::new("/usr/bin/ditto")
        .args(["-c", "-k", "--keepParent"])
        .arg(&bundle)
        .arg(&zip)
        .status()
        .unwrap();
    assert!(zipped.success());
    let applications = tempfile::tempdir().unwrap();
    let installed = applications.path().join("Mellow.app");
    std::fs::create_dir(&installed).unwrap();
    let installation = Installation::MacosBundle {
        bundle: installed.clone(),
        designated_requirement:
            "identifier \"org.mellow.desktop\" and certificate leaf = H\"0000000000000000000000000000000000000000\""
                .to_owned(),
    };
    assert_eq!(
        package::stage_package(
            &installation,
            Version::new(9, 0, 0),
            "mellow-desktop-9.0.0-universal-apple-darwin.zip",
            &std::fs::read(&zip).unwrap()
        )
        .err(),
        Some(UpdateProblem::WrongCertificate)
    );
    assert_eq!(
        file_names_in(applications.path()),
        ["Mellow.app"],
        "the staging folder is removed"
    );
}

#[test]
fn leftover_staging_folders_are_removed_and_nothing_else() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::create_dir(folder.path().join(".mellow-update-abc")).unwrap();
    std::fs::write(folder.path().join(".mellow-update-abc").join("file"), b"").unwrap();
    std::fs::write(folder.path().join(".mellow-update-file"), b"").unwrap();
    std::fs::create_dir(folder.path().join("Mellow.app")).unwrap();
    // Another running instance's ready update, its lock held.
    let held = folder.path().join(".mellow-update-held");
    std::fs::create_dir(&held).unwrap();
    let lock = std::fs::File::create(held.join("lock")).unwrap();
    lock.try_lock().unwrap();
    // A crashed instance's, its lock released.
    let released = folder.path().join(".mellow-update-released");
    std::fs::create_dir(&released).unwrap();
    std::fs::write(released.join("lock"), b"").unwrap();
    package::remove_stale_staging_directories(folder.path());
    assert_eq!(
        file_names_in(folder.path()),
        [".mellow-update-file", ".mellow-update-held", "Mellow.app"]
    );
    drop(lock);
}

#[test]
fn an_update_never_restarts_the_app_during_a_call() {
    let ready = UpdateStatus::Ready(Version::new(9, 0, 0));
    assert!(can_restart_to_update(&ready, false));
    assert!(!can_restart_to_update(&ready, true));
    assert!(!can_restart_to_update(
        &UpdateStatus::Downloading(Version::new(9, 0, 0)),
        false
    ));
    assert!(!can_restart_to_update(
        &UpdateStatus::Failed(UpdateProblem::Network),
        false
    ));
}

#[test]
fn checks_are_spread_by_their_jitter() {
    let base = Duration::from_hours(24);
    let jitter = Duration::from_hours(2);
    assert_eq!(delay_with_jitter(base, jitter, 0), base);
    let jitter_milliseconds = u64::try_from(jitter.as_millis()).unwrap();
    assert_eq!(
        delay_with_jitter(base, jitter, jitter_milliseconds),
        base + jitter
    );
    assert_eq!(
        delay_with_jitter(base, jitter, jitter_milliseconds + 1),
        base
    );
    assert!(delay_with_jitter(base, jitter, u64::MAX) <= base + jitter);
    assert_eq!(delay_with_jitter(base, Duration::ZERO, u64::MAX), base);
}
