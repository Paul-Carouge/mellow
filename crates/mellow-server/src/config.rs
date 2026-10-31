//! `mellow.toml` plus `MELLOW_*` environment overrides (ARCHITECTURE.md §4.10). Validated at
//! startup so a typo fails fast with a precise message instead of misbehaving later.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use ipnet::IpNet;
use serde::{Deserialize, Serialize};

use crate::domain::account_rules::GuildCreationPolicy;
use crate::domain::account_rules::RegistrationMode;

pub const DATA_DIRECTORY_VARIABLE: &str = "MELLOW_DATA_DIRECTORY";
pub const MASTER_KEY_FILE_VARIABLE: &str = "MELLOW_MASTER_KEY_FILE";
pub const LISTEN_ADDRESS_VARIABLE: &str = "MELLOW_LISTEN_ADDRESS";
/// Loopback by default: without `[tls]`, the server must sit behind a TLS reverse proxy.
pub const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:8080";
/// TLS-ALPN-01 challenges arrive on 443, so a TLS instance listens there by default (ADR-0013).
pub const DEFAULT_TLS_LISTEN_ADDRESS: &str = "[::]:443";
/// Created next to the master key file by `init --domain`.
pub const DEFAULT_CERTIFICATE_DIRECTORY_NAME: &str = "certificates";
/// Let's Encrypt's limit for names on one certificate.
const MAXIMUM_TLS_DOMAINS: usize = 100;
const MAXIMUM_DOMAIN_LENGTH: usize = 253;
const MAXIMUM_DOMAIN_LABEL_LENGTH: usize = 63;
/// A proxy speaks plain http to Mellow, so it must sit on a private network: loopback, RFC 1918,
/// shared address space (CGNAT, Tailscale), link-local, and their IPv6 counterparts.
const PRIVATE_PROXY_NETWORKS: [&str; 9] = [
    "127.0.0.0/8",
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "100.64.0.0/10",
    "169.254.0.0/16",
    "::1/128",
    "fc00::/7",
    "fe80::/10",
];
pub const DATABASE_FILE_NAME: &str = "mellow.db";
pub const BACKUP_DIRECTORY_NAME: &str = "backups";
const ATTACHMENT_DIRECTORY_NAME: &str = "attachments";
/// 25 MB, as Discord's default upload limit.
pub const DEFAULT_MAXIMUM_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;

fn default_maximum_attachment_bytes() -> u64 {
    DEFAULT_MAXIMUM_ATTACHMENT_BYTES
}

/// 2 GiB per account, so open registration cannot fill a small server's disk in one evening.
pub const DEFAULT_MAXIMUM_ATTACHMENT_STORAGE_BYTES_PER_USER: u64 = 2 * 1024 * 1024 * 1024;

fn default_maximum_attachment_storage_bytes_per_user() -> u64 {
    DEFAULT_MAXIMUM_ATTACHMENT_STORAGE_BYTES_PER_USER
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub data_directory: PathBuf,
    /// Kept outside the data directory so a copied data directory or backup reveals nothing (ADR-0006).
    pub master_key_file: PathBuf,
    pub listen_address: SocketAddr,
    #[serde(default)]
    pub registration_mode: RegistrationMode,
    /// Who may create servers: `"administrators"` (default) or `"everyone"`.
    #[serde(default)]
    pub guild_creation: GuildCreationPolicy,
    /// Set when a TLS reverse proxy on this host forwards to Mellow. Without `[tls]`, required to
    /// listen on a non-loopback address (tokens would otherwise travel in clear), and makes the
    /// client address come from the last `X-Forwarded-For` hop.
    #[serde(default)]
    pub behind_tls_reverse_proxy: bool,
    /// Proxies trusted for `X-Forwarded-For` besides loopback, e.g. `["172.18.0.0/16"]` for a
    /// proxy in a Docker network. Requires `behind_tls_reverse_proxy`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_proxy_networks: Vec<IpNet>,
    /// Largest file accepted as an attachment (ADR-0020).
    #[serde(default = "default_maximum_attachment_bytes")]
    pub maximum_attachment_bytes: u64,
    /// What all of one account's uploads may take on disk (ADR-0020).
    #[serde(default = "default_maximum_attachment_storage_bytes_per_user")]
    pub maximum_attachment_storage_bytes_per_user: u64,
    /// Voice channels (ADR-0030); on by default, so the table is usually absent. Tables come after
    /// values in TOML.
    #[serde(default, skip_serializing_if = "VoiceConfig::is_default")]
    pub voice: VoiceConfig,
    /// Built-in TLS with ACME certificates (ADR-0013). Last field: TOML writes tables after values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsConfig>,
}

