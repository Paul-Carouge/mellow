//! `mellow.toml` plus `MELLOW_*` environment overrides (ARCHITECTURE.md §4.10). Validated at
//! startup so a typo fails fast with a precise message instead of misbehaving later.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DATA_DIRECTORY_VARIABLE: &str = "MELLOW_DATA_DIRECTORY";
pub const MASTER_KEY_FILE_VARIABLE: &str = "MELLOW_MASTER_KEY_FILE";
pub const LISTEN_ADDRESS_VARIABLE: &str = "MELLOW_LISTEN_ADDRESS";
/// Loopback by default: until built-in TLS lands, the server must sit behind a TLS reverse proxy.
pub const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:8080";
pub const DATABASE_FILE_NAME: &str = "mellow.db";
pub const BACKUP_DIRECTORY_NAME: &str = "backups";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub data_directory: PathBuf,
    /// Kept outside the data directory so a copied data directory or backup reveals nothing (ADR-0006).
    pub master_key_file: PathBuf,
    pub listen_address: SocketAddr,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read the configuration file {path}")]
    CannotRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid configuration file {path}: {message}")]
    InvalidFile { path: PathBuf, message: String },
    #[error("{variable} is not a valid listen address (expected e.g. 127.0.0.1:8080): {value}")]
    InvalidListenAddressVariable {
        variable: &'static str,
        value: String,
    },
    #[error("{field} must be an absolute path, got {path}")]
    RelativePath { field: &'static str, path: PathBuf },
    #[error("{field} must not contain `..`, got {path}")]
    ParentDirectoryComponent { field: &'static str, path: PathBuf },
    #[error("{field} must be valid UTF-8, got {path}")]
    NonUtf8Path { field: &'static str, path: PathBuf },
    #[error("cannot resolve {path} on disk")]
    CannotResolve {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "the master key file must not be inside the data directory ({path}), or backups would carry the key"
    )]
    MasterKeyInsideDataDirectory { path: PathBuf },
}

/// The environment overrides that apply on top of the file, read once by the caller.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvironmentOverrides {
    pub data_directory: Option<String>,
    pub master_key_file: Option<String>,
    pub listen_address: Option<String>,
}

impl EnvironmentOverrides {
    #[must_use]
    pub fn from_process_environment() -> Self {
        let variable = |name| std::env::var(name).ok().filter(|value| !value.is_empty());
        Self {
            data_directory: variable(DATA_DIRECTORY_VARIABLE),
            master_key_file: variable(MASTER_KEY_FILE_VARIABLE),
            listen_address: variable(LISTEN_ADDRESS_VARIABLE),
        }
    }
}

