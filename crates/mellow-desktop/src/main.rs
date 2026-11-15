//! `mellow`, the desktop client.

// Upstream cause: macOS ld warns once a debug binary holds more than 16 MB of `__eh_frame`, which
// its compact unwind table cannot encode; with the voice engine (str0m, WebRTC's processing,
// libvpx) the app crosses it. Only the speed of unwinding a panic is affected.
#![cfg_attr(target_os = "macos", allow(linker_messages))]
// No console window behind the app on Windows (ADR-0060); debug builds keep theirs for the logs.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::process::ExitCode;

use mellow_desktop::components::gallery::gallery_requested;
use mellow_desktop::launch_preview::preview_requested;
use mellow_desktop::session::LOGOUT_ON_EXIT_TIMEOUT;
use mellow_desktop::single_instance::{InstanceClaim, claim_single_instance};
use mellow_desktop::theme::EmbeddedAssets;
use mellow_desktop::tokio_bridge::build_network_runtime;

fn main() -> ExitCode {
    // Answered before anything starts: the release workflow checks the installed copy with it.
    if std::env::args_os()
        .nth(1)
        .is_some_and(|argument| argument == "--version")
    {
        attach_to_the_parent_console();
        // The one line a command-line caller asked for.
        #[allow(clippy::print_stdout)]
        {
            println!("mellow {}", env!("CARGO_PKG_VERSION"));
        }
        return ExitCode::SUCCESS;
    }
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    // A bad MELLOW_PROFILE ends the launch here, before any thread could read the everyday one.
    let _ = mellow_desktop::profile::active_profile_name();

    // Before anything else: a second launch only shows the running Mellow's window. The component
    // gallery and the launch preview open next to a running Mellow.
    let claim = (gallery_requested().is_none() && preview_requested().is_none())
        .then(claim_single_instance);
    let show_requests = match claim {
        None => None,
        Some(Ok(InstanceClaim::First(show_requests))) => Some(show_requests),
        Some(Ok(InstanceClaim::AlreadyRunning)) => {
            tracing::info!("Mellow already runs: its window was asked to show");
            return ExitCode::SUCCESS;
        }
        Some(Err(error)) => {
            tracing::warn!(error = %error.kind(), "cannot make sure only one Mellow runs");
            None
        }
    };

    // Kept alive until GPUI's loop returns, so network tasks outlive every view.
    let network_runtime = match build_network_runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!("{error:#}");
            return ExitCode::FAILURE;
        }
    };
    let network_handle = network_runtime.handle().clone();

    let application = gpui_platform::application().with_assets(EmbeddedAssets);
    // macOS: the Dock icon clicked, or the app opened again, with no window open.
    application.on_reopen(mellow_desktop::background::show_main_window);
    application.run(move |cx| {
        if let Err(error) = mellow_desktop::start(network_handle, show_requests, cx) {
            tracing::error!("{error:#}");
            // GPUI's `run` never returns on macOS, so the failure status is set here.
            std::process::exit(1);
        }
    });
    // Where `run` returns (Linux), dropping the runtime would wait without limit for a blocking
    // secret-store call stuck on a prompt nobody sees; the quit already waited for sign-out work.
    network_runtime.shutdown_timeout(LOGOUT_ON_EXIT_TIMEOUT);
    ExitCode::SUCCESS
}

/// A Windows app without a console prints nowhere: when started from a terminal without its
/// output redirected, write to that terminal instead.
#[cfg(windows)]
#[allow(unsafe_code)]
fn attach_to_the_parent_console() {
    use windows::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE,
    };
    // SAFETY: both calls take plain values and only read or change this process's console.
    unsafe {
        // A redirected output (a file, a pipe) is kept; null or invalid means there is none.
        if GetStdHandle(STD_OUTPUT_HANDLE).is_err() {
            // Started from Explorer: no terminal to attach to, and nothing is printed, as before.
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

#[cfg(not(windows))]
fn attach_to_the_parent_console() {}
