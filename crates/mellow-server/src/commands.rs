//! `mellow-server init` and `mellow-server serve` (ARCHITECTURE.md §4.10).

use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use tokio::net::TcpListener;

use crate::config::{ServerConfig, TlsConfig};
use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::purge_expired_sessions_periodically;
use crate::http::AppState;
use crate::storage::{
    MasterKey, create_private_directory, migrate_with_backup, open_encrypted_database,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitializeOptions {
    pub config_path: PathBuf,
    pub data_directory: PathBuf,
    pub master_key_file: PathBuf,
    pub listen_address: SocketAddr,
    pub registration_mode: RegistrationMode,
    /// Built-in TLS (ADR-0013); `None` for plain HTTP behind a reverse proxy or on loopback.
    pub tls: Option<TlsConfig>,
}

/// Creates the master key, the data directory with a migrated database, and the config file.
///
/// # Errors
/// Refuses to touch an existing config or master key, so re-running `init` can never destroy an instance.
pub async fn initialize_instance(options: &InitializeOptions) -> Result<ServerConfig> {
    let config = ServerConfig {
        data_directory: std::path::absolute(&options.data_directory)
            .context("cannot resolve the data directory path")?,
        master_key_file: std::path::absolute(&options.master_key_file)
            .context("cannot resolve the master key file path")?,
        listen_address: options.listen_address,
        registration_mode: options.registration_mode,
        guild_creation: crate::domain::account_rules::GuildCreationPolicy::default(),
        behind_tls_reverse_proxy: false,
        trusted_proxy_networks: Vec::new(),
        maximum_attachment_bytes: crate::config::DEFAULT_MAXIMUM_ATTACHMENT_BYTES,
        maximum_attachment_storage_bytes_per_user:
            crate::config::DEFAULT_MAXIMUM_ATTACHMENT_STORAGE_BYTES_PER_USER,
        voice: crate::config::VoiceConfig::default(),
        link_previews: crate::config::LinkPreviewsConfig::default(),
        gifs: crate::config::GifsConfig::default(),
        tls: options
            .tls
            .clone()
            .map(|tls| -> Result<TlsConfig> {
                Ok(TlsConfig {
                    certificate_directory: std::path::absolute(&tls.certificate_directory)
                        .context("cannot resolve the certificate directory path")?,
                    ..tls
                })
            })
            .transpose()?,
    };
    // Every check that can fail runs before the key is written, so a failed `init` leaves
    // nothing behind and can simply be run again.
    config.validate()?;
    let config_text = config.to_toml()?;
    for existing_path in [
        &options.config_path,
        &config.master_key_file,
        &config.database_path(),
    ] {
        if existing_path.exists() {
            bail!(
                "{} already exists; `init` never overwrites an instance",
                existing_path.display()
            );
        }
    }
    if let Some(config_directory) = options
        .config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(config_directory).with_context(|| {
            format!(
                "cannot create the configuration directory {}",
                config_directory.display()
            )
        })?;
    }
    create_private_directory(&config.data_directory).with_context(|| {
        format!(
            "cannot create the data directory {}",
            config.data_directory.display()
        )
    })?;
    if let Some(key_directory) = config.master_key_file.parent() {
        create_private_directory(key_directory).with_context(|| {
            format!(
                "cannot create the master key directory {}",
                key_directory.display()
            )
        })?;
    }
    config.ensure_master_key_outside_data_directory_on_disk()?;
    if let Some(tls) = &config.tls {
        crate::tls::prepare_certificate_directory(
            &tls.certificate_directory,
            &config.data_directory,
        )?;
    }

    let database_key = {
        let master_key = MasterKey::generate()?;
        master_key.write_new_file(&config.master_key_file)?;
        master_key.derive_database_key()
    };
    let database = open_encrypted_database(&config.database_path(), &database_key).await?;
    let migration = migrate_with_backup(&database, &config.backup_directory()).await;
    database.close().await;
    migration?;

    write_new_file(&options.config_path, &config_text)?;
    Ok(config)
}

fn write_new_file(path: &Path, content: &str) -> Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("cannot create {}", path.display()))?;
    file.write_all(content.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))
}

/// Starts the background link preview fetcher when previews are on (ADR-0041).
fn start_link_previews(
    config: &ServerConfig,
    state: &AppState,
) -> Result<Option<tokio::task::JoinHandle<()>>> {
    if !config.link_previews.enabled {
        return Ok(None);
    }
    let fetcher = crate::domain::link_previews::fetcher::LinkFetcher::new(&format!(
        "Mellow/{} (link preview; +https://github.com/Paul-Carouge/mellow)",
        env!("CARGO_PKG_VERSION")
    ))
    .context("cannot build the link preview client")?;
    state.messages.enable_link_previews();
    Ok(Some(tokio::spawn(
        crate::domain::messages::fetch_link_previews_in_background(
            state.messages.clone(),
            state.attachments.clone(),
            fetcher,
        ),
    )))
}