impl ServerConfig {
    /// # Errors
    /// Fails if the file cannot be read, does not parse, or the result is invalid.
    pub fn load(path: &Path, overrides: &EnvironmentOverrides) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::CannotRead {
            path: path.to_path_buf(),
            source,
        })?;
        let mut config: Self = toml::from_str(&text).map_err(|error| ConfigError::InvalidFile {
            path: path.to_path_buf(),
            // `Display` includes line, column and the offending snippet.
            message: error.to_string(),
        })?;
        config.apply_overrides(overrides)?;
        config.validate()?;
        Ok(config)
    }

    fn apply_overrides(&mut self, overrides: &EnvironmentOverrides) -> Result<(), ConfigError> {
        if let Some(data_directory) = &overrides.data_directory {
            self.data_directory = PathBuf::from(data_directory);
        }
        if let Some(master_key_file) = &overrides.master_key_file {
            self.master_key_file = PathBuf::from(master_key_file);
        }
        if let Some(listen_address) = &overrides.listen_address {
            self.listen_address =
                listen_address
                    .parse()
                    .map_err(|_| ConfigError::InvalidListenAddressVariable {
                        variable: LISTEN_ADDRESS_VARIABLE,
                        value: listen_address.clone(),
                    })?;
        }
        Ok(())
    }

    /// # Errors
    /// Relative paths (they would depend on the service's working directory) and a master key
    /// stored inside the data directory are rejected.
    pub fn validate(&self) -> Result<(), ConfigError> {
        for (field, path) in [
            ("data_directory", &self.data_directory),
            ("master_key_file", &self.master_key_file),
        ] {
            if !path.is_absolute() {
                return Err(ConfigError::RelativePath {
                    field,
                    path: path.clone(),
                });
            }
            // `..` would let the lexical "key outside data" check below be bypassed.
            if path
                .components()
                .any(|component| component == std::path::Component::ParentDir)
            {
                return Err(ConfigError::ParentDirectoryComponent {
                    field,
                    path: path.clone(),
                });
            }
            if path.to_str().is_none() {
                return Err(ConfigError::NonUtf8Path {
                    field,
                    path: path.clone(),
                });
            }
        }
        if self.master_key_file.starts_with(&self.data_directory) {
            return Err(ConfigError::MasterKeyInsideDataDirectory {
                path: self.master_key_file.clone(),
            });
        }
        Ok(())
    }

    /// Same rule as `validate`, on resolved paths: symbolic links and case-insensitive file
    /// systems cannot place the key inside the data directory. Both must already exist on disk.
    ///
    /// # Errors
    /// Fails if either path cannot be resolved or the key is inside the data directory.
    pub fn ensure_master_key_outside_data_directory_on_disk(&self) -> Result<(), ConfigError> {
        let resolve = |path: &Path| {
            std::fs::canonicalize(path).map_err(|source| ConfigError::CannotResolve {
                path: path.to_path_buf(),
                source,
            })
        };
        let data_directory = resolve(&self.data_directory)?;
        let key_directory = resolve(self.master_key_file.parent().unwrap_or(Path::new("/")))?;
        if key_directory.starts_with(&data_directory) {
            return Err(ConfigError::MasterKeyInsideDataDirectory {
                path: self.master_key_file.clone(),
            });
        }
        Ok(())
    }

    #[must_use]
    pub fn database_path(&self) -> PathBuf {
        self.data_directory.join(DATABASE_FILE_NAME)
    }

    #[must_use]
    pub fn backup_directory(&self) -> PathBuf {
        self.data_directory.join(BACKUP_DIRECTORY_NAME)
    }

    /// # Errors
    /// Fails on non-UTF-8 paths, which `validate` rejects beforehand.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_CONFIG: &str = r#"
        data_directory = "/var/lib/mellow"
        master_key_file = "/etc/mellow/master.key"
        listen_address = "127.0.0.1:8080"
    "#;

    fn load_text(
        text: &str,
        overrides: &EnvironmentOverrides,
    ) -> Result<ServerConfig, ConfigError> {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mellow.toml");
        std::fs::write(&path, text).unwrap();
        ServerConfig::load(&path, overrides)
    }

    #[test]
    fn valid_file_loads_and_round_trips_through_toml() {
        let config = load_text(VALID_CONFIG, &EnvironmentOverrides::default()).unwrap();
        assert_eq!(
            config.database_path(),
            PathBuf::from("/var/lib/mellow/mellow.db")
        );
        let reparsed: ServerConfig = toml::from_str(&config.to_toml().unwrap()).unwrap();
        assert_eq!(reparsed, config);
    }

    #[test]
    fn environment_overrides_win_over_the_file() {
        let overrides = EnvironmentOverrides {
            data_directory: Some("/srv/mellow".into()),
            master_key_file: None,
            listen_address: Some("0.0.0.0:9000".into()),
        };
        let config = load_text(VALID_CONFIG, &overrides).unwrap();
        assert_eq!(config.data_directory, PathBuf::from("/srv/mellow"));
        assert_eq!(config.listen_address, "0.0.0.0:9000".parse().unwrap());
    }

    #[test]
    fn invalid_values_fail_with_a_precise_error() {
        let bad_address = EnvironmentOverrides {
            listen_address: Some("localhost".into()),
            ..EnvironmentOverrides::default()
        };
        assert!(matches!(
            load_text(VALID_CONFIG, &bad_address),
            Err(ConfigError::InvalidListenAddressVariable { .. })
        ));
        assert!(matches!(
            load_text(
                &VALID_CONFIG.replace("/var/lib/mellow", "data"),
                &EnvironmentOverrides::default()
            ),
            Err(ConfigError::RelativePath {
                field: "data_directory",
                ..
            })
        ));
        assert!(matches!(
            load_text(
                &format!("{VALID_CONFIG}\nunknown_key = 1"),
                &EnvironmentOverrides::default()
            ),
            Err(ConfigError::InvalidFile { .. })
        ));
    }

    #[test]
    fn parent_directory_components_are_rejected() {
        let text = VALID_CONFIG.replace(
            "/etc/mellow/master.key",
            "/var/lib/other/../mellow/master.key",
        );
        assert!(matches!(
            load_text(&text, &EnvironmentOverrides::default()),
            Err(ConfigError::ParentDirectoryComponent {
                field: "master_key_file",
                ..
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_cannot_place_the_key_inside_the_data_directory() {
        let directory = tempfile::tempdir().unwrap();
        let data_directory = directory.path().join("data");
        std::fs::create_dir(&data_directory).unwrap();
        let link_to_data = directory.path().join("looks-separate");
        std::os::unix::fs::symlink(&data_directory, &link_to_data).unwrap();
        let config = ServerConfig {
            data_directory,
            master_key_file: link_to_data.join("master.key"),
            listen_address: DEFAULT_LISTEN_ADDRESS.parse().unwrap(),
        };

        assert!(
            config.validate().is_ok(),
            "the lexical check cannot see the link"
        );
        assert!(matches!(
            config.ensure_master_key_outside_data_directory_on_disk(),
            Err(ConfigError::MasterKeyInsideDataDirectory { .. })
        ));
    }

    #[test]
    fn master_key_inside_the_data_directory_is_rejected() {
        let text = VALID_CONFIG.replace("/etc/mellow/master.key", "/var/lib/mellow/master.key");
        assert!(matches!(
            load_text(&text, &EnvironmentOverrides::default()),
            Err(ConfigError::MasterKeyInsideDataDirectory { .. })
        ));
    }
}
