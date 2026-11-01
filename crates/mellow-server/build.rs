//! Embeds the web client's build (`web/dist`) in the binary when the `web-client` feature is on
//! (ADR-0019), so an instance serves it with no extra file. Without the feature the table is empty
//! and `cargo build` needs no Node toolchain.

use std::error::Error;
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::path::{Path, PathBuf};

type BuildResult<T> = Result<T, Box<dyn Error>>;

fn main() -> BuildResult<()> {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=CARGO_FEATURE_WEB_CLIENT");
    let output_directory = PathBuf::from(environment_variable("OUT_DIR")?);
    let mut table = String::from("pub(crate) static WEB_CLIENT_ASSETS: &[(&str, &[u8])] = &[\n");
    // Names the embedded build in ETags: it changes whenever any file of `dist` does.
    let mut build_hasher = DefaultHasher::new();
    if std::env::var_os("CARGO_FEATURE_WEB_CLIENT").is_some() {
        let dist =
            PathBuf::from(environment_variable("CARGO_MANIFEST_DIR")?).join("../../web/dist");
        let dist = dist.canonicalize().map_err(|_| {
            format!(
                "the web-client feature needs the web client's build: run `npm ci && npm run build` in web/ (looked for {})",
                dist.display()
            )
        })?;
        if !dist.join("index.html").is_file() {
            return Err(format!(
                "{} has no index.html: run `npm run build` in web/",
                dist.display()
            )
            .into());
        }
        println!("cargo::rerun-if-changed={}", dist.display());
        let mut files = Vec::new();
        collect_files(&dist, &mut files)?;
        files.sort();
        for file in files {
            let relative = file
                .strip_prefix(&dist)?
                .to_str()
                .ok_or("web build file names must be UTF-8")?
                .replace('\\', "/");
            relative.hash(&mut build_hasher);
            std::fs::read(&file)?.hash(&mut build_hasher);
            writeln!(
                table,
                "    ({relative:?}, include_bytes!({:?})),",
                file.display().to_string()
            )?;
        }
    }
    table.push_str("];\n");
    writeln!(
        table,
        "pub(crate) const WEB_CLIENT_BUILD_TAG: &str = \"{}-{:016x}\";",
        environment_variable("CARGO_PKG_VERSION")?.to_string_lossy(),
        build_hasher.finish()
    )?;
    std::fs::write(output_directory.join("web_client_assets.rs"), table)?;
    Ok(())
}

fn environment_variable(name: &str) -> BuildResult<std::ffi::OsString> {
    std::env::var_os(name).ok_or_else(|| format!("cargo did not set {name}").into())
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) -> BuildResult<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        // A dotfile (`.DS_Store`) or a link out of the build must never be published.
        if entry.file_name().to_string_lossy().starts_with('.') || entry.file_type()?.is_symlink() {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}