/// Opens the database, applies migrations, then serves HTTP until `shutdown` completes.
///
/// # Errors
/// Fails on a missing database or master key, a wrong key, a failed migration, or a port
/// already in use.
pub async fn serve_instance(
    config: &ServerConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    config.ensure_master_key_outside_data_directory_on_disk()?;
    let database_path = config.database_path();
    // Without this check SQLite would silently create an empty instance (e.g. after a typo in
    // the data directory), and clients would see everything gone.
    if !database_path.is_file() {
        bail!(
            "no database at {}; run `mellow-server init` first or fix data_directory",
            database_path.display()
        );
    }
    if let Some(tls) = &config.tls {
        crate::tls::prepare_certificate_directory(
            &tls.certificate_directory,
            &config.data_directory,
        )?;
    }
    // The master key is dropped (and wiped) as soon as the database and attachment keys are
    // derived.
    let (database_key, attachment_key, gif_customer_key) = {
        let master_key = MasterKey::read_file(&config.master_key_file)?;
        (
            master_key.derive_database_key(),
            master_key.derive_attachment_key(),
            master_key.derive_gif_customer_key(),
        )
    };
    let attachment_files = crate::storage::attachment_files::AttachmentFiles::new(
        config.attachment_directory(),
        &attachment_key,
    )?;
    // Bound before migrating: a second `serve` started by mistake fails here instead of
    // migrating the database under the running instance.
    let listener = TcpListener::bind(config.listen_address)
        .await
        .with_context(|| format!("cannot listen on {}", config.listen_address))?;
    let database = open_encrypted_database(&database_path, &database_key).await?;

    let result = async {
        let migration = migrate_with_backup(&database, &config.backup_directory()).await?;
        if !migration.applied_versions.is_empty() {
            tracing::info!(
                applied_versions = ?migration.applied_versions,
                backup_path = ?migration.backup_path,
                "database migrated"
            );
        }
        let moved_files = attachment_files.move_files_to_their_shards().await?;
        if moved_files.moved > 0 || moved_files.failed > 0 {
            tracing::info!(
                moved = moved_files.moved,
                failed = moved_files.failed,
                "attachment files moved to their shards"
            );
        }
        let mut state = AppState::new(database.clone(), config, attachment_files);
        if let Some(klipy_app_key) = &config.gifs.klipy_app_key {
            state = state.with_gifs(crate::domain::gifs::Gifs::new(
                klipy_app_key.clone(),
                gif_customer_key,
            ));
        }
        let session_purge = tokio::spawn(purge_expired_sessions_periodically(database.clone()));
        let attachment_purge = tokio::spawn(
            crate::domain::attachments::purge_attachments_periodically(state.attachments.clone()),
        );
        let attachment_optimizer = tokio::spawn(
            crate::domain::attachments::optimize_attachments_in_background(
                state.attachments.clone(),
                crate::domain::image_recompression::ImageRecompression::new(
                    database.clone(),
                    state.attachments.clone(),
                    state.profile_images.clone(),
                    state.guilds.clone(),
                ),
            ),
        );
        let link_preview_fetcher = start_link_previews(config, &state)?;
        let served = match &config.tls {
            Some(tls) => crate::tls::serve_tls_until_shutdown(listener, state, tls, shutdown).await,
            None => serve_until_shutdown(listener, state, shutdown).await,
        };
        // Awaited, so the purge holds no connection when the database closes.
        session_purge.abort();
        let _cancelled = session_purge.await;
        attachment_purge.abort();
        let _cancelled = attachment_purge.await;
        attachment_optimizer.abort();
        let _cancelled = attachment_optimizer.await;
        if let Some(link_preview_fetcher) = link_preview_fetcher {
            link_preview_fetcher.abort();
            let _cancelled = link_preview_fetcher.await;
        }
        served
    }
    .await;

    // SQLCipher crashes if a connection is still closing when the process exits (ADR-0006).
    database.close().await;
    result
}

pub struct CreateUserOptions {
    pub username: String,
    pub display_name: Option<String>,
    pub email: String,
    pub password: zeroize::Zeroizing<String>,
    pub is_instance_administrator: bool,
}

/// The password is never printed (logs must not contain secrets).
impl std::fmt::Debug for CreateUserOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CreateUserOptions")
            .field("username", &self.username)
            .field("display_name", &self.display_name)
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .field("is_instance_administrator", &self.is_instance_administrator)
            .finish()
    }
}

/// `mellow-server admin create-user`: works whatever the registration mode, so the first
/// administrator can be created on a closed instance. Safe while `serve` runs (SQLite WAL).
///
/// # Errors
/// Fails on a missing database or key, invalid input, or a taken username or email.
pub async fn create_user(
    config: &ServerConfig,
    options: CreateUserOptions,
) -> Result<mellow_protocol::CurrentUser> {
    let database = open_instance_database(config).await?;
    let result = async {
        Ok(crate::domain::accounts::Accounts::new(
            database.clone(),
            config.registration_mode,
            std::sync::Arc::default(),
            std::sync::Arc::default(),
        )
        .create_account(
            crate::domain::accounts::NewAccount {
                username: options.username,
                display_name: options.display_name,
                email: options.email,
                password: options.password,
                is_instance_administrator: options.is_instance_administrator,
            },
            crate::domain::clock::now_unix_milliseconds(),
        )
        .await?)
    }
    .await;
    database.close().await;
    result
}

/// `mellow-server admin set-administrator`: gives an existing account instance administration, or
/// takes it away. Applies to the account's next request; clients learn it at their next `Ready`.
/// Safe while `serve` runs (SQLite WAL).
///
/// # Errors
/// Fails on a missing database or key, or an unknown username.
pub async fn set_instance_administrator(
    config: &ServerConfig,
    username: &str,
    is_instance_administrator: bool,
) -> Result<()> {
    let database = open_instance_database(config).await?;
    let result = async {
        let exists = crate::storage::accounts::set_instance_administrator(
            &database,
            &crate::domain::account_names::account_name_key(username),
            is_instance_administrator,
        )
        .await?;
        if !exists {
            bail!("no account is named {username}");
        }
        Ok(())
    }
    .await;
    database.close().await;
    result
}

