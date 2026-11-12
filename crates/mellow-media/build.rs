//! Builds libvpx (VP8 only, real-time only) from its pinned release source and links it
//! statically, with the small C face `src/vp8_shim.c` (ADR-0051: no system library).
//!
//! The source is the release archive, checked against its SHA-256, downloaded once into the
//! build directory; `MELLOW_LIBVPX_SOURCE_DIR` points at an unpacked copy instead, for offline
//! or packaged builds. Needs `make`, a C compiler, and `nasm` (or `yasm`) on x86-64.
//!
//! On Windows (MSVC), libvpx's configure writes a Visual Studio solution that `MSBuild` builds:
//! `sh` and `make` come from MSYS2 or Git for Windows, and the assembler must be `yasm` (the
//! generated projects pass it yasm-only flags).

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
    let target = variable("TARGET")?;
    let (library_directory, library_name) = if target.ends_with("-pc-windows-msvc") {
        build_with_msbuild(&source, &build_directory, &target)?
    } else {
        if !build_directory.join("libvpx.a").exists() {
            configure_and_make(&source, &build_directory, &target)?;
        }
        (build_directory, "vpx")
    };
    cc::Build::new()
        .file("src/vp8_shim.c")
        .include(&source)
        .warnings(true)
        .try_compile("mellow_vp8_shim")
        .map_err(|error| error.to_string())?;
    println!(
        "cargo:rustc-link-search=native={}",
        library_directory.display()
    );
    println!("cargo:rustc-link-lib=static={library_name}");
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
    // Relative to the output directory: GNU tar (MSYS2's, on Windows) reads the colon of `D:\…`
    // as a remote host.
    let archive_name = format!("libvpx-{LIBVPX_VERSION}.tar.gz");
    let archive = output_directory.join(&archive_name);
    run(Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--output",
        ])
        .arg(&archive_name)
        .arg(LIBVPX_ARCHIVE_URL)
        .current_dir(output_directory))?;
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
        .arg(&archive_name)
        .current_dir(output_directory))?;
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

/// The configure options every platform shares: VP8 only, real time, the library alone.
const CONFIGURE_OPTIONS: [&str; 15] = [
    "--enable-static",
    "--disable-shared",
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
];

/// `sh configure` run in `build_directory`, with the shared options and `extra_options`.
fn configure(source: &Path, build_directory: &Path, extra_options: &[String]) -> BuildResult<()> {
    fs::create_dir_all(build_directory).map_err(|error| error.to_string())?;
    // The script finds its own directory from `$0`, which a Windows path's backslashes break.
    let configure_script = source
        .join("configure")
        .display()
        .to_string()
        .replace('\\', "/");
    run(Command::new("sh")
        .arg(configure_script)
        .args(CONFIGURE_OPTIONS)
        .args(extra_options)
        .arg(format!("--size-limit={LARGEST_DECODED_PICTURE}"))
        .current_dir(build_directory))
    .map_err(|reason| format!("{reason} (on x86-64, install nasm or yasm)"))
}

/// libvpx's make, without Cargo's jobserver flags, which would make it wait for tokens it never
/// gets.
fn make(build_directory: &Path) -> Command {
    let mut command = Command::new("make");
    command
        .env_remove("MAKEFLAGS")
        .env_remove("MFLAGS")
        .current_dir(build_directory);
    command
}

fn configure_and_make(source: &Path, build_directory: &Path, target: &str) -> BuildResult<()> {
    configure(
        source,
        build_directory,
        &[
            format!("--target={}", libvpx_target(target)),
            "--enable-pic".into(),
        ],
    )?;
    let jobs = env::var("NUM_JOBS").unwrap_or_else(|_| "1".into());
    run(make(build_directory)
        .arg(format!("-j{jobs}"))
        .arg("libvpx.a"))
}

/// Configures libvpx for Visual Studio, lets its make write the solution without building it,
/// then builds the Release library with `MSBuild`. Returns the library's directory and name
/// (`vpxmd`: the DLL C runtime, as Rust's MSVC targets use).
fn build_with_msbuild(
    source: &Path,
    build_directory: &Path,
    target: &str,
) -> BuildResult<(PathBuf, &'static str)> {
    let library_name = "vpxmd";
    let library_directory = build_directory.join("x64").join("Release");
    if library_directory
        .join(format!("{library_name}.lib"))
        .exists()
    {
        return Ok((library_directory, library_name));
    }
    if !target.starts_with("x86_64-") {
        return Err(format!(
            "libvpx is only built for x86-64 on Windows, not {target}"
        ));
    }
    let visual_studio = match cc::windows_registry::find_vs_version()? {
        cc::windows_registry::VsVers::Vs17 => "vs17",
        cc::windows_registry::VsVers::Vs18 => "vs18",
        older => {
            return Err(format!(
                "Visual Studio 2022 or newer is needed, found {older:?}"
            ));
        }
    };
    configure(
        source,
        build_directory,
        &[
            format!("--target=x86_64-win64-{visual_studio}"),
            "--as=yasm".into(),
        ],
    )?;
    // Writes the projects and the solution; MSBuild below builds only the Release library.
    run(make(build_directory).env("NO_LAUNCH_DEVENV", "1"))?;
    let mut msbuild = cc::windows_registry::find(target, "msbuild.exe")
        .ok_or("MSBuild was not found (install Visual Studio's C++ build tools)")?;
    run(msbuild
        .args([
            "vpx.sln",
            "-m",
            "-t:Build",
            "-p:Configuration=Release",
            "-p:Platform=x64",
        ])
        .current_dir(build_directory))?;
    Ok((library_directory, library_name))
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
