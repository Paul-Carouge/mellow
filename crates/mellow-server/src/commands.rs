//! `mellow-server init` and `mellow-server serve` (ARCHITECTURE.md §4.10).

use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use tokio::net::TcpListener;

use crate::config::{ServerConfig, TlsConfig};
use crate::domain::account_rules::RegistrationMode;
use crate::domain::accounts::purge_expired_sessions_periodically;
use crate::http::{AppState, make_service};
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
        behind_tls_reverse_proxy: false,
        trusted_proxy_networks: Vec::new(),
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
    let pool = open_encrypted_database(&config.database_path(), &database_key).await?;
    let migration = migrate_with_backup(&pool, &config.backup_directory()).await;
    pool.close().await;
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
    // The master key is dropped (and wiped) as soon as the database key is derived.
    let database_key = MasterKey::read_file(&config.master_key_file)?.derive_database_key();
    // Bound before migrating: a second `serve` started by mistake fails here instead of
    // migrating the database under the running instance.
    let listener = TcpListener::bind(config.listen_address)
        .await
        .with_context(|| format!("cannot listen on {}", config.listen_address))?;
    let pool = open_encrypted_database(&database_path, &database_key).await?;

    let result = async {
        let migration = migrate_with_backup(&pool, &config.backup_directory()).await?;
        if !migration.applied_versions.is_empty() {
            tracing::info!(
                applied_versions = ?migration.applied_versions,
                backup_path = ?migration.backup_path,
                "database migrated"
            );
        }
        let state = AppState::new(pool.clone(), config);
        let session_purge = tokio::spawn(purge_expired_sessions_periodically(pool.clone()));
        let served = match &config.tls {
            Some(tls) => crate::tls::serve_tls_until_shutdown(listener, state, tls, shutdown).await,
            None => serve_until_shutdown(listener, state, shutdown).await,
        };
        // Awaited, so the purge holds no connection when the pool closes.
        session_purge.abort();
        let _cancelled = session_purge.await;
        served
    }
    .await;

    // SQLCipher crashes if a connection is still closing when the process exits (ADR-0006).
    pool.close().await;
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
    let database_path = config.database_path();
    if !database_path.is_file() {
        bail!(
            "no database at {}; run `mellow-server init` first",
            database_path.display()
        );
    }
    let database_key = MasterKey::read_file(&config.master_key_file)?.derive_database_key();
    let pool = open_encrypted_database(&database_path, &database_key).await?;
    let result = async {
        migrate_with_backup(&pool, &config.backup_directory()).await?;
        Ok(crate::domain::accounts::Accounts::new(
            pool.clone(),
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
    pool.close().await;
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
    axum::serve(listener, make_service(state))
        .with_graceful_shutdown(shutdown)
        .await
        .context("HTTP server stopped with an error")
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
        let pool = open_encrypted_database(&config.database_path(), &key.derive_database_key())
            .await
            .unwrap();
        let has_users_table: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE name = 'users')")
                .fetch_one(&pool)
                .await
                .unwrap();
        pool.close().await;
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
        let pool = open_encrypted_database(&config.database_path(), &key.derive_database_key())
            .await
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop_sender, stop_receiver) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve_until_shutdown(
            listener,
            AppState::new(pool.clone(), &config),
            async {
                let _ = stop_receiver.await;
            },
        ));

        let response = raw_http_get(address, "/api/v1/health").await;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains(r#""status":"ok""#), "{response}");

        stop_sender.send(()).unwrap();
        server.await.unwrap().unwrap();
        pool.close().await;
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
}