/// `mellow-server admin reset-two-factor`: turns off an account's two-factor sign-in for a user who
/// lost both their device and their recovery codes (ADR-0049). The reset is recorded in
/// `two_factor_resets`, in the same transaction. Returns whether two-factor sign-in was on. Safe
/// while `serve` runs (SQLite WAL); sign-ins waiting for a code then fail and start again.
///
/// # Errors
/// Fails on a missing database or key, or an unknown username.
pub async fn reset_two_factor(config: &ServerConfig, username: &str) -> Result<bool> {
    use crate::storage::two_factor::{TwoFactorReset, reset_two_factor_by_username};
    let database = open_instance_database(config).await?;
    let result = async {
        let now = crate::domain::clock::now_unix_milliseconds();
        let reset_id =
            crate::domain::snowflake_generator::SnowflakeGenerator::default().next_id(now)?;
        match reset_two_factor_by_username(&database, username, reset_id, now).await? {
            TwoFactorReset::UnknownUser => bail!("no account is named {username}"),
            TwoFactorReset::WasNotEnabled => Ok(false),
            TwoFactorReset::Reset => Ok(true),
        }
    }
    .await;
    database.close().await;
    result
}

/// Opens the instance database for an administration command that writes, migrated, after
/// checking it exists so a typo in the data directory never creates an empty instance.
async fn open_instance_database(config: &ServerConfig) -> Result<crate::storage::Database> {
    let database = open_existing_instance_database(config).await?;
    if let Err(error) = migrate_with_backup(&database, &config.backup_directory()).await {
        database.close().await;
        return Err(error.into());
    }
    Ok(database)
}

/// As `open_instance_database`, for a command that only reads or copies (`backup`, a dry run):
/// refuses a database this binary would migrate rather than migrating it under a server that may
/// still run the previous version.
async fn open_unmigrated_instance_database(
    config: &ServerConfig,
) -> Result<crate::storage::Database> {
    let database = open_existing_instance_database(config).await?;
    if let Err(error) = crate::storage::ensure_no_pending_migration(&database).await {
        database.close().await;
        return Err(error.into());
    }
    Ok(database)
}

async fn open_existing_instance_database(
    config: &ServerConfig,
) -> Result<crate::storage::Database> {
    let database_path = config.database_path();
    if !database_path.is_file() {
        bail!(
            "no database at {}; run `mellow-server init` first",
            database_path.display()
        );
    }
    let database_key = MasterKey::read_file(&config.master_key_file)?.derive_database_key();
    Ok(open_encrypted_database(&database_path, &database_key).await?)
}

/// Suffix of a backup being written; renamed away once the backup is complete and checked.
const SUFFIX_OF_PARTIAL_BACKUPS: &str = ".partial";

/// `mellow-server backup`: a consistent copy of the database and of the attachment files (avatars,
/// icons and sounds included) in a new directory laid out like the data directory, made while
/// `serve` runs. The master key is not in it: back it up separately (ADR-0006).
///
/// The database is copied first, then the files. A file is never changed in place (every version
/// has its own id), so a file the copied database points at can only be missing when it was
/// deleted after the database was copied; the backup is then refused, to be run again.
///
/// # Errors
/// Fails on a missing database or key, an existing destination, any I/O error, or a file the
/// copied database points at that is not in the copy. Nothing but a complete backup is left.
pub async fn back_up_instance(
    config: &ServerConfig,
    output_directory: Option<&Path>,
) -> Result<PathBuf> {
    back_up_instance_with_clock(
        config,
        output_directory,
        crate::domain::clock::now_unix_milliseconds,
    )
    .await
}

/// `back_up_instance`, reading the time from `now_unix_milliseconds`: once to name the backup,
/// once more when it is complete.
async fn back_up_instance_with_clock(
    config: &ServerConfig,
    output_directory: Option<&Path>,
    now_unix_milliseconds: impl Fn() -> u64,
) -> Result<PathBuf> {
    let database = open_unmigrated_instance_database(config).await?;
    let result = async {
        let started_at_unix_milliseconds = now_unix_milliseconds();
        let destination = output_directory
            .map_or_else(|| config.backup_directory(), Path::to_path_buf)
            .join(format!("backup-{started_at_unix_milliseconds}"));
        let partial = PathBuf::from(format!(
            "{}{SUFFIX_OF_PARTIAL_BACKUPS}",
            destination.display()
        ));
        if destination.exists() || partial.exists() {
            bail!("{} already exists", destination.display());
        }
        create_private_directory(&partial)
            .with_context(|| format!("cannot create {}", partial.display()))?;
        let written = write_backup(config, &database, &partial).await;
        if let Err(error) = written {
            let _ = std::fs::remove_dir_all(&partial);
            return Err(error);
        }
        std::fs::rename(&partial, &destination)
            .with_context(|| format!("cannot rename {}", partial.display()))?;
        // The end, not the start: a backup that took hours still holds every picture stored
        // before ADR-0033, which never change in place.
        crate::storage::backups::record_completed_backup(&database, now_unix_milliseconds())
            .await?;
        Ok(destination)
    }
    .await;
    database.close().await;
    result
}

