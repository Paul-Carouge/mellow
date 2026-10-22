fn main() {
    // Debug info lets the tests find elements by accessible label; release binaries stay lean.
    let is_debug_build = std::env::var("PROFILE").is_ok_and(|profile| profile == "debug");
    let configuration = slint_build::CompilerConfiguration::new().with_debug_info(is_debug_build);
    // A build script has no caller to report to; failing loudly with the compiler message is the only option.
    #[allow(clippy::expect_used)]
    slint_build::compile_with_config("ui/app.slint", configuration)
        .expect("ui/app.slint must compile");
}
