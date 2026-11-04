//! `mellow-server`: one binary to initialise, administer and run a Mellow instance.

// A command-line program: its messages to the operator go to stderr.
#![allow(clippy::print_stderr)]

use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand, ValueEnum};
use mellow_server::commands::{
    CreateUserOptions, InitializeOptions, back_up_instance, create_user, initialize_instance,
    pictures_to_recompress, request_image_recompression, serve_instance,
    set_instance_administrator, shutdown_signal,
};
use mellow_server::config::{
    DEFAULT_CERTIFICATE_DIRECTORY_NAME, DEFAULT_LISTEN_ADDRESS, DEFAULT_TLS_LISTEN_ADDRESS,
    EnvironmentOverrides, ServerConfig, TlsConfig,
};
use mellow_server::domain::account_rules::RegistrationMode;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer as _};
use zeroize::Zeroizing;

const DEFAULT_CONFIG_PATH: &str = "mellow.toml";
const DEFAULT_LOG_FILTER: &str = "info";
/// The WebRTC stack (str0m and its own crates) logs addresses and SDP at debug level, and warns
/// once per malformed packet (a flood under attack): capped at error whatever `RUST_LOG` says,
/// since logs never carry IP addresses (ADR-0030).
const WEBRTC_LOG_TARGETS: [&str; 6] = [
    "str0m",
    "str0m_proto",
    "str0m_aws_lc_rs",
    "is",
    "dimpl",
    "sctp_proto",
];

/// The outbound HTTP client of link previews (ADR-0041): its debug lines name the hosts and
/// addresses of links posted in conversations, which logs never carry.
/// Only `hyper_util` logs connections through `tracing` (reqwest and rustls use `log`, which is
/// not bridged); capping `rustls` would also cap `rustls_acme`, whose renewal lines matter.
const LINK_PREVIEW_CLIENT_LOG_TARGETS: [&str; 1] = ["hyper_util"];

/// A second, per-layer filter: unlike directives added to the `EnvFilter`, `RUST_LOG` cannot
/// override it.
fn webrtc_log_cap() -> Targets {
    Targets::new()
        .with_default(LevelFilter::TRACE)
        .with_targets(
            WEBRTC_LOG_TARGETS
                .iter()
                .chain(LINK_PREVIEW_CLIENT_LOG_TARGETS.iter())
                .map(|target| (*target, LevelFilter::ERROR)),
        )
}

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
        /// Defaults to 127.0.0.1:8080, or [::]:443 with --domain.
        #[arg(long)]
        listen_address: Option<SocketAddr>,
        /// Who may create an account; the administrator can always create accounts.
        #[arg(long, value_enum, default_value_t = RegistrationModeArgument::Closed)]
        registration_mode: RegistrationModeArgument,
        /// Public name of the instance; enables built-in TLS with Let's Encrypt certificates.
        /// Repeat for several names. DNS must point here and port 443 must be reachable.
        #[arg(long = "domain")]
        domains: Vec<String>,
        /// Given to Let's Encrypt for account problems.
        #[arg(long, requires = "domains")]
        contact_email: Option<String>,
        /// Holds the TLS private keys; defaults to `certificates` next to the master key file.
        #[arg(long, requires = "domains")]
        certificate_directory: Option<PathBuf>,
        /// Use Let's Encrypt staging (untrusted certificates) to try a setup.
        #[arg(long, requires = "domains")]
        use_staging_certificates: bool,
    },
    /// Run the server until Ctrl+C or SIGTERM.
    Serve {
        #[arg(long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
    },
    /// Install the latest signed release in place of this executable (ADR-0003); the previous
    /// one is kept for `--rollback`. Restart the service afterwards.
    Update {
        /// Only print the installed and latest versions.
        #[arg(long, conflicts_with = "rollback")]
        check: bool,
        /// Put the executable replaced by the last update back. The database is not touched:
        /// after a migration, restore its pre-migration backup from `backups/` as well.
        #[arg(long)]
        rollback: bool,
    },
    /// Copy the database and the attachment files into a new directory while the server runs;
    /// restoring it needs the master key file too, which is not copied.
    Backup {
        #[arg(long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        /// Where the backup directory is created; defaults to `backups/` in the data directory.
        /// Move the backup to another disk afterwards.
        #[arg(long)]
        output_directory: Option<PathBuf>,
    },
    /// Instance administration.
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
}