/// Writes the database copy and the files into `destination`, then checks the copy is whole.
async fn write_backup(
    config: &ServerConfig,
    database: &crate::storage::Database,
    destination: &Path,
) -> Result<()> {
    let database_copy = destination.join(crate::config::DATABASE_FILE_NAME);
    database
        .write_snapshot(&database_copy)
        .await
        .context("cannot copy the database")?;
    let attachment_source = config.attachment_directory();
    let attachment_copy = destination.join(crate::config::ATTACHMENT_DIRECTORY_NAME);
    let copy_destination = attachment_copy.clone();
    tokio::task::spawn_blocking(move || {
        copy_attachment_files(&attachment_source, &copy_destination)
    })
    .await??;

    let master_key = MasterKey::read_file(&config.master_key_file)?;
    let copied_files = crate::storage::attachment_files::AttachmentFiles::new(
        attachment_copy,
        &master_key.derive_attachment_key(),
    )?;
    let copied_database =
        open_encrypted_database(&database_copy, &master_key.derive_database_key()).await?;
    let file_ids = crate::domain::attachments::file_ids_in_use(&copied_database).await;
    copied_database.close().await;
    let missing_files = file_ids?
        .into_iter()
        .filter(|id| copied_files.existing_path_of(*id).is_none())
        .count();
    if missing_files > 0 {
        bail!(
            "{missing_files} files were deleted while they were being copied; run the backup again"
        );
    }
    Ok(())
}

/// Copies the sealed files as they are (they stay encrypted), without the temporary files of
/// writes under way. No attachment directory means no file was ever stored.
fn copy_attachment_files(source: &Path, destination: &Path) -> Result<()> {
    create_private_directory(destination)
        .with_context(|| format!("cannot create {}", destination.display()))?;
    let shards = match std::fs::read_dir(source) {
        Ok(shards) => shards,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read {}", source.display()));
        }
    };
    for shard in shards {
        let shard = shard?;
        if !shard.file_type()?.is_dir() {
            continue;
        }
        let shard_copy = destination.join(shard.file_name());
        create_private_directory(&shard_copy)?;
        for file in std::fs::read_dir(shard.path())? {
            let file = file?;
            let is_partial = file
                .file_name()
                .to_string_lossy()
                .ends_with(crate::storage::attachment_files::SUFFIX_OF_TEMPORARY_FILES);
            // Deleted since it was listed: the check after the copy tells whether it mattered.
            if !file.file_type()?.is_file() || is_partial {
                continue;
            }
            match std::fs::copy(file.path(), shard_copy.join(file.file_name())) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("cannot copy {}", file.path().display()));
                }
            }
        }
    }
    Ok(())
}

/// A backup older than this does not allow a recompression: it must hold what is recompressed.
const MAXIMUM_BACKUP_AGE_BEFORE_RECOMPRESSION_MILLISECONDS: u64 = 60 * 60 * 1_000;
/// Share of the bytes recompression saves, measured on ADR-0033's 25-picture corpus against what
/// the lossless pipeline before it stored: attachments 17.4 MiB instead of 32.9 MiB; photo
/// avatars a tenth of their 512 px lossless size and banners 2.7 MB down to 160 KB; a 256 px logo
/// icon 16 KB down to 4 KB. Measured at a score of 85: since the 2026-11-05 amendment pictures are
/// stored at 80, about 20 % smaller, so these forecasts are conservative. A forecast: the real
/// figure depends on the pictures.
const EXPECTED_SAVED_PERCENT_OF_ATTACHMENTS: u64 = 47;
const EXPECTED_SAVED_PERCENT_OF_PROFILE_IMAGES: u64 = 90;
const EXPECTED_SAVED_PERCENT_OF_ICONS: u64 = 75;

/// One kind of picture stored before ADR-0033, for `admin recompress-images --dry-run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitingPictureKind {
    pub name: &'static str,
    pub count: u64,
    pub stored_bytes: u64,
    pub expected_saved_bytes: u64,
}

/// What `admin recompress-images` would take: avatars and banners, guild and channel icons, and
/// picture attachments stored before ADR-0033. Custom emoji are not recompressed.
///
/// # Errors
/// Fails on a missing database or key, or a database error.
pub async fn pictures_to_recompress(config: &ServerConfig) -> Result<Vec<WaitingPictureKind>> {
    let database = open_unmigrated_instance_database(config).await?;
    let waiting = crate::storage::image_recompression::pictures_to_recompress(&database).await;
    database.close().await;
    let waiting = waiting?;
    let kind = |name, pictures: crate::storage::image_recompression::WaitingPictures, percent| {
        WaitingPictureKind {
            name,
            count: pictures.count,
            stored_bytes: pictures.stored_bytes,
            expected_saved_bytes: pictures.stored_bytes / 100 * percent,
        }
    };
    Ok(vec![
        kind(
            "avatars and banners",
            waiting.profile_images,
            EXPECTED_SAVED_PERCENT_OF_PROFILE_IMAGES,
        ),
        kind(
            "guild and channel icons",
            waiting.icons,
            EXPECTED_SAVED_PERCENT_OF_ICONS,
        ),
        kind(
            "picture attachments",
            waiting.attachments,
            EXPECTED_SAVED_PERCENT_OF_ATTACHMENTS,
        ),
    ])
}