/// UDP port of the voice media, which the router must forward and the firewall must open.
pub const DEFAULT_VOICE_UDP_PORT: u16 = 50_000;
/// People connected to voice across the instance; one media thread sustains about this many
/// (ADR-0030 measurements).
pub const DEFAULT_MAXIMUM_VOICE_PARTICIPANTS: u32 = 50;

/// The `[voice]` table of `mellow.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct VoiceConfig {
    pub enabled: bool,
    /// One UDP port for all voice traffic; 0 picks a free port (tests).
    pub udp_port: u16,
    /// The IPv4 address or DNS name clients send voice to. When absent, the name the client
    /// reached the instance at (its `Host` header) is used, which suits a box forwarding the port.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_address: Option<String>,
    pub max_participants: u32,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            udp_port: DEFAULT_VOICE_UDP_PORT,
            public_address: None,
            max_participants: DEFAULT_MAXIMUM_VOICE_PARTICIPANTS,
        }
    }
}

impl VoiceConfig {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if let Some(address) = &self.public_address
            && !crate::voice::is_valid_host(address)
        {
            return Err(ConfigError::InvalidVoicePublicAddress {
                address: address.clone(),
            });
        }
        if self.max_participants == 0 {
            return Err(ConfigError::NoVoiceParticipants);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsConfig {
    /// Public names of the instance, e.g. `["chat.example.org"]`; DNS must point them here.
    pub domains: Vec<String>,
    /// Given to the certificate authority for account problems; optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_email: Option<String>,
    /// Holds the ACME account and certificate private keys: outside the data directory, so
    /// backups do not carry them, and private to the service user.
    pub certificate_directory: PathBuf,
    /// Let's Encrypt staging: untrusted certificates, generous rate limits, for trying a setup.
    #[serde(default)]
    pub use_staging_certificates: bool,
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
        "listening on {address} would send session tokens unencrypted; add a [tls] table (`init --domain`), listen on 127.0.0.1, or set behind_tls_reverse_proxy = true once a TLS reverse proxy fronts Mellow"
    )]
    PlainHttpOnPublicAddress { address: SocketAddr },
    #[error(
        "[tls] and behind_tls_reverse_proxy cannot both be set: either Mellow terminates TLS or the proxy does"
    )]
    TlsAndReverseProxy,
    #[error(
        "trusted_proxy_networks needs behind_tls_reverse_proxy = true: without a proxy, nobody's X-Forwarded-For is trusted"
    )]
    TrustedProxyNetworksWithoutReverseProxy,
    #[error(
        "trusted_proxy_networks entry {network} is not inside a private network, so clients could pick their own rate-limit key; list only the proxy's address, e.g. 172.18.0.10/32"
    )]
    TrustedProxyNetworkNotPrivate { network: IpNet },
    #[error(
        "trusted_proxy_networks entry {network} has host bits set and would trust all of {}; write {}/32 (or /128) for one proxy",
        network.trunc(),
        network.addr()
    )]
    TrustedProxyNetworkWithHostBits { network: IpNet },
    #[error("[tls] needs between 1 and {MAXIMUM_TLS_DOMAINS} domains")]
    InvalidTlsDomainCount,
    #[error(
        "{domain:?} is not a domain name Mellow can get a certificate for (lowercase letters, digits, hyphens and dots; no wildcard, no IP address)"
    )]
    InvalidTlsDomain { domain: String },
    #[error("{email:?} is not a valid contact email")]
    InvalidContactEmail { email: String },
    #[error(
        "the certificate directory must not be inside the data directory ({path}), or backups would carry the TLS private keys"
    )]
    CertificateDirectoryInsideDataDirectory { path: PathBuf },
    #[error(
        "the master key file must not be inside the data directory ({path}), or backups would carry the key"
    )]
    MasterKeyInsideDataDirectory { path: PathBuf },
    #[error(
        "[voice] public_address {address:?} must be an IPv4 address or a DNS name, without port"
    )]
    InvalidVoicePublicAddress { address: String },
    #[error("[voice] max_participants must be at least 1; set enabled = false to turn voice off")]
    NoVoiceParticipants,
}

