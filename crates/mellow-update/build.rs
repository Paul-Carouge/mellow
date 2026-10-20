//! Records the target triple, which names the release asset this binary updates from.

fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    println!("cargo:rustc-env=MELLOW_UPDATE_TARGET={target}");
}