/// `admin recompress-images --confirm`: asks the server to recompress the pictures stored before
/// ADR-0033, one at a time in the background (at its next start when it is not running). Answers
/// how many pictures wait; nothing is asked when none does.
///
/// # Errors
/// Refuses without a backup made by `mellow-server backup` in the last hour; fails on a missing
/// database or key, or a database error.
pub async fn request_image_recompression(config: &ServerConfig) -> Result<u64> {
    let database = open_instance_database(config).await?;
    let result = async {
        let now_unix_milliseconds = crate::domain::clock::now_unix_milliseconds();
        let latest_backup = crate::storage::backups::latest_completed_backup(&database).await?;
        let has_recent_backup = latest_backup.is_some_and(|completed| {
            now_unix_milliseconds.saturating_sub(completed)
                <= MAXIMUM_BACKUP_AGE_BEFORE_RECOMPRESSION_MILLISECONDS
        });
        if !has_recent_backup {
            bail!(
                "no backup was made in the last hour: run `mellow-server backup` first, \
                 recompression replaces the stored pictures and keeps no original"
            );
        }
        let waiting = crate::storage::image_recompression::pictures_to_recompress(&database)
            .await?
            .count();
        if waiting > 0 {
            crate::storage::image_recompression::request_recompression(
                &database,
                now_unix_milliseconds,
            )
            .await?;
        }
        Ok(waiting)
    }
    .await;
    database.close().await;
    result
}

