//! Builds libvpx (VP8 only, real-time only) from its pinned release source and links it
//! statically, with the small C face `src/vp8_shim.c` (ADR-0051: no system library).
//!
//! The source is the release archive, checked against its SHA-256, downloaded once into the
//! build directory; `MELLOW_LIBVPX_SOURCE_DIR` points at an unpacked copy instead, for offline
//! or packaged builds. Needs `make`, a C compiler, and `nasm` (or `yasm`) on x86-64.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest as _, Sha256};

const LIBVPX_VERSION: &str = "1.17.0";
/// The `WebM` project's own release tarball: fixed bytes, unlike GitHub's tag archives, which are
/// generated on demand and have changed before (2023), breaking every pinned digest.
const LIBVPX_ARCHIVE_URL: &str =
    "https://storage.googleapis.com/downloads.webmproject.org/releases/webm/libvpx-1.17.0.tar.gz";
const LIBVPX_ARCHIVE_SHA256: &str =
    "7f98aae5c52c46e85b7b26feabcab9b1554605e771bd1df1d6e6a00ee22fa8ed";
const SOURCE_DIRECTORY_VARIABLE: &str = "MELLOW_LIBVPX_SOURCE_DIR";
/// The server refuses larger `h` keyframes (ADR-0037), so the decoder never needs more: a
/// crafted stream cannot make it allocate beyond this.
const LARGEST_DECODED_PICTURE: &str = "2560x1440";

type BuildResult<T> = Result<T, String>;

fn main() {
    if let Err(reason) = build() {
        // A build script reports failure by exiting non-zero; the message says what to install.
        #[allow(clippy::print_stderr)]
        {
            eprintln!("mellow-media: libvpx could not be built: {reason}");
        }
        std::process::exit(1);
    }
}

fn build() -> BuildResult<()> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/vp8_shim.c");
    println!("cargo:rerun-if-env-changed={SOURCE_DIRECTORY_VARIABLE}");
    let output_directory = PathBuf::from(variable("OUT_DIR")?);
    let source = match env::var_os(SOURCE_DIRECTORY_VARIABLE) {
        Some(directory) => PathBuf::from(directory),
        None => downloaded_source(&output_directory)?,
    };
    let build_directory = output_directory.join(format!("libvpx-{LIBVPX_VERSION}-build"));
    let library = build_directory.join("libvpx.a");
    if !library.exists() {
        configure_and_make(&source, &build_directory)?;
    }
    cc::Build::new()
        .file("src/vp8_shim.c")
        .include(&source)
        .warnings(true)
        .try_compile("mellow_vp8_shim")
        .map_err(|error| error.to_string())?;
    println!(
        "cargo:rustc-link-search=native={}",
        build_directory.display()
    );
    println!("cargo:rustc-link-lib=static=vpx");
    Ok(())
}

fn variable(name: &str) -> BuildResult<String> {
    env::var(name).map_err(|_| format!("Cargo did not set {name}"))
}

/// The release archive, verified then unpacked, reused across builds of the same directory.
fn downloaded_source(output_directory: &Path) -> BuildResult<PathBuf> {
    let unpacked = output_directory.join(format!("libvpx-{LIBVPX_VERSION}"));
    if unpacked.join("configure").exists() {
        return Ok(unpacked);
    }
    let archive = output_directory.join(format!("libvpx-{LIBVPX_VERSION}.tar.gz"));
    run(Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--output",
        ])
        .arg(&archive)
        .arg(LIBVPX_ARCHIVE_URL))?;
    let bytes = fs::read(&archive).map_err(|error| error.to_string())?;
    let digest = Sha256::digest(&bytes)
        .iter()
        .fold(String::new(), |mut hexadecimal, byte| {
            // Writing to a String cannot fail.
            let _ = write!(hexadecimal, "{byte:02x}");
            hexadecimal
        });
    if digest != LIBVPX_ARCHIVE_SHA256 {
        return Err(format!(
            "the downloaded archive's SHA-256 is {digest}, expected {LIBVPX_ARCHIVE_SHA256}"
        ));
    }
    run(Command::new("tar")
        .arg("xzf")
        .arg(&archive)
        .arg("-C")
        .arg(output_directory))?;
    Ok(unpacked)
}

/// libvpx's own name for Cargo's target; `generic-gnu` (plain C, slow) for the others.
fn libvpx_target(target: &str) -> &'static str {
    match target {
        "aarch64-apple-darwin" => "arm64-darwin20-gcc",
        "x86_64-apple-darwin" => "x86_64-darwin20-gcc",
        "aarch64-unknown-linux-gnu" => "arm64-linux-gcc",
        "x86_64-unknown-linux-gnu" => "x86_64-linux-gcc",
        _ => "generic-gnu",
    }
}

fn configure_and_make(source: &Path, build_directory: &Path) -> BuildResult<()> {
    fs::create_dir_all(build_directory).map_err(|error| error.to_string())?;
    let target = variable("TARGET")?;
    run(Command::new("sh")
        .arg(source.join("configure"))
        .arg(format!("--target={}", libvpx_target(&target)))
        .args([
            "--enable-static",
            "--disable-shared",
            "--enable-pic",
            "--enable-vp8",
            "--disable-vp9",
            "--enable-realtime-only",
            "--disable-examples",
            "--disable-tools",
            "--disable-docs",
            "--disable-unit-tests",
            "--disable-install-docs",
            "--disable-install-bins",
            "--disable-install-srcs",
            "--disable-webm-io",
            "--disable-libyuv",
            "--disable-debug-libs",
        ])
        .arg(format!("--size-limit={LARGEST_DECODED_PICTURE}"))
        .current_dir(build_directory))
    .map_err(|reason| format!("{reason} (on x86-64, install nasm or yasm)"))?;
    let jobs = env::var("NUM_JOBS").unwrap_or_else(|_| "1".into());
    run(Command::new("make")
        .arg(format!("-j{jobs}"))
        .arg("libvpx.a")
        // Cargo's jobserver flags would make libvpx's make wait for tokens it never gets.
        .env_remove("MAKEFLAGS")
        .env_remove("MFLAGS")
        .current_dir(build_directory))
}

fn run(command: &mut Command) -> BuildResult<()> {
    let status = command
        .status()
        .map_err(|error| format!("{command:?} could not start: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{command:?} failed: {status}"))
    }
}
