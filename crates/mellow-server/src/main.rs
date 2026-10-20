//! `mellow-server`: one binary to initialise, administer and run a Mellow instance.

use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand, ValueEnum};
use mellow_server::commands::{
    CreateUserOptions, InitializeOptions, create_user, initialize_instance, serve_instance,
    shutdown_signal,
};
use mellow_server::config::{
    DEFAULT_CERTIFICATE_DIRECTORY_NAME, DEFAULT_LISTEN_ADDRESS, DEFAULT_TLS_LISTEN_ADDRESS,
    EnvironmentOverrides, ServerConfig, TlsConfig,
};
use mellow_server::domain::account_rules::RegistrationMode;
use tracing_subscriber::EnvFilter;
use zeroize::Zeroizing;

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
        Command::Admin {
            command:
                AdminCommand::CreateUser {
                    config,
                    username,
                    email,
                    display_name,
                    administrator,
                },
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
    }
}
