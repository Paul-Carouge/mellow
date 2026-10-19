fn main() {
    // A build script has no caller to report to; failing loudly with the compiler message is the only option.
    #[allow(clippy::expect_used)]
    slint_build::compile("ui/app.slint").expect("ui/app.slint must compile");
}