/// Split from `serve_instance` so tests can bind an ephemeral port and read its address.
///
/// # Errors
/// Fails if the listener stops with an I/O error.
pub(crate) async fn serve_until_shutdown(
    listener: TcpListener,
    state: AppState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<()> {
    if let Ok(address) = listener.local_addr() {
        tracing::info!(%address, "listening");
    }
    crate::tls::serve_plain_until_shutdown(listener, state, shutdown).await
}

/// Completes on Ctrl+C, or on SIGTERM (what systemd sends) on Unix.
pub async fn shutdown_signal() {
    let interrupt = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(%error, "cannot listen for Ctrl+C");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
    tracing::info!("shutting down");
}

// Every test here creates a master key, which is refused off Unix for now (see master_key.rs).
#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;

    fn options_in(directory: &Path) -> InitializeOptions {
        InitializeOptions {
            config_path: directory.join("mellow.toml"),
            data_directory: directory.join("data"),
            master_key_file: directory.join("keys").join("master.key"),
            listen_address: "127.0.0.1:0".parse().unwrap(),
            registration_mode: RegistrationMode::Open,
            tls: None,
        }
    }

    #[tokio::test]
    async fn init_refuses_a_shared_certificate_directory_and_leaves_no_key_behind() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().unwrap();
        let shared = directory.path().join("shared-certificates");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
        let options = InitializeOptions {
            tls: Some(TlsConfig {
                domains: vec!["chat.example.org".into()],
                contact_email: None,
                certificate_directory: shared,
                use_staging_certificates: false,
            }),
            ..options_in(directory.path())
        };
        assert!(initialize_instance(&options).await.is_err());
        assert!(!options.master_key_file.exists());
        assert!(!options.config_path.exists());
    }

    #[tokio::test]
    async fn init_with_a_domain_writes_the_tls_table_and_a_private_certificate_directory() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().unwrap();
        let certificate_directory = directory.path().join("keys").join("certificates");
        let options = InitializeOptions {
            listen_address: "[::]:443".parse().unwrap(),
            tls: Some(TlsConfig {
                domains: vec!["chat.example.org".into()],
                contact_email: None,
                certificate_directory: certificate_directory.clone(),
                use_staging_certificates: true,
            }),
            ..options_in(directory.path())
        };
        let created = initialize_instance(&options).await.unwrap();

        let mode = std::fs::metadata(&certificate_directory)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        let loaded = ServerConfig::load(
            &options.config_path,
            &crate::config::EnvironmentOverrides::default(),
        )
        .unwrap();
        assert_eq!(loaded, created);
        assert_eq!(loaded.tls.unwrap().domains, ["chat.example.org"]);
    }

    #[tokio::test]
    async fn init_creates_a_migrated_encrypted_instance_and_refuses_to_run_twice() {
        let directory = tempfile::tempdir().unwrap();
        let options = options_in(directory.path());

        let config = initialize_instance(&options).await.unwrap();
        assert_eq!(
            ServerConfig::load(
                &options.config_path,
                &crate::config::EnvironmentOverrides::default()
            )
            .unwrap(),
            config
        );
        let key = MasterKey::read_file(&config.master_key_file).unwrap();
        let database = open_encrypted_database(&config.database_path(), &key.derive_database_key())
            .await
            .unwrap();
        let has_users_table: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE name = 'users')")
                .fetch_one(database.writer())
                .await
                .unwrap();
        database.close().await;
        assert!(has_users_table);

        let key_before = std::fs::read(&config.master_key_file).unwrap();
        assert!(initialize_instance(&options).await.is_err());
        assert_eq!(std::fs::read(&config.master_key_file).unwrap(), key_before);
    }

    #[tokio::test]
    async fn init_checks_everything_before_writing_the_key() {
        let directory = tempfile::tempdir().unwrap();
        let options = options_in(directory.path());
        std::fs::create_dir_all(&options.data_directory).unwrap();
        std::fs::write(
            options.data_directory.join("mellow.db"),
            b"another instance",
        )
        .unwrap();

        let result = initialize_instance(&options).await;

        assert!(result.is_err());
        assert!(
            !options.master_key_file.exists(),
            "no orphan key may be left behind"
        );
        assert!(!options.config_path.exists());
    }

    #[tokio::test]
    async fn serve_refuses_to_create_an_empty_instance() {
        let directory = tempfile::tempdir().unwrap();
        let config = initialize_instance(&options_in(directory.path()))
            .await
            .unwrap();
        std::fs::remove_file(config.database_path()).unwrap();

        let error = serve_instance(&config, async {}).await.unwrap_err();
        assert!(error.to_string().contains("no database"), "{error:#}");
        assert!(!config.database_path().exists());
    }

    #[tokio::test]
    async fn serve_starts_and_stops_cleanly_when_shutdown_is_requested() {
        let directory = tempfile::tempdir().unwrap();
        let config = initialize_instance(&options_in(directory.path()))
            .await
            .unwrap();
        serve_instance(&config, async {}).await.unwrap();
    }

    #[tokio::test]
    async fn served_instance_answers_health_over_tcp_and_stops_on_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let config = initialize_instance(&options_in(directory.path()))
            .await
            .unwrap();
        let key = MasterKey::read_file(&config.master_key_file).unwrap();
        let database = open_encrypted_database(&config.database_path(), &key.derive_database_key())
            .await
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop_sender, stop_receiver) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve_until_shutdown(
            listener,
            AppState::new(
                database.clone(),
                &config,
                crate::storage::attachment_files::AttachmentFiles::new(
                    config.attachment_directory(),
                    &key.derive_attachment_key(),
                )
                .unwrap(),
            ),
            async {
                let _ = stop_receiver.await;
            },
        ));

        let response = raw_http_get(address, "/api/v1/health").await;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains(r#""status":"ok""#), "{response}");

        stop_sender.send(()).unwrap();
        server.await.unwrap().unwrap();
        database.close().await;
    }

    /// Minimal HTTP/1.1 client, so the test needs no HTTP client dependency.
    async fn raw_http_get(address: SocketAddr, path: &str) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }

    fn new_user(username: &str, is_instance_administrator: bool) -> CreateUserOptions {
        CreateUserOptions {
            username: username.into(),
            display_name: None,
            email: format!("{username}@example.org"),
            password: zeroize::Zeroizing::new("correct horse battery".into()),
            is_instance_administrator,
        }
    }

    #[tokio::test]
    async fn create_user_works_with_registration_closed_and_rejects_duplicates() {
        let directory = tempfile::tempdir().unwrap();
        let mut options = options_in(directory.path());
        options.registration_mode = RegistrationMode::Closed;
        let config = initialize_instance(&options).await.unwrap();

        let admin = create_user(&config, new_user("admin", true)).await.unwrap();
        assert!(admin.is_instance_administrator);
        assert!(
            create_user(&config, new_user("admin", false))
                .await
                .is_err()
        );
        let printed = format!("{:?}", new_user("someone", false));
        assert!(!printed.contains("correct horse"), "{printed}");
    }

    #[tokio::test]
    async fn an_existing_account_is_made_administrator_and_back() {
        let directory = tempfile::tempdir().unwrap();
        let config = initialize_instance(&options_in(directory.path()))
            .await
            .unwrap();
        create_user(&config, new_user("paul", false)).await.unwrap();

        set_instance_administrator(&config, "Paul", true)
            .await
            .unwrap();
        let promoted = is_administrator(&config, "paul").await;
        set_instance_administrator(&config, "paul", false)
            .await
            .unwrap();
        let demoted = is_administrator(&config, "paul").await;
        let unknown = set_instance_administrator(&config, "nobody", true).await;

        assert!(promoted);
        assert!(!demoted);
        assert!(unknown.is_err());
    }

    #[tokio::test]
    async fn an_administrator_resets_two_factor_sign_in_and_the_reset_is_recorded() {
        let directory = tempfile::tempdir().unwrap();
        let config = initialize_instance(&options_in(directory.path()))
            .await
            .unwrap();
        let user = create_user(&config, new_user("paul", false)).await.unwrap();
        let key = MasterKey::read_file(&config.master_key_file)
            .unwrap()
            .derive_database_key();
        let database = open_encrypted_database(&config.database_path(), &key)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO two_factor_secrets (user_id, secret, enabled_at_unix_milliseconds, \
             created_at_unix_milliseconds) VALUES (?, zeroblob(20), 1, 1)",
        )
        .bind(i64::try_from(user.id.raw()).unwrap())
        .execute(database.writer())
        .await
        .unwrap();
        database.close().await;

        let first = reset_two_factor(&config, "Paul").await.unwrap();
        let second = reset_two_factor(&config, "paul").await.unwrap();
        let unknown = reset_two_factor(&config, "nobody").await;

        let database = open_encrypted_database(&config.database_path(), &key)
            .await
            .unwrap();
        let (secrets, resets): (i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM two_factor_secrets), \
             (SELECT COUNT(*) FROM two_factor_resets)",
        )
        .fetch_one(database.readers())
        .await
        .unwrap();
        // The trail names the account as stored, and outlives it.
        sqlx::query("DELETE FROM users")
            .execute(database.writer())
            .await
            .unwrap();
        let recorded: String = sqlx::query_scalar("SELECT username FROM two_factor_resets")
            .fetch_one(database.readers())
            .await
            .unwrap();
        database.close().await;
        assert_eq!(recorded, "paul");
        assert!(first);
        assert!(!second, "nothing left to reset");
        assert!(unknown.is_err());
        assert_eq!((secrets, resets), (0, 1));
    }

    async fn is_administrator(config: &ServerConfig, username: &str) -> bool {
        let key = MasterKey::read_file(&config.master_key_file)
            .unwrap()
            .derive_database_key();
        let database = open_encrypted_database(&config.database_path(), &key)
            .await
            .unwrap();
        let is_administrator: bool =
            sqlx::query_scalar("SELECT is_instance_administrator FROM users WHERE username = ?")
                .bind(username)
                .fetch_one(database.readers())
                .await
                .unwrap();
        database.close().await;
        is_administrator
    }

    #[tokio::test]
    async fn create_user_refuses_to_create_an_instance() {
        let directory = tempfile::tempdir().unwrap();
        let config = initialize_instance(&options_in(directory.path()))
            .await
            .unwrap();
        std::fs::remove_file(config.database_path()).unwrap();

        let error = create_user(&config, new_user("admin", true))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no database"), "{error:#}");
    }

    /// Gives `username` an avatar stored before ADR-0033, its file written only when
    /// `is_file_written`; answers the file's path.
    async fn give_legacy_avatar(
        config: &ServerConfig,
        username: &str,
        is_file_written: bool,
    ) -> PathBuf {
        let key = MasterKey::read_file(&config.master_key_file).unwrap();
        let files = crate::storage::attachment_files::AttachmentFiles::new(
            config.attachment_directory(),
            &key.derive_attachment_key(),
        )
        .unwrap();
        let id = crate::domain::snowflake_generator::SnowflakeGenerator::default()
            .next_id(crate::domain::clock::now_unix_milliseconds())
            .unwrap();
        if is_file_written {
            files.write(id, b"lossless webp".to_vec()).await.unwrap();
        }
        let database = open_encrypted_database(&config.database_path(), &key.derive_database_key())
            .await
            .unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "INSERT INTO profile_images (id, user_id, size_bytes, created_at_unix_milliseconds, \
             compression_version) SELECT {id}, id, 13, 0, 0 FROM users WHERE username = '{username}'; \
             UPDATE users SET avatar_id = {id} WHERE username = '{username}';"
        )))
        .execute(database.writer())
        .await
        .unwrap();
        database.close().await;
        files.path_of(id)
    }

    async fn is_recompression_requested(config: &ServerConfig) -> bool {
        let key = MasterKey::read_file(&config.master_key_file)
            .unwrap()
            .derive_database_key();
        let database = open_encrypted_database(&config.database_path(), &key)
            .await
            .unwrap();
        let is_requested =
            crate::storage::image_recompression::is_recompression_requested(&database)
                .await
                .unwrap();
        database.close().await;
        is_requested
    }

    #[tokio::test]
    async fn a_backup_copies_the_database_and_the_sealed_files_as_they_are() {
        let directory = tempfile::tempdir().unwrap();
        let config = initialize_instance(&options_in(directory.path()))
            .await
            .unwrap();
        create_user(&config, new_user("paul", true)).await.unwrap();
        let avatar_file = give_legacy_avatar(&config, "paul", true).await;
        let write_under_way = avatar_file.with_extension("partial");
        std::fs::write(&write_under_way, b"half a file").unwrap();
        let marker_of_finished_move = config
            .attachment_directory()
            .join(crate::storage::attachment_files::NAME_OF_MARKER_OF_FINISHED_MOVE);
        std::fs::write(&marker_of_finished_move, b"").unwrap();

        let backup = back_up_instance(&config, None).await.unwrap();

        let key = MasterKey::read_file(&config.master_key_file).unwrap();
        let copied_database = open_encrypted_database(
            &backup.join(crate::config::DATABASE_FILE_NAME),
            &key.derive_database_key(),
        )
        .await
        .unwrap();
        let copied_users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(copied_database.readers())
            .await
            .unwrap();
        copied_database.close().await;
        let copy_of = |file: &Path| {
            backup
                .join(crate::config::ATTACHMENT_DIRECTORY_NAME)
                .join(file.strip_prefix(config.attachment_directory()).unwrap())
        };
        let copied_header = std::fs::read(backup.join(crate::config::DATABASE_FILE_NAME)).unwrap();
        let entries_in_backup_directory = std::fs::read_dir(config.backup_directory())
            .unwrap()
            .count();

        assert_eq!(backup.parent(), Some(config.backup_directory().as_path()));
        assert_eq!(entries_in_backup_directory, 1, "only the backup itself");
        assert_eq!(copied_users, 1);
        assert!(
            !copied_header.starts_with(b"SQLite format 3"),
            "still encrypted"
        );
        assert_eq!(
            std::fs::read(copy_of(&avatar_file)).unwrap(),
            std::fs::read(&avatar_file).unwrap(),
            "copied sealed, as it is"
        );
        assert!(
            !copy_of(&write_under_way).exists(),
            "a write under way is not a file"
        );
        assert!(
            !copy_of(&marker_of_finished_move).exists(),
            "a restored backup checks its shards again"
        );
    }

    #[tokio::test]
    async fn a_backup_keeps_files_not_yet_moved_to_their_shard() {
        let directory = tempfile::tempdir().unwrap();
        let config = initialize_instance(&options_in(directory.path()))
            .await
            .unwrap();
        create_user(&config, new_user("paul", true)).await.unwrap();
        let avatar_file = give_legacy_avatar(&config, "paul", true).await;
        let id = mellow_protocol::Snowflake::from_raw(
            avatar_file
                .file_name()
                .and_then(|name| name.to_str()?.parse().ok())
                .unwrap(),
        );
        let key = MasterKey::read_file(&config.master_key_file).unwrap();
        let files = crate::storage::attachment_files::AttachmentFiles::new(
            config.attachment_directory(),
            &key.derive_attachment_key(),
        )
        .unwrap();
        let legacy_file = files.legacy_path_of(id);
        std::fs::create_dir_all(legacy_file.parent().unwrap()).unwrap();
        std::fs::rename(&avatar_file, &legacy_file).unwrap();

        let backup = back_up_instance(&config, None).await.unwrap();

        let copied_file = backup.join(crate::config::ATTACHMENT_DIRECTORY_NAME).join(
            legacy_file
                .strip_prefix(config.attachment_directory())
                .unwrap(),
        );
        assert_eq!(
            std::fs::read(copied_file).unwrap(),
            std::fs::read(&legacy_file).unwrap()
        );
    }

    #[tokio::test]
    async fn a_recompression_needs_a_backup_made_in_the_last_hour() {
        let directory = tempfile::tempdir().unwrap();
        let config = initialize_instance(&options_in(directory.path()))
            .await
            .unwrap();
        create_user(&config, new_user("paul", true)).await.unwrap();
        give_legacy_avatar(&config, "paul", true).await;

        let waiting = pictures_to_recompress(&config).await.unwrap();
        let refused_without_backup = request_image_recompression(&config).await;
        let is_requested_without_backup = is_recompression_requested(&config).await;
        back_up_instance(&config, None).await.unwrap();
        let requested = request_image_recompression(&config).await.unwrap();
        let is_requested = is_recompression_requested(&config).await;
        let key = MasterKey::read_file(&config.master_key_file)
            .unwrap()
            .derive_database_key();
        let database = open_encrypted_database(&config.database_path(), &key)
            .await
            .unwrap();
        sqlx::raw_sql(
            "DELETE FROM image_recompression_requests; \
             UPDATE completed_backups SET completed_at_unix_milliseconds = 0;",
        )
        .execute(database.writer())
        .await
        .unwrap();
        database.close().await;
        let refused_with_an_old_backup = request_image_recompression(&config).await;

        assert_eq!(waiting[0].name, "avatars and banners");
        assert_eq!((waiting[0].count, waiting[0].stored_bytes), (1, 13));
        assert!(
            refused_without_backup
                .unwrap_err()
                .to_string()
                .contains("mellow-server backup")
        );
        assert!(!is_requested_without_backup);
        assert_eq!(requested, 1);
        assert!(is_requested);
        assert!(refused_with_an_old_backup.is_err());
        assert!(!is_recompression_requested(&config).await);
    }

    #[tokio::test]
    async fn a_backup_that_took_hours_allows_a_recompression_once_it_is_complete() {
        const TWO_HOURS_IN_MILLISECONDS: u64 = 2 * 60 * 60 * 1_000;
        let directory = tempfile::tempdir().unwrap();
        let config = initialize_instance(&options_in(directory.path()))
            .await
            .unwrap();
        create_user(&config, new_user("paul", true)).await.unwrap();
        give_legacy_avatar(&config, "paul", true).await;
        let has_started = std::sync::atomic::AtomicBool::new(false);
        // The first reading names the backup: as if it had started two hours ago.
        let clock = || {
            let now = crate::domain::clock::now_unix_milliseconds();
            if has_started.swap(true, std::sync::atomic::Ordering::Relaxed) {
                now
            } else {
                now - TWO_HOURS_IN_MILLISECONDS
            }
        };

        back_up_instance_with_clock(&config, None, clock)
            .await
            .unwrap();
        let requested = request_image_recompression(&config).await;

        assert_eq!(requested.unwrap(), 1);
    }

    #[tokio::test]
    async fn a_backup_and_a_dry_run_refuse_a_database_they_would_migrate_and_leave_it_as_is() {
        let directory = tempfile::tempdir().unwrap();
        let config = initialize_instance(&options_in(directory.path()))
            .await
            .unwrap();
        let key = MasterKey::read_file(&config.master_key_file)
            .unwrap()
            .derive_database_key();
        let applied_migrations = |database: crate::storage::Database| async move {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
                .fetch_one(database.writer())
                .await
                .unwrap();
            database.close().await;
            count
        };
        // As if the binary had just been updated: its last migration is not applied yet.
        let database = open_encrypted_database(&config.database_path(), &key)
            .await
            .unwrap();
        sqlx::raw_sql(
            "DELETE FROM _sqlx_migrations \
             WHERE version = (SELECT MAX(version) FROM _sqlx_migrations)",
        )
        .execute(database.writer())
        .await
        .unwrap();
        let applied_before = applied_migrations(database).await;

        let backup = back_up_instance(&config, None).await;
        let dry_run = pictures_to_recompress(&config).await;
        let applied_after = applied_migrations(
            open_encrypted_database(&config.database_path(), &key)
                .await
                .unwrap(),
        )
        .await;

        for error in [backup.unwrap_err(), dry_run.unwrap_err()] {
            assert!(
                error.to_string().contains("restart the server"),
                "{error:#}"
            );
        }
        assert_eq!(applied_after, applied_before, "nothing was migrated");
        assert!(!config.backup_directory().exists(), "no backup was written");
    }

    #[tokio::test]
    async fn a_backup_missing_a_file_its_database_points_at_is_refused_and_left_nowhere() {
        let directory = tempfile::tempdir().unwrap();
        let config = initialize_instance(&options_in(directory.path()))
            .await
            .unwrap();
        create_user(&config, new_user("paul", true)).await.unwrap();
        // As if the file had been deleted between the database copy and the file copy.
        give_legacy_avatar(&config, "paul", false).await;
        let output = directory.path().join("elsewhere");

        let error = back_up_instance(&config, Some(&output)).await.unwrap_err();
        let left = std::fs::read_dir(&output).unwrap().count();

        assert!(
            error.to_string().contains("run the backup again"),
            "{error:#}"
        );
        assert_eq!(left, 0, "no partial backup is left");
    }
}
