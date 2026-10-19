//! `mellow-server`: one binary to initialise and run a Mellow instance.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use mellow_server::commands::{
    InitializeOptions, initialize_instance, serve_instance, shutdown_signal,
};
use mellow_server::config::{DEFAULT_LISTEN_ADDRESS, EnvironmentOverrides, ServerConfig};
use tracing_subscriber::EnvFilter;

const DEFAULT_CONFIG_PATH: &str = "mellow.toml";
const DEFAULT_LOG_FILTER: &str = "info";

#[derive(Debug, Parser)]
#[command(version, about = "Mellow server: self-hosted chat in one binary")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create the master key, the encrypted database and the configuration file.
    Init {
        #[arg(long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        /// Where the database, attachments and backups live.
        #[arg(long)]
        data_directory: PathBuf,
        /// Must be outside the data directory; back it up separately, it cannot be recovered.
        #[arg(long)]
        master_key_file: PathBuf,
        #[arg(long, default_value = DEFAULT_LISTEN_ADDRESS)]
        listen_address: SocketAddr,
    },
    /// Run the server until Ctrl+C or SIGTERM.
    Serve {
        #[arg(long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
    },
}

fn main() -> ExitCode {
    // Logs never contain message content, tokens, passwords or emails (CLAUDE.md).
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER)),
        )
        .init();
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("error: cannot start the async runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(Cli::parse())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Init {
            config,
            data_directory,
            master_key_file,
            listen_address,
        } => {
            let created = initialize_instance(&InitializeOptions {
                config_path: config.clone(),
                data_directory,
                master_key_file,
                listen_address,
            })
            .await?;
            eprintln!(
                "Instance created.\n  config:     {}\n  data:       {}\n  master key: {}\n\n\
                 Back up the master key file now and keep it apart from the data directory:\n\
                 without it the database cannot be decrypted.\n\
                 Start the server with: mellow-server serve --config {}",
                config.display(),
                created.data_directory.display(),
                created.master_key_file.display(),
                config.display(),
            );
            Ok(())
        }
        Command::Serve { config } => {
            let config =
                ServerConfig::load(&config, &EnvironmentOverrides::from_process_environment())?;
            serve_instance(&config, shutdown_signal()).await
        }
    }
}