/// Peers are compared in canonical form, so an IPv4-mapped IPv6 network would never match: it is
/// refused as not private, like any network outside `PRIVATE_PROXY_NETWORKS`.
fn validate_trusted_proxy_network(network: IpNet) -> Result<(), ConfigError> {
    if network != network.trunc() {
        return Err(ConfigError::TrustedProxyNetworkWithHostBits { network });
    }
    let is_private = PRIVATE_PROXY_NETWORKS
        .iter()
        .filter_map(|private_network| private_network.parse::<IpNet>().ok())
        .any(|private_network| private_network.contains(&network));
    if !is_private {
        return Err(ConfigError::TrustedProxyNetworkNotPrivate { network });
    }
    Ok(())
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
        let certificate_directory = self.tls.as_ref().map(|tls| &tls.certificate_directory);
        for (field, path) in [
            ("data_directory", Some(&self.data_directory)),
            ("master_key_file", Some(&self.master_key_file)),
            ("tls.certificate_directory", certificate_directory),
        ]
        .into_iter()
        .filter_map(|(field, path)| Some((field, path?)))
        {
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
        self.voice.validate()?;
        if !self.trusted_proxy_networks.is_empty() && !self.behind_tls_reverse_proxy {
            return Err(ConfigError::TrustedProxyNetworksWithoutReverseProxy);
        }
        for network in &self.trusted_proxy_networks {
            validate_trusted_proxy_network(*network)?;
        }
        if let Some(tls) = &self.tls {
            if self.behind_tls_reverse_proxy {
                return Err(ConfigError::TlsAndReverseProxy);
            }
            tls.validate()?;
            if tls.certificate_directory.starts_with(&self.data_directory) {
                return Err(ConfigError::CertificateDirectoryInsideDataDirectory {
                    path: tls.certificate_directory.clone(),
                });
            }
        } else if !self.listen_address.ip().is_loopback() && !self.behind_tls_reverse_proxy {
            return Err(ConfigError::PlainHttpOnPublicAddress {
                address: self.listen_address,
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

    /// Plain HTTP on loopback without a reverse proxy: only reachable from this machine, so a
    /// development instance.
    #[must_use]
    pub fn is_development_listener(&self) -> bool {
        self.listen_address.ip().is_loopback()
            && self.tls.is_none()
            && !self.behind_tls_reverse_proxy
    }

    #[must_use]
    pub fn database_path(&self) -> PathBuf {
        self.data_directory.join(DATABASE_FILE_NAME)
    }

    #[must_use]
    pub fn backup_directory(&self) -> PathBuf {
        self.data_directory.join(BACKUP_DIRECTORY_NAME)
    }

    #[must_use]
    pub fn attachment_directory(&self) -> PathBuf {
        self.data_directory.join(ATTACHMENT_DIRECTORY_NAME)
    }

    /// # Errors
    /// Fails on non-UTF-8 paths, which `validate` rejects beforehand.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(self)
    }
}

impl TlsConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        if self.domains.is_empty() || self.domains.len() > MAXIMUM_TLS_DOMAINS {
            return Err(ConfigError::InvalidTlsDomainCount);
        }
        if let Some(domain) = self
            .domains
            .iter()
            .find(|domain| !is_certifiable_domain(domain))
        {
            return Err(ConfigError::InvalidTlsDomain {
                domain: domain.clone(),
            });
        }
        if let Some(email) = &self.contact_email
            && !is_plausible_email(email)
        {
            return Err(ConfigError::InvalidContactEmail {
                email: email.clone(),
            });
        }
        Ok(())
    }
}

/// A public DNS name with at least two labels, as certificate authorities accept them over
/// TLS-ALPN-01: no wildcard, no IP address, no uppercase (names are compared as written).
fn is_certifiable_domain(domain: &str) -> bool {
    let labels: Vec<&str> = domain.split('.').collect();
    let is_valid_label = |label: &&str| {
        (1..=MAXIMUM_DOMAIN_LABEL_LENGTH).contains(&label.len())
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    let is_all_numeric_top_level = labels
        .last()
        .is_some_and(|label| label.bytes().all(|byte| byte.is_ascii_digit()));
    domain.len() <= MAXIMUM_DOMAIN_LENGTH
        && labels.len() >= 2
        && labels.iter().all(is_valid_label)
        && !is_all_numeric_top_level
}

/// Only rules out obvious mistakes; the certificate authority does the real check.
fn is_plausible_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !email
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        && !domain.contains('@')
}

// The fixtures use Unix absolute paths, and the server refuses Windows for now (ADR-0006).
#[cfg(test)]
#[cfg(unix)]
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
    fn voice_is_on_by_default_and_its_table_is_checked() {
        let config = load_text(VALID_CONFIG, &EnvironmentOverrides::default()).unwrap();
        assert_eq!(config.voice, VoiceConfig::default());
        assert!(config.voice.enabled);
        assert_eq!(config.voice.udp_port, 50_000);
        assert!(
            !config.to_toml().unwrap().contains("[voice]"),
            "defaults stay implicit"
        );

        let custom = load_text(
            &format!(
                "{VALID_CONFIG}\n[voice]\nudp_port = 40000\npublic_address = \"chat.example.org\"\n"
            ),
            &EnvironmentOverrides::default(),
        )
        .unwrap();
        assert_eq!(
            custom.voice,
            VoiceConfig {
                udp_port: 40_000,
                public_address: Some("chat.example.org".into()),
                ..VoiceConfig::default()
            }
        );
        let reparsed: ServerConfig = toml::from_str(&custom.to_toml().unwrap()).unwrap();
        assert_eq!(reparsed, custom);
        for (table, expected) in [
            ("public_address = \"192.0.2.1:50000\"", "public_address"),
            ("max_participants = 0", "max_participants"),
            ("udp_port = 70000", "udp_port"),
        ] {
            let error = load_text(
                &format!("{VALID_CONFIG}\n[voice]\n{table}\n"),
                &EnvironmentOverrides::default(),
            )
            .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[test]
    fn only_a_plain_loopback_listener_without_proxy_counts_as_development() {
        let development = load_text(VALID_CONFIG, &EnvironmentOverrides::default()).unwrap();
        assert!(development.is_development_listener());
        let proxied = load_text(
            &format!("behind_tls_reverse_proxy = true\n{VALID_CONFIG}"),
            &EnvironmentOverrides::default(),
        )
        .unwrap();
        assert!(!proxied.is_development_listener());
        let exposed = ServerConfig {
            listen_address: "0.0.0.0:8080".parse().unwrap(),
            behind_tls_reverse_proxy: true,
            ..development
        };
        assert!(!exposed.is_development_listener());
    }

    #[test]
    fn environment_overrides_win_over_the_file() {
        let overrides = EnvironmentOverrides {
            data_directory: Some("/srv/mellow".into()),
            master_key_file: None,
            listen_address: Some("127.0.0.2:9000".into()),
        };
        let config = load_text(VALID_CONFIG, &overrides).unwrap();
        assert_eq!(config.data_directory, PathBuf::from("/srv/mellow"));
        assert_eq!(config.listen_address, "127.0.0.2:9000".parse().unwrap());
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
            registration_mode: RegistrationMode::Closed,
            guild_creation: GuildCreationPolicy::Administrators,
            behind_tls_reverse_proxy: false,
            trusted_proxy_networks: Vec::new(),
            maximum_attachment_bytes: DEFAULT_MAXIMUM_ATTACHMENT_BYTES,
            maximum_attachment_storage_bytes_per_user:
                DEFAULT_MAXIMUM_ATTACHMENT_STORAGE_BYTES_PER_USER,
            voice: VoiceConfig::default(),
            tls: None,
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
    fn trusted_proxy_networks_are_parsed_and_need_a_reverse_proxy() {
        let networks = "trusted_proxy_networks = [\"172.18.0.0/16\", \"fd00::/8\"]";
        assert!(matches!(
            load_text(
                &format!("{networks}\n{VALID_CONFIG}"),
                &EnvironmentOverrides::default()
            ),
            Err(ConfigError::TrustedProxyNetworksWithoutReverseProxy)
        ));
        let proxied = load_text(
            &format!("{networks}\nbehind_tls_reverse_proxy = true\n{VALID_CONFIG}"),
            &EnvironmentOverrides::default(),
        )
        .unwrap();
        assert_eq!(
            proxied.trusted_proxy_networks,
            vec![
                "172.18.0.0/16".parse::<IpNet>().unwrap(),
                "fd00::/8".parse().unwrap()
            ]
        );
        assert!(matches!(
            load_text(
                &format!(
                    "trusted_proxy_networks = [\"172.18.0.0/33\"]\nbehind_tls_reverse_proxy = true\n{VALID_CONFIG}"
                ),
                &EnvironmentOverrides::default()
            ),
            Err(ConfigError::InvalidFile { .. })
        ));
        let load_network = |network: &str| {
            load_text(
                &format!(
                    "trusted_proxy_networks = [\"{network}\"]\nbehind_tls_reverse_proxy = true\n{VALID_CONFIG}"
                ),
                &EnvironmentOverrides::default(),
            )
        };
        for public_network in [
            "0.0.0.0/0",
            "::/0",
            "0.0.0.0/1",
            "8.8.8.0/24",
            "172.0.0.0/8",
            "::ffff:172.18.0.0/112",
        ] {
            assert!(
                matches!(
                    load_network(public_network),
                    Err(ConfigError::TrustedProxyNetworkNotPrivate { .. })
                ),
                "{public_network}"
            );
        }
        assert!(matches!(
            load_network("172.18.0.4/16"),
            Err(ConfigError::TrustedProxyNetworkWithHostBits { .. })
        ));
        for private_network in ["172.18.0.4/32", "10.0.0.0/8", "100.100.1.2/32", "fd12::/64"] {
            assert!(load_network(private_network).is_ok(), "{private_network}");
        }
    }

    #[test]
    fn public_addresses_need_a_tls_reverse_proxy_and_registration_defaults_to_closed() {
        let public = VALID_CONFIG.replace("127.0.0.1:8080", "0.0.0.0:8080");
        assert!(matches!(
            load_text(&public, &EnvironmentOverrides::default()),
            Err(ConfigError::PlainHttpOnPublicAddress { .. })
        ));
        let proxied = load_text(
            &format!("{public}\nbehind_tls_reverse_proxy = true"),
            &EnvironmentOverrides::default(),
        )
        .unwrap();
        assert!(proxied.behind_tls_reverse_proxy);
        assert_eq!(proxied.registration_mode, RegistrationMode::Closed);
        let open = load_text(
            &format!("{VALID_CONFIG}\nregistration_mode = \"open\""),
            &EnvironmentOverrides::default(),
        )
        .unwrap();
        assert_eq!(open.registration_mode, RegistrationMode::Open);
    }

    #[test]
    fn only_administrators_create_servers_unless_the_config_says_everyone() {
        let default = load_text(VALID_CONFIG, &EnvironmentOverrides::default()).unwrap();
        assert_eq!(default.guild_creation, GuildCreationPolicy::Administrators);
        assert!(default.guild_creation.allows(true));
        assert!(!default.guild_creation.allows(false));
        let everyone = load_text(
            &format!("{VALID_CONFIG}\nguild_creation = \"everyone\""),
            &EnvironmentOverrides::default(),
        )
        .unwrap();
        assert!(everyone.guild_creation.allows(false));
        assert!(
            load_text(
                &format!("{VALID_CONFIG}\nguild_creation = \"owners\""),
                &EnvironmentOverrides::default(),
            )
            .is_err()
        );
    }

    #[test]
    fn master_key_inside_the_data_directory_is_rejected() {
        let text = VALID_CONFIG.replace("/etc/mellow/master.key", "/var/lib/mellow/master.key");
        assert!(matches!(
            load_text(&text, &EnvironmentOverrides::default()),
            Err(ConfigError::MasterKeyInsideDataDirectory { .. })
        ));
    }

    const TLS_TABLE: &str = r#"
        [tls]
        domains = ["chat.example.org"]
        contact_email = "admin@example.org"
        certificate_directory = "/etc/mellow/certificates"
    "#;

    fn public_tls_config() -> String {
        format!(
            "{}{TLS_TABLE}",
            VALID_CONFIG.replace("127.0.0.1:8080", "[::]:443")
        )
    }

    #[test]
    fn tls_allows_a_public_address_and_round_trips() {
        let config = load_text(&public_tls_config(), &EnvironmentOverrides::default()).unwrap();
        let tls = config.tls.clone().unwrap();
        assert_eq!(tls.domains, ["chat.example.org"]);
        assert!(!tls.use_staging_certificates);
        let reparsed: ServerConfig = toml::from_str(&config.to_toml().unwrap()).unwrap();
        assert_eq!(reparsed, config);
    }

    #[test]
    fn invalid_tls_settings_fail_with_a_precise_error() {
        let load = |text: &str| load_text(text, &EnvironmentOverrides::default());
        let config = public_tls_config();
        assert!(matches!(
            load(&format!(
                "{VALID_CONFIG}\nbehind_tls_reverse_proxy = true\n{TLS_TABLE}"
            )),
            Err(ConfigError::TlsAndReverseProxy)
        ));
        assert!(matches!(
            load(&config.replace(r#"["chat.example.org"]"#, "[]")),
            Err(ConfigError::InvalidTlsDomainCount)
        ));
        for domain in [
            "*.example.org",
            "Chat.example.org",
            "localhost",
            "192.168.1.10",
            "-a.example.org",
            "a..org",
        ] {
            assert!(
                matches!(
                    load(&config.replace("chat.example.org", domain)),
                    Err(ConfigError::InvalidTlsDomain { .. })
                ),
                "{domain}"
            );
        }
        assert!(matches!(
            load(&config.replace("admin@example.org", "admin example.org")),
            Err(ConfigError::InvalidContactEmail { .. })
        ));
        assert!(matches!(
            load(&config.replace("/etc/mellow/certificates", "/var/lib/mellow/acme")),
            Err(ConfigError::CertificateDirectoryInsideDataDirectory { .. })
        ));
        assert!(matches!(
            load(&config.replace("/etc/mellow/certificates", "certificates")),
            Err(ConfigError::RelativePath {
                field: "tls.certificate_directory",
                ..
            })
        ));
    }
}