#[derive(Debug, Subcommand)]
enum AdminCommand {
    /// Create an account, e.g. the first administrator. Asks for the password (not echoed),
    /// or reads one line from standard input when it is not a terminal.
    CreateUser {
        #[arg(long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        #[arg(long)]
        username: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        display_name: Option<String>,
        /// Give the account access to instance administration.
        #[arg(long)]
        administrator: bool,
    },
    /// Make an existing account an instance administrator (who may create servers when
    /// `guild_creation = "administrators"`), or take it away with `--revoke`.
    SetAdministrator {
        #[arg(long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        #[arg(long)]
        username: String,
        #[arg(long)]
        revoke: bool,
    },
    /// Store the pictures kept before visually lossless compression (ADR-0033) again, smaller:
    /// avatars, banners, guild and channel icons and picture attachments. No original is kept.
    #[command(group = clap::ArgGroup::new("mode").required(true))]
    RecompressImages {
        #[arg(long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        /// Only count the pictures and estimate the space saved.
        #[arg(long, group = "mode")]
        dry_run: bool,
        /// Ask the server to recompress them, one at a time in the background. Needs a backup
        /// made with `mellow-server backup` in the last hour.
        #[arg(long, group = "mode")]
        confirm: bool,
    },
}

/// Kept in the binary so the domain does not depend on clap.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum RegistrationModeArgument {
    Open,
    Closed,
}

impl From<RegistrationModeArgument> for RegistrationMode {
    fn from(argument: RegistrationModeArgument) -> Self {
        match argument {
            RegistrationModeArgument::Open => Self::Open,
            RegistrationModeArgument::Closed => Self::Closed,
        }
    }
}

fn main() -> ExitCode {
    // Logs never contain message content, tokens, passwords or emails (CLAUDE.md).
    let requested_levels =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_LOG_FILTER));
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_filter(requested_levels)
                .with_filter(webrtc_log_cap()),
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

fn load_config(path: &std::path::Path) -> Result<ServerConfig> {
    Ok(ServerConfig::load(
        path,
        &EnvironmentOverrides::from_process_environment(),
    )?)
}

fn read_password() -> Result<Zeroizing<String>> {
    if std::io::stdin().is_terminal() {
        return Ok(Zeroizing::new(
            rpassword::prompt_password("Password: ").context("cannot read the password")?,
        ));
    }
    let mut line = Zeroizing::new(String::new());
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("cannot read the password from standard input")?;
    Ok(Zeroizing::new(
        line.trim_end_matches(['\r', '\n']).to_owned(),
    ))
}

/// The usual cause of a refused replacement is a root-owned install run without sudo.
fn with_sudo_hint(error: mellow_update::UpdateError) -> anyhow::Error {
    let is_permission_denied = matches!(
        &error,
        mellow_update::UpdateError::ReplaceExecutable(source)
            if source.kind() == std::io::ErrorKind::PermissionDenied
    );
    let error = anyhow::Error::new(error);
    if is_permission_denied {
        error.context("permission denied: run it with sudo, as the owner of the executable")
    } else {
        error
    }
}

async fn update_executable(check: bool, rollback: bool) -> Result<()> {
    let is_in_container = ["/.dockerenv", "/run/.containerenv"]
        .iter()
        .any(|marker| std::path::Path::new(marker).exists());
    if is_in_container {
        anyhow::bail!(
            "this server runs in a container, whose files do not survive a restart: update the \
             image instead (deploy/docker/mellow-update)"
        );
    }
    let executable = std::env::current_exe().context("cannot locate this executable")?;
    if rollback {
        mellow_update::roll_back_executable(&executable).map_err(with_sudo_hint)?;
        eprintln!(
            "Rolled back {}. Restart the service to run the previous version.",
            executable.display()
        );
        return Ok(());
    }
    let client = mellow_update::UpdateClient::new()?;
    let release = client.fetch_latest_release().await?;
    let current = mellow_update::current_version();
    if check {
        // The answer itself, on standard output for scripts.
        writeln!(
            std::io::stdout(),
            "installed: {current}\nlatest:    {}",
            release.version
        )
        .context("cannot write to standard output")?;
        return Ok(());
    }
    if !mellow_update::is_newer_than_current(&release) {
        eprintln!("Mellow {current} is up to date.");
        return Ok(());
    }
    eprintln!("Downloading Mellow {}…", release.version);
    let contents = client
        .download_verified_executable(&release, mellow_update::Program::Server)
        .await?;
    mellow_update::install_executable(&contents, &executable).map_err(with_sudo_hint)?;
    eprintln!(
        "Installed Mellow {} (signature verified) in {}; the previous executable is {}.\n\
         Restart the service to run it, e.g. `sudo systemctl restart mellow`. The database is \
         backed up before any migration; `mellow-server update --rollback` puts the previous \
         executable back.",
        release.version,
        executable.display(),
        mellow_update::previous_executable_path(&executable).display(),
    );
    Ok(())
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Init {
            config,
            data_directory,
            master_key_file,
            listen_address,
            registration_mode,
            domains,
            contact_email,
            certificate_directory,
            use_staging_certificates,
        } => {
            let tls = (!domains.is_empty()).then(|| TlsConfig {
                domains,
                contact_email,
                certificate_directory: certificate_directory.unwrap_or_else(|| {
                    master_key_file
                        .parent()
                        .unwrap_or(std::path::Path::new(""))
                        .join(DEFAULT_CERTIFICATE_DIRECTORY_NAME)
                }),
                use_staging_certificates,
            });
            let default_listen_address = if tls.is_some() {
                DEFAULT_TLS_LISTEN_ADDRESS
            } else {
                DEFAULT_LISTEN_ADDRESS
            };
            let listen_address = match listen_address {
                Some(address) => address,
                None => default_listen_address
                    .parse()
                    .context("invalid built-in default listen address")?,
            };
            let created = initialize_instance(&InitializeOptions {
                config_path: config.clone(),
                data_directory,
                master_key_file,
                listen_address,
                registration_mode: registration_mode.into(),
                tls,
            })
            .await?;
            eprintln!(
                "Instance created.\n  config:     {}\n  data:       {}\n  master key: {}\n\n\
                 Back up the master key file now and keep it apart from the data directory:\n\
                 without it the database cannot be decrypted.\n\
                 Create the first administrator with:\n  \
                 mellow-server admin create-user --config {} --username <name> --email <address> --administrator\n\
                 Then start the server with: mellow-server serve --config {}",
                config.display(),
                created.data_directory.display(),
                created.master_key_file.display(),
                config.display(),
                config.display(),
            );
            Ok(())
        }
        Command::Serve { config } => {
            serve_instance(&load_config(&config)?, shutdown_signal()).await
        }
        Command::Update { check, rollback } => update_executable(check, rollback).await,
        Command::Backup {
            config,
            output_directory,
        } => {
            let backup =
                back_up_instance(&load_config(&config)?, output_directory.as_deref()).await?;
            eprintln!(
                "Backed up to {}.\nMove it to another disk, and keep a copy of the master key file \
                 apart from it: the backup cannot be decrypted without it.",
                backup.display()
            );
            Ok(())
        }
        Command::Admin { command } => run_admin_command(command).await,
    }
}

