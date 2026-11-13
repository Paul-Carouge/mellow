// ScreenCaptureKit exists from macOS 12.3 while the app starts from 11.0 (ADR-0055): weak-linking
// it lets dyld start the app without it, and screen sharing then reports itself unavailable
// (mellow-media checks for `SCStream` at run time). The binary imports no ScreenCaptureKit symbol.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg-bins=-Wl,-weak_framework,ScreenCaptureKit");
    }
}
