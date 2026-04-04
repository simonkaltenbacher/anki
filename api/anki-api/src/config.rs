use std::env;
use std::net::IpAddr;
use std::path::PathBuf;

use serde::Deserialize;
use thiserror::Error;

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 50051;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerConnectionMode {
    Plaintext,
    Tls {
        tls: TlsTransportConfig,
        auth: TlsAuthMode,
    },
    Spiffe(SpiffeTransportConfig),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TlsAuthMode {
    Disabled,
    ApiKey(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsTransportConfig {
    pub cert_path: String,
    pub key_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpiffeTransportConfig {
    pub allowed_client_id: String,
    pub workload_api_socket: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct RuntimeOverrides {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub api_key: Option<String>,
    pub anki_version: Option<String>,
    pub auth_disabled: Option<bool>,
    pub allow_non_local: Option<bool>,
    pub transport_mode: Option<String>,
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
    pub spiffe_allowed_client_id: Option<String>,
    pub spiffe_workload_api_socket: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct FileConfig {
    pub enabled: Option<bool>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub api_key: Option<String>,
    pub anki_version: Option<String>,
    pub auth_disabled: Option<bool>,
    pub allow_non_local: Option<bool>,
    pub transport_mode: Option<String>,
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
    pub spiffe_allowed_client_id: Option<String>,
    pub spiffe_workload_api_socket: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub anki_version: Option<String>,
    pub allow_non_local: bool,
    pub connection_mode: ServerConnectionMode,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid ANKI_PUBLIC_API_PORT value: {0}")]
    InvalidPort(String),
    #[error("invalid {key} value: {value}")]
    InvalidBoolean { key: &'static str, value: String },
    #[error("{key} cannot be empty")]
    EmptyValue { key: &'static str },
    #[error("failed to read api config file {path}: {source}")]
    ConfigFileRead {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse api config file {path}: {source}")]
    ConfigFileParse {
        path: String,
        source: toml::de::Error,
    },
    #[error("api_key is required when auth is enabled")]
    MissingApiKey,
    #[error("api_key is only supported when transport_mode=tls")]
    ApiKeyRequiresTls,
    #[error("non-local bind requires allow_non_local=true")]
    NonLocalBindNotAllowed,
    #[error("invalid ANKI_PUBLIC_API_TRANSPORT_MODE value: {0}")]
    InvalidTransportMode(String),
    #[error("tls_cert_path is required when transport_mode=tls")]
    MissingTlsCertPath,
    #[error("tls_key_path is required when transport_mode=tls")]
    MissingTlsKeyPath,
    #[error("spiffe_allowed_client_id is required when transport_mode=spiffe")]
    MissingSpiffeAllowedClientId,
}

impl ServerConfig {
    pub fn resolve(runtime: RuntimeOverrides, file: FileConfig) -> Result<Self, ConfigError> {
        let host = pick_value(
            runtime.host,
            env::var("ANKI_PUBLIC_API_HOST").ok(),
            file.host,
            DEFAULT_HOST.to_owned(),
        );
        let port = pick_value(runtime.port, env_port()?, file.port, DEFAULT_PORT);
        let api_key = pick_value(runtime.api_key, env_api_key()?, file.api_key, String::new());
        let anki_version = pick_value(
            runtime.anki_version,
            env::var("ANKI_PUBLIC_API_ANKI_VERSION").ok(),
            file.anki_version,
            String::new(),
        );
        let auth_disabled = pick_value(
            runtime.auth_disabled,
            env_bool("ANKI_PUBLIC_API_AUTH_DISABLED")?,
            file.auth_disabled,
            false,
        );
        let allow_non_local = pick_value(
            runtime.allow_non_local,
            env_bool("ANKI_PUBLIC_API_ALLOW_NON_LOCAL")?,
            file.allow_non_local,
            false,
        );
        let transport_mode = pick_value(
            runtime.transport_mode,
            env_transport_mode()?,
            file.transport_mode,
            "plaintext".to_owned(),
        );
        let tls_cert_path = pick_value(
            runtime.tls_cert_path,
            env_string("ANKI_PUBLIC_API_TLS_CERT_PATH")?,
            file.tls_cert_path,
            String::new(),
        );
        let tls_key_path = pick_value(
            runtime.tls_key_path,
            env_string("ANKI_PUBLIC_API_TLS_KEY_PATH")?,
            file.tls_key_path,
            String::new(),
        );
        let spiffe_allowed_client_id = pick_value(
            runtime.spiffe_allowed_client_id,
            env_string("ANKI_PUBLIC_API_SPIFFE_ALLOWED_CLIENT_ID")?,
            file.spiffe_allowed_client_id,
            String::new(),
        );
        let spiffe_workload_api_socket = pick_value(
            runtime.spiffe_workload_api_socket,
            env_string("ANKI_PUBLIC_API_SPIFFE_WORKLOAD_API_SOCKET")?,
            file.spiffe_workload_api_socket,
            String::new(),
        );
        let api_key = non_empty(api_key);
        let anki_version = non_empty(anki_version);
        let tls_cert_path = non_empty(tls_cert_path);
        let tls_key_path = non_empty(tls_key_path);
        let spiffe_allowed_client_id = non_empty(spiffe_allowed_client_id);
        let spiffe_workload_api_socket = non_empty(spiffe_workload_api_socket);

        let config = Self {
            host,
            port,
            anki_version,
            allow_non_local,
            connection_mode: resolve_connection_mode(
                &transport_mode,
                api_key,
                auth_disabled,
                tls_cert_path,
                tls_key_path,
                spiffe_allowed_client_id,
                spiffe_workload_api_socket,
            )?,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if !self.allow_non_local && !is_local_host(&self.host) {
            return Err(ConfigError::NonLocalBindNotAllowed);
        }
        if matches!(self.connection_mode, ServerConnectionMode::Spiffe(_)) && !self.allow_non_local
        {
            tracing::debug!(
                "SPIFFE transport enabled without allow_non_local; server remains loopback-only"
            );
        }
        Ok(())
    }
}

impl FileConfig {
    pub fn load_default() -> Result<Self, ConfigError> {
        let Some(path) = default_config_path() else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }

        let content =
            std::fs::read_to_string(&path).map_err(|source| ConfigError::ConfigFileRead {
                path: path.display().to_string(),
                source,
            })?;
        let parsed: FileConfigDoc =
            toml::from_str(&content).map_err(|source| ConfigError::ConfigFileParse {
                path: path.display().to_string(),
                source,
            })?;
        Ok(parsed.anki_public_api.into())
    }

    pub fn has_runtime_fields_set(&self) -> bool {
        self.host.is_some()
            || self.port.is_some()
            || self.api_key.is_some()
            || self.anki_version.is_some()
            || self.auth_disabled.is_some()
            || self.allow_non_local.is_some()
            || self.transport_mode.is_some()
            || self.tls_cert_path.is_some()
            || self.tls_key_path.is_some()
            || self.spiffe_allowed_client_id.is_some()
            || self.spiffe_workload_api_socket.is_some()
    }
}

fn env_port() -> Result<Option<u16>, ConfigError> {
    match env::var("ANKI_PUBLIC_API_PORT") {
        Ok(value) => value
            .parse::<u16>()
            .map(Some)
            .map_err(|_| ConfigError::InvalidPort(value)),
        Err(_) => Ok(None),
    }
}

fn env_bool(key: &'static str) -> Result<Option<bool>, ConfigError> {
    match env::var(key) {
        Ok(value) => match value.as_str() {
            "1" | "true" | "TRUE" | "True" => Ok(Some(true)),
            "0" | "false" | "FALSE" | "False" => Ok(Some(false)),
            _ => Err(ConfigError::InvalidBoolean { key, value }),
        },
        Err(_) => Ok(None),
    }
}

fn env_api_key() -> Result<Option<String>, ConfigError> {
    match env::var("ANKI_PUBLIC_API_KEY") {
        Ok(value) if value.is_empty() => Err(ConfigError::EmptyValue {
            key: "ANKI_PUBLIC_API_KEY",
        }),
        Ok(value) => Ok(Some(value)),
        Err(_) => Ok(None),
    }
}

fn env_string(key: &'static str) -> Result<Option<String>, ConfigError> {
    match env::var(key) {
        Ok(value) if value.is_empty() => Err(ConfigError::EmptyValue { key }),
        Ok(value) => Ok(Some(value)),
        Err(_) => Ok(None),
    }
}

fn env_transport_mode() -> Result<Option<String>, ConfigError> {
    match env::var("ANKI_PUBLIC_API_TRANSPORT_MODE") {
        Ok(value) if value.eq_ignore_ascii_case("plaintext") => Ok(Some("plaintext".to_owned())),
        Ok(value) if value.eq_ignore_ascii_case("tls") => Ok(Some("tls".to_owned())),
        Ok(value) if value.eq_ignore_ascii_case("spiffe") => Ok(Some("spiffe".to_owned())),
        Ok(value) => Err(ConfigError::InvalidTransportMode(value)),
        Err(_) => Ok(None),
    }
}

fn pick_value<T>(runtime: Option<T>, env: Option<T>, file: Option<T>, default: T) -> T {
    runtime.or(env).or(file).unwrap_or(default)
}

fn non_empty(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}

fn resolve_connection_mode(
    mode: &str,
    api_key: Option<String>,
    auth_disabled: bool,
    tls_cert_path: Option<String>,
    tls_key_path: Option<String>,
    spiffe_allowed_client_id: Option<String>,
    spiffe_workload_api_socket: Option<String>,
) -> Result<ServerConnectionMode, ConfigError> {
    match mode {
        "plaintext" => {
            if api_key.is_some() {
                return Err(ConfigError::ApiKeyRequiresTls);
            }
            Ok(ServerConnectionMode::Plaintext)
        }
        "tls" => Ok(ServerConnectionMode::Tls {
            tls: TlsTransportConfig {
                cert_path: tls_cert_path.ok_or(ConfigError::MissingTlsCertPath)?,
                key_path: tls_key_path.ok_or(ConfigError::MissingTlsKeyPath)?,
            },
            auth: if auth_disabled {
                TlsAuthMode::Disabled
            } else {
                TlsAuthMode::ApiKey(api_key.ok_or(ConfigError::MissingApiKey)?)
            },
        }),
        "spiffe" => {
            if api_key.is_some() {
                return Err(ConfigError::ApiKeyRequiresTls);
            }
            Ok(ServerConnectionMode::Spiffe(SpiffeTransportConfig {
                allowed_client_id: spiffe_allowed_client_id
                    .ok_or(ConfigError::MissingSpiffeAllowedClientId)?,
                workload_api_socket: spiffe_workload_api_socket,
            }))
        }
        other => Err(ConfigError::InvalidTransportMode(other.to_owned())),
    }
}

fn is_local_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    host.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn default_config_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Anki2/public-api.toml"))
}

#[cfg(target_os = "macos")]
fn default_config_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("Anki2/public-api.toml"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn default_config_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("Anki2/public-api.toml"))
}

#[derive(Debug, Default, Deserialize)]
struct FileConfigDoc {
    #[serde(default)]
    anki_public_api: FileConfigToml,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfigToml {
    enabled: Option<bool>,
    host: Option<String>,
    port: Option<u16>,
    api_key: Option<String>,
    anki_version: Option<String>,
    auth_disabled: Option<bool>,
    allow_non_local: Option<bool>,
    transport_mode: Option<String>,
    tls_cert_path: Option<String>,
    tls_key_path: Option<String>,
    spiffe_allowed_client_id: Option<String>,
    spiffe_workload_api_socket: Option<String>,
}

impl From<FileConfigToml> for FileConfig {
    fn from(value: FileConfigToml) -> Self {
        Self {
            enabled: value.enabled,
            host: value.host,
            port: value.port,
            api_key: value.api_key,
            anki_version: value.anki_version,
            auth_disabled: value.auth_disabled,
            allow_non_local: value.allow_non_local,
            transport_mode: value.transport_mode,
            tls_cert_path: value.tls_cert_path,
            tls_key_path: value.tls_key_path,
            spiffe_allowed_client_id: value.spiffe_allowed_client_id,
            spiffe_workload_api_socket: value.spiffe_workload_api_socket,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ConfigError;
    use super::FileConfig;
    use super::RuntimeOverrides;
    use super::ServerConfig;
    use super::ServerConnectionMode;
    use super::SpiffeTransportConfig;
    use super::TlsAuthMode;
    use super::TlsTransportConfig;

    #[test]
    fn resolve_defaults_to_plaintext_transport() {
        let config =
            ServerConfig::resolve(RuntimeOverrides::default(), FileConfig::default()).unwrap();

        assert_eq!(config.connection_mode, ServerConnectionMode::Plaintext);
    }

    #[test]
    fn resolve_rejects_api_key_for_plaintext_transport() {
        let err = ServerConfig::resolve(
            RuntimeOverrides::default(),
            FileConfig {
                api_key: Some("test-key".to_owned()),
                ..FileConfig::default()
            },
        )
        .unwrap_err();

        assert!(matches!(err, ConfigError::ApiKeyRequiresTls));
    }

    #[test]
    fn resolve_supports_tls_transport() {
        let config = ServerConfig::resolve(
            RuntimeOverrides {
                transport_mode: Some("tls".to_owned()),
                tls_cert_path: Some("/tmp/server.pem".to_owned()),
                tls_key_path: Some("/tmp/server.key".to_owned()),
                api_key: Some("test-key".to_owned()),
                ..RuntimeOverrides::default()
            },
            FileConfig::default(),
        )
        .unwrap();

        assert_eq!(
            config.connection_mode,
            ServerConnectionMode::Tls {
                tls: TlsTransportConfig {
                    cert_path: "/tmp/server.pem".to_owned(),
                    key_path: "/tmp/server.key".to_owned(),
                },
                auth: TlsAuthMode::ApiKey("test-key".to_owned()),
            }
        );
    }

    #[test]
    fn resolve_supports_spiffe_transport() {
        let config = ServerConfig::resolve(
            RuntimeOverrides {
                transport_mode: Some("spiffe".to_owned()),
                spiffe_allowed_client_id: Some("spiffe://example.org/anki-edit".to_owned()),
                ..RuntimeOverrides::default()
            },
            FileConfig::default(),
        )
        .unwrap();

        assert_eq!(
            config.connection_mode,
            ServerConnectionMode::Spiffe(SpiffeTransportConfig {
                allowed_client_id: "spiffe://example.org/anki-edit".to_owned(),
                workload_api_socket: None,
            })
        );
    }

    #[test]
    fn resolve_rejects_tls_without_api_key() {
        let err = ServerConfig::resolve(
            RuntimeOverrides {
                transport_mode: Some("tls".to_owned()),
                tls_cert_path: Some("/tmp/server.pem".to_owned()),
                tls_key_path: Some("/tmp/server.key".to_owned()),
                ..RuntimeOverrides::default()
            },
            FileConfig::default(),
        )
        .unwrap_err();

        assert!(matches!(err, ConfigError::MissingApiKey));
    }

    #[test]
    fn resolve_requires_allowed_client_id_for_spiffe_transport() {
        let err = ServerConfig::resolve(
            RuntimeOverrides {
                transport_mode: Some("spiffe".to_owned()),
                ..RuntimeOverrides::default()
            },
            FileConfig::default(),
        )
        .unwrap_err();

        assert!(matches!(err, ConfigError::MissingSpiffeAllowedClientId));
    }

    #[test]
    fn resolve_rejects_invalid_transport_mode() {
        let err = ServerConfig::resolve(
            RuntimeOverrides {
                transport_mode: Some("bogus".to_owned()),
                ..RuntimeOverrides::default()
            },
            FileConfig::default(),
        )
        .unwrap_err();

        assert!(matches!(err, ConfigError::InvalidTransportMode(mode) if mode == "bogus"));
    }

    #[test]
    fn resolve_rejects_api_key_for_spiffe_transport() {
        let err = ServerConfig::resolve(
            RuntimeOverrides {
                transport_mode: Some("spiffe".to_owned()),
                spiffe_allowed_client_id: Some("spiffe://example.org/anki-edit".to_owned()),
                api_key: Some("test-key".to_owned()),
                ..RuntimeOverrides::default()
            },
            FileConfig::default(),
        )
        .unwrap_err();

        assert!(matches!(err, ConfigError::ApiKeyRequiresTls));
    }

    #[test]
    fn resolve_rejects_tls_without_cert_path() {
        let err = ServerConfig::resolve(
            RuntimeOverrides {
                transport_mode: Some("tls".to_owned()),
                tls_key_path: Some("/tmp/server.key".to_owned()),
                api_key: Some("test-key".to_owned()),
                ..RuntimeOverrides::default()
            },
            FileConfig::default(),
        )
        .unwrap_err();

        assert!(matches!(err, ConfigError::MissingTlsCertPath));
    }

    #[test]
    fn resolve_rejects_tls_without_key_path() {
        let err = ServerConfig::resolve(
            RuntimeOverrides {
                transport_mode: Some("tls".to_owned()),
                tls_cert_path: Some("/tmp/server.pem".to_owned()),
                api_key: Some("test-key".to_owned()),
                ..RuntimeOverrides::default()
            },
            FileConfig::default(),
        )
        .unwrap_err();

        assert!(matches!(err, ConfigError::MissingTlsKeyPath));
    }
}
