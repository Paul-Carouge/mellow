//! `mellow`, the desktop client.

// Upstream cause: macOS ld warns once a debug binary holds more than 16 MB of `__eh_frame`, which
// its compact unwind table cannot encode; with the voice engine (str0m, WebRTC's processing,
// libvpx) the app crosses it. Only the speed of unwinding a panic is affected.
#![cfg_attr(target_os = "macos", allow(linker_messages))]

use std::process::ExitCode;

use mellow_desktop::components::gallery::gallery_requested;
use mellow_desktop::session::LOGOUT_ON_EXIT_TIMEOUT;
use mellow_desktop::single_instance::{InstanceClaim, claim_single_instance};
use mellow_desktop::theme::EmbeddedAssets;
use mellow_desktop::tokio_bridge::build_network_runtime;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    // A bad MELLOW_PROFILE ends the launch here, before any thread could read the everyday one.
    let _ = mellow_desktop::profile::active_profile_name();

    // Before anything else: a second launch only shows the running Mellow's window. The component
    // gallery opens next to a running Mellow.
    let claim = gallery_requested().is_none().then(claim_single_instance);
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
