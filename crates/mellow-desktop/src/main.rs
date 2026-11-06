//! `mellow`, the desktop client.

use std::process::ExitCode;

use mellow_desktop::theme::EmbeddedAssets;
use mellow_desktop::tokio_bridge::build_network_runtime;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    // Kept alive until GPUI's loop returns, so network tasks outlive every view.
    let network_runtime = match build_network_runtime() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!("{error:#}");
            return ExitCode::FAILURE;
        }
    };
    let network_handle = network_runtime.handle().clone();

    gpui_platform::application()
        .with_assets(EmbeddedAssets)
        .run(move |cx| {
            if let Err(error) = mellow_desktop::start(network_handle, cx) {
                tracing::error!("{error:#}");
                // GPUI's `run` never returns on macOS, so the failure status is set here.
                std::process::exit(1);
            }
        });
    ExitCode::SUCCESS
}