async fn run_admin_command(command: AdminCommand) -> Result<()> {
    match command {
        AdminCommand::CreateUser {
            config,
            username,
            email,
            display_name,
            administrator,
        } => {
            let config = load_config(&config)?;
            let user = create_user(
                &config,
                CreateUserOptions {
                    username,
                    display_name,
                    email,
                    password: read_password()?,
                    is_instance_administrator: administrator,
                },
            )
            .await?;
            eprintln!(
                "Created {} ({}){}.",
                user.username,
                user.id,
                if user.is_instance_administrator {
                    ", instance administrator"
                } else {
                    ""
                }
            );
            Ok(())
        }
        AdminCommand::SetAdministrator {
            config,
            username,
            revoke,
        } => {
            set_instance_administrator(&load_config(&config)?, &username, !revoke).await?;
            eprintln!(
                "{username} is {} an instance administrator.",
                if revoke { "no longer" } else { "now" }
            );
            Ok(())
        }
        AdminCommand::RecompressImages {
            config,
            dry_run,
            confirm: _,
        } => {
            let config = load_config(&config)?;
            if dry_run {
                print_pictures_to_recompress(&pictures_to_recompress(&config).await?)
            } else {
                let waiting = request_image_recompression(&config).await?;
                if waiting == 0 {
                    eprintln!("No picture is left to recompress.");
                } else {
                    eprintln!(
                        "Recompression of {waiting} pictures requested. A running server starts \
                         within a minute, one picture at a time after new uploads; otherwise it \
                         starts with the server. Follow it with --dry-run."
                    );
                }
                Ok(())
            }
        }
    }
}

const BYTES_PER_MEBIBYTE: f64 = 1_048_576.0;

/// The dry run's answer, on standard output for scripts.
fn print_pictures_to_recompress(
    kinds: &[mellow_server::commands::WaitingPictureKind],
) -> Result<()> {
    #[allow(clippy::cast_precision_loss)] // Shown with one decimal: the loss is invisible.
    let mebibytes = |bytes: u64| bytes as f64 / BYTES_PER_MEBIBYTE;
    let mut output = std::io::stdout().lock();
    writeln!(output, "pictures stored before ADR-0033:")?;
    for kind in kinds {
        writeln!(
            output,
            "  {}: {} ({:.1} MiB), about {:.1} MiB saved",
            kind.name,
            kind.count,
            mebibytes(kind.stored_bytes),
            mebibytes(kind.expected_saved_bytes)
        )?;
    }
    writeln!(
        output,
        "Savings are estimated from ADR-0033's measurements; custom emoji are left as they are."
    )
    .context("cannot write to standard output")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webrtc_crates_log_errors_only_and_others_are_left_to_rust_log() {
        let cap = webrtc_log_cap();
        for target in [
            "str0m",
            "str0m::rtp::srtp",
            "is::agent",
            "dimpl",
            "sctp_proto",
        ] {
            assert!(!cap.would_enable(target, &tracing::Level::WARN), "{target}");
            assert!(cap.would_enable(target, &tracing::Level::ERROR), "{target}");
        }
        assert!(cap.would_enable("mellow_server::voice", &tracing::Level::TRACE));
    }
}
